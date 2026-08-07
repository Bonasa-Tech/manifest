import { assert } from 'chai';
import {
  isAuthorizedBearer,
  isValidSolanaSignature,
  parseBoundedQueryInteger,
} from '../../../scripts/stats_utils/httpValidation';

describe('stats HTTP validation', () => {
  it('accepts bounded integer query values', () => {
    assert.equal(
      parseBoundedQueryInteger(undefined, 100, 1, 500, 'limit'),
      100,
    );
    assert.equal(parseBoundedQueryInteger('500', 100, 1, 500, 'limit'), 500);
  });

  it('rejects malformed or excessive query values', () => {
    for (const value of ['-1', '1.5', '501', 'not-a-number']) {
      assert.throws(() =>
        parseBoundedQueryInteger(value, 100, 1, 500, 'limit'),
      );
    }
  });

  it('caps public orderbook depth', () => {
    assert.equal(parseBoundedQueryInteger('500', 100, 1, 500, 'depth'), 500);
    for (const value of ['0', '501', 'Infinity']) {
      assert.throws(() =>
        parseBoundedQueryInteger(value, 100, 1, 500, 'depth'),
      );
    }
  });

  it('compares bearer credentials without accepting partial tokens', () => {
    assert.isTrue(
      isAuthorizedBearer('Bearer operator-secret', 'operator-secret'),
    );
    assert.isFalse(isAuthorizedBearer('Bearer operator', 'operator-secret'));
    assert.isFalse(isAuthorizedBearer(undefined, 'operator-secret'));
  });

  it('validates base58 transaction signatures', () => {
    assert.isTrue(isValidSolanaSignature('1'.repeat(64)));
    assert.isFalse(isValidSolanaSignature('0'.repeat(64)));
    assert.isFalse(isValidSolanaSignature('1'.repeat(63)));
  });
});
