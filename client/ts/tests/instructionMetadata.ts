import { PublicKey } from '@solana/web3.js';
import { assert } from 'chai';
import {
  createBatchUpdateInstruction,
  createGlobalEvictInstruction,
} from '../src/manifest/instructions';

const key = (value: number): PublicKey =>
  new PublicKey(new Uint8Array(32).fill(value));

describe('generated instruction metadata', () => {
  it('marks BatchUpdate transfer vaults writable', () => {
    const instruction = createBatchUpdateInstruction(
      {
        payer: key(1),
        market: key(2),
        baseMint: key(3),
        baseGlobal: key(4),
        baseGlobalVault: key(5),
        baseMarketVault: key(6),
        baseTokenProgram: key(7),
        quoteMint: key(8),
        quoteGlobal: key(9),
        quoteGlobalVault: key(10),
        quoteMarketVault: key(11),
        quoteTokenProgram: key(12),
      },
      {
        params: {
          traderIndexHint: null,
          cancels: [],
          orders: [],
        },
      },
    );

    for (const index of [5, 6, 10, 11]) {
      assert.isTrue(instruction.keys[index].isWritable);
    }
  });

  it('marks GlobalEvict transfer token accounts writable', () => {
    const instruction = createGlobalEvictInstruction(
      {
        payer: key(1),
        global: key(2),
        mint: key(3),
        globalVault: key(4),
        traderToken: key(5),
        evicteeToken: key(6),
      },
      { params: { amountAtoms: 1 } },
    );

    assert.isTrue(instruction.keys[4].isWritable);
    assert.isTrue(instruction.keys[5].isWritable);
  });
});
