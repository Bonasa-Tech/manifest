import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  decodeCollection,
  FEE_COLLECTOR,
  FeeCollectionIndexer,
  type CollectionTransaction,
  type Rpc,
} from './stats_utils/feeCollections';

function transaction(): CollectionTransaction {
  return {
    slot: 5,
    blockTime: 150,
    transaction: { message: { accountKeys: ['payer', FEE_COLLECTOR] } },
    meta: { preBalances: [100000, 100000], postBalances: [45000, 150000] },
  };
}

test('counts wallet deposits without recognized instructions or collector signature', () => {
  assert.deepEqual(decodeCollection(transaction()), { lamports: '50000' });
});
test('uses the actual gain after gas or funding, without adding back expenses', () => {
  const tx = transaction();
  tx.transaction.message.accountKeys = [FEE_COLLECTOR];
  tx.meta!.preBalances = [100000];
  tx.meta!.postBalances = [125000];
  assert.deepEqual(decodeCollection(tx), { lamports: '25000' });
});
test('ignores zero and negative changes, including failed-transaction gas', () => {
  const tx = transaction();
  tx.meta!.postBalances[1] = 100000;
  assert.equal(decodeCollection(tx), null);
  tx.meta!.postBalances[1] = 95000;
  assert.equal(decodeCollection(tx), null);
});
test('rejects missing metadata, missing collector, and unsafe balances', () => {
  const tx = transaction();
  tx.meta!.preBalances[1] = Number.MAX_SAFE_INTEGER + 1;
  assert.throws(() => decodeCollection(tx), /Unsafe/);
  tx.transaction.message.accountKeys = ['payer'];
  assert.throws(() => decodeCollection(tx), /Collector missing/);
  tx.meta = null;
  assert.throws(() => decodeCollection(tx), /metadata/);
});
test('resolves collector from address lookup tables', () => {
  const tx = transaction();
  tx.transaction.message.accountKeys = ['payer'];
  tx.meta!.loadedAddresses = { writable: [FEE_COLLECTOR], readonly: [] };
  assert.equal(decodeCollection(tx)?.lamports, '50000');
});
function harness(startMillis = 3600000) {
  let missing = false;
  let horizon = 10800;
  let page = [
    { signature: 'a', slot: 5, blockTime: 7200, err: null },
    { signature: 'old', slot: 1, blockTime: 3599, err: null },
  ];
  const fetched: string[] = [];
  const deltas: Record<string, number> = {};
  const rpc: Rpc = async <T>(method: string, params: unknown[]): Promise<T> => {
    if (method === 'getSlot') return 10 as T;
    if (method === 'getBlockTime') return horizon as T;
    if (method === 'getSignaturesForAddress') return page as T;
    const signature = params[0] as string;
    fetched.push(signature);
    if (missing) return null as T;
    const tx = transaction();
    const entry = page.find((entry) => entry.signature === signature)!;
    tx.blockTime = entry.blockTime;
    tx.slot = entry.slot;
    tx.meta!.postBalances[1] =
      tx.meta!.preBalances[1] + (deltas[signature] ?? 50000);
    return tx as T;
  };
  return {
    indexer: new FeeCollectionIndexer(rpc, () => startMillis),
    fetched,
    setDelta: (signature: string, amount: number) => {
      deltas[signature] = amount;
    },
    setMissing: (v: boolean) => {
      missing = v;
    },
    setPage: (v: typeof page) => {
      page = v;
    },
    setHorizon: (v: number) => {
      horizon = v;
    },
  };
}
test('replays idempotently and serves half-open intervals including genuine zero', async () => {
  const h = harness();
  await h.indexer.poll();
  await h.indexer.poll();
  assert.deepEqual(h.fetched, ['a']);
  assert.equal((await h.indexer.query(3600, 7200)).feesLamports, '0');
  assert.equal((await h.indexer.query(7200, 10800)).feesLamports, '50000');
  await assert.rejects(h.indexer.query(0, 10800), /not fully/);
  await assert.rejects(h.indexer.query(3600, 14400), /not fully/);
  await assert.rejects(h.indexer.query(7200, 7200), RangeError);
});
test('missing transactions do not advance coverage and recover on retry', async () => {
  const h = harness();
  h.setMissing(true);
  await assert.rejects(h.indexer.poll(), /Missing fee transaction/);
  await assert.rejects(h.indexer.query(3600, 10800), /not fully/);
  h.setMissing(false);
  await h.indexer.poll();
  assert.equal((await h.indexer.query(3600, 10800)).feesLamports, '50000');
});
test('empty/pruned history fails instead of claiming complete zero', async () => {
  const h = harness();
  h.setPage([]);
  await assert.rejects(h.indexer.poll(), /history ended/);
  await assert.rejects(h.indexer.query(3600, 10800), /not fully/);
});
test('missing cursor cannot be replaced by reaching startup boundary', async () => {
  const h = harness();
  await h.indexer.poll();
  h.setPage([{ signature: 'old', slot: 1, blockTime: 3599, err: null }]);
  h.setHorizon(14400);
  await assert.rejects(h.indexer.poll(), /cursor missing/);
  assert.equal((await h.indexer.query(3600, 10800)).feesLamports, '50000');
  await assert.rejects(h.indexer.query(3600, 14400), /not fully/);
});
test('partial scans are discarded until coverage commits, then deduplicated', async () => {
  const h = harness();
  h.setPage([{ signature: 'a', slot: 5, blockTime: 7200, err: null }]);
  await assert.rejects(h.indexer.poll(), /pagination/);
  await assert.rejects(h.indexer.query(3600, 10800), /not fully/);
  h.setPage([
    { signature: 'a', slot: 5, blockTime: 7200, err: null },
    { signature: 'old', slot: 1, blockTime: 3599, err: null },
  ]);
  await h.indexer.poll();
  assert.equal((await h.indexer.query(3600, 10800)).feesLamports, '50000');
});
test('restart resets coverage and does not fetch or report earlier collections', async () => {
  const original = harness();
  await original.indexer.poll();
  assert.equal(
    (await original.indexer.query(3600, 10800)).feesLamports,
    '50000',
  );
  const restarted = harness(10800000);
  restarted.setPage([
    { signature: 'new', slot: 6, blockTime: 14400, err: null },
    { signature: 'a', slot: 5, blockTime: 7200, err: null },
  ]);
  restarted.setHorizon(18000);
  await restarted.indexer.poll();
  assert.deepEqual(restarted.fetched, ['new']);
  await assert.rejects(restarted.indexer.query(3600, 18000), /not fully/);
  const result = await restarted.indexer.query(10800, 18000);
  assert.equal(result.feesLamports, '50000');
  assert.equal(result.coverageStartTimestamp, 10800);
});
test('startup partial hour and unfinished hours are unavailable', async () => {
  const h = harness(7200001);
  h.setHorizon(15000);
  await h.indexer.poll();
  assert.deepEqual(h.fetched, []);
  assert.equal(h.indexer.startTimestamp, 7201);
  await assert.rejects(h.indexer.query(7200, 10800), /not fully/);
  assert.equal((await h.indexer.query(10800, 14400)).feesLamports, '0');
  await assert.rejects(h.indexer.query(14400, 18000), /not fully/);
  await assert.rejects(h.indexer.query(10801, 14400), RangeError);
});
test('retains only seven days and bounds queries to 168 hours', async () => {
  const h = harness();
  await h.indexer.poll();
  h.setHorizon(7200 + 7 * 86400);
  await h.indexer.poll();
  assert.equal((await h.indexer.query(7200, 10800)).feesLamports, '50000');
  h.setHorizon(10800 + 7 * 86400);
  await h.indexer.poll();
  await assert.rejects(h.indexer.query(7200, 10800), /not fully/);
  const result = await h.indexer.query(10800, 10800 + 7 * 86400);
  assert.equal(result.feesLamports, '0');
  assert.equal(result.coverageStartTimestamp, 10800);
  await assert.rejects(h.indexer.query(10800, 14400 + 7 * 86400), RangeError);
});

test('sums all positive transactions in an hour without subtracting outgoing transactions', async () => {
  const h = harness();
  h.setPage([
    { signature: 'deposit', slot: 9, blockTime: 7500, err: null },
    { signature: 'outflow', slot: 8, blockTime: 7400, err: null },
    { signature: 'resizing', slot: 7, blockTime: 7300, err: null },
    { signature: 'old', slot: 1, blockTime: 3599, err: null },
  ]);
  h.setDelta('deposit', 100000);
  h.setDelta('outflow', -50000);
  h.setDelta('resizing', 20000);
  await h.indexer.poll();
  await h.indexer.poll();
  const result = await h.indexer.query(7200, 10800);
  assert.equal(result.feesLamports, '120000');
  assert.equal(result.revenueLamports, '120000');
});
