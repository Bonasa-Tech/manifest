import {
  Connection,
  PublicKey,
  ParsedAccountData,
  RpcResponseAndContext,
  AccountInfo,
} from '@solana/web3.js';
import { ManifestClient } from '../../client/ts/src/client';
import { getVaultAddress } from '../../client/ts/src/utils/market';
import { getGlobalVaultAddress } from '../../client/ts/src/utils/global';
import { sendDiscordNotification } from './utils';
import { SOL_MINT, USDC_MINT, USDT_MINT, PYUSD_MINT } from './constants';

// Type for monitored mints mapping
type MonitoredMintsMap = { readonly [symbol: string]: string };

// Mints to monitor for TVL changes
const MONITORED_MINTS: MonitoredMintsMap = {
  SOL: SOL_MINT,
  USDC: USDC_MINT,
  USDT: USDT_MINT,
  PYUSD: PYUSD_MINT,
} as const;

// TVL change threshold (10%)
const TVL_CHANGE_THRESHOLD: number = 0.1;

// Type for vault fetch info
interface VaultFetchInfo {
  mint: PublicKey;
  vault: PublicKey;
}

// Type for token decimals mapping
type TokenDecimalsMap = { readonly [symbol: string]: number };

const TOKEN_DECIMALS: TokenDecimalsMap = {
  SOL: 9,
  USDC: 6,
  USDT: 6,
  PYUSD: 6,
} as const;

export interface TvlSnapshot {
  timestamp: number;
  tvlByMint: Map<string, bigint>; // mint -> atoms
}

export class TvlMonitor {
  private readonly connection: Connection;
  private readonly discordWebhookUrl: string | undefined;
  private previousSnapshot: TvlSnapshot | null = null;

  constructor(connection: Connection, discordWebhookUrl?: string) {
    this.connection = connection;
    this.discordWebhookUrl = discordWebhookUrl;
  }

  /**
   * Fetch current TVL for all monitored mints from market vaults and global accounts
   */
  async fetchCurrentTvl(): Promise<TvlSnapshot> {
    const tvlByMint: Map<string, bigint> = new Map<string, bigint>();

    // Initialize all monitored mints to 0
    const mintAddresses: string[] = Object.values(MONITORED_MINTS);
    for (const mint of mintAddresses) {
      tvlByMint.set(mint, BigInt(0));
    }

    // Fetch all market vault balances
    await this.fetchMarketVaultBalances(tvlByMint);

    // Fetch all global vault balances
    await this.fetchGlobalVaultBalances(tvlByMint);

    const snapshot: TvlSnapshot = {
      timestamp: Date.now(),
      tvlByMint,
    };

    return snapshot;
  }

