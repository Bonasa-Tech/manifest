export const FEE_COLLECTOR = 'B6dmr2UAn2wgjdm3T4N1Vjd8oPYRRTguByW7AEngkeL6';
const HOUR = 3600;
const RETENTION = 7 * 24 * HOUR;

export const COLLECTION_LABEL = 'Collector SOL balance increases';

export type CollectionTransaction = {
  slot: number;
  blockTime: number | null;
  transaction: { message: { accountKeys: string[] } };
  meta: null | {
    preBalances: number[];
    postBalances: number[];
    loadedAddresses?: { writable: string[]; readonly: string[] };
  };
};

function atoms(value: number): bigint {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new Error('Unsafe lamport amount from RPC');
  }
  return BigInt(value);
}

/** Count the collector's actual SOL gain in each transaction, regardless of
 * program, instruction, sender, or signer. Gas and other outflows within the
 * transaction are already reflected in the balance change; do not add them back.
 * Separate outgoing transactions do not offset positive incoming transactions. */
export function decodeCollection(
  tx: CollectionTransaction,
): null | { lamports: string } {
  if (!tx.meta) throw new Error('Missing transaction metadata');
  const keys = [
    ...tx.transaction.message.accountKeys,
    ...(tx.meta.loadedAddresses?.writable ?? []),
    ...(tx.meta.loadedAddresses?.readonly ?? []),
  ];
  const collector = keys.indexOf(FEE_COLLECTOR);
  if (collector < 0)
    throw new Error('Collector missing from transaction accounts');
  const increase =
    atoms(tx.meta.postBalances[collector]) -
    atoms(tx.meta.preBalances[collector]);
  return increase > 0n ? { lamports: increase.toString() } : null;
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

type Collection = {
  blockTime: number;
  lamports: bigint;
};

export class FeeCollectionIndexer {
  private running = false;
  private signature?: string;
  private indexedThrough = 0;
  private readonly hourlyLamports = new Map<number, bigint>();
  readonly startTimestamp: number;

  constructor(
    private rpc: Rpc,
    now: () => number = Date.now,
  ) {
    // Exclude the partial startup second: coverage begins at the first full
    // second after this process's indexer is created. Nothing is persisted.
    this.startTimestamp = Math.ceil(now() / 1000);
  }

  async poll(): Promise<void> {
    if (this.running) return;
    this.running = true;
    const pending = new Map<string, Collection>();
    try {
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
            minContextSlot: slot,
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
        for (const entry of page) {
          if (entry.slot > slot) continue;
          newest ??= entry.signature;
          if (entry.signature === this.signature) {
            done = true;
            break;
          }
          if (entry.blockTime === null)
            throw new Error(`Missing block time: ${entry.signature}`);
          if (entry.blockTime < this.startTimestamp) {
            if (this.signature)
              throw new Error('Fee cursor missing from RPC history');
            done = true;
            break;
          }
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
            pending.set(entry.signature, {
              blockTime: entry.blockTime,
              lamports: BigInt(receipt.lamports),
            });
          }
        }
        const next = page[page.length - 1].signature;
        if (!done && next === before)
          throw new Error('Fee RPC pagination did not advance');
        before = next;
      }
      // Commit the whole scan synchronously. Failed polls leave the previously
      // completed coverage and totals intact, and retry from the same cursor.
      for (const collection of pending.values()) {
        const hour = Math.floor(collection.blockTime / HOUR) * HOUR;
        this.hourlyLamports.set(
          hour,
          (this.hourlyLamports.get(hour) ?? 0n) + collection.lamports,
        );
      }
      this.signature = newest ?? this.signature;
      this.indexedThrough = Math.max(
        this.indexedThrough,
        Math.floor(horizon / HOUR) * HOUR,
      );
      // Keep seven days of completed hours plus the pending current hour.
      for (const hour of this.hourlyLamports.keys()) {
        if (hour < this.indexedThrough - RETENTION)
          this.hourlyLamports.delete(hour);
      }
    } finally {
      this.running = false;
    }
  }

  async query(start: number, end: number) {
    if (
      !Number.isSafeInteger(start) ||
      !Number.isSafeInteger(end) ||
      start < 0 ||
      end <= start ||
      start % HOUR !== 0 ||
      end % HOUR !== 0 ||
      end - start > RETENTION
    ) {
      throw new RangeError(
        'Use whole-hour UTC Unix timestamps with 0 < end-start <= 7 days',
      );
    }
    const coverageStart = Math.max(
      Math.ceil(this.startTimestamp / HOUR) * HOUR,
      this.indexedThrough - RETENTION,
    );
    if (start < coverageStart || end > this.indexedThrough) {
      throw new Error('Requested fee interval is not fully indexed');
    }
    let lamports = 0n;
    for (let hour = start; hour < end; hour += HOUR) {
      lamports += this.hourlyLamports.get(hour) ?? 0n;
    }
    return {
      startTimestamp: start,
      endTimestamp: end,
      indexedThrough: this.indexedThrough,
      coverageStartTimestamp: coverageStart,
      collector: FEE_COLLECTOR,
      complete: true,
      methodology: 'collector-positive-sol-delta-v1',
      feesLamports: lamports.toString(),
      revenueLamports: lamports.toString(),
      breakdown: { [COLLECTION_LABEL]: lamports.toString() },
    };
  }
}
