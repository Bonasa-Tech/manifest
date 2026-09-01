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
 * JavaScript cannot represent exactly. Callers needing larger u64 quantities
 * should construct instructions from integer atoms with a bigint/BN API.
 */
export function tokenAmountToAtoms(
  amountTokens: number,
  decimals: number,
): number {
  if (!Number.isFinite(amountTokens) || amountTokens < 0) {
    throw new RangeError('Token amount must be a finite non-negative number');
  }
  if (!Number.isSafeInteger(decimals) || decimals < 0 || decimals > 255) {
    throw new RangeError('Mint decimals must be an integer between 0 and 255');
  }

  // Parse the number's canonical decimal representation rather than doing a
  // binary floating-point multiplication. This keeps values such as 0.29
  // exact and makes excess decimal precision an explicit error.
  const match: RegExpMatchArray | null = amountTokens
    .toString()
    .match(/^(\d+)(?:\.(\d+))?(?:e([+-]?\d+))?$/i);
  if (!match) {
    throw new RangeError('Token amount could not be represented as decimal');
  }
  const fraction: string = match[2] ?? '';
  const decimalExponent: number = Number(match[3] ?? '0');
  const digits: bigint = BigInt(`${match[1]}${fraction}`);
  const atomExponent: number = decimals + decimalExponent - fraction.length;

  let atoms: bigint;
  if (atomExponent >= 0) {
    atoms = digits * 10n ** BigInt(atomExponent);
  } else {
    const divisor: bigint = 10n ** BigInt(-atomExponent);
    if (digits % divisor !== 0n) {
      throw new RangeError(
        `Token amount has more than ${decimals} decimal places`,
      );
    }
    atoms = digits / divisor;
  }

  if (atoms > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw new RangeError(
      'Token amount exceeds JavaScript safe-integer atom precision',
    );
  }
  return Number(atoms);
}

type BNInstance = InstanceType<typeof BN>;

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
