import { expect } from 'chai';
import { safeAtomsToNumber } from '../../../scripts/stats_utils/safeAtoms';

describe('safe atom conversion', () => {
  it('preserves safe integer atoms exactly', () => {
    expect(safeAtomsToNumber('9007199254740991')).to.equal(
      Number.MAX_SAFE_INTEGER,
    );
  });

  it('rejects u64 atoms that JavaScript would round', () => {
    expect(() => safeAtomsToNumber('9007199254740993')).to.throw(RangeError);
    expect(() => safeAtomsToNumber('18446744073709551615')).to.throw(
      RangeError,
    );
  });
});
