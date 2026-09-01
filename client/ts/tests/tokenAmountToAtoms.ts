import { expect } from 'chai';
import { tokenAmountToAtoms } from '../src/utils/numbers';

describe('tokenAmountToAtoms', () => {
  it('converts representable token amounts without floating-point drift', () => {
    expect(tokenAmountToAtoms(0.29, 2)).to.equal(29);
    expect(tokenAmountToAtoms(1.000001, 6)).to.equal(1_000_001);
    expect(tokenAmountToAtoms(1e-6, 6)).to.equal(1);
    expect(tokenAmountToAtoms(3 * 0.1, 6)).to.equal(300_000);
  });

  it('rejects excess precision and unsafe atom values', () => {
    expect(() => tokenAmountToAtoms(0.0000001, 6)).to.throw(RangeError);
    expect(() =>
      tokenAmountToAtoms(Number.MAX_SAFE_INTEGER / 100 + 1, 2),
    ).to.throw(RangeError);
  });
});
