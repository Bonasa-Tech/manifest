import { PublicKey } from '@solana/web3.js';
import { assert } from 'chai';
import { ManifestClient } from '../src/client';
import { getGlobalAddress } from '../src/utils/global';

const key = (value: number): PublicKey =>
  new PublicKey(new Uint8Array(32).fill(value));

function clientWithInitializedGlobals(): ManifestClient {
  const client = Object.create(ManifestClient.prototype) as ManifestClient;

  Object.assign(client, {
    wrapper: { address: key(1) },
    payer: key(2),
    market: { address: key(3) },
    baseMint: { address: key(4) },
    quoteMint: { address: key(5) },
    baseGlobal: {},
    quoteGlobal: {},
    isBase22: false,
    isQuote22: false,
  });

  return client;
}

function assertIncludesBothGlobals(instructionKeys: PublicKey[]): void {
  assert.deepInclude(
    instructionKeys,
    getGlobalAddress(key(4)),
    'base global should be included',
  );
  assert.deepInclude(
    instructionKeys,
    getGlobalAddress(key(5)),
    'quote global should be included',
  );
}

describe('batchUpdate cancellation accounts', () => {
  it('includes all initialized globals for cancel-all without replacements', () => {
    const instruction = clientWithInitializedGlobals().batchUpdateIx(
      [],
      [],
      true,
    );

    assertIncludesBothGlobals(instruction.keys.map(({ pubkey }) => pubkey));
  });

  it('includes all initialized globals for selective cancellation', () => {
    const instruction = clientWithInitializedGlobals().batchUpdateIx(
      [],
      [{ clientOrderId: 1 }],
      false,
    );

    assertIncludesBothGlobals(instruction.keys.map(({ pubkey }) => pubkey));
  });

  it('does not add globals to an empty non-cancel batch', () => {
    const instruction = clientWithInitializedGlobals().batchUpdateIx(
      [],
      [],
      false,
    );
    const instructionKeys = instruction.keys.map(({ pubkey }) => pubkey);

    assert.notDeepInclude(instructionKeys, getGlobalAddress(key(4)));
    assert.notDeepInclude(instructionKeys, getGlobalAddress(key(5)));
  });
});
