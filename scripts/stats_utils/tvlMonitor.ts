import {
  Connection,
  PublicKey,
  ParsedAccountData,
  RpcResponseAndContext,
  AccountInfo,
  GetProgramAccountsResponse,
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

// TVL increase threshold (5x increase = 400% change)
const TVL_INCREASE_THRESHOLD: number = 4.0;
// TVL decrease threshold (80% decrease)
const TVL_DECREASE_THRESHOLD: number = 0.8;

// Persistence check delay (5 minutes in milliseconds)
const PERSISTENCE_CHECK_DELAY_MS: number = 5 * 60 * 1000;

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
  incompleteMarkets: string[];
}

interface PendingAlert {
  symbol: string;
  mint: string;
  previousTvl: bigint;
  detectedTvl: bigint;
  percentChange: number;
  detectedAt: number;
}

export class TvlMonitor {
  private static readonly MAX_COMPARISON_BASELINES: number = 32;
  private readonly connection: Connection;
  private readonly discordWebhookUrl: string | undefined;
  private comparisonBaselines: Map<string, TvlSnapshot> = new Map();
  private pendingAlerts: Map<string, PendingAlert> = new Map();

  constructor(connection: Connection, discordWebhookUrl?: string) {
    this.connection = connection;
    this.discordWebhookUrl = discordWebhookUrl;
  }

