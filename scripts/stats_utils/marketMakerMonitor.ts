import { sendDiscordNotification } from './utils';

// Threshold for new market maker alert (in USDC equivalent)
const NEW_MARKET_MAKER_THRESHOLD_USDC: number = 100_000;

// Threshold for $1M lifetime volume milestone
const MILLION_MAKER_THRESHOLD_USDC: number = 1_000_000;

// Large hourly volume threshold ($1 million in one hour)
const MAKER_HOURLY_VOLUME_THRESHOLD_USDC: number = 1_000_000;

// Percentage change threshold for hourly volume (50%)
const MAKER_VOLUME_CHANGE_PERCENT_THRESHOLD: number = 0.5;

// Minimum volume to track for percentage-based change alerts
const MIN_VOLUME_FOR_PERCENT_ALERT_USDC: number = 50_000;

// Type for hourly maker volume snapshot
interface MakerVolumeSnapshot {
  timestamp: number;
  volumeByTrader: Map<string, number>;
}

export class MarketMakerMonitor {
  private readonly discordWebhookUrl: string | undefined;
  private previousSnapshot: MakerVolumeSnapshot | null = null;

  // Set of traders who have already been alerted as new market makers ($100k)
  private alertedNewMarketMakers: Set<string> = new Set();

  // Set of traders who have already been alerted for $1M milestone
  private alertedMillionMakers: Set<string> = new Set();

  // Callback to get current maker volumes from stats server
  private readonly getMakerVolumes: () => Map<string, number>;

  constructor(
    discordWebhookUrl: string | undefined,
    getMakerVolumes: () => Map<string, number>,
  ) {
    this.discordWebhookUrl = discordWebhookUrl;
    this.getMakerVolumes = getMakerVolumes;
  }

  /**
   * Check for new market makers and large volume changes.
   * Should be called every hour.
   */
  async checkHourlyChanges(): Promise<void> {
    const currentVolumes: Map<string, number> = new Map(this.getMakerVolumes());
    const currentSnapshot: MakerVolumeSnapshot = {
      timestamp: Date.now(),
      volumeByTrader: currentVolumes,
    };

    // On first run, initialize existing market makers to avoid false alerts
    if (!this.previousSnapshot) {
      this.initializeExistingMarketMakers(currentVolumes);
      this.previousSnapshot = currentSnapshot;
      return;
    }

    // Check for new market makers crossing thresholds
    await this.checkMarketMakerMilestones(currentVolumes);

    // Check for large volume changes in existing market makers
    await this.checkVolumeChanges(
      this.previousSnapshot.volumeByTrader,
      currentVolumes,
    );

    this.previousSnapshot = currentSnapshot;
  }

  /**
   * Check for traders who have crossed market maker milestones ($100k, $1M)
   */
  private async checkMarketMakerMilestones(
    currentVolumes: Map<string, number>,
  ): Promise<void> {
    for (const [trader, volume] of currentVolumes) {
      // Check $100k threshold (new market maker)
      if (volume >= NEW_MARKET_MAKER_THRESHOLD_USDC) {
        if (!this.alertedNewMarketMakers.has(trader)) {
          this.alertedNewMarketMakers.add(trader);
          await this.sendNewMarketMakerAlert(trader, volume);
        }
      }

      // Check $1M threshold (million maker milestone)
      if (volume >= MILLION_MAKER_THRESHOLD_USDC) {
        if (!this.alertedMillionMakers.has(trader)) {
          this.alertedMillionMakers.add(trader);
          await this.sendMillionMakerAlert(trader, volume);
        }
      }
    }
  }

  /**
   * Check for large volume changes in existing market makers
   */
  private async checkVolumeChanges(
    previousVolumes: Map<string, number>,
    currentVolumes: Map<string, number>,
  ): Promise<void> {
    for (const [trader, currentVolume] of currentVolumes) {
      const previousVolume: number = previousVolumes.get(trader) ?? 0;

      // Calculate hourly volume (delta)
      const hourlyVolume: number = currentVolume - previousVolume;

      // Skip if no activity this hour
      if (hourlyVolume <= 0) {
        continue;
      }

      // Alert if hourly volume exceeds $1 million threshold
      if (hourlyVolume >= MAKER_HOURLY_VOLUME_THRESHOLD_USDC) {
        await this.sendLargeHourlyVolumeAlert(
          trader,
          currentVolume,
          hourlyVolume,
        );
        continue; // Don't also send percentage alert for same trader
      }

      // Alert if hourly volume is 50%+ of previous total (for traders with $50k+ volume)
      if (
        previousVolume >= MIN_VOLUME_FOR_PERCENT_ALERT_USDC &&
        hourlyVolume / previousVolume >= MAKER_VOLUME_CHANGE_PERCENT_THRESHOLD
      ) {
        const percentChange: number = hourlyVolume / previousVolume;
        await this.sendPercentChangeAlert(
          trader,
          previousVolume,
          currentVolume,
          hourlyVolume,
          percentChange,
        );
      }
    }
  }

