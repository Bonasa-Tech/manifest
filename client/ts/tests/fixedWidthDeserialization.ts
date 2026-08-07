import { assert } from 'chai';
import { BaseAtoms } from '../src/manifest/accounts/BaseAtoms';
import { GlobalAtoms } from '../src/manifest/accounts/GlobalAtoms';
import { QuoteAtoms } from '../src/manifest/accounts/QuoteAtoms';
import { QuoteAtomsPerBaseAtom } from '../src/manifest/accounts/QuoteAtomsPerBaseAtom';

describe('fixed-width deserialization', () => {
  for (const [name, account] of Object.entries({
    BaseAtoms,
    GlobalAtoms,
    QuoteAtoms,
    QuoteAtomsPerBaseAtom,
  })) {
    it(`rejects truncated ${name} values`, () => {
      assert.throws(() => account.deserialize(Buffer.alloc(7)), /truncated/);
      assert.throws(() => account.deserialize(Buffer.alloc(8), 1), /truncated/);
    });
  }
});
