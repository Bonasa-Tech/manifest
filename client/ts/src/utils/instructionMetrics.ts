import * as promClient from 'prom-client';
import { PROGRAM_ID } from '../manifest';
import {
  BatchUpdateStruct,
  batchUpdateInstructionDiscriminator,
} from '../manifest/instructions/BatchUpdate';
import { OrderType } from '../manifest/types/OrderType';
import { executedInstructions } from './transactionInstructions';
import { extractProgramComputeUnits } from './programLogs';

/**
 * Instruction names indexed by discriminator, see
 * programs/manifest/src/program/instruction.rs.
 */
export const MANIFEST_INSTRUCTION_NAMES: readonly string[] = [
  'CreateMarket',
  'ClaimSeat',
  'Deposit',
  'Withdraw',
  'Swap',
  'Expand',
  'BatchUpdate',
  'GlobalCreate',
  'GlobalAddTrader',
  'GlobalDeposit',
  'GlobalWithdraw',
  'GlobalEvict',
  'GlobalClean',
  'SwapV2',
];
const UNKNOWN_INSTRUCTION: string = 'Unknown';

export interface ManifestInstructionSummary {
  instruction: string;
  /** Compute units consumed, absent when the consumed log line was truncated. */
  computeUnits?: number;
  /** BatchUpdate only: the order type of every order placed, in order. */
  orderTypes?: string[];
}

// Live monitoring of program usage, covering every Manifest instruction in a
// successful transaction rather than only the ones that produced fills.
// Instruction counts come from the transaction's instruction data, so they
// stay exact even when logs are truncated; compute units come from the
// runtime's "consumed" log lines and are best effort.
//
// Useful queries:
//   batch updates per day
//     increase(manifest_instructions{instruction="BatchUpdate"}[1d])
//   orders placed per day (a batch update can carry many orders)
//     sum(increase(manifest_batch_update_orders[1d]))
//   swaps per day
//     increase(manifest_instructions{instruction=~"Swap|SwapV2"}[1d])
//   average compute units per swap
//     increase(manifest_instruction_compute_units_sum{instruction=~"Swap|SwapV2"}[1d])
//       / increase(manifest_instruction_compute_units_count{instruction=~"Swap|SwapV2"}[1d])
//   average compute units per order, across all order types
//     sum(increase(manifest_order_compute_units_sum[1d]))
//       / sum(increase(manifest_order_compute_units_count[1d]))
const manifestInstructions = new promClient.Counter({
  name: 'manifest_instructions',
  help: 'Number of Manifest instructions executed in successful transactions, top-level and CPI, by instruction name',
  labelNames: ['instruction'] as const,
});
const manifestInstructionComputeUnits = new promClient.Histogram({
  name: 'manifest_instruction_compute_units',
  help: 'Compute units consumed per Manifest instruction, including the CPIs it made',
  labelNames: ['instruction'] as const,
  // 1-2-5 series from 100 so cheap instructions still resolve; the top
  // bucket sits above any realistic single-instruction budget.
  buckets: [
    100, 200, 500, 1_000, 2_000, 5_000, 10_000, 20_000, 50_000, 100_000,
    200_000, 500_000,
  ],
});
const manifestBatchUpdateOrders = new promClient.Counter({
  name: 'manifest_batch_update_orders',
  help: 'Number of orders placed through Manifest BatchUpdate instructions, by order type',
  labelNames: ['orderType'] as const,
});
// A BatchUpdate's compute units are split evenly across the orders it placed,
// one observation per order. The shares of one batch add up to exactly its
// consumed units, so the sum over every order type divided by the count is
// the true average per order. Per-type averages are an even-split estimate,
// because the runtime does not meter orders individually.
const manifestOrderComputeUnits = new promClient.Histogram({
  name: 'manifest_order_compute_units',
  help: 'Compute units per order placed through BatchUpdate: the instruction total split evenly across its orders',
  labelNames: ['orderType'] as const,
  // Orders are expected to land between 100 and 10,000 units, so most of the
  // resolution goes there, with two buckets of headroom above.
  buckets: [
    100, 150, 200, 300, 500, 750, 1_000, 1_500, 2_000, 3_000, 5_000, 7_500,
    10_000, 15_000, 20_000,
  ],
});

