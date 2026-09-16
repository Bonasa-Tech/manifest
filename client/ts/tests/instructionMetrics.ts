import { assert } from 'chai';
import bs58 from 'bs58';
import * as promClient from 'prom-client';
import { PublicKey } from '@solana/web3.js';
import { PROGRAM_ID } from '../src/manifest';
import { createBatchUpdateInstruction } from '../src/manifest/instructions/BatchUpdate';
import { OrderType } from '../src/manifest/types/OrderType';
import {
  recordManifestInstructionMetrics,
  splitComputeUnits,
  summarizeManifestInstructions,
} from '../src/utils/instructionMetrics';

const MANIFEST: string = PROGRAM_ID.toBase58();
const WRAPPER: string = 'wMNFSTkir3HgyZTsB7uqu3i7FA73grFCptPXgrZjksL';
const TOKEN: string = 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA';
const PAYER: string = '11111111111111111111111111111111';

const SWAP: number = 4;
const BATCH_UPDATE: number = 6;
const SWAP_V2: number = 13;

/**
 * Instruction data exactly as the client serializes it: a cancel (so the
 * decoder has to skip past it) and two orders of different types.
 */
const BATCH_UPDATE_DATA: Uint8Array = createBatchUpdateInstruction(
  { payer: new PublicKey(PAYER), market: new PublicKey(PAYER) },
  {
    params: {
      traderIndexHint: null,
      cancels: [{ orderSequenceNumber: 7, orderIndexHint: null }],
      orders: [
        {
          baseAtoms: 1_000,
          priceMantissa: 1,
          priceExponent: 0,
          isBid: true,
          lastValidSlot: 0,
          orderType: OrderType.Limit,
        },
        {
          baseAtoms: 2_000,
          priceMantissa: 2,
          priceExponent: -1,
          isBid: false,
          lastValidSlot: 0,
          orderType: OrderType.PostOnly,
        },
      ],
    },
  },
).data;

/**
 * Legacy-format transaction: a top-level Swap, a wrapper instruction that
 * CPIs into Manifest BatchUpdate, then a top-level SwapV2.
 */
function legacyTx(
  logMessages: string[],
  batchUpdateData: Uint8Array = BATCH_UPDATE_DATA,
): any {
  return {
    transaction: {
      message: {
        accountKeys: [PAYER, MANIFEST, WRAPPER, TOKEN],
        instructions: [
          { programIdIndex: 1, accounts: [0], data: bs58.encode([SWAP]) },
          { programIdIndex: 2, accounts: [0], data: bs58.encode([0]) },
          { programIdIndex: 1, accounts: [0], data: bs58.encode([SWAP_V2]) },
        ],
      },
    },
    meta: {
      err: null,
      logMessages,
      innerInstructions: [
        {
          index: 1,
          instructions: [
            {
              programIdIndex: 1,
              accounts: [0],
              data: bs58.encode(batchUpdateData),
              stackHeight: 2,
            },
          ],
        },
      ],
    },
  };
}

const FULL_LOGS: string[] = [
  `Program ${MANIFEST} invoke [1]`,
  `Program ${TOKEN} invoke [2]`,
  `Program ${TOKEN} consumed 4645 of 150000 compute units`,
  `Program ${TOKEN} success`,
  `Program ${MANIFEST} consumed 30000 of 200000 compute units`,
  `Program ${MANIFEST} success`,
  `Program ${WRAPPER} invoke [1]`,
  `Program ${MANIFEST} invoke [2]`,
  `Program ${MANIFEST} consumed 45000 of 170000 compute units`,
  `Program ${MANIFEST} success`,
  `Program ${WRAPPER} consumed 60000 of 170000 compute units`,
  `Program ${WRAPPER} success`,
  `Program ${MANIFEST} invoke [1]`,
  `Program ${MANIFEST} consumed 25000 of 110000 compute units`,
  `Program ${MANIFEST} success`,
];

