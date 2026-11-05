import { assert } from 'chai';
import { GetProgramAccountsResponse, PublicKey } from '@solana/web3.js';
import {
  isAuthorizedBearer,
  isValidSolanaSignature,
  parseBoundedQueryInteger,
  parseOptionalUnixTimestamp,
  positiveIntFromEnv,
  validateUnixTimestampRange,
} from '../../../scripts/stats_utils/httpValidation';
import { truncateMarketAccounts } from '../../../scripts/stats_utils/marketFetcher';

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

  it('truncates an RPC market collection above the configured bound', () => {
    const syntheticAccounts: GetProgramAccountsResponse = [1, 2, 3].map(
      (fill: number) => ({
        pubkey: new PublicKey(new Uint8Array(32).fill(fill)),
        account: {
          data: Buffer.alloc(0),
          executable: false,
          lamports: 0,
          owner: PublicKey.default,
        },
      }),
    );
    const priorityPk: string = syntheticAccounts[2].pubkey.toBase58();
    const isPriority = (marketPk: string): boolean => marketPk === priorityPk;

    const kept: GetProgramAccountsResponse = truncateMarketAccounts(
      syntheticAccounts,
      isPriority,
      2,
    );
    assert.lengthOf(kept, 2);
    // Priority accounts survive truncation ahead of earlier non-priority ones.
    assert.equal(kept[0].pubkey.toBase58(), priorityPk);
    assert.equal(
      kept[1].pubkey.toBase58(),
      syntheticAccounts[0].pubkey.toBase58(),
    );
    // At or under the bound, the collection passes through untouched.
    assert.strictEqual(
      truncateMarketAccounts(syntheticAccounts, isPriority, 3),
      syntheticAccounts,
    );
  });

  it('falls back to the default for invalid capacity env values', () => {
    const name = 'STATS_TEST_CAPACITY';
    delete process.env[name];
    assert.equal(positiveIntFromEnv(name, 40), 40);
    for (const value of ['', ' ', '0', '-1', '1.5', 'lots']) {
      process.env[name] = value;
      assert.equal(positiveIntFromEnv(name, 40), 40);
    }
    process.env[name] = '64';
    assert.equal(positiveIntFromEnv(name, 40), 64);
    delete process.env[name];
  });
});
