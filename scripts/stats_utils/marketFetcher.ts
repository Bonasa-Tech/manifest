import {
  Connection,
  GetProgramAccountsResponse,
  AccountInfo,
} from '@solana/web3.js';
import { ManifestClient } from '../../client/ts/src';
import { MANIFEST_PROGRAM_ID, MARKET_DISCRIMINATOR } from './constants';

export const MAX_TRACKED_MARKETS: number = 5_000;

export function enforceMarketAccountLimit(
  marketProgramAccounts: GetProgramAccountsResponse,
  maximumMarkets: number = MAX_TRACKED_MARKETS,
): GetProgramAccountsResponse {
  if (marketProgramAccounts.length > maximumMarkets) {
    throw new RangeError(
      `RPC returned ${marketProgramAccounts.length} markets; refusing to track more than ${maximumMarkets}`,
    );
  }
  return marketProgramAccounts;
}

/**
 * Fetch all market program accounts from the Manifest program
 * Tries to get full account data first, falls back to pubkeys only if that fails
 */
export async function fetchMarketProgramAccounts(
  connection: Connection,
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
  return enforceMarketAccountLimit(marketProgramAccounts);
}
