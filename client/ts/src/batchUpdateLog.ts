import { BatchUpdateLog } from './manifest/accounts/BatchUpdateLog';
import {
  CancelOrderLogEntry,
  cancelOrderLogEntryBeet,
} from './manifest/types/CancelOrderLogEntry';
import {
  PlaceOrderLogEntry,
  placeOrderLogEntryBeet,
} from './manifest/types/PlaceOrderLogEntry';

/** Discriminant that starts a `BatchUpdateLog` program data entry. */
export const BATCH_UPDATE_LOG_DISCRIMINANT: Buffer = Buffer.from([
  184, 213, 71, 201, 110, 248, 249, 131,
]);

/**
 * A decoded batch update event: the header plus the cancelled and placed
 * orders it carries. Batch updates emit one of these instead of one
 * `CancelOrderLog`/`PlaceOrderLog` per order.
 */
export interface DecodedBatchUpdateLog {
  header: BatchUpdateLog;
  cancels: CancelOrderLogEntry[];
  orders: PlaceOrderLogEntry[];
}

/**
 * Decodes the payload of a `Program data:` entry (discriminant included).
 * Returns null when the discriminant is not `BatchUpdateLog`.
 */
export function decodeBatchUpdateLog(
  data: Buffer,
): DecodedBatchUpdateLog | null {
  if (
    data.length < 8 ||
    !data.subarray(0, 8).equals(BATCH_UPDATE_LOG_DISCRIMINANT)
  ) {
    return null;
  }
  const [header, initialOffset] = BatchUpdateLog.deserialize(data, 8);
  let offset: number = initialOffset;
  const cancels: CancelOrderLogEntry[] = [];
  for (let i = 0; i < header.numCancels; i++) {
    const [entry, next] = cancelOrderLogEntryBeet.deserialize(data, offset);
    cancels.push(entry);
    offset = next;
  }
  const orders: PlaceOrderLogEntry[] = [];
  for (let i = 0; i < header.numOrders; i++) {
    const [entry, next] = placeOrderLogEntryBeet.deserialize(data, offset);
    orders.push(entry);
    offset = next;
  }
  return { header, cancels, orders };
}
