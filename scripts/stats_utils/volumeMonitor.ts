import { FillLogResult, Market } from '../../client/ts/src';
import { sendDiscordNotification } from './utils';
import { SOL_MINT, STABLECOIN_MINTS } from './constants';

// Volume decrease threshold (90% decrease to trigger alert)
const VOLUME_DECREASE_THRESHOLD: number = 0.9;
// Volume increase threshold (10x increase = 900% change)
const VOLUME_INCREASE_THRESHOLD: number = 9.0;

// Large fill threshold in USDC ($1 million)
const LARGE_FILL_THRESHOLD_USDC: number = 1_000_000;

// Time to wait before finalizing a transaction's fills (ms)
const TRANSACTION_FINALIZE_DELAY_MS: number = 5_000;

// Type for hourly volume snapshot
interface HourlyVolumeSnapshot {
  timestamp: number;
  totalVolumeUsdc: number;
}

// Type for fill value calculation result
interface FillValueResult {
  valueUsdc: number;
  symbol: string;
}

// Type for buffered transaction fills
interface TransactionFillBuffer {
  fills: FillLogResult[];
  totalValueUsdc: number;
  lastSeenMs: number;
  markets: Set<string>;
  taker: string;
  aggregator?: string;
}

export class VolumeMonitor {
  private readonly discordWebhookUrl: string | undefined;
  private previousHourlyVolume: HourlyVolumeSnapshot | null = null;
  private currentHourVolumeUsdc: number = 0;
  private currentHourStartTime: number = Date.now();

  // Buffer for aggregating fills by transaction signature
  private transactionBuffer: Map<string, TransactionFillBuffer> = new Map();

  // Callback to get SOL price from stats server
  private readonly getSolPrice: () => number;

  // Callback to get market object from stats server
  private readonly getMarket: (marketPk: string) => Market | undefined;

  // Callback to get ticker symbols from stats server
  private readonly getTicker: (marketPk: string) => [string, string] | undefined;

  constructor(
    discordWebhookUrl: string | undefined,
    getSolPrice: () => number,
    getMarket: (marketPk: string) => Market | undefined,
    getTicker: (marketPk: string) => [string, string] | undefined,
  ) {
    this.discordWebhookUrl = discordWebhookUrl;
    this.getSolPrice = getSolPrice;
    this.getMarket = getMarket;
    this.getTicker = getTicker;
  }

  /**
   * Process a fill and check for large fill alerts.
   * Aggregates fills by transaction signature before checking threshold.
   * Also accumulates volume for hourly tracking.
   */
  async processFill(fill: FillLogResult): Promise<void> {
    const market: Market | undefined = this.getMarket(fill.market);
    if (!market) {
      return;
    }

    const fillValue: FillValueResult | null = this.calculateFillValueUsdc(
      fill,
      market,
    );
    if (!fillValue) {
      return;
    }

    // Accumulate hourly volume
    this.currentHourVolumeUsdc += fillValue.valueUsdc;

    // Finalize any old transactions before processing new fill
    await this.finalizeOldTransactions();

    // Buffer this fill by transaction signature
    const signature: string = fill.signature;
    const existing: TransactionFillBuffer | undefined =
      this.transactionBuffer.get(signature);

    if (existing) {
      existing.fills.push(fill);
      existing.totalValueUsdc += fillValue.valueUsdc;
      existing.lastSeenMs = Date.now();
      existing.markets.add(fillValue.symbol);
    } else {
      this.transactionBuffer.set(signature, {
        fills: [fill],
        totalValueUsdc: fillValue.valueUsdc,
        lastSeenMs: Date.now(),
        markets: new Set([fillValue.symbol]),
        taker: fill.taker,
        aggregator: fill.aggregator,
      });
    }
  }

  /**
   * Finalize transactions that haven't received new fills recently.
   * Sends alerts for transactions exceeding the threshold.
   */
  private async finalizeOldTransactions(): Promise<void> {
    const now: number = Date.now();
    const signaturestoFinalize: string[] = [];

    this.transactionBuffer.forEach((buffer, signature) => {
      if (now - buffer.lastSeenMs >= TRANSACTION_FINALIZE_DELAY_MS) {
        signaturestoFinalize.push(signature);
      }
    });

    for (const signature of signaturestoFinalize) {
      const buffer: TransactionFillBuffer | undefined =
        this.transactionBuffer.get(signature);
      if (buffer && buffer.totalValueUsdc >= LARGE_FILL_THRESHOLD_USDC) {
        await this.sendLargeTransactionAlert(signature, buffer);
      }
      this.transactionBuffer.delete(signature);
    }
  }

  /**
   * Calculate the USDC equivalent value of a fill
   */
  private calculateFillValueUsdc(
    fill: FillLogResult,
    market: Market,
  ): FillValueResult | null {
    const quoteMint: string = market.quoteMint().toBase58();
    const quoteDecimals: number = market.quoteDecimals();
    const quoteAtoms: number = Number(fill.quoteAtoms);
    const quoteTokens: number = quoteAtoms / 10 ** quoteDecimals;

    // Get ticker symbol for display
    const ticker: [string, string] | undefined = this.getTicker(fill.market);
    const symbol: string = ticker ? `${ticker[0]}/${ticker[1]}` : fill.market.slice(0, 8);

    // If quote is a stablecoin, use 1:1 conversion
    if (STABLECOIN_MINTS.has(quoteMint)) {
      return { valueUsdc: quoteTokens, symbol };
    }

    // If quote is SOL, convert using SOL price
    if (quoteMint === SOL_MINT) {
      const solPrice: number = this.getSolPrice();
      if (solPrice > 0) {
        return { valueUsdc: quoteTokens * solPrice, symbol };
      }
    }

    // Cannot determine USDC value for other quote mints
    return null;
  }

