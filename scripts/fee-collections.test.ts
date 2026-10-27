import assert from 'node:assert/strict';
import { test } from 'node:test';
import bs58 from 'bs58';
import { readFileSync, readdirSync } from 'node:fs';
import type { Pool } from 'pg';
import {
  decodeCollection,
  FEE_COLLECTOR,
  FeeCollectionIndexer,
  type CollectionTransaction,
  type Rpc,
} from './stats_utils/feeCollections';

const CORE = 'MNFSTqtC93rEfYHB6hF82sKdZpUDFWkViLByLd1k1Ms';
const SYSTEM = '11111111111111111111111111111111';
function transaction(): CollectionTransaction {
  return {
    slot: 5,
    blockTime: 150,
    transaction: {
      message: {
        accountKeys: [FEE_COLLECTOR, CORE, 'market', SYSTEM],
        header: { numRequiredSignatures: 1 },
        instructions: [
          { programIdIndex: 1, accounts: [0, 2], data: bs58.encode([14]) },
        ],
      },
    },
    meta: {
      err: null,
      innerInstructions: [],
      fee: 5000,
      preBalances: [100000, 0, 0, 0],
      postBalances: [145000, 0, 0, 0],
    },
  };
}

test('counts direct resizing receipts and adds back gas', () => {
  assert.deepEqual(decodeCollection(transaction()), {
    lamports: '50000',
    fundingLamports: '0',
  });
});
test('adds collector-funded expansion back to gross receipts', () => {
  const tx = transaction();
  const data = Buffer.alloc(12);
  data.writeUInt32LE(2);
  data.writeBigUInt64LE(20000n, 4);
  tx.meta!.postBalances[0] -= 20000;
  tx.meta!.innerInstructions = [
    {
      index: 0,
      instructions: [
        { programIdIndex: 3, accounts: [0, 2], data: bs58.encode(data) },
      ],
    },
  ];
  assert.deepEqual(decodeCollection(tx), {
    lamports: '50000',
    fundingLamports: '20000',
  });
});
test('sponsored collections do not add the sponsor gas to revenue', () => {
  const tx = transaction();
  tx.transaction.message.accountKeys[0] = 'sponsor';
  tx.transaction.message.accountKeys.push(FEE_COLLECTOR);
  tx.transaction.message.header.numRequiredSignatures = 5;
  tx.transaction.message.instructions[0].accounts[0] = 4;
  tx.meta!.preBalances.push(100000);
  tx.meta!.postBalances.push(150000);
  assert.equal(decodeCollection(tx)?.lamports, '50000');
});
test('failed transactions and unrelated wallet deposits are excluded', () => {
  const tx = transaction();
  tx.meta!.err = { InstructionError: [0, 'failure'] };
  assert.equal(decodeCollection(tx), null);
  tx.meta!.err = null;
  tx.transaction.message.instructions[0].programIdIndex = 3;
  assert.equal(decodeCollection(tx), null);
});
test('rejects mixed transfers rather than attributing a wallet balance delta', () => {
  const tx = transaction();
  tx.transaction.message.instructions.push({
    programIdIndex: 3,
    accounts: [0, 2],
    data: bs58.encode([2]),
  });
  assert.throws(() => decodeCollection(tx), /Mixed/);
});
test('rejects missing metadata, unsafe integers, and unexplained outflows', () => {
  const tx = transaction();
  tx.meta!.preBalances[0] = Number.MAX_SAFE_INTEGER + 1;
  assert.throws(() => decodeCollection(tx), /Unsafe/);
  tx.meta!.preBalances[0] = 200000;
  assert.throws(() => decodeCollection(tx), /outflow/);
  tx.meta = null;
  assert.throws(() => decodeCollection(tx), /metadata/);
});
test('resolves address lookup table keys', () => {
  const tx = transaction();
  tx.transaction.message.accountKeys.splice(2);
  tx.meta!.loadedAddresses = { writable: ['market'], readonly: [SYSTEM] };
  assert.equal(decodeCollection(tx)?.lamports, '50000');
});
for (const file of readdirSync('scripts/fixtures/fee-collections')) {
  test(`mainnet ${file} receipt reconciles with wallet balances`, () => {
    const fixture = JSON.parse(
      readFileSync(`scripts/fixtures/fee-collections/${file}`, 'utf8'),
    );
    assert.deepEqual(decodeCollection(fixture.transaction), fixture.expected);
  });
}

