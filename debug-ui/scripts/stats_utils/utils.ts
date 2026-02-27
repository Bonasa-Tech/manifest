import { sleep } from '@/lib/util';
import {
  KNOWN_AGGREGATORS,
  SOL_MINT,
  CBBTC_MINT,
  WBTC_MINT,
  STABLECOIN_MINTS,
} from './constants';
import {
  AccountInfo,
  GetProgramAccountsResponse,
  PublicKey,
} from '@solana/web3.js';
import { Market } from '@bonasa-tech/manifest-sdk';

/**
 * Retry a database operation with exponential backoff
 */
export async function withRetry<T>(
  operation: () => Promise<T>,
  maxRetries = 3,
  delay = 1000,
): Promise<T> {
  let lastError;
  for (let attempt = 0; attempt < maxRetries; attempt++) {
    try {
      return await operation();
    } catch (error) {
      console.error(
        `Database operation failed (attempt ${attempt + 1}/${maxRetries}):`,
        error,
      );
      lastError = error;
      if (attempt < maxRetries - 1) {
        await sleep(delay * Math.pow(2, attempt)); // Exponential backoff
      }
    }
  }
  throw lastError;
}

/**
 * Check if an address is a known aggregator
 */
export function isKnownAggregator(address: string): boolean {
  return KNOWN_AGGREGATORS.has(address);
}

/**
 * Resolve the actual trader address, accounting for known aggregators
 */
export function resolveActualTrader(
  trader: string,
  originalSigner?: string,
): string {
  if (originalSigner && isKnownAggregator(trader)) {
    return originalSigner;
  }
  return trader;
}

/**
 * Split an array into chunks of a given size
 */
export function chunks<T>(array: T[], size: number): T[][] {
  return Array.apply(0, new Array(Math.ceil(array.length / size))).map(
    (_, index) => array.slice(index * size, (index + 1) * size),
  );
}

/**
 * Fetch BTC price from CoinGecko as fallback
 */
export async function fetchBtcPriceFromCoinGecko(): Promise<number> {
  try {
    const response = await fetch(
      'https://api.coingecko.com/api/v3/simple/price?ids=bitcoin&vs_currencies=usd',
    );
    const data = await response.json();
    return data.bitcoin?.usd || 0;
  } catch (error) {
    console.error('Error fetching BTC price from CoinGecko:', error);
    return 0;
  }
}

/**
 * Send a Discord webhook notification
 */
export async function sendDiscordNotification(
  webhookUrl: string,
  message: string,
  options: {
    title?: string;
    color?: number;
    timestamp?: boolean;
  } = {},
): Promise<void> {
  try {
    const embed: any = {
      description: message,
      color: options.color || 15158332, // Red color for alerts
    };

    if (options.title) {
      embed.title = options.title;
    }

    if (options.timestamp) {
      embed.timestamp = new Date().toISOString();
    }

    const payload = {
      embeds: [embed],
    };

    const response = await fetch(webhookUrl, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify(payload),
    });

    if (!response.ok) {
      console.error(
        `Failed to send Discord notification: ${response.status} ${response.statusText}`,
      );
    }
  } catch (error) {
    console.error('Error sending Discord notification:', error);
  }
}

/**
 * Calculate lifetime volume across all markets in USDC equivalent
 * @param marketProgramAccounts - Array of market program accounts
 * @param solPrice - SOL price in USDC (normalized)
 * @param cbbtcPrice - CBBTC price in USDC (normalized)
 * @returns Total lifetime volume in USDC equivalent
 */
export function getLifetimeVolumeForMarkets(
  marketProgramAccounts: GetProgramAccountsResponse,
  solPrice: number,
  cbbtcPrice: number,
): number {
  return marketProgramAccounts
    .map(
      (
        value: Readonly<{
          account: AccountInfo<Buffer>;
          pubkey: PublicKey;
        }>,
      ) => {
        try {
          const marketPk: string = value.pubkey.toBase58();
          const market: Market = Market.loadFromBuffer({
            buffer: value.account.data,
            address: new PublicKey(marketPk),
          });
          const quoteMint = market.quoteMint().toBase58();

          // Track stablecoin quote volume directly (USDC, USDT, PYUSD, USDS, USD1, CASH, USDG)
          if (STABLECOIN_MINTS.has(quoteMint)) {
            return Number(market.quoteVolume()) / 10 ** market.quoteDecimals();
          }

          // Convert SOL quote volume to USDC equivalent
          if (quoteMint == SOL_MINT && solPrice > 0) {
            const solVolumeNormalized =
              Number(market.quoteVolume()) / 10 ** market.quoteDecimals();
            return solVolumeNormalized * solPrice;
          }

          // Convert CBBTC/WBTC quote volume to USDC equivalent
          if (
            (quoteMint == CBBTC_MINT || quoteMint == WBTC_MINT) &&
            cbbtcPrice > 0
          ) {
            const cbbtcVolumeNormalized =
              Number(market.quoteVolume()) / 10 ** market.quoteDecimals();
            return cbbtcVolumeNormalized * cbbtcPrice;
          }

          return 0;
        } catch (err) {
          console.error('Error processing market account:', err);
          return 0;
        }
      },
    )
    .reduce((sum, num) => sum + num, 0);
}