  /**
   * Send alert for a large transaction (aggregated fills)
   */
  private async sendLargeTransactionAlert(
    signature: string,
    buffer: TransactionFillBuffer,
  ): Promise<void> {
    if (!this.discordWebhookUrl) {
      return;
    }

    const formattedValue: string = this.formatUsdValue(buffer.totalValueUsdc);
    const marketsStr: string = Array.from(buffer.markets).join(', ');
    const fillCount: number = buffer.fills.length;

    // Determine overall side from first fill (all fills in aggregator tx have same direction)
    const firstFill: FillLogResult = buffer.fills[0];
    const side: string = firstFill.takerIsBuy ? 'BUY' : 'SELL';

    // Get original signer if available
    const originalSigner: string | undefined = (firstFill as any).originalSigner;
    const signerDisplay: string = originalSigner
      ? `\`${originalSigner.slice(0, 8)}...\``
      : `\`${buffer.taker.slice(0, 8)}...\``;

    const message: string[] = [
      `**${side} across ${fillCount} fills**`,
      `Total Value: ${formattedValue}`,
      `Markets: ${marketsStr}`,
      `Signer: ${signerDisplay}`,
    ];

    if (buffer.aggregator) {
      message.push(`Aggregator: ${buffer.aggregator}`);
    }

    message.push(`Tx: \`${signature.slice(0, 16)}...\``);

    await sendDiscordNotification(this.discordWebhookUrl, message.join('\n'), {
      title: '💰 Large Transaction Alert',
      color: 0xffd700, // Gold color
      timestamp: true,
    });
  }

  /**
   * Check hourly volume change and send alert if threshold exceeded.
   * Should be called every hour.
   */
  async checkHourlyVolumeChange(): Promise<void> {
    const currentSnapshot: HourlyVolumeSnapshot = {
      timestamp: Date.now(),
      totalVolumeUsdc: this.currentHourVolumeUsdc,
    };

    if (this.previousHourlyVolume && this.previousHourlyVolume.totalVolumeUsdc > 0) {
      const previousVolume: number = this.previousHourlyVolume.totalVolumeUsdc;
      const currentVolume: number = currentSnapshot.totalVolumeUsdc;
      const percentChange: number = (currentVolume - previousVolume) / previousVolume;

      // Alert on decreases of more than 90% or increases of more than 10x
      if (percentChange < -VOLUME_DECREASE_THRESHOLD || percentChange > VOLUME_INCREASE_THRESHOLD) {
        await this.sendVolumeChangeAlert(
          previousVolume,
          currentVolume,
          percentChange,
        );
      }
    }

    // Reset for next hour
    this.previousHourlyVolume = currentSnapshot;
    this.currentHourVolumeUsdc = 0;
    this.currentHourStartTime = Date.now();
  }

  /**
   * Send alert for hourly volume change
   */
  private async sendVolumeChangeAlert(
    previousVolume: number,
    currentVolume: number,
    percentChange: number,
  ): Promise<void> {
    if (!this.discordWebhookUrl) {
      return;
    }

    const direction: string = percentChange > 0 ? 'increased' : 'decreased';
    const emoji: string = percentChange > 0 ? '📈' : '📉';
    const percentChangeAbs: number = Math.abs(percentChange);

    const message: string = [
      `**Hourly volume ${direction} by ${(percentChangeAbs * 100).toFixed(1)}%**`,
      `Previous hour: ${this.formatUsdValue(previousVolume)}`,
      `Current hour: ${this.formatUsdValue(currentVolume)}`,
      `Change: ${percentChange > 0 ? '+' : ''}${(percentChange * 100).toFixed(1)}%`,
    ].join('\n');

    await sendDiscordNotification(this.discordWebhookUrl, message, {
      title: `${emoji} Hourly Volume Alert`,
      color: percentChange > 0 ? 0x00ff00 : 0xff0000,
      timestamp: true,
    });
  }

  /**
   * Format USD value with appropriate suffix (K, M, B)
   */
  private formatUsdValue(value: number): string {
    if (value >= 1_000_000_000) {
      return `$${(value / 1_000_000_000).toFixed(2)}B`;
    }
    if (value >= 1_000_000) {
      return `$${(value / 1_000_000).toFixed(2)}M`;
    }
    if (value >= 1_000) {
      return `$${(value / 1_000).toFixed(2)}K`;
    }
    return `$${value.toFixed(2)}`;
  }

  /**
   * Format number with commas
   */
  private formatNumber(value: number): string {
    if (value >= 1_000_000) {
      return `${(value / 1_000_000).toFixed(2)}M`;
    }
    if (value >= 1_000) {
      return `${(value / 1_000).toFixed(2)}K`;
    }
    return value.toLocaleString(undefined, { maximumFractionDigits: 4 });
  }

  /**
   * Get current hour's accumulated volume (for debugging/monitoring)
   */
  getCurrentHourVolume(): number {
    return this.currentHourVolumeUsdc;
  }
}
