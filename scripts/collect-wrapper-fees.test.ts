/**
 * Unit tests for the defrag compute estimate that drives v1 batch packing.
 * Pure functions only: no RPC, no keypair, no validator.
 *
 * Run with: yarn test:collect-wrapper-fees
 */
import { PublicKey } from '@solana/web3.js';
import * as assert from 'node:assert/strict';
import { WebSocketServer } from 'ws';
import {
  PendingFillMarkets,
  createFillWatcher,
  estimateBatchDefragCu,
  estimateDefragCu,
  packCollectTransactions,
  splitBatch,
} from './collect-wrapper-fees';

// Mirrors the script's own constants; a copy here fails the test if either
// side moves without the other being reconsidered.
const CU_BUDGET_PER_TRANSACTION = 1_300_000;
const V1_MAX_WRAPPERS = 61;
// --defrag sends a Defrag and a Collect per wrapper, so the 64-instruction
// limit binds before the 61-address one: 64 / 2 = 32.
const V1_MAX_DEFRAG_WRAPPERS = 32;
const COLLECTOR = new PublicKey(new Uint8Array(32).fill(7));

// Measured on the 2026-10-22 mainnet sweep: the cheapest already-packed
// wrapper, and the per-wrapper cost of the 618,304-byte outlier.
const MEASURED_PACKED_WRAPPER_CU = 1_155;
const MEASURED_LARGEST_WRAPPER_SPACE = 618_304;
const MEASURED_LARGEST_WRAPPER_CU = 566_509;

type TestWrapper = {
  pubkey: PublicKey;
  trader: PublicKey;
  amount: bigint;
  space: number;
};

function wrapper(index: number, space: number): TestWrapper {
  const bytes = new Uint8Array(32);
  bytes[0] = index % 256;
  bytes[1] = Math.floor(index / 256);
  bytes[31] = 1;
  return {
    pubkey: new PublicKey(bytes),
    trader: new PublicKey(bytes),
    amount: 1_000_000n,
    space,
  };
}

