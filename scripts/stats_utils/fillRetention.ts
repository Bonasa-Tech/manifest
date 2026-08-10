import { FillLogResult } from '../../client/ts/src';

export function fillIdentity(fill: FillLogResult): string {
  return `${fill.signature}:${fill.invocationIndex ?? 0}:${fill.takerSequenceNumber}:${fill.makerSequenceNumber}`;
}

/**
 * Retain each on-chain fill once while enforcing a hard memory bound. The
 * trusted fill feed may legitimately replay events after reconnecting; this is
 * resource management, not validation of the feed's contents.
 */
export function appendUniqueFill(
  fills: FillLogResult[],
  fill: FillLogResult,
  maximumFills: number,
  scope: string,
): boolean {
  const identity: string = fillIdentity(fill);
  const isDuplicate: boolean = fills.some(
    (candidate: FillLogResult) => fillIdentity(candidate) === identity,
  );
  if (isDuplicate) return false;
  if (fills.length >= maximumFills) {
    throw new Error(`too many fills retained for ${scope}`);
  }
  fills.push(fill);
  return true;
}
