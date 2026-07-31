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
