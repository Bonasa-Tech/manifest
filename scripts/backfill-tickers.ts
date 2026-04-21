import { Connection, PublicKey } from '@solana/web3.js';
import { MANIFEST_PROGRAM_ID, MARKET_DISCRIMINATOR } from './stats_utils/constants';
import { Market } from '@bonasa-tech/manifest-sdk';
import { getVaultAddress } from '@bonasa-tech/manifest-sdk/utils';

const STATS_SERVER_URL = 'https://mfx-stats-mainnet.fly.dev';
const PRIMARY_RPC_URL = 'https://rpc.shyft.to?api_key=hji2tMNbrRzaTuyn';
const FALLBACK_RPC_URL = 'https://rpc.shyft.to?api_key=PyFQrhOnpzF4wRAk';

// Rate limiting - be conservative to avoid RPC spam
const DELAY_BETWEEN_MARKETS_MS = 500;
const DELAY_BETWEEN_SIGNATURE_CHECKS_MS = 200;

async function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function fetchAllMarkets(connection: Connection): Promise<
  { pubkey: PublicKey; baseVault: PublicKey }[]
> {
  console.log('Fetching all Manifest markets...');

  const accounts = await connection.getProgramAccounts(MANIFEST_PROGRAM_ID, {
    filters: [
      {
        memcmp: {
          offset: 0,
          bytes: MARKET_DISCRIMINATOR.toString('base64'),
          encoding: 'base64',
        },
      },
    ],
  });

  console.log(`Found ${accounts.length} markets`);

  const markets: { pubkey: PublicKey; baseVault: PublicKey }[] = [];

  for (const account of accounts) {
    try {
      const market = Market.loadFromBuffer({
        address: account.pubkey,
        buffer: account.account.data,
      });
      const baseMint = market.baseMint();
      const baseVault = getVaultAddress(account.pubkey, baseMint);
      markets.push({
        pubkey: account.pubkey,
        baseVault,
      });
    } catch (error) {
      console.error(`Failed to deserialize market ${account.pubkey.toBase58()}:`, error);
    }
  }

  return markets;
}

async function findFirstFillSignature(
  connection: Connection,
  baseVault: PublicKey,
): Promise<string | null> {
  try {
    // Get signatures for the base vault - limit to 10 in case some don't have fills
    const signatures = await connection.getSignaturesForAddress(baseVault, {
      limit: 10,
    });

    if (signatures.length === 0) {
      return null;
    }

    // Return the first signature (most recent)
    return signatures[0].signature;
  } catch (error) {
    console.error(`Failed to get signatures for vault ${baseVault.toBase58()}:`, error);
    return null;
  }
}

async function backfillSignature(signature: string): Promise<boolean> {
  try {
    const url = `${STATS_SERVER_URL}/backfill?signature=${signature}`;
    const response = await fetch(url);

    if (!response.ok) {
      console.error(`Backfill failed for ${signature}: ${response.status} ${response.statusText}`);
      return false;
    }

    const result = await response.json();
    return result.success === true;
  } catch (error) {
    console.error(`Failed to backfill ${signature}:`, error);
    return false;
  }
}

async function run() {
  let connection = new Connection(PRIMARY_RPC_URL, 'confirmed');

  let markets: { pubkey: PublicKey; baseVault: PublicKey }[];

  try {
    markets = await fetchAllMarkets(connection);
  } catch (error) {
    console.error('Primary RPC failed, trying fallback...');
    connection = new Connection(FALLBACK_RPC_URL, 'confirmed');
    markets = await fetchAllMarkets(connection);
  }

  console.log(`\nProcessing ${markets.length} markets to find fills...\n`);

  let successCount = 0;
  let noFillsCount = 0;
  let errorCount = 0;

  for (let i = 0; i < markets.length; i++) {
    const market = markets[i];
    const marketPk = market.pubkey.toBase58();

    process.stdout.write(`[${i + 1}/${markets.length}] ${marketPk}... `);

    await sleep(DELAY_BETWEEN_SIGNATURE_CHECKS_MS);

    const signature = await findFirstFillSignature(connection, market.baseVault);

    if (!signature) {
      console.log('no signatures found');
      noFillsCount++;
      continue;
    }

    process.stdout.write(`found sig ${signature.slice(0, 8)}... `);

    const success = await backfillSignature(signature);

    if (success) {
      console.log('backfilled!');
      successCount++;
    } else {
      console.log('backfill failed');
      errorCount++;
    }

    await sleep(DELAY_BETWEEN_MARKETS_MS);
  }

  console.log('\n--- Summary ---');
  console.log(`Total markets: ${markets.length}`);
  console.log(`Successfully backfilled: ${successCount}`);
  console.log(`No signatures found: ${noFillsCount}`);
  console.log(`Errors: ${errorCount}`);
}

run().catch((error) => {
  console.error('Fatal error:', error);
  process.exit(1);
});
