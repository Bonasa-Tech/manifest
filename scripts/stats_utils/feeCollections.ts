import bs58 from 'bs58';
import type { Pool, PoolClient } from 'pg';

// Conservative start shared with the prepared DefiLlama adapter.
export const DEFAULT_FEE_START_TIMESTAMP =
  Date.parse('2026-10-26T00:00:00Z') / 1000;
export const FEE_COLLECTOR = 'B6dmr2UAn2wgjdm3T4N1Vjd8oPYRRTguByW7AEngkeL6';
const CORE = 'MNFSTqtC93rEfYHB6hF82sKdZpUDFWkViLByLd1k1Ms';
const WRAPPER = 'wMNFSTkir3HgyZTsB7uqu3i7FA73grFCptPXgrZjksL';
const SYSTEM = '11111111111111111111111111111111';
const COMPUTE = 'ComputeBudget111111111111111111111111111111';
export const COLLECTION_LABEL = 'Fee collections (including account resizing)';

type Instruction = { programIdIndex: number; accounts: number[]; data: string };
export type CollectionTransaction = {
  slot: number;
  blockTime: number | null;
  transaction: {
    message: {
      accountKeys: string[];
      header: { numRequiredSignatures: number };
      instructions: Instruction[];
    };
  };
  meta: null | {
    err: unknown;
    fee: number;
    preBalances: number[];
    postBalances: number[];
    loadedAddresses?: { writable: string[]; readonly: string[] };
    innerInstructions?: { index: number; instructions: Instruction[] }[] | null;
  };
};

function atoms(value: number): bigint {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new Error('Unsafe lamport amount from RPC');
  }
  return BigInt(value);
}

/** Collection transactions mutate lamports directly, so transfer-only indexing
 * misses them. Accept only known collection instructions; mixed transactions
 * require review rather than attributing unrelated wallet movements to fees. */
export function decodeCollection(tx: CollectionTransaction): null | {
  lamports: string;
  fundingLamports: string;
} {
  if (!tx.meta) throw new Error('Missing transaction metadata');
  if (tx.meta.err != null) return null;
  const { message } = tx.transaction;
  const keys = [
    ...message.accountKeys,
    ...(tx.meta.loadedAddresses?.writable ?? []),
    ...(tx.meta.loadedAddresses?.readonly ?? []),
  ];
  const collector = keys.indexOf(FEE_COLLECTOR);
  const isCollection = (ix: Instruction): boolean => {
    const tag = bs58.decode(ix.data)[0];
    return (
      (keys[ix.programIdIndex] === CORE &&
        [12, 14].includes(tag) &&
        keys[ix.accounts[0]] === FEE_COLLECTOR) ||
      (keys[ix.programIdIndex] === WRAPPER &&
        tag === 7 &&
        keys[ix.accounts[2]] === FEE_COLLECTOR)
    );
  };
  const inner =
    tx.meta.innerInstructions?.flatMap((group) => group.instructions) ?? [];
  if (![...message.instructions, ...inner].some(isCollection)) return null;
  if (!Array.isArray(tx.meta.innerInstructions)) {
    throw new Error('Missing inner instructions for collection');
  }
  if (collector < 0 || collector >= message.header.numRequiredSignatures) {
    throw new Error('Collection without collector signature');
  }
  for (const ix of message.instructions) {
    const program = keys[ix.programIdIndex];
    const tag = bs58.decode(ix.data)[0];
    const wrapperDefrag =
      program === WRAPPER &&
      tag === 8 &&
      keys[ix.accounts[1]] === FEE_COLLECTOR;
    if (!isCollection(ix) && !wrapperDefrag && program !== COMPUTE) {
      throw new Error('Mixed or unsupported collection transaction');
    }
  }
  // Defrag may fund market expansion before reclaiming rent. Add that funding
  // back to the wallet delta to measure gross receipts, not maintenance profit.
  let funding = 0n;
  for (const ix of inner) {
    if (
      keys[ix.programIdIndex] !== SYSTEM ||
      keys[ix.accounts[0]] !== FEE_COLLECTOR
    )
      continue;
    const data = Buffer.from(bs58.decode(ix.data));
    if (data.length !== 12 || data.readUInt32LE(0) !== 2) {
      throw new Error('Unsupported collector funding instruction');
    }
    funding += data.readBigUInt64LE(4);
  }
  const gas = collector === 0 ? atoms(tx.meta.fee) : 0n;
  const receipts =
    atoms(tx.meta.postBalances[collector]) -
    atoms(tx.meta.preBalances[collector]) +
    gas +
    funding;
  if (receipts < 0n) throw new Error('Unexplained collector outflow');
  return { lamports: receipts.toString(), fundingLamports: funding.toString() };
}

