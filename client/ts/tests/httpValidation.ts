import { assert } from 'chai';
import { GetProgramAccountsResponse } from '@solana/web3.js';
import {
  isAuthorizedBearer,
  isValidSolanaSignature,
  parseBoundedQueryInteger,
  parseOptionalUnixTimestamp,
  validateUnixTimestampRange,
} from '../../../scripts/stats_utils/httpValidation';
import { enforceMarketAccountLimit } from '../../../scripts/stats_utils/marketFetcher';

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

  it('bounds explicit timestamp query windows', () => {
    const maximumSeconds: number = 31 * 24 * 60 * 60;
    const start: number | undefined = parseOptionalUnixTimestamp(
      '100',
      'start',
    );
    const end: number | undefined = parseOptionalUnixTimestamp('200', 'end');
    assert.doesNotThrow(() =>
      validateUnixTimestampRange(start, end, maximumSeconds),
    );
    assert.throws(() =>
      validateUnixTimestampRange(1, maximumSeconds + 2, maximumSeconds),
    );
    assert.throws(() =>
      validateUnixTimestampRange(undefined, end, maximumSeconds),
    );
    assert.throws(() => parseOptionalUnixTimestamp('1.5', 'start'));
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

  it('rejects an RPC market collection above the configured bound', () => {
    const syntheticAccounts: GetProgramAccountsResponse = new Array(3).fill(
      {},
    ) as GetProgramAccountsResponse;
    assert.throws(
      () => enforceMarketAccountLimit(syntheticAccounts, 2),
      /refusing to track more than 2/,
    );
    assert.lengthOf(enforceMarketAccountLimit(syntheticAccounts, 3), 3);
  });
});