  /**
   * Send alert for new market maker ($100k threshold)
   */
  private async sendNewMarketMakerAlert(
    trader: string,
    volume: number,
  ): Promise<void> {
    if (!this.discordWebhookUrl) {
      return;
    }

    const formattedVolume: string = this.formatUsdValue(volume);

    const message: string = [
      `**New market maker detected**`,
      `Trader: \`${trader.slice(0, 8)}...${trader.slice(-4)}\``,
      `Maker Volume: ${formattedVolume}`,
      `[View on Solscan](https://solscan.io/account/${trader})`,
    ].join('\n');

    await sendDiscordNotification(this.discordWebhookUrl, message, {
      title: '🏦 New Market Maker ($100K)',
      color: 0x00ff00,
      timestamp: true,
    });
  }

  /**
   * Send alert for $1M lifetime volume milestone
   */
  private async sendMillionMakerAlert(
    trader: string,
    volume: number,
  ): Promise<void> {
    if (!this.discordWebhookUrl) {
      return;
    }

    const formattedVolume: string = this.formatUsdValue(volume);

    const message: string = [
      `**Market maker crossed $1M lifetime volume**`,
      `Trader: \`${trader.slice(0, 8)}...${trader.slice(-4)}\``,
      `Maker Volume: ${formattedVolume}`,
      `[View on Solscan](https://solscan.io/account/${trader})`,
    ].join('\n');

    await sendDiscordNotification(this.discordWebhookUrl, message, {
      title: '🎉 Million Dollar Market Maker',
      color: 0x9932cc,
      timestamp: true,
    });
  }

  /**
   * Send alert for large hourly volume ($1M+ in one hour)
   */
  private async sendLargeHourlyVolumeAlert(
    trader: string,
    currentVolume: number,
    hourlyVolume: number,
  ): Promise<void> {
    if (!this.discordWebhookUrl) {
      return;
    }

    const message: string = [
      `**$1M+ maker volume in last hour**`,
      `Trader: \`${trader.slice(0, 8)}...${trader.slice(-4)}\``,
      `Hourly Volume: +${this.formatUsdValue(hourlyVolume)}`,
      `Total Volume: ${this.formatUsdValue(currentVolume)}`,
      `[View on Solscan](https://solscan.io/account/${trader})`,
    ].join('\n');

    await sendDiscordNotification(this.discordWebhookUrl, message, {
      title: '📈 Massive Hourly Volume',
      color: 0xff4500,
      timestamp: true,
    });
  }

  /**
   * Send alert for 50%+ hourly volume increase
   */
  private async sendPercentChangeAlert(
    trader: string,
    previousVolume: number,
    currentVolume: number,
    hourlyVolume: number,
    percentChange: number,
  ): Promise<void> {
    if (!this.discordWebhookUrl) {
      return;
    }

    const message: string = [
      `**Large maker volume increase**`,
      `Trader: \`${trader.slice(0, 8)}...${trader.slice(-4)}\``,
      `Previous Total: ${this.formatUsdValue(previousVolume)}`,
      `Current Total: ${this.formatUsdValue(currentVolume)}`,
      `Hourly Volume: +${this.formatUsdValue(hourlyVolume)} (+${(percentChange * 100).toFixed(1)}%)`,
      `[View on Solscan](https://solscan.io/account/${trader})`,
    ].join('\n');

    await sendDiscordNotification(this.discordWebhookUrl, message, {
      title: '📈 Market Maker Volume Spike',
      color: 0xffd700,
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
   * Initialize with existing market makers to avoid false alerts on startup
   */
  initializeExistingMarketMakers(volumes: Map<string, number>): void {
    for (const [trader, volume] of volumes) {
      if (volume >= NEW_MARKET_MAKER_THRESHOLD_USDC) {
        this.alertedNewMarketMakers.add(trader);
      }
      if (volume >= MILLION_MAKER_THRESHOLD_USDC) {
        this.alertedMillionMakers.add(trader);
      }
    }
  }
}