export type Rpc = <T>(method: string, params: unknown[]) => Promise<T>;
export function feeRpc(url: string): Rpc {
  return async <T>(method: string, params: unknown[]): Promise<T> => {
    const response = await fetch(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
      signal: AbortSignal.timeout(30_000),
    });
    if (!response.ok) throw new Error(`Fee RPC HTTP ${response.status}`);
    const body = await response.json();
    if (body.error) throw new Error(`Fee RPC ${method}: ${body.error.message}`);
    return body.result as T;
  };
}

type Signature = {
  signature: string;
  slot: number;
  blockTime: number | null;
  err: unknown;
};

export class FeeCollectionIndexer {
  private running = false;
  constructor(
    private pool: Pool,
    private rpc: Rpc,
    readonly startTimestamp: number,
  ) {
    if (!Number.isSafeInteger(startTimestamp) || startTimestamp <= 0)
      throw new Error('Invalid FEES_START_TIMESTAMP');
  }

  async initialize(): Promise<void> {
    await this.pool.query(`CREATE TABLE IF NOT EXISTS manifest_fee_collections (
      signature TEXT PRIMARY KEY, slot BIGINT NOT NULL, block_time BIGINT NOT NULL,
      lamports NUMERIC(30,0) NOT NULL, funding_lamports NUMERIC(30,0) NOT NULL
    );
    CREATE INDEX IF NOT EXISTS manifest_fee_collections_time ON manifest_fee_collections(block_time);
    CREATE TABLE IF NOT EXISTS manifest_fee_cursor (
      collector TEXT PRIMARY KEY, start_timestamp BIGINT NOT NULL,
      signature TEXT, indexed_through BIGINT NOT NULL DEFAULT 0
    )`);
    await this.pool.query(
      `INSERT INTO manifest_fee_cursor (collector,start_timestamp) VALUES ($1,$2)
      ON CONFLICT DO NOTHING`,
      [FEE_COLLECTOR, this.startTimestamp],
    );
  }