  /**
   * Sleep helper
   */
  private sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms));
  }

  private hasSameIncompleteMarkets(
    left: readonly string[],
    right: readonly string[],
  ): boolean {
    if (left.length !== right.length) {
      return false;
    }
    const rightMarkets: Set<string> = new Set(right);
    return left.every((market: string): boolean => rightMarkets.has(market));
  }

  private getComparisonKey(incompleteMarkets: readonly string[]): string {
    return [...incompleteMarkets].sort().join(',');
  }

  private rememberComparisonBaseline(
    comparisonKey: string,
    snapshot: TvlSnapshot,
  ): void {
    // Refresh insertion order so the bounded map evicts the least recently
    // used exclusion set. Keeping separate baselines prevents a transient RPC
    // failure from replacing the last complete aggregate, while a persistently
    // unreadable permissionless market can still be monitored on later runs.
    this.comparisonBaselines.delete(comparisonKey);
    this.comparisonBaselines.set(comparisonKey, snapshot);
    if (this.comparisonBaselines.size > TvlMonitor.MAX_COMPARISON_BASELINES) {
      const oldestKey: string | undefined = this.comparisonBaselines
        .keys()
        .next().value;
      if (oldestKey !== undefined) {
        this.comparisonBaselines.delete(oldestKey);
      }
    }
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
    const incompleteMarkets: string[] =
      await this.fetchMarketVaultBalances(tvlByMint);

    // Fetch all global vault balances
    await this.fetchGlobalVaultBalances(tvlByMint);

    const snapshot: TvlSnapshot = {
      timestamp: Date.now(),
      tvlByMint,
      incompleteMarkets,
    };

    return snapshot;
  }

  /**
   * Fetch balances from all market vaults for monitored mints
   */
  private async fetchMarketVaultBalances(
    tvlByMint: Map<string, bigint>,
  ): Promise<string[]> {
    const monitoredMintSet: Set<string> = new Set(
      Object.values(MONITORED_MINTS),
    );

    const marketAccounts: GetProgramAccountsResponse =
      await ManifestClient.getMarketProgramAccounts(this.connection, {
        offset: 0,
        length: 80,
      });
    const incompleteMarkets: string[] = [];

    // Process in batches to avoid rate limiting. Only vault failures for a
    // market known to contain a monitored mint affect snapshot completeness.
    const batchSize: number = 10;
    for (let i: number = 0; i < marketAccounts.length; i += batchSize) {
      const batch: GetProgramAccountsResponse = marketAccounts.slice(
        i,
        i + batchSize,
      );
      await Promise.all(
        batch.map(
          async (
            marketAccount: GetProgramAccountsResponse[number],
          ): Promise<void> => {
            const marketPk: PublicKey = marketAccount.pubkey;
            const marketData: Buffer = marketAccount.account.data;
            // MarketFixed stores baseMint at bytes 16..48 and quoteMint at
            // 48..80. Reading only these fixed fields avoids a second RPC and
            // classifies relevance before any fallible vault lookup.
            if (marketData.length < 80) {
              console.error(
                `TVL collection could not classify truncated market ${marketPk.toBase58()}`,
              );
              // Without both mint fields we cannot prove that this market is
              // irrelevant. Exclude it explicitly so a missing vault cannot
              // masquerade as a comparable TVL decrease.
              incompleteMarkets.push(marketPk.toBase58());
              return;
            }
            const baseMint: PublicKey = new PublicKey(
              marketData.subarray(16, 48),
            );
            const quoteMint: PublicKey = new PublicKey(
              marketData.subarray(48, 80),
            );

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

            // Failures for irrelevant markets cannot change any monitored TVL.
            if (vaultsToFetch.length === 0) {
              return;
            }

            try {
              const vaultPubkeys: PublicKey[] = vaultsToFetch.map(
                (v: VaultFetchInfo): PublicKey => v.vault,
              );
              const accounts: RpcResponseAndContext<
                (AccountInfo<Buffer | ParsedAccountData> | null)[]
              > = await this.connection.getMultipleParsedAccounts(vaultPubkeys);
              const marketBalances: Map<string, bigint> = new Map();

              for (let j: number = 0; j < vaultsToFetch.length; j++) {
                const accountInfo: AccountInfo<
                  Buffer | ParsedAccountData
                > | null = accounts.value[j];
                if (!accountInfo?.data) {
                  throw new Error(
                    `Vault ${vaultsToFetch[j].vault.toBase58()} was not returned by RPC`,
                  );
                }
                const parsedData: ParsedAccountData =
                  accountInfo.data as ParsedAccountData;
                const amountValue: unknown =
                  parsedData.parsed?.info?.tokenAmount?.amount;
                if (
                  typeof amountValue !== 'string' ||
                  !/^\d+$/.test(amountValue)
                ) {
                  throw new Error(
                    `Vault ${vaultsToFetch[j].vault.toBase58()} did not contain a parsed token amount`,
                  );
                }
                const amountStr: string = amountValue;
                const amount: bigint = BigInt(amountStr);
                const mintKey: string = vaultsToFetch[j].mint.toBase58();
                const currentMarketBalance: bigint =
                  marketBalances.get(mintKey) ?? BigInt(0);
                marketBalances.set(mintKey, currentMarketBalance + amount);
              }

              // Merge only after every requested vault for this market was
              // readable, so an excluded market never contributes a partial
              // balance to otherwise comparable degraded snapshots.
              for (const [mintKey, amount] of marketBalances) {
                const current: bigint = tvlByMint.get(mintKey) ?? BigInt(0);
                tvlByMint.set(mintKey, current + amount);
              }
            } catch (error: unknown) {
              console.error(
                `TVL collection skipped market ${marketPk.toBase58()}:`,
                error,
              );
              incompleteMarkets.push(marketPk.toBase58());
            }
          },
        ),
      );
    }
    return incompleteMarkets;
  }

  /**
   * Fetch balances from all global vaults for monitored mints
   */
  private async fetchGlobalVaultBalances(
    tvlByMint: Map<string, bigint>,
  ): Promise<void> {
    // For each monitored mint, fetch its global vault. A missing account is a
    // legitimate zero balance; RPC or parsing failures still reject the cycle.
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
  }

  /**
   * Check TVL changes and send alerts if threshold exceeded AND persists after 5 minutes
   */
  async checkAndAlert(): Promise<boolean> {
    const currentSnapshot: TvlSnapshot = await this.fetchCurrentTvl();
    const comparisonKey: string = this.getComparisonKey(
      currentSnapshot.incompleteMarkets,
    );
    const previousSnapshot: TvlSnapshot | undefined =
      this.comparisonBaselines.get(comparisonKey);
    if (currentSnapshot.incompleteMarkets.length > 0) {
      console.warn(
        `TVL snapshot excludes ${currentSnapshot.incompleteMarkets.length} unreadable market(s); comparing against baselines with the same exclusions`,
      );
    }

    if (previousSnapshot) {
      const entries: [string, string][] = Object.entries(MONITORED_MINTS);
      for (const [symbol, mint] of entries) {
        const previousTvl: bigint =
          previousSnapshot.tvlByMint.get(mint) ?? BigInt(0);
        const currentTvl: bigint =
          currentSnapshot.tvlByMint.get(mint) ?? BigInt(0);

        if (previousTvl === BigInt(0)) {
          continue;
        }

        // Calculate percentage change
        const previousNum: number = Number(previousTvl);
        const currentNum: number = Number(currentTvl);
        const percentChange: number = (currentNum - previousNum) / previousNum;

        // Alert on >5x increase (percentChange > 4.0) or >80% decrease (percentChange < -0.8)
        const shouldAlert: boolean =
          percentChange > TVL_INCREASE_THRESHOLD ||
          percentChange < -TVL_DECREASE_THRESHOLD;

        if (shouldAlert) {
          // Threshold exceeded - schedule persistence check
          this.pendingAlerts.set(mint, {
            symbol,
            mint,
            previousTvl,
            detectedTvl: currentTvl,
            percentChange,
            detectedAt: Date.now(),
          });
        }
      }

      // Process pending alerts that need persistence check
      await this.processPendingAlerts(currentSnapshot.incompleteMarkets);
    }

    this.rememberComparisonBaseline(comparisonKey, currentSnapshot);

    // Ask the scheduler for a short retry when an exclusion set is first
    // observed. If it was transient, the complete baseline is still intact;
    // if it persists, the new set has a baseline on the retry and monitoring
    // can continue without a hot loop.
    return previousSnapshot !== undefined || comparisonKey.length === 0;
  }

  /**
   * Process pending alerts - wait 5 minutes and verify the change persists
   */
  private async processPendingAlerts(
    expectedIncompleteMarkets: readonly string[],
  ): Promise<void> {
    if (this.pendingAlerts.size === 0) {
      return;
    }

    await this.sleep(PERSISTENCE_CHECK_DELAY_MS);

    // Fetch fresh TVL data
    const verificationSnapshot: TvlSnapshot = await this.fetchCurrentTvl();
    if (
      !this.hasSameIncompleteMarkets(
        expectedIncompleteMarkets,
        verificationSnapshot.incompleteMarkets,
      )
    ) {
      console.warn(
        'TVL unreadable-market set changed during verification; discarding incomparable pending alerts',
      );
      this.pendingAlerts.clear();
      return;
    }

    for (const [mint, pendingAlert] of this.pendingAlerts) {
      const { symbol, previousTvl, percentChange } = pendingAlert;
      const verificationTvl: bigint =
        verificationSnapshot.tvlByMint.get(mint) ?? BigInt(0);

      // Check if the change still persists
      const previousNum: number = Number(previousTvl);
      const verificationNum: number = Number(verificationTvl);
      const currentPercentChange: number =
        (verificationNum - previousNum) / previousNum;

      const wasIncrease: boolean = percentChange > 0;
      const stillIncreased: boolean = currentPercentChange > 0;
      const wasDecrease: boolean = percentChange < 0;
      const stillDecreased: boolean = currentPercentChange < 0;

      // Alert only if direction matches and still exceeds threshold
      const stillExceedsThreshold: boolean =
        currentPercentChange > TVL_INCREASE_THRESHOLD ||
        currentPercentChange < -TVL_DECREASE_THRESHOLD;
      const directionPersists: boolean =
        (wasIncrease && stillIncreased) || (wasDecrease && stillDecreased);

      if (stillExceedsThreshold && directionPersists) {
        const direction: string =
          currentPercentChange > 0 ? 'increased' : 'decreased';
        const emoji: string = currentPercentChange > 0 ? '📈' : '📉';

        const message: string = [
          `**${symbol} TVL ${direction} by ${(Math.abs(currentPercentChange) * 100).toFixed(2)}%** (persisted after 5 min)`,
          `Previous: ${this.formatAtoms(previousTvl, symbol)} ${symbol}`,
          `Current: ${this.formatAtoms(verificationTvl, symbol)} ${symbol}`,
          `Change: ${currentPercentChange > 0 ? '+' : ''}${(currentPercentChange * 100).toFixed(2)}%`,
          ...(expectedIncompleteMarkets.length > 0
            ? [
                `Degraded snapshot: excludes ${expectedIncompleteMarkets.length} consistently unreadable market(s)`,
              ]
            : []),
        ].join('\n');

        if (this.discordWebhookUrl) {
          try {
            await sendDiscordNotification(this.discordWebhookUrl, message, {
              title: `${emoji} TVL Alert: ${symbol}`,
              color: currentPercentChange > 0 ? 0x00ff00 : 0xff0000,
              timestamp: true,
            });
          } catch (error: unknown) {
            // Alert delivery is best-effort. A webhook outage must not retain
            // the old comparison baseline or replay the same TVL movement on
            // every later cycle.
            console.error(`Failed to send ${symbol} TVL alert:`, error);
          }
        }
      }
    }

    // Clear pending alerts after processing
    this.pendingAlerts.clear();
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
}
