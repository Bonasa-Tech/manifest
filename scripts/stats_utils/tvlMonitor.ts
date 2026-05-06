import {
  Connection,
  PublicKey,
  ParsedAccountData,
  AccountInfo,
} from '@solana/web3.js';
import { ManifestClient } from '../../client/ts/src/client';
import { getVaultAddress } from '../../client/ts/src/utils/market';
import { getGlobalVaultAddress } from '../../client/ts/src/utils/global';
import { sendDiscordNotification } from './utils';
import {
  MANIFEST_PROGRAM_ID,
  SOL_MINT,
  USDC_MINT,
  USDT_MINT,
  PYUSD_MINT,
} from './constants';
import bs58 from 'bs58';

// Mints to monitor for TVL changes
const MONITORED_MINTS: { [symbol: string]: string } = {
  SOL: SOL_MINT,
  USDC: USDC_MINT,
  USDT: USDT_MINT,
  PYUSD: PYUSD_MINT,
};

// TVL change threshold (10%)
const TVL_CHANGE_THRESHOLD = 0.1;

// Global discriminator for filtering accounts
const GLOBAL_DISCRIMINANT = Buffer.from([
  1, 170, 151, 47, 187, 160, 180, 149,
]);
const GLOBAL_DISCRIMINANT_BASE58 = bs58.encode(GLOBAL_DISCRIMINANT);

export interface TvlSnapshot {
  timestamp: number;
  tvlByMint: Map<string, bigint>; // mint -> atoms
}

export class TvlMonitor {
  private connection: Connection;
  private discordWebhookUrl: string | undefined;
  private previousSnapshot: TvlSnapshot | null = null;

  constructor(connection: Connection, discordWebhookUrl?: string) {
    this.connection = connection;
    this.discordWebhookUrl = discordWebhookUrl;
  }

  /**
   * Fetch current TVL for all monitored mints from market vaults and global accounts
   */
  async fetchCurrentTvl(): Promise<TvlSnapshot> {
    const tvlByMint = new Map<string, bigint>();

    // Initialize all monitored mints to 0
    for (const mint of Object.values(MONITORED_MINTS)) {
      tvlByMint.set(mint, BigInt(0));
    }

    // Fetch all market vault balances
    await this.fetchMarketVaultBalances(tvlByMint);

    // Fetch all global vault balances
    await this.fetchGlobalVaultBalances(tvlByMint);

    return {
      timestamp: Date.now(),
      tvlByMint,
    };
  }

  /**
   * Fetch balances from all market vaults for monitored mints
   */
  private async fetchMarketVaultBalances(
    tvlByMint: Map<string, bigint>,
  ): Promise<void> {
    const monitoredMintSet = new Set(Object.values(MONITORED_MINTS));

    try {
      const marketPks = await ManifestClient.listMarketPublicKeys(
        this.connection,
      );

      // Process in batches to avoid rate limiting
      const batchSize = 10;
      for (let i = 0; i < marketPks.length; i += batchSize) {
        const batch = marketPks.slice(i, i + batchSize);
        await Promise.all(
          batch.map(async (marketPk) => {
            try {
              const client = await ManifestClient.getClientReadOnly(
                this.connection,
                marketPk,
              );
              const baseMint = client.market.baseMint();
              const quoteMint = client.market.quoteMint();

              const vaultsToFetch: { mint: PublicKey; vault: PublicKey }[] = [];

              if (monitoredMintSet.has(baseMint.toBase58())) {
                vaultsToFetch.push({
                  mint: baseMint,
                  vault: getVaultAddress(marketPk, baseMint),
                });
              }
              if (monitoredMintSet.has(quoteMint.toBase58())) {
                vaultsToFetch.push({
                  mint: quoteMint,
                  vault: getVaultAddress(marketPk, quoteMint),
                });
              }

              if (vaultsToFetch.length > 0) {
                const accounts = await this.connection.getMultipleParsedAccounts(
                  vaultsToFetch.map((v) => v.vault),
                );

                for (let j = 0; j < vaultsToFetch.length; j++) {
                  const accountInfo = accounts.value[j];
                  if (accountInfo?.data) {
                    const amount = BigInt(
                      (accountInfo.data as ParsedAccountData).parsed?.info
                        ?.tokenAmount?.amount ?? '0',
                    );
                    const mintKey = vaultsToFetch[j].mint.toBase58();
                    const current = tvlByMint.get(mintKey) ?? BigInt(0);
                    tvlByMint.set(mintKey, current + amount);
                  }
                }
              }
            } catch (error) {
              console.error(
                `Error fetching market vault for ${marketPk.toBase58()}:`,
                error,
              );
            }
          }),
        );
      }
    } catch (error) {
      console.error('Error fetching market vaults:', error);
    }
  }

