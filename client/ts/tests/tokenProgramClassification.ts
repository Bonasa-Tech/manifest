import { assert } from 'chai';
import { TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID } from '@solana/spl-token';
import { PublicKey } from '@solana/web3.js';
import { isToken2022Program } from '../src/client';

describe('token program classification', () => {
  it('uses mint ownership rather than extension data', () => {
    assert.isFalse(isToken2022Program(TOKEN_PROGRAM_ID));
    assert.isTrue(isToken2022Program(TOKEN_2022_PROGRAM_ID));
  });

  it('rejects unsupported mint owners', () => {
    const unsupportedProgram: PublicKey = new PublicKey(
      new Uint8Array(32).fill(7),
    );
    assert.throws(() => isToken2022Program(unsupportedProgram));
  });
});