/**
 * Split `total` compute units into `parts` integer shares that add up to
 * exactly `total`, with the remainder spread one unit at a time from the
 * front so no share is off by more than one.
 */
export function splitComputeUnits(total: number, parts: number): number[] {
  const base: number = Math.floor(total / parts);
  const remainder: number = total - base * parts;
  return Array.from({ length: parts }, (_, i) =>
    i < remainder ? base + 1 : base,
  );
}

export function manifestInstructionName(
  discriminator: number | undefined,
): string {
  return discriminator === undefined
    ? UNKNOWN_INSTRUCTION
    : (MANIFEST_INSTRUCTION_NAMES[discriminator] ?? UNKNOWN_INSTRUCTION);
}

/**
 * Decode the order types of the orders inside a BatchUpdate instruction.
 * Returns undefined for data the generated layout cannot parse, which a
 * successful transaction should never carry.
 */
function decodeBatchUpdateOrderTypes(data: Uint8Array): string[] | undefined {
  try {
    const [decoded] = BatchUpdateStruct.deserialize(Buffer.from(data));
    return decoded.params.orders.map(
      (order) => OrderType[order.orderType] ?? UNKNOWN_INSTRUCTION,
    );
  } catch (error) {
    console.warn('Failed to decode BatchUpdate instruction data:', error);
    return undefined;
  }
}

/**
 * Every Manifest instruction the transaction executed, in execution order,
 * with the compute units its invocation consumed where the logs still hold
 * that line. Instruction order from the transaction matches the invocation
 * order in the logs, which is how the two are associated.
 */
export function summarizeManifestInstructions(
  tx: any,
): ManifestInstructionSummary[] {
  const manifestProgramId: string = PROGRAM_ID.toBase58();
  const computeUnits: Map<number, number> = extractProgramComputeUnits(
    tx.meta?.logMessages ?? [],
    manifestProgramId,
  );
  const summaries: ManifestInstructionSummary[] = [];
  let invocationIndex: number = 0;
  for (const ix of executedInstructions(tx)) {
    if (ix.programId !== manifestProgramId) {
      continue;
    }
    const summary: ManifestInstructionSummary = {
      instruction: manifestInstructionName(ix.data[0]),
    };
    const consumed: number | undefined = computeUnits.get(invocationIndex++);
    if (consumed !== undefined) {
      summary.computeUnits = consumed;
    }
    if (ix.data[0] === batchUpdateInstructionDiscriminator) {
      const orderTypes: string[] | undefined = decodeBatchUpdateOrderTypes(
        ix.data,
      );
      if (orderTypes) {
        summary.orderTypes = orderTypes;
      }
    }
    summaries.push(summary);
  }
  return summaries;
}

/**
 * Record instruction counts and compute units for a successful transaction.
 * Never throws: a transaction this cannot decode is logged and skipped so
 * metrics cannot take down the fill feed.
 */
export function recordManifestInstructionMetrics(
  tx: any,
): ManifestInstructionSummary[] {
  let summaries: ManifestInstructionSummary[];
  try {
    summaries = summarizeManifestInstructions(tx);
  } catch (error) {
    console.warn('Failed to summarize Manifest instructions:', error);
    return [];
  }
  for (const { instruction, computeUnits, orderTypes } of summaries) {
    manifestInstructions.inc({ instruction });
    if (computeUnits !== undefined) {
      manifestInstructionComputeUnits.observe({ instruction }, computeUnits);
    }
    for (const orderType of orderTypes ?? []) {
      manifestBatchUpdateOrders.inc({ orderType });
    }
    if (computeUnits !== undefined && orderTypes && orderTypes.length > 0) {
      const shares: number[] = splitComputeUnits(
        computeUnits,
        orderTypes.length,
      );
      orderTypes.forEach((orderType, i) => {
        manifestOrderComputeUnits.observe({ orderType }, shares[i]);
      });
    }
  }
  return summaries;
}