  /**
   * Fetch balances from all global vaults for monitored mints
   */
  private async fetchGlobalVaultBalances(
    tvlByMint: Map<string, bigint>,
  ): Promise<void> {
    try {
      // Get all global accounts
      const globalAccounts = await this.connection.getProgramAccounts(
        MANIFEST_PROGRAM_ID,
        {
          filters: [
            {
              memcmp: {
                offset: 0,
                bytes: GLOBAL_DISCRIMINANT_BASE58,
              },
            },
          ],
        },
      );

      // For each monitored mint, check if there's a global account and fetch its vault
      const monitoredMints = Object.values(MONITORED_MINTS);
      const vaultAddresses = monitoredMints.map((mint) =>
        getGlobalVaultAddress(new PublicKey(mint)),
      );

      const vaultAccounts =
        await this.connection.getMultipleParsedAccounts(vaultAddresses);

      for (let i = 0; i < monitoredMints.length; i++) {
        const accountInfo = vaultAccounts.value[i];
        if (accountInfo?.data) {
          const amount = BigInt(
            (accountInfo.data as ParsedAccountData).parsed?.info?.tokenAmount
              ?.amount ?? '0',
          );
          const mintKey = monitoredMints[i];
          const current = tvlByMint.get(mintKey) ?? BigInt(0);
          tvlByMint.set(mintKey, current + amount);
        }
      }
    } catch (error) {
      console.error('Error fetching global vaults:', error);
    }
  }

  /**
   * Check TVL changes and send alerts if threshold exceeded
   */
  async checkAndAlert(): Promise<void> {
    console.log('TVL Monitor: Starting hourly TVL check...');

    const currentSnapshot = await this.fetchCurrentTvl();

    if (this.previousSnapshot) {
      for (const [symbol, mint] of Object.entries(MONITORED_MINTS)) {
        const previousTvl = this.previousSnapshot.tvlByMint.get(mint) ?? BigInt(0);
        const currentTvl = currentSnapshot.tvlByMint.get(mint) ?? BigInt(0);

        if (previousTvl === BigInt(0)) {
          console.log(
            `TVL Monitor: ${symbol} - No previous TVL data, skipping comparison`,
          );
          continue;
        }

        // Calculate percentage change
        // Use Number conversion carefully for large values
        const previousNum = Number(previousTvl);
        const currentNum = Number(currentTvl);
        const percentChange = (currentNum - previousNum) / previousNum;
        const percentChangeAbs = Math.abs(percentChange);

        console.log(
          `TVL Monitor: ${symbol} - Previous: ${previousTvl}, Current: ${currentTvl}, Change: ${(percentChange * 100).toFixed(2)}%`,
        );

        if (percentChangeAbs >= TVL_CHANGE_THRESHOLD) {
          const direction = percentChange > 0 ? 'increased' : 'decreased';
          const emoji = percentChange > 0 ? '📈' : '📉';

          const message = [
            `**${symbol} TVL ${direction} by ${(percentChangeAbs * 100).toFixed(2)}%**`,
            `Previous: ${this.formatAtoms(previousTvl, symbol)} ${symbol}`,
            `Current: ${this.formatAtoms(currentTvl, symbol)} ${symbol}`,
            `Change: ${percentChange > 0 ? '+' : ''}${(percentChange * 100).toFixed(2)}%`,
          ].join('\n');

          console.log(`TVL Monitor: Alert triggered for ${symbol}!`);

          if (this.discordWebhookUrl) {
            await sendDiscordNotification(this.discordWebhookUrl, message, {
              title: `${emoji} TVL Alert: ${symbol}`,
              color: percentChange > 0 ? 0x00ff00 : 0xff0000,
              timestamp: true,
            });
          }
        }
      }
    } else {
      console.log('TVL Monitor: First run, storing initial snapshot');
      for (const [symbol, mint] of Object.entries(MONITORED_MINTS)) {
        const tvl = currentSnapshot.tvlByMint.get(mint) ?? BigInt(0);
        console.log(
          `TVL Monitor: ${symbol} - Initial TVL: ${this.formatAtoms(tvl, symbol)} ${symbol}`,
        );
      }
    }

    this.previousSnapshot = currentSnapshot;
    console.log('TVL Monitor: Hourly check complete');
  }

  /**
   * Format atoms to human-readable format based on mint
   */
  private formatAtoms(atoms: bigint, symbol: string): string {
    // Decimals for each token
    const decimals: { [key: string]: number } = {
      SOL: 9,
      USDC: 6,
      USDT: 6,
      PYUSD: 6,
    };

    const dec = decimals[symbol] ?? 9;
    const divisor = BigInt(10 ** dec);
    const wholePart = atoms / divisor;
    const fractionalPart = atoms % divisor;

    // Format with commas for whole part
    const wholeStr = wholePart.toLocaleString();
    const fracStr = fractionalPart.toString().padStart(dec, '0').slice(0, 2);

    return `${wholeStr}.${fracStr}`;
  }

  /**
   * Get the previous snapshot (for testing/debugging)
   */
  getPreviousSnapshot(): TvlSnapshot | null {
    return this.previousSnapshot;
  }

  /**
   * Set previous snapshot (for testing/initialization)
   */
  setPreviousSnapshot(snapshot: TvlSnapshot): void {
    this.previousSnapshot = snapshot;
  }
}
