import {
  Connection,
  GetProgramAccountsResponse,
  AccountInfo,
} from '@solana/web3.js';
import { ManifestClient } from '../../client/ts/src';
import { MANIFEST_PROGRAM_ID, MARKET_DISCRIMINATOR } from './constants';

// Backstop against unbounded memory growth, not a tuning knob. Mainnet crossed
// 5,000 live market accounts on 2026-10-30. Startup truncates to this bound
// (markets with saved volume kept first) and dead markets are evicted between
// the watermarks below, so exceeding it on chain degrades to "quietest markets
// untracked" rather than a startup crash or silently dropped fills.
export const MAX_TRACKED_MARKETS: number = 10_000;

// Eviction watermarks. Once the tracked set grows past the high water mark,
// markets with no volume in the last 24 hours are evicted down to the low
// water mark. The gap between the low water mark and MAX_TRACKED_MARKETS is
// the burst headroom for brand-new markets between checkpoint advances.
export const MARKET_EVICTION_HIGH_WATER: number = Math.floor(
  MAX_TRACKED_MARKETS * 0.9,
);
export const MARKET_EVICTION_LOW_WATER: number = Math.floor(
  MAX_TRACKED_MARKETS * 0.8,
);

// Bound an RPC market collection to maximumMarkets, keeping accounts the
// caller marks as priority (e.g. markets with saved volume history) ahead of
// the rest. Relative order is otherwise preserved.
export function truncateMarketAccounts(
  marketProgramAccounts: GetProgramAccountsResponse,
  isPriority: (marketPk: string) => boolean,
  maximumMarkets: number = MAX_TRACKED_MARKETS,
): GetProgramAccountsResponse {
  if (marketProgramAccounts.length <= maximumMarkets) {
    return marketProgramAccounts;
  }
  const priority: GetProgramAccountsResponse[number][] = [];
  const rest: GetProgramAccountsResponse[number][] = [];
  for (const account of marketProgramAccounts) {
    (isPriority(account.pubkey.toBase58()) ? priority : rest).push(account);
  }
  console.error(
    `RPC returned ${marketProgramAccounts.length} markets; tracking ${maximumMarkets} ` +
      `(${priority.length} priority) and dropping ${marketProgramAccounts.length - maximumMarkets}. ` +
      'Dropped markets are re-tracked lazily on their next fill once eviction frees room.',
  );
  return priority.concat(rest).slice(0, maximumMarkets);
}

/**
 * Fetch all market program accounts from the Manifest program
 * Tries to get full account data first, falls back to pubkeys only if that fails
 */
export async function fetchMarketProgramAccounts(
  connection: Connection,
  isPriority: (marketPk: string) => boolean = () => false,
): Promise<GetProgramAccountsResponse> {
  let marketProgramAccounts: GetProgramAccountsResponse;

  try {
    marketProgramAccounts =
      await ManifestClient.getMarketProgramAccounts(connection);
  } catch (error) {
    console.error(
      'Failed to get market program accounts with data, retrying with pubkeys only:',
      error,
    );

    // Fallback: Get pubkeys only without data
    try {
      const marketPubkeys = await connection.getProgramAccounts(
        MANIFEST_PROGRAM_ID,
        {
          dataSlice: { offset: 0, length: 0 }, // Request no data, just pubkeys
          filters: [
            {
              memcmp: {
                offset: 0,
                bytes: MARKET_DISCRIMINATOR.toString('base64'),
                encoding: 'base64',
              },
            },
          ],
        },
      );

      // Create dummy accounts with empty data for initialization
      marketProgramAccounts = marketPubkeys.map(({ pubkey }) => ({
        pubkey,
        account: {
          data: Buffer.alloc(0), // Empty buffer
          executable: false,
          lamports: 0,
          owner: MANIFEST_PROGRAM_ID,
        } as AccountInfo<Buffer>,
      }));

      console.log(
        `Initialized with ${marketProgramAccounts.length} market pubkeys (no data)`,
      );
    } catch (fallbackError) {
      console.error('Fallback pubkey-only request also failed:', fallbackError);
      console.log('Initializing with empty markets');
      marketProgramAccounts = [];
    }
  }

  // Apply the limit after both RPC paths so a size violation cannot be caught
  // as a transport failure and silently replaced with another unbounded result.
  return truncateMarketAccounts(marketProgramAccounts, isPriority);
}