  async poll(): Promise<void> {
    if (this.running) return;
    this.running = true;
    let lock: PoolClient | undefined;
    let locked = false;
    try {
      lock = await this.pool.connect();
      locked = (
        await lock.query('SELECT pg_try_advisory_lock(62619402) AS locked')
      ).rows[0].locked;
      if (!locked) return;
      const state = (
        await lock.query(
          'SELECT * FROM manifest_fee_cursor WHERE collector=$1',
          [FEE_COLLECTOR],
        )
      ).rows[0];
      if (!state || Number(state.start_timestamp) !== this.startTimestamp)
        throw new Error('Fee start changed; explicit reindex required');
      // Snapshot the finalized horizon BEFORE querying signatures. Transactions
      // newer than it may also be stored, but do not advance coverage past it.
      const slot = await this.rpc<number>('getSlot', [
        { commitment: 'finalized' },
      ]);
      const horizon = await this.rpc<number | null>('getBlockTime', [slot]);
      if (horizon === null) throw new Error('Missing finalized block time');
      let before: string | undefined;
      let newest: string | undefined;
      let done = false;
      while (!done) {
        const page = await this.rpc<Signature[]>('getSignaturesForAddress', [
          FEE_COLLECTOR,
          {
            commitment: 'finalized',
            limit: 1000,
            ...(before ? { before } : {}),
          },
        ]);
        if (!page.length) {
          // An empty/pruned history cannot establish the initial lower bound or
          // prove we reached a previously committed cursor.
          throw new Error(
            'Fee history ended before cursor/start boundary; archival RPC required',
          );
        }
        newest ??= page[0].signature;
        for (const entry of page) {
          if (entry.signature === state.signature) {
            done = true;
            break;
          }
          if (entry.blockTime === null)
            throw new Error(`Missing block time: ${entry.signature}`);
          if (entry.blockTime < this.startTimestamp) {
            if (state.signature)
              throw new Error('Fee cursor missing from RPC history');
            done = true;
            break;
          }
          if (entry.err != null) continue;
          const tx = await this.rpc<CollectionTransaction | null>(
            'getTransaction',
            [
              entry.signature,
              {
                commitment: 'finalized',
                encoding: 'json',
                maxSupportedTransactionVersion: 1,
              },
            ],
          );
          if (!tx)
            throw new Error(`Missing fee transaction: ${entry.signature}`);
          const receipt = decodeCollection(tx);
          if (receipt) {
            if (tx.blockTime !== entry.blockTime || tx.slot !== entry.slot)
              throw new Error('Fee transaction metadata mismatch');
            await lock.query(
              `INSERT INTO manifest_fee_collections
              (signature,slot,block_time,lamports,funding_lamports) VALUES ($1,$2,$3,$4,$5)
              ON CONFLICT (signature) DO NOTHING`,
              [
                entry.signature,
                tx.slot,
                tx.blockTime,
                receipt.lamports,
                receipt.fundingLamports,
              ],
            );
          }
        }
        const next = page[page.length - 1].signature;
        if (!done && next === before)
          throw new Error('Fee RPC pagination did not advance');
        before = next;
      }
      // Rows may have been persisted during a partial scan. Coverage advances
      // only after every signature was processed; retries safely deduplicate.
      await lock.query(
        `UPDATE manifest_fee_cursor SET signature=$2,
        indexed_through=GREATEST(indexed_through,$3) WHERE collector=$1`,
        [FEE_COLLECTOR, newest ?? state.signature, horizon],
      );
    } finally {
      try {
        if (locked) await lock!.query('SELECT pg_advisory_unlock(62619402)');
      } finally {
        lock?.release();
        this.running = false;
      }
    }
  }

  async query(start: number, end: number) {
    if (
      !Number.isSafeInteger(start) ||
      !Number.isSafeInteger(end) ||
      start < 0 ||
      end <= start ||
      end - start > 31 * 86400
    ) {
      throw new RangeError(
        'Use integer Unix timestamps with 0 < end-start <= 31 days',
      );
    }
    // One statement gives coverage and totals a consistent database snapshot.
    const result = await this.pool.query(
      `SELECT c.start_timestamp,c.indexed_through,
      COALESCE((SELECT SUM(lamports) FROM manifest_fee_collections WHERE block_time >= $2 AND block_time < $3),0)::text AS lamports,
      COALESCE((SELECT SUM(funding_lamports) FROM manifest_fee_collections WHERE block_time >= $2 AND block_time < $3),0)::text AS funding_lamports
      FROM manifest_fee_cursor c WHERE collector=$1`,
      [FEE_COLLECTOR, start, end],
    );
    const row = result.rows[0];
    if (
      !row ||
      Number(row.start_timestamp) !== this.startTimestamp ||
      start < Number(row.start_timestamp) ||
      end > Number(row.indexed_through)
    ) {
      throw new Error('Requested fee interval is not fully indexed');
    }
    return {
      startTimestamp: start,
      endTimestamp: end,
      indexedThrough: Number(row.indexed_through),
      collector: FEE_COLLECTOR,
      complete: true,
      methodology: 'collection-basis-v1',
      feesLamports: row.lamports as string,
      revenueLamports: row.lamports as string,
      fundingLamports: row.funding_lamports as string,
      breakdown: { [COLLECTION_LABEL]: row.lamports as string },
    };
  }
}
