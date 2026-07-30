import { assert } from 'chai';
import { FillLogResult } from '../src/types';
import { computeInferredRemainders } from '../src/utils/inferFills';

function fill(
  invocationIndex: number,
  baseAtoms: string,
  quoteAtoms: string,
): FillLogResult {
  return {
    market: 'market',
    maker: '',
    taker: 'taker',
    baseAtoms,
    quoteAtoms,
    priceAtoms: Number(quoteAtoms) / Number(baseAtoms),
    takerIsBuy: true,
    isMakerGlobal: false,
    makerSequenceNumber: '0',
    takerSequenceNumber: '0',
    signature: 'signature',
    slot: 1,
    invocationIndex,
  };
}

describe('inferred fill reconciliation', () => {
  it('subtracts a parsed fill only from its own invocation', () => {
    const remainders = computeInferredRemainders(
      [fill(0, '10', '20'), fill(1, '30', '60')],
      [fill(0, '4', '8')],
    );

    assert.deepEqual(
      remainders.map(({ invocationIndex, baseAtoms, quoteAtoms }) => ({
        invocationIndex,
        baseAtoms,
        quoteAtoms,
      })),
      [
        { invocationIndex: 0, baseAtoms: '6', quoteAtoms: '12' },
        { invocationIndex: 1, baseAtoms: '30', quoteAtoms: '60' },
      ],
    );
  });
});
