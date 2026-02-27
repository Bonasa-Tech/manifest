import {
  Connection,
  GetProgramAccountsResponse,
  AccountInfo,
  PublicKey,
} from '@solana/web3.js';
import { ManifestClient } from '@bonasa-tech/manifest-sdk';
import { MANIFEST_PROGRAM_ID, MARKET_DISCRIMINATOR } from './constants';

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

  return marketProgramAccounts;
}
