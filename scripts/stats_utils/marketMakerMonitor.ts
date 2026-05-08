import { sendDiscordNotification } from './utils';

// Threshold for new market maker alert (in USDC equivalent)
const NEW_MARKET_MAKER_THRESHOLD_USDC: number = 100_000;

// Large change threshold for existing market makers (50%)
const MAKER_VOLUME_CHANGE_THRESHOLD: number = 0.5;

// Minimum volume to track for large changes (avoid noise from small traders)
const MIN_VOLUME_FOR_CHANGE_ALERT_USDC: number = 50_000;

// Type for hourly maker volume snapshot
interface MakerVolumeSnapshot {
  timestamp: number;
  volumeByTrader: Map<string, number>;
}

export class MarketMakerMonitor {
  private readonly discordWebhookUrl: string | undefined;
  private previousSnapshot: MakerVolumeSnapshot | null = null;

  // Set of traders who have already been alerted as new market makers
  private alertedNewMarketMakers: Set<string> = new Set();

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

    // Check for new market makers crossing the threshold
    await this.checkNewMarketMakers(currentVolumes);

    // Check for large volume changes in existing market makers
    await this.checkVolumeChanges(
      this.previousSnapshot.volumeByTrader,
      currentVolumes,
    );

    this.previousSnapshot = currentSnapshot;
  }

  /**
   * Check for traders who have crossed the new market maker threshold
   */
  private async checkNewMarketMakers(
    currentVolumes: Map<string, number>,
  ): Promise<void> {
    for (const [trader, volume] of currentVolumes) {
      if (volume >= NEW_MARKET_MAKER_THRESHOLD_USDC) {
        if (!this.alertedNewMarketMakers.has(trader)) {
          this.alertedNewMarketMakers.add(trader);
          await this.sendNewMarketMakerAlert(trader, volume);
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

      // Skip if below minimum threshold for change alerts
      if (
        currentVolume < MIN_VOLUME_FOR_CHANGE_ALERT_USDC &&
        previousVolume < MIN_VOLUME_FOR_CHANGE_ALERT_USDC
      ) {
        continue;
      }

      // Calculate hourly volume (delta)
      const hourlyVolume: number = currentVolume - previousVolume;

      // Skip if no significant activity this hour
      if (hourlyVolume <= 0) {
        continue;
      }

      // Check for large percentage change relative to previous total
      if (previousVolume > 0) {
        const percentChange: number = hourlyVolume / previousVolume;

        if (percentChange >= MAKER_VOLUME_CHANGE_THRESHOLD) {
          await this.sendVolumeChangeAlert(
            trader,
            previousVolume,
            currentVolume,
            hourlyVolume,
            percentChange,
          );
        }
      }
    }
  }

  /**
   * Send alert for new market maker
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
      title: '🏦 New Market Maker',
      color: 0x00ff00,
      timestamp: true,
    });
  }

  /**
   * Send alert for large volume change
   */
  private async sendVolumeChangeAlert(
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
    }
  }
}