describe('manifest instruction metrics', () => {
  it('names every Manifest instruction in execution order with its compute units', () => {
    assert.deepEqual(summarizeManifestInstructions(legacyTx(FULL_LOGS)), [
      { instruction: 'Swap', computeUnits: 30000 },
      {
        instruction: 'BatchUpdate',
        computeUnits: 45000,
        orderTypes: ['Limit', 'PostOnly'],
      },
      { instruction: 'SwapV2', computeUnits: 25000 },
    ]);
  });

  it('keeps counting instructions after the logs are truncated', () => {
    const truncated: string[] = [...FULL_LOGS.slice(0, 6), 'Log truncated'];
    assert.deepEqual(summarizeManifestInstructions(legacyTx(truncated)), [
      { instruction: 'Swap', computeUnits: 30000 },
      {
        instruction: 'BatchUpdate',
        orderTypes: ['Limit', 'PostOnly'],
      },
      { instruction: 'SwapV2' },
    ]);
  });

  it('counts a BatchUpdate whose params cannot be decoded, without orders', () => {
    const summaries = summarizeManifestInstructions(
      legacyTx(FULL_LOGS, Uint8Array.from([BATCH_UPDATE])),
    );
    assert.deepEqual(summaries[1], {
      instruction: 'BatchUpdate',
      computeUnits: 45000,
    });
  });

  it('resolves a v0 message whose program id comes from a lookup table', () => {
    const tx: any = {
      transaction: {
        message: {
          staticAccountKeys: [PAYER],
          compiledInstructions: [
            {
              programIdIndex: 1,
              accountKeyIndexes: [0],
              data: BATCH_UPDATE_DATA,
            },
            {
              programIdIndex: 1,
              accountKeyIndexes: [0],
              data: Uint8Array.from([]),
            },
            {
              programIdIndex: 1,
              accountKeyIndexes: [0],
              data: Uint8Array.from([200]),
            },
          ],
        },
      },
      meta: {
        err: null,
        loadedAddresses: { writable: [], readonly: [MANIFEST] },
        innerInstructions: [],
        logMessages: [
          `Program ${MANIFEST} invoke [1]`,
          `Program log: Program ${MANIFEST} consumed 1 of 2 compute units`,
          `Program ${MANIFEST} consumed 12345 of 200000 compute units`,
          `Program ${MANIFEST} success`,
        ],
      },
    };
    assert.deepEqual(summarizeManifestInstructions(tx), [
      {
        instruction: 'BatchUpdate',
        computeUnits: 12345,
        orderTypes: ['Limit', 'PostOnly'],
      },
      { instruction: 'Unknown' },
      { instruction: 'Unknown' },
    ]);
  });

  it('ignores transactions that never invoke Manifest', () => {
    const tx: any = {
      transaction: {
        message: {
          accountKeys: [PAYER, TOKEN],
          instructions: [
            { programIdIndex: 1, accounts: [0], data: bs58.encode([3]) },
          ],
        },
      },
      meta: { err: null, logMessages: [], innerInstructions: null },
    };
    assert.deepEqual(summarizeManifestInstructions(tx), []);
  });

  const metricValue = async (
    metricName: string,
    labels: Record<string, string> = {},
    series: string = metricName,
  ): Promise<number> => {
    const metric = promClient.register.getSingleMetric(metricName);
    assert.isDefined(metric, `metric ${metricName} is registered`);
    // Histogram values carry a per-series metricName (_bucket, _sum, _count)
    // that the shared Metric type does not declare.
    const { values } = (await metric!.get()) as {
      values: {
        metricName?: string;
        value: number;
        labels: Partial<Record<string, string | number>>;
      }[];
    };
    return (
      values.find(
        (v) =>
          (v.metricName ?? metricName) === series &&
          Object.entries(labels).every(
            ([key, value]) => v.labels[key] === value,
          ),
      )?.value ?? 0
    );
  };

  /**
   * Every series the feed's averages are built from, so a test can assert
   * exactly which ones a transaction moves and by how much.
   */
  const snapshot = async (): Promise<Record<string, number>> => {
    const instructionCu = (instruction: string, series: string) =>
      metricValue(
        'manifest_instruction_compute_units',
        { instruction },
        `manifest_instruction_compute_units_${series}`,
      );
    const orderCu = (orderType: string, series: string) =>
      metricValue(
        'manifest_order_compute_units',
        { orderType },
        `manifest_order_compute_units_${series}`,
      );
    return {
      swaps: await metricValue('manifest_instructions', {
        instruction: 'Swap',
      }),
      swapV2s: await metricValue('manifest_instructions', {
        instruction: 'SwapV2',
      }),
      batchUpdates: await metricValue('manifest_instructions', {
        instruction: 'BatchUpdate',
      }),
      limitOrders: await metricValue('manifest_batch_update_orders', {
        orderType: 'Limit',
      }),
      postOnlyOrders: await metricValue('manifest_batch_update_orders', {
        orderType: 'PostOnly',
      }),
      swapCuCount: await instructionCu('Swap', 'count'),
      swapCuSum: await instructionCu('Swap', 'sum'),
      swapV2CuCount: await instructionCu('SwapV2', 'count'),
      batchUpdateCuCount: await instructionCu('BatchUpdate', 'count'),
      batchUpdateCuSum: await instructionCu('BatchUpdate', 'sum'),
      limitOrderCuCount: await orderCu('Limit', 'count'),
      limitOrderCuSum: await orderCu('Limit', 'sum'),
      postOnlyOrderCuCount: await orderCu('PostOnly', 'count'),
      postOnlyOrderCuSum: await orderCu('PostOnly', 'sum'),
    };
  };

  const metricDelta = async (tx: any): Promise<Record<string, number>> => {
    const before = await snapshot();
    recordManifestInstructionMetrics(tx);
    const after = await snapshot();
    return Object.fromEntries(
      Object.keys(after).map((key) => [key, after[key] - before[key]]),
    );
  };

  it('records counts, orders and compute units to prometheus', async () => {
    assert.deepEqual(await metricDelta(legacyTx(FULL_LOGS)), {
      swaps: 1,
      swapV2s: 1,
      batchUpdates: 1,
      limitOrders: 1,
      postOnlyOrders: 1,
      swapCuCount: 1,
      swapCuSum: 30000,
      swapV2CuCount: 1,
      batchUpdateCuCount: 1,
      batchUpdateCuSum: 45000,
      // The batch's 45000 units split evenly across its two orders.
      limitOrderCuCount: 1,
      limitOrderCuSum: 22500,
      postOnlyOrderCuCount: 1,
      postOnlyOrderCuSum: 22500,
    });
  });

  it('counts truncated instructions and orders without adding compute unit samples', async () => {
    // The logs end after the first Swap, so the BatchUpdate and SwapV2 have
    // no consumed line. They must still be counted, and their compute unit
    // series must not move at all: a zero or partial sample would drag the
    // per-swap and per-order averages down.
    const truncated: string[] = [...FULL_LOGS.slice(0, 6), 'Log truncated'];
    assert.deepEqual(await metricDelta(legacyTx(truncated)), {
      swaps: 1,
      swapV2s: 1,
      batchUpdates: 1,
      limitOrders: 1,
      postOnlyOrders: 1,
      swapCuCount: 1,
      swapCuSum: 30000,
      swapV2CuCount: 0,
      batchUpdateCuCount: 0,
      batchUpdateCuSum: 0,
      limitOrderCuCount: 0,
      limitOrderCuSum: 0,
      postOnlyOrderCuCount: 0,
      postOnlyOrderCuSum: 0,
    });
  });

  it('splits compute units into integer shares that add up exactly', () => {
    assert.deepEqual(splitComputeUnits(10000, 3), [3334, 3333, 3333]);
    assert.deepEqual(splitComputeUnits(45000, 2), [22500, 22500]);
    assert.deepEqual(splitComputeUnits(7, 1), [7]);
    for (const [total, parts] of [
      [10000, 3],
      [99999, 7],
      [1, 4],
    ]) {
      const shares: number[] = splitComputeUnits(total, parts);
      assert.equal(shares.length, parts);
      assert.equal(
        shares.reduce((sum, share) => sum + share, 0),
        total,
      );
      assert.isAtMost(Math.max(...shares) - Math.min(...shares), 1);
    }
  });

  it('does not throw on a transaction it cannot decode', () => {
    assert.deepEqual(recordManifestInstructionMetrics({ meta: null }), []);
  });
});
