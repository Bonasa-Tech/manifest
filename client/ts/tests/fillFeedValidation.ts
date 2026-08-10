import { expect } from 'chai';
import { generateKeyPairSync, sign } from 'crypto';
import {
  canonicalJson,
  validateSignedFillEnvelope,
} from '../../../scripts/stats_utils/fillFeedValidation';
import { FillLogResult } from '../src';

describe('signed fill-feed validation', () => {
  const fill: FillLogResult = {
    market: 'So11111111111111111111111111111111111111112',
    maker: 'So11111111111111111111111111111111111111112',
    taker: 'So11111111111111111111111111111111111111112',
    baseAtoms: '1',
    quoteAtoms: '2',
    priceAtoms: 2,
    takerIsBuy: true,
    isMakerGlobal: false,
    makerSequenceNumber: '3',
    takerSequenceNumber: '4',
    slot: 5,
    signature: 'transaction-signature',
  };
  const { publicKey, privateKey } = generateKeyPairSync('ed25519');
  const publicKeyPem = publicKey
    .export({ type: 'spki', format: 'pem' })
    .toString();

  function envelope(value: FillLogResult = fill) {
    return {
      fill: value,
      feedSignature: sign(
        null,
        Buffer.from(canonicalJson(value)),
        privateKey,
      ).toString('base64'),
    };
  }

  it('accepts a signed, allowlisted fill', () => {
    expect(
      validateSignedFillEnvelope(
        envelope(),
        publicKeyPem,
        new Set([fill.market]),
      ),
    ).to.deep.equal(fill);
  });

  it('rejects tampering, unknown fields, and unknown markets', () => {
    expect(() =>
      validateSignedFillEnvelope(
        { ...envelope(), fill: { ...fill, quoteAtoms: '9' } },
        publicKeyPem,
        new Set([fill.market]),
      ),
    ).to.throw(/signature/);
    expect(() =>
      validateSignedFillEnvelope(
        envelope({ ...fill, unexpected: true } as FillLogResult),
        publicKeyPem,
        new Set([fill.market]),
      ),
    ).to.throw(/unexpected fill field/);
    expect(() =>
      validateSignedFillEnvelope(envelope(), publicKeyPem, new Set()),
    ).to.throw(/allowlist/);
  });
});