const tests: [string, () => void | Promise<void>][] = [
  [
    'a small wrapper no longer reserves a fiftieth of the budget',
    () => {
      // The old 60,000 base put floor(1_300_000/60_000) = 21 small wrappers in
      // a transaction no matter their real cost. Anything at or above 40,625
      // per wrapper (1_300_000/32) reimposes a compute cap before the
      // instruction limit binds.
      const small = estimateDefragCu(1_600);
      assert.ok(
        small < CU_BUDGET_PER_TRANSACTION / V1_MAX_DEFRAG_WRAPPERS,
        `estimate ${small} still caps batches below the v1 address limit`,
      );
    },
  ],
  [
    'the estimate stays above the measured floor',
    () => {
      assert.ok(
        estimateDefragCu(0) > MEASURED_PACKED_WRAPPER_CU,
        'estimate dips under the cheapest wrapper ever measured',
      );
    },
  ],
  [
    'the estimate covers the largest wrapper ever swept',
    () => {
      assert.ok(
        estimateDefragCu(MEASURED_LARGEST_WRAPPER_SPACE) >
          MEASURED_LARGEST_WRAPPER_CU,
        'estimate underprices the 618KB wrapper',
      );
    },
  ],
  [
    'the estimate never exceeds one transaction budget',
    () => {
      assert.equal(
        estimateDefragCu(64 * 1024 * 1024),
        CU_BUDGET_PER_TRANSACTION,
      );
    },
  ],
  [
    'tail-sized wrappers pack to the v1 instruction limit',
    () => {
      // 4,288 bytes is the modal size in the 2026-10-22 sweep's tail.
      const wrappers = Array.from({ length: 150 }, (_, at) =>
        wrapper(at, 4_288),
      );
      const batches = packCollectTransactions(
        wrappers,
        COLLECTOR,
        undefined,
        true,
      );
      assert.equal(
        batches[0].length,
        V1_MAX_DEFRAG_WRAPPERS,
        'first batch should fill the instruction limit, not stop at 21',
      );
    },
  ],
  [
    'a large wrapper still gets a transaction to itself',
    () => {
      const batches = packCollectTransactions(
        [
          wrapper(0, MEASURED_LARGEST_WRAPPER_SPACE),
          wrapper(1, MEASURED_LARGEST_WRAPPER_SPACE),
        ],
        COLLECTOR,
        undefined,
        true,
      );
      assert.equal(batches.length, 2);
      assert.equal(batches[0].length, 1);
    },
  ],
  [
    'a large wrapper does not strand the small ones in front of it',
    () => {
      // The shape that wasted transactions on mainnet: wrappers arrive ordered
      // by lamports recovered, so cheap and expensive ones interleave. A
      // forward-only pass closed a transaction at every large wrapper and left
      // each small one alone, sending this as five transactions.
      const wrappers = [
        wrapper(0, 1_600),
        wrapper(1, MEASURED_LARGEST_WRAPPER_SPACE),
        wrapper(2, 1_600),
        wrapper(3, MEASURED_LARGEST_WRAPPER_SPACE),
        wrapper(4, 1_600),
      ];
      const batches = packCollectTransactions(
        wrappers,
        COLLECTOR,
        undefined,
        true,
      );
      assert.equal(batches.length, 3, 'the three small wrappers should share');
      assert.deepEqual(
        batches.map((batch) => batch.length),
        [3, 1, 1],
      );
      assert.deepEqual(
        batches[0].map(({ space }) => space),
        [1_600, 1_600, 1_600],
      );
      // Still in value order within the transaction that collects them.
      assert.deepEqual(
        batches[0].map(({ pubkey }) => pubkey.toBase58()),
        [wrappers[0], wrappers[2], wrappers[4]].map(({ pubkey }) =>
          pubkey.toBase58(),
        ),
      );
    },
  ],
  [
    'no batch is packed past the compute budget',
    () => {
      const sizes = [928, 1_600, 4_288, 8_128, 36_352, 290_752, 618_304];
      const wrappers = Array.from({ length: 200 }, (_, at) =>
        wrapper(at, sizes[at % sizes.length]),
      );
      const batches = packCollectTransactions(
        wrappers,
        COLLECTOR,
        undefined,
        true,
      );
      for (const [at, batch] of batches.entries()) {
        const budgeted = estimateBatchDefragCu(batch, true);
        assert.ok(
          budgeted <= CU_BUDGET_PER_TRANSACTION,
          `batch ${at} budgeted ${budgeted} CU, over the per-transaction budget`,
        );
        assert.ok(
          batch.length <= V1_MAX_DEFRAG_WRAPPERS,
          `batch ${at} holds ${batch.length} wrappers, over the v1 instruction limit`,
        );
      }
      assert.equal(
        batches.reduce((total, batch) => total + batch.length, 0),
        wrappers.length,
        'packing dropped wrappers',
      );
    },
  ],
  [
    'a failed batch halves instead of ending the run',
    () => {
      // Packing backfills low-value wrappers into earlier transactions, so a
      // simulation failure must not be fatal: a cheap wrapper that always
      // fails would otherwise sit in the first transaction of every pass and
      // block every wrapper behind it, high-value ones included. Halving
      // isolates one bad wrapper and also shrinks a batch whose compute
      // estimate was too low, without having to tell those apart.
      const batch = Array.from({ length: 7 }, (_, at) => wrapper(at, 1_600));
      const [first, second] = splitBatch(batch);
      assert.equal(first.length, 4);
      assert.equal(second.length, 3);
      assert.deepEqual(
        [...first, ...second].map(({ pubkey }) => pubkey.toBase58()),
        batch.map(({ pubkey }) => pubkey.toBase58()),
        'halving must preserve order and lose nothing',
      );

      // Halving a pair isolates each one, which is what bottoms out at a
      // single wrapper the run can report and skip.
      assert.deepEqual(
        splitBatch(batch.slice(0, 2)).map((half) => half.length),
        [1, 1],
      );
      // Repeated halving terminates: every split is strictly smaller.
      let longest = batch;
      for (let step = 0; step < 10 && longest.length > 1; step += 1) {
        const [half] = splitBatch(longest);
        assert.ok(half.length < longest.length, 'split did not shrink');
        longest = half;
      }
      assert.equal(longest.length, 1);
    },
  ],
  [
    'the explicit cap still wins',
    () => {
      const wrappers = Array.from({ length: 50 }, (_, at) =>
        wrapper(at, 1_600),
      );
      const batches = packCollectTransactions(wrappers, COLLECTOR, 5, true);
      assert.ok(
        batches.every((batch) => batch.length <= 5),
        '--max-wrappers-per-tx was not honoured',
      );
    },
  ],
  [
    'collect without defrag is unaffected by the compute estimate',
    () => {
      const wrappers = Array.from({ length: 150 }, (_, at) =>
        wrapper(at, 618_304),
      );
      const batches = packCollectTransactions(
        wrappers,
        COLLECTOR,
        undefined,
        false,
      );
      assert.equal(batches[0].length, V1_MAX_WRAPPERS);
      assert.equal(estimateBatchDefragCu(batches[0], false), 0);
    },
  ],
  [
    'a confirmed sweep clears only fills that preceded that sweep',
    () => {
      const pending = new PendingFillMarkets();
      pending.record('market-a');
      const beforeSweep = pending.snapshot();

      // This second fill lands while the transaction built from beforeSweep
      // is in flight, so acknowledging that transaction must not drop it.
      pending.record('market-a');
      assert.equal(pending.acknowledge(['market-a'], beforeSweep), 0);
      assert.equal(pending.size, 1);

      const nextSweep = pending.snapshot();
      assert.equal(pending.acknowledge(['market-a'], nextSweep), 1);
      assert.equal(pending.size, 0);
    },
  ],
  [
    'the integrated fill watcher owns one reconnecting socket',
    async () => {
      const server = new WebSocketServer({ port: 0, host: '127.0.0.1' });
      await new Promise<void>((resolve) => server.once('listening', resolve));
      const address = server.address();
      assert.ok(address !== null && typeof address === 'object');

      let connections = 0;
      let activeConnections = 0;
      let maximumActiveConnections = 0;
      server.on('connection', (socket) => {
        connections += 1;
        activeConnections += 1;
        maximumActiveConnections = Math.max(
          maximumActiveConnections,
          activeConnections,
        );
        socket.once('close', () => {
          activeConnections -= 1;
        });
      });

      const watcher = createFillWatcher(`ws://127.0.0.1:${address.port}`);
      const waitFor = async (condition: () => boolean, timeoutMs = 4_000) => {
        const deadline = Date.now() + timeoutMs;
        while (!condition()) {
          if (Date.now() >= deadline) throw new Error('condition timed out');
          await new Promise((resolve) => setTimeout(resolve, 20));
        }
      };

      try {
        await watcher.waitUntilConnected();
        assert.equal(connections, 1);
        for (const socket of server.clients) {
          socket.send(JSON.stringify({ market: 'market-a', slot: 123 }));
        }
        await waitFor(() => watcher.snapshot().has('market-a'));
        assert.equal(watcher.snapshot().size, 1);

        for (const socket of server.clients) socket.close();
        await waitFor(() => connections === 2);
        assert.equal(maximumActiveConnections, 1);
      } finally {
        watcher.close();
        for (const socket of server.clients) socket.terminate();
        await new Promise<void>((resolve, reject) => {
          server.close((error) => (error ? reject(error) : resolve()));
        });
      }
    },
  ],
];

async function main(): Promise<void> {
  let failures = 0;
  for (const [name, body] of tests) {
    try {
      await body();
      console.log(`  ok  ${name}`);
    } catch (error) {
      failures += 1;
      console.error(`FAIL  ${name}`);
      console.error(`      ${error instanceof Error ? error.message : error}`);
    }
  }
  console.log(`\n${tests.length - failures}/${tests.length} passed`);
  if (failures > 0) process.exit(1);
}

void main();
