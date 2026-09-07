import { bignum } from '@metaplex-foundation/beet';
import { BN } from 'bn.js';

/**
 * Converts a beet.bignum to a number.
 *
 * @param n The number to convert
 */
export function toNum(n: bignum): number {
  let target: number;
  if (typeof n === 'number') {
    target = n;
  } else {
    target = n.toString() as any as number;
  }
  return target;
}

/**
 * Converts a beet.bignum to a bigint without precision loss. Use this instead
 * of toNum when summing atom quantities that can exceed Number.MAX_SAFE_INTEGER
 * (2^53), e.g. vault balances on high-supply markets.
 *
 * @param n The number to convert
 */
export function toBigInt(n: bignum): bigint {
  if (typeof n === 'number') {
    return BigInt(n);
  }
  return BigInt(n.toString());
}

/**
 * Convert a UI token amount to atoms without silently rounding a value that
 * JavaScript cannot represent exactly. Tiny sub-atom errors introduced by
 * ordinary floating-point arithmetic are rounded; meaningful fractional atoms
 * are rejected by default. Callers that intentionally accept UI amounts
 * between atom boundaries must select an explicit rounding mode. Callers
 * receive a number when the atom value is a safe integer and a BN otherwise,
 * so valid u64 instruction amounts never pass through an imprecise number.
 */
export type TokenAmountRoundingMode = 'reject' | 'floor' | 'round';

export function tokenAmountToAtoms(
  amountTokens: number,
  decimals: number,
  roundingMode: TokenAmountRoundingMode = 'reject',
): bignum {
  if (!Number.isFinite(amountTokens) || amountTokens < 0) {
    throw new RangeError('Token amount must be a finite non-negative number');
  }
  if (!Number.isSafeInteger(decimals) || decimals < 0 || decimals > 255) {
    throw new RangeError('Mint decimals must be an integer between 0 and 255');
  }

  // Prefer the number's canonical decimal representation. This avoids
  // introducing an error by multiplying valid literals such as
  // 260.337344506 by 10**9 in binary floating point.
  const decimalMatch: RegExpMatchArray | null = amountTokens
    .toString()
    .match(/^(\d+)(?:\.(\d+))?(?:e([+-]?\d+))?$/i);
  if (decimalMatch === null) {
    throw new RangeError('Token amount could not be represented as decimal');
  }
  const fraction: string = decimalMatch[2] ?? '';
  const decimalExponent: number = Number(decimalMatch[3] ?? '0');
  const digits: bigint = BigInt(`${decimalMatch[1]}${fraction}`);
  const atomExponent: number = decimals + decimalExponent - fraction.length;
  let exactAtoms: bigint | undefined;
  if (atomExponent >= 0) {
    exactAtoms = digits * 10n ** BigInt(atomExponent);
  } else {
    const divisor: bigint = 10n ** BigInt(-atomExponent);
    if (digits % divisor === 0n) {
      exactAtoms = digits / divisor;
    }
  }
  if (exactAtoms !== undefined) {
    return atomsBigIntToBignum(exactAtoms);
  }

  if (roundingMode !== 'reject') {
    // atomExponent is negative here because non-negative exponents are exact.
    // Round the number's canonical decimal value with integer arithmetic so
    // the result does not depend on a second binary floating-point operation.
    const divisor: bigint = 10n ** BigInt(-atomExponent);
    const wholeAtoms: bigint = digits / divisor;
    const remainder: bigint = digits % divisor;
    const roundedAtoms: bigint =
      roundingMode === 'round' && remainder * 2n >= divisor
        ? wholeAtoms + 1n
        : wholeAtoms;
    return atomsBigIntToBignum(roundedAtoms);
  }

  // Computed values can stringify with a tiny tail (3 * 0.1 becomes
  // 0.30000000000000004). Permit a few ULPs of multiplication noise, but cap
  // the allowance below half an atom so a meaningful fractional atom can
  // never be rounded into a signed instruction.
  const scaledAtoms: number = amountTokens * 10 ** decimals;
  const roundedAtoms: number = Math.round(scaledAtoms);
  const subAtomError: number = Math.abs(scaledAtoms - roundedAtoms);
  const relativeFloatingPointNoiseAtoms: number =
    Math.abs(scaledAtoms) * Number.EPSILON * 4;
  const maxFloatingPointNoiseAtoms: number = Math.min(
    0.25,
    Math.max(1e-6, relativeFloatingPointNoiseAtoms),
  );
  if (subAtomError > maxFloatingPointNoiseAtoms) {
    throw new RangeError(
      `Token amount has more than ${decimals} decimal places`,
    );
  }
  return atomsBigIntToBignum(BigInt(roundedAtoms));
}

type BNInstance = InstanceType<typeof BN>;

const U64_MAX: bigint = (1n << 64n) - 1n;

function atomsBigIntToBignum(atoms: bigint): bignum {
  if (atoms > U64_MAX) {
    throw new RangeError('Token amount exceeds unsigned 64-bit atom range');
  }
  return atoms <= BigInt(Number.MAX_SAFE_INTEGER)
    ? Number(atoms)
    : new BN(atoms.toString());
}

const BN_NUMBER_MAX: BNInstance = new BN(2 ** 48 - 1);
const BN_10: BNInstance = new BN(10);

/**
 * Converts a beet.bignum to a number after dividing by 10**18
 *
 * @param n The number to convert
 */
export function convertU128(n: bignum): number {
  if (typeof n === 'number') {
    return n;
  }

  let mantissa: BNInstance = n.clone();
  for (let exponent: number = -18; exponent < 20; exponent += 1) {
    if (mantissa.lte(BN_NUMBER_MAX)) {
      return mantissa.toNumber() * 10 ** exponent;
    }
    mantissa = mantissa.div(BN_10);
  }

  throw 'unreachable';
}