  /**
   * Fetch balances from all market vaults for monitored mints
   */
  private async fetchMarketVaultBalances(
    tvlByMint: Map<string, bigint>,
  ): Promise<void> {
    const monitoredMintSet: Set<string> = new Set(
      Object.values(MONITORED_MINTS),
    );

    try {
      const marketPks: PublicKey[] = await ManifestClient.listMarketPublicKeys(
        this.connection,
      );

      // Process in batches to avoid rate limiting
      const batchSize: number = 10;
      for (let i: number = 0; i < marketPks.length; i += batchSize) {
        const batch: PublicKey[] = marketPks.slice(i, i + batchSize);
        await Promise.all(
          batch.map(async (marketPk: PublicKey): Promise<void> => {
            try {
              const client: ManifestClient =
                await ManifestClient.getClientReadOnly(
                  this.connection,
                  marketPk,
                );
              const baseMint: PublicKey = client.market.baseMint();
              const quoteMint: PublicKey = client.market.quoteMint();

              const vaultsToFetch: VaultFetchInfo[] = [];

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
                const vaultPubkeys: PublicKey[] = vaultsToFetch.map(
                  (v: VaultFetchInfo): PublicKey => v.vault,
                );
                const accounts: RpcResponseAndContext<
                  (AccountInfo<Buffer | ParsedAccountData> | null)[]
                > = await this.connection.getMultipleParsedAccounts(vaultPubkeys);

                for (let j: number = 0; j < vaultsToFetch.length; j++) {
                  const accountInfo: AccountInfo<
                    Buffer | ParsedAccountData
                  > | null = accounts.value[j];
                  if (accountInfo?.data) {
                    const parsedData: ParsedAccountData =
                      accountInfo.data as ParsedAccountData;
                    const amountStr: string =
                      parsedData.parsed?.info?.tokenAmount?.amount ?? '0';
                    const amount: bigint = BigInt(amountStr);
                    const mintKey: string = vaultsToFetch[j].mint.toBase58();
                    const current: bigint = tvlByMint.get(mintKey) ?? BigInt(0);
                    tvlByMint.set(mintKey, current + amount);
                  }
                }
              }
            } catch (error: unknown) {
              console.error(
                `Error fetching market vault for ${marketPk.toBase58()}:`,
                error,
              );
            }
          }),
        );
      }
    } catch (error: unknown) {
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
      // For each monitored mint, fetch its global vault
      const monitoredMints: string[] = Object.values(MONITORED_MINTS);
      const vaultAddresses: PublicKey[] = monitoredMints.map(
        (mint: string): PublicKey => getGlobalVaultAddress(new PublicKey(mint)),
      );

      const vaultAccounts: RpcResponseAndContext<
        (AccountInfo<Buffer | ParsedAccountData> | null)[]
      > = await this.connection.getMultipleParsedAccounts(vaultAddresses);

      for (let i: number = 0; i < monitoredMints.length; i++) {
        const accountInfo: AccountInfo<Buffer | ParsedAccountData> | null =
          vaultAccounts.value[i];
        if (accountInfo?.data) {
          const parsedData: ParsedAccountData =
            accountInfo.data as ParsedAccountData;
          const amountStr: string =
            parsedData.parsed?.info?.tokenAmount?.amount ?? '0';
          const amount: bigint = BigInt(amountStr);
          const mintKey: string = monitoredMints[i];
          const current: bigint = tvlByMint.get(mintKey) ?? BigInt(0);
          tvlByMint.set(mintKey, current + amount);
        }
      }
    } catch (error: unknown) {
      console.error('Error fetching global vaults:', error);
    }
  }

  /**
   * Check TVL changes and send alerts if threshold exceeded
   */
  async checkAndAlert(): Promise<void> {
    console.log('TVL Monitor: Starting hourly TVL check...');

    const currentSnapshot: TvlSnapshot = await this.fetchCurrentTvl();

    if (this.previousSnapshot) {
      const entries: [string, string][] = Object.entries(MONITORED_MINTS);
      for (const [symbol, mint] of entries) {
        const previousTvl: bigint =
          this.previousSnapshot.tvlByMint.get(mint) ?? BigInt(0);
        const currentTvl: bigint =
          currentSnapshot.tvlByMint.get(mint) ?? BigInt(0);

        if (previousTvl === BigInt(0)) {
          console.log(
            `TVL Monitor: ${symbol} - No previous TVL data, skipping comparison`,
          );
          continue;
        }

        // Calculate percentage change
        // Use Number conversion carefully for large values
        const previousNum: number = Number(previousTvl);
        const currentNum: number = Number(currentTvl);
        const percentChange: number = (currentNum - previousNum) / previousNum;
        const percentChangeAbs: number = Math.abs(percentChange);

        console.log(
          `TVL Monitor: ${symbol} - Previous: ${previousTvl}, Current: ${currentTvl}, Change: ${(percentChange * 100).toFixed(2)}%`,
        );

        if (percentChangeAbs >= TVL_CHANGE_THRESHOLD) {
          const direction: string =
            percentChange > 0 ? 'increased' : 'decreased';
          const emoji: string = percentChange > 0 ? '📈' : '📉';

          const message: string = [
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
      const entries: [string, string][] = Object.entries(MONITORED_MINTS);
      for (const [symbol, mint] of entries) {
        const tvl: bigint = currentSnapshot.tvlByMint.get(mint) ?? BigInt(0);
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
    const dec: number = TOKEN_DECIMALS[symbol] ?? 9;
    const divisor: bigint = BigInt(10 ** dec);
    const wholePart: bigint = atoms / divisor;
    const fractionalPart: bigint = atoms % divisor;

    // Format with commas for whole part
    const wholeStr: string = wholePart.toLocaleString();
    const fracStr: string = fractionalPart
      .toString()
      .padStart(dec, '0')
      .slice(0, 2);

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
