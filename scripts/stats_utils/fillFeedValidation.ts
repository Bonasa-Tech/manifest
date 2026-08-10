import { createPublicKey, verify } from 'crypto';
import { PublicKey } from '@solana/web3.js';
import { FillLogResult } from '../../client/ts/src';

export type SignedFillEnvelope = {
  fill: FillLogResult;
  feedSignature: string;
};

const REQUIRED_KEYS = [
  'market',
  'maker',
  'taker',
  'baseAtoms',
  'quoteAtoms',
  'priceAtoms',
  'takerIsBuy',
  'isMakerGlobal',
  'makerSequenceNumber',
  'takerSequenceNumber',
  'slot',
  'signature',
] as const;
const OPTIONAL_KEYS = new Set([
  'invocationIndex',
  'originalSigner',
  'aggregator',
  'originatingProtocol',
  'signers',
  'blockTime',
]);
const U64_MAX = (1n << 64n) - 1n;

export function canonicalJson(value: unknown): string {
  if (Array.isArray(value)) {
    return `[${value.map(canonicalJson).join(',')}]`;
  }
  if (value !== null && typeof value === 'object') {
    return `{${Object.entries(value as Record<string, unknown>)
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([key, item]) => `${JSON.stringify(key)}:${canonicalJson(item)}`)
      .join(',')}}`;
  }
  return JSON.stringify(value);
}

function requirePublicKey(
  value: unknown,
  field: string,
): asserts value is string {
  if (typeof value !== 'string') throw new Error(`${field} must be a string`);
  try {
    new PublicKey(value);
  } catch {
    throw new Error(`${field} must be a valid public key`);
  }
}

function requireU64(value: unknown, field: string): asserts value is string {
  if (typeof value !== 'string' || !/^(0|[1-9][0-9]*)$/.test(value)) {
    throw new Error(`${field} must be a canonical unsigned integer string`);
  }
  if (BigInt(value) > U64_MAX) throw new Error(`${field} exceeds u64`);
}

export function validateSignedFillEnvelope(
  input: unknown,
  feedPublicKeyPem: string,
  allowedMarkets: ReadonlySet<string>,
): FillLogResult {
  if (input === null || typeof input !== 'object' || Array.isArray(input)) {
    throw new Error('fill-feed message must be an object');
  }
  const envelope = input as Record<string, unknown>;
  if (
    Object.keys(envelope).length !== 2 ||
    !('fill' in envelope) ||
    !('feedSignature' in envelope)
  ) {
    throw new Error('fill-feed envelope has unexpected fields');
  }
  if (typeof envelope.feedSignature !== 'string') {
    throw new Error('feedSignature must be base64');
  }
  if (
    envelope.fill === null ||
    typeof envelope.fill !== 'object' ||
    Array.isArray(envelope.fill)
  ) {
    throw new Error('fill must be an object');
  }
  const fill = envelope.fill as Record<string, unknown>;
  const allowedKeys = new Set<string>([...REQUIRED_KEYS, ...OPTIONAL_KEYS]);
  for (const key of Object.keys(fill)) {
    if (!allowedKeys.has(key)) throw new Error(`unexpected fill field: ${key}`);
  }
  for (const key of REQUIRED_KEYS) {
    if (!(key in fill)) throw new Error(`missing fill field: ${key}`);
  }

  for (const field of ['market', 'maker', 'taker'] as const) {
    requirePublicKey(fill[field], field);
  }
  if (fill.originalSigner !== undefined) {
    requirePublicKey(fill.originalSigner, 'originalSigner');
  }
  if (fill.signers !== undefined) {
    if (!Array.isArray(fill.signers) || fill.signers.length > 32) {
      throw new Error('signers must be an array with at most 32 entries');
    }
    fill.signers.forEach((signer, index) =>
      requirePublicKey(signer, `signers[${index}]`),
    );
  }
  for (const field of [
    'baseAtoms',
    'quoteAtoms',
    'makerSequenceNumber',
    'takerSequenceNumber',
  ] as const) {
    requireU64(fill[field], field);
  }
  if (!Number.isSafeInteger(fill.slot) || (fill.slot as number) < 0) {
    throw new Error('slot must be a non-negative safe integer');
  }
  if (
    typeof fill.priceAtoms !== 'number' ||
    !Number.isFinite(fill.priceAtoms)
  ) {
    throw new Error('priceAtoms must be finite');
  }
  if (
    typeof fill.takerIsBuy !== 'boolean' ||
    typeof fill.isMakerGlobal !== 'boolean'
  ) {
    throw new Error('fill side flags must be booleans');
  }
  if (typeof fill.signature !== 'string' || fill.signature.length > 128) {
    throw new Error('transaction signature is invalid');
  }
  if (!allowedMarkets.has(fill.market as string)) {
    throw new Error('fill market is not in the configured market allowlist');
  }

  const valid = verify(
    null,
    Buffer.from(canonicalJson(fill)),
    createPublicKey(feedPublicKeyPem),
    Buffer.from(envelope.feedSignature, 'base64'),
  );
  if (!valid) throw new Error('fill-feed signature is invalid');
  return fill as FillLogResult;
}

export function canonicalFillIdentity(fill: FillLogResult): string {
  return `${fill.signature}:${fill.invocationIndex ?? 0}:${fill.takerSequenceNumber}:${fill.makerSequenceNumber}`;
}
