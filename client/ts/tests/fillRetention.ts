import { expect } from 'chai';
import { FillLogResult } from '../src';
import {
  appendUniqueFill,
  fillIdentity,
} from '../../../scripts/stats_utils/fillRetention';

function testFill(sequence: string): FillLogResult {
  return {
    market: 'market',
    maker: 'maker',
    taker: 'taker',
    baseAtoms: '1',
    quoteAtoms: '2',
    priceAtoms: 2,
    takerIsBuy: true,
    isMakerGlobal: false,
    makerSequenceNumber: sequence,
    takerSequenceNumber: sequence,
    slot: 42,
    signature: 'signature',
    invocationIndex: 0,
  };
}

describe('fill retention', () => {
  it('deduplicates replayed on-chain fills', () => {
    const fills: FillLogResult[] = [];
    const fill: FillLogResult = testFill('1');

    expect(appendUniqueFill(fills, fill, 2, 'test')).to.equal(true);
    expect(appendUniqueFill(fills, fill, 2, 'test')).to.equal(false);
    expect(fills).to.deep.equal([fill]);
    expect(fillIdentity(fills[0])).to.equal(fillIdentity(fill));
  });

  it('rejects growth beyond the configured bound', () => {
    const fills: FillLogResult[] = [testFill('1'), testFill('2')];

    expect(() => appendUniqueFill(fills, testFill('3'), 2, 'test')).to.throw(
      'too many fills retained for test',
    );
    expect(fills).to.have.length(2);
  });
});
