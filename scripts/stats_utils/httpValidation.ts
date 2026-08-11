import { timingSafeEqual } from 'crypto';

export function parseBoundedQueryInteger(
  value: unknown,
  defaultValue: number,
  minimum: number,
  maximum: number,
  name: string,
): number {
  if (value === undefined) {
    return defaultValue;
  }
  if (typeof value !== 'string' || !/^\d+$/.test(value)) {
    throw new RangeError(`${name} must be a non-negative integer`);
  }
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < minimum || parsed > maximum) {
    throw new RangeError(
      `${name} must be an integer between ${minimum} and ${maximum}`,
    );
  }
  return parsed;
}

export function parseOptionalUnixTimestamp(
  value: unknown,
  name: string,
): number | undefined {
  if (value === undefined) return undefined;
  if (typeof value !== 'string' || !/^\d+$/.test(value)) {
    throw new RangeError(`${name} must be a positive integer Unix timestamp`);
  }
  const parsed: number = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < 1) {
    throw new RangeError(`${name} must be a positive integer Unix timestamp`);
  }
  return parsed;
}

export function validateUnixTimestampRange(
  startTimestamp: number | undefined,
  endTimestamp: number | undefined,
  maximumSeconds: number,
): void {
  if ((startTimestamp === undefined) !== (endTimestamp === undefined)) {
    throw new RangeError('start and end must be provided together');
  }
  if (startTimestamp === undefined || endTimestamp === undefined) return;
  if (startTimestamp > endTimestamp) {
    throw new RangeError('start must be less than or equal to end');
  }
  if (endTimestamp - startTimestamp > maximumSeconds) {
    throw new RangeError(
      `timestamp range must not exceed ${maximumSeconds} seconds`,
    );
  }
}

export function isAuthorizedBearer(
  authorization: string | undefined,
  expectedToken: string,
): boolean {
  if (!authorization?.startsWith('Bearer ') || expectedToken.length === 0) {
    return false;
  }
  const supplied = Buffer.from(authorization.slice('Bearer '.length));
  const expected = Buffer.from(expectedToken);
  return (
    supplied.length === expected.length && timingSafeEqual(supplied, expected)
  );
}

export function isValidSolanaSignature(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    value.length >= 64 &&
    value.length <= 88 &&
    /^[1-9A-HJ-NP-Za-km-z]+$/.test(value)
  );
}