function harness() {
  const state = {
    start_timestamp: 100,
    signature: null as string | null,
    indexed_through: 0,
  };
  const rows = new Map<
    string,
    { time: number; amount: bigint; funding: bigint }
  >();
  const pool = {
    async connect() {
      return {
        query: async (sql: string, args: any[] = []) =>
          sql.includes('advisory_')
            ? { rows: [{ locked: true }] }
            : pool.query(sql, args),
        release() {},
      };
    },
    async query(sql: string, args: any[] = []) {
      if (sql.startsWith('SELECT *')) return { rows: [{ ...state }] };
      if (sql.startsWith('INSERT INTO manifest_fee_collections')) {
        if (!rows.has(args[0]))
          rows.set(args[0], {
            time: args[2],
            amount: BigInt(args[3]),
            funding: BigInt(args[4]),
          });
      } else if (sql.startsWith('UPDATE')) {
        state.signature = args[1];
        state.indexed_through = args[2];
      } else if (sql.startsWith('SELECT c.')) {
        const selected = [...rows.values()].filter(
          (r) => r.time >= args[1] && r.time < args[2],
        );
        return {
          rows: [
            {
              ...state,
              lamports: selected.reduce((n, r) => n + r.amount, 0n).toString(),
              funding_lamports: '0',
            },
          ],
        };
      }
      return { rows: [] };
    },
  };
  let missing = false;
  let page = [
    { signature: 'a', slot: 5, blockTime: 150, err: null },
    { signature: 'old', slot: 1, blockTime: 99, err: null },
  ];
  const rpc: Rpc = async <T>(method: string): Promise<T> => {
    return (
      method === 'getSlot'
        ? 10
        : method === 'getBlockTime'
          ? 200
          : method === 'getSignaturesForAddress'
            ? page
            : missing
              ? null
              : transaction()
    ) as T;
  };
  const indexer = new FeeCollectionIndexer(pool as unknown as Pool, rpc, 100);
  return {
    indexer,
    state,
    rows,
    setMissing: (v: boolean) => {
      missing = v;
    },
    setPage: (v: typeof page) => {
      page = v;
    },
  };
}
test('replays idempotently and serves half-open intervals including genuine zero', async () => {
  const h = harness();
  await h.indexer.poll();
  await h.indexer.poll();
  assert.equal(h.rows.size, 1);
  assert.equal((await h.indexer.query(100, 150)).feesLamports, '0');
  assert.equal((await h.indexer.query(150, 200)).feesLamports, '50000');
  await assert.rejects(h.indexer.query(99, 200), /not fully/);
  await assert.rejects(h.indexer.query(100, 201), /not fully/);
  await assert.rejects(h.indexer.query(150, 150), RangeError);
});
test('missing transactions do not advance coverage and recover on retry', async () => {
  const h = harness();
  h.setMissing(true);
  await assert.rejects(h.indexer.poll(), /Missing fee transaction/);
  assert.equal(h.state.indexed_through, 0);
  h.setMissing(false);
  await h.indexer.poll();
  assert.equal(h.state.indexed_through, 200);
});
test('empty/pruned history fails instead of claiming complete zero', async () => {
  const h = harness();
  h.setPage([]);
  await assert.rejects(h.indexer.poll(), /archival/);
  assert.equal(h.state.indexed_through, 0);
});
test('missing saved cursor cannot be replaced by reaching start boundary', async () => {
  const h = harness();
  h.state.signature = 'missing';
  await assert.rejects(h.indexer.poll(), /cursor missing/);
  assert.equal(h.state.indexed_through, 0);
});
test('partial writes are hidden until coverage commits, then deduplicated', async () => {
  const h = harness();
  h.setPage([{ signature: 'a', slot: 5, blockTime: 150, err: null }]);
  await assert.rejects(h.indexer.poll(), /pagination/);
  assert.equal(h.rows.size, 1);
  await assert.rejects(h.indexer.query(100, 200), /not fully/);
  h.setPage([
    { signature: 'a', slot: 5, blockTime: 150, err: null },
    { signature: 'old', slot: 1, blockTime: 99, err: null },
  ]);
  await h.indexer.poll();
  assert.equal((await h.indexer.query(100, 200)).feesLamports, '50000');
});

test('wrapper collect includes resizing proceeds; defrag alone earns nothing', () => {
  const tx = transaction();
  tx.transaction.message.accountKeys[1] =
    'wMNFSTkir3HgyZTsB7uqu3i7FA73grFCptPXgrZjksL';
  tx.transaction.message.instructions = [
    { programIdIndex: 1, accounts: [2, 0], data: bs58.encode([8]) },
    { programIdIndex: 1, accounts: [2, 3, 0], data: bs58.encode([7]) },
  ];
  assert.equal(decodeCollection(tx)?.lamports, '50000');
  tx.transaction.message.instructions.pop();
  assert.equal(decodeCollection(tx), null);
});

test('cleaning rewards are counted even when all proceeds pay transaction gas', () => {
  const tx = transaction();
  tx.transaction.message.instructions[0].data = bs58.encode([12]);
  tx.meta!.postBalances[0] = tx.meta!.preBalances[0];
  assert.equal(decodeCollection(tx)?.lamports, '5000');
});

test('missing inner-instruction history fails instead of undercounting funding', () => {
  const tx = transaction();
  tx.meta!.innerInstructions = null;
  assert.throws(() => decodeCollection(tx), /Missing inner instructions/);
});
