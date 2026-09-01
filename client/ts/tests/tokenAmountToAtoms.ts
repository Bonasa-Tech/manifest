import { expect } from 'chai';
import { tokenAmountToAtoms } from '../src/utils/numbers';

describe('tokenAmountToAtoms', () => {
  it('converts representable token amounts without floating-point drift', () => {
    expect(tokenAmountToAtoms(0.29, 2)).to.equal(29);
    expect(tokenAmountToAtoms(1.000001, 6)).to.equal(1_000_001);
    expect(tokenAmountToAtoms(1e-6, 6)).to.equal(1);
    expect(tokenAmountToAtoms(3 * 0.1, 6)).to.equal(300_000);
    expect(tokenAmountToAtoms(260.337344506, 9)).to.equal(260_337_344_506);
    expect(tokenAmountToAtoms(8545.518514384, 9)).to.equal(8_545_518_514_384);
  });

  it('rejects excess precision and unsafe atom values', () => {
    expect(() => tokenAmountToAtoms(0.0000001, 6)).to.throw(RangeError);
    expect(() => tokenAmountToAtoms(0.5, 0)).to.throw(RangeError);
    expect(() =>
      tokenAmountToAtoms(Number.MAX_SAFE_INTEGER / 100 + 1, 2),
    ).to.throw(RangeError);
  });

  it('requires callers to choose how meaningful fractional atoms round', () => {
    expect(tokenAmountToAtoms(0.3333333333333333, 6, 'floor')).to.equal(
      333_333,
    );
    expect(tokenAmountToAtoms(1.9, 0, 'floor')).to.equal(1);
    expect(tokenAmountToAtoms(1.4, 0, 'round')).to.equal(1);
    expect(tokenAmountToAtoms(1.5, 0, 'round')).to.equal(2);
  });
});
