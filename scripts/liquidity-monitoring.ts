import 'dotenv/config';

import { Connection, GetProgramAccountsResponse } from '@solana/web3.js';
import { ManifestClient, Market } from '../client/ts/src';
import { Pool } from 'pg';
import * as promClient from 'prom-client';
import express from 'express';
import promBundle from 'express-prom-bundle';
import cors from 'cors';
import {
  SOL_MINT,
  CBBTC_MINT,
  WBTC_MINT,
  CBBTC_USDC_MARKET,
  STABLECOIN_MINTS,
} from './stats_utils/constants';
import { createExpensiveQueryAdmission } from './stats_utils/httpAdmission';
import {
  parseOptionalUnixTimestamp,
  validateUnixTimestampRange,
} from './stats_utils/httpValidation';

// Configuration constants
const MONITORING_INTERVAL_MS = 5 * 60 * 1000; // 5 minutes
const MIN_VOLUME_THRESHOLD_USD = 1_000; // $1k minimum 24hr volume
const SPREAD_BPS = [10, 50, 100, 200]; // 0.1%, 0.5%, 1%, 2%
const MIN_NOTIONAL_USD = 10; // $10 minimum total notional to be considered a market maker
const PORT = 3001;

// Environment variables
const { RPC_URL, DATABASE_URL } = process.env;

if (!RPC_URL) {
  throw new Error('RPC_URL missing from env');
}

if (!DATABASE_URL) {
  throw new Error('DATABASE_URL missing from env');
}

// Prometheus metrics
const marketMakerDepth = new promClient.Gauge({
  name: 'market_maker_depth',
  help: 'Market maker depth at various spreads',
  labelNames: ['market', 'trader', 'side', 'spread_bps'] as const,
});

// Accepted operational tradeoff: trader pubkeys are permissionless labels and
// historical rows are append-only. Cardinality/storage exhaustion is
// theoretically possible through funded identity churn, but current scale is
// nowhere near operational limits. If growth becomes material, cap exported
// identities, remove stale label sets, and partition/expire snapshot history.
const marketMakerUptime = new promClient.Gauge({
  name: 'market_maker_uptime',
  help: 'Market maker uptime percentage',
  labelNames: ['market', 'trader'] as const,
});

const marketVolume24h = new promClient.Gauge({
  name: 'market_volume_24h',
  help: '24 hour volume in USD for markets',
  labelNames: ['market'] as const,
});

interface MarketMakerStats {
  trader: string;
  market: string;
  bidDepth: { [spreadBps: number]: number };
  askDepth: { [spreadBps: number]: number };
  totalNotionalUsd: number;
  isActive: boolean;
  timestamp: Date;
}

interface MarketInfo {
  address: string;
  baseMint: string;
  quoteMint: string;
  volume24hUsd: number;
  lastPrice: number;
  baseDecimals: number;
  quoteDecimals: number;
}

interface TickerResponse {
  ticker_id: string;
  target_currency: string;
  target_volume?: number;
  last_price?: number;
}

interface SolPriceResult {
  priceUsd: number;
  warning: string | null;
}

export class LiquidityMonitor {
  private connection: Connection;
  public pool: Pool;
  private markets: Map<string, Market> = new Map();
  private marketInfo: Map<string, MarketInfo> = new Map();

  private isMonitoring = false;
  private monitoringStartedAtMs: number | null = null;
  private lastSuccessfulMonitoringAtMs: number | null = null;
  private lastMonitoringError: string | null = null;
  private lastMonitoringWarning: string | null = null;
  private lastFailedMarkets: string[] = [];
  private lastMarketVolumes: Map<string, number> = new Map();
  private lastSolPriceUsd: number = 0;
  private marketStatsCache: { expiresAtMs: number; rows: any[] } | undefined;

  constructor() {
    this.connection = new Connection(RPC_URL!);
    this.pool = new Pool({
      connectionString: DATABASE_URL!,
      ssl: { rejectUnauthorized: true }, // Reject database MITM certificates.
      statement_timeout: 15_000,
      max: 3,
      min: 1,
      idleTimeoutMillis: 20000,
      connectionTimeoutMillis: 8000,
    });

    this.pool.on('error', (err) => {
      console.error('Unexpected database pool error:', err);
    });
  }

  /**
   * Initialize database schema
   */
  async initDatabase(): Promise<void> {
    try {
      // Market maker stats table
      await this.pool.query(`
        CREATE TABLE IF NOT EXISTS market_maker_stats (
          id SERIAL PRIMARY KEY,
          market TEXT NOT NULL,
          trader TEXT NOT NULL,
          timestamp TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
          is_active BOOLEAN NOT NULL,
          total_notional_usd NUMERIC DEFAULT 0,
          bid_depth_10_bps NUMERIC DEFAULT 0,
          bid_depth_50_bps NUMERIC DEFAULT 0,
          bid_depth_100_bps NUMERIC DEFAULT 0,
          bid_depth_200_bps NUMERIC DEFAULT 0,
          ask_depth_10_bps NUMERIC DEFAULT 0,
          ask_depth_50_bps NUMERIC DEFAULT 0,
          ask_depth_100_bps NUMERIC DEFAULT 0,
          ask_depth_200_bps NUMERIC DEFAULT 0
        )
      `);

      // Market info table
      await this.pool.query(`
        CREATE TABLE IF NOT EXISTS market_info_snapshots (
          id SERIAL PRIMARY KEY,
          market TEXT NOT NULL,
          timestamp TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
          volume_24h_usd NUMERIC NOT NULL,
          last_price NUMERIC NOT NULL,
          total_unique_makers INTEGER DEFAULT 0,
          avg_bid_depth NUMERIC DEFAULT 0,
          avg_ask_depth NUMERIC DEFAULT 0
        )
      `);

      // Create indexes for performance
      await this.pool.query(`
        CREATE INDEX IF NOT EXISTS idx_market_maker_stats_timestamp 
        ON market_maker_stats(timestamp)
      `);

      await this.pool.query(`
        CREATE INDEX IF NOT EXISTS idx_market_maker_stats_market_trader 
        ON market_maker_stats(market, trader)
      `);

      // Add composite index for time-range queries
      await this.pool.query(`
        CREATE INDEX IF NOT EXISTS idx_market_maker_stats_market_trader_timestamp 
        ON market_maker_stats(market, trader, timestamp)
      `);

      const constraintExists = await this.pool.query(`
        SELECT 1 FROM information_schema.table_constraints 
        WHERE table_name = 'market_maker_stats' 
        AND constraint_name = 'unique_market_trader_timestamp'
      `);

      if (constraintExists.rows.length === 0) {
        await this.pool.query(`
          ALTER TABLE market_maker_stats 
          ADD CONSTRAINT unique_market_trader_timestamp 
          UNIQUE (market, trader, timestamp)
        `);
        console.log('Added unique constraint to prevent duplicate records');
      } else {
        console.log('Unique constraint already exists');
      }

      console.log('Database schema initialized');
    } catch (error) {
      console.error('Error initializing database:', error);
      throw error;
    }
  }

  /**
   * Fetch market volume data from the stats API
   */
  async fetchMarketVolumes(): Promise<Map<string, number>> {
    try {
      const response = await fetch('https://mfx-stats-mainnet.fly.dev/tickers');
      if (!response.ok) {
        throw new Error(`Ticker request failed with HTTP ${response.status}`);
      }
      const tickers: TickerResponse[] =
        (await response.json()) as TickerResponse[];

      const volumeMap = new Map<string, number>();

      // Get SOL price from SOL/USDC market
      const solUsdcTicker = tickers.find(
        (t: TickerResponse) =>
          t.ticker_id === 'ENhU8LsaR7vDD2G1CsWcsuSGNrih9Cv5WZEk7q9kPapQ',
      );
      const solPrice = solUsdcTicker?.last_price || 0;

      // Get CBBTC price from CBBTC/USDC market
      const cbbtcUsdcTicker = tickers.find(
        (t: TickerResponse) => t.ticker_id === CBBTC_USDC_MARKET,
      );
      const cbbtcPrice = cbbtcUsdcTicker?.last_price || 0;

      for (const ticker of tickers) {
        const quoteMint = ticker.target_currency;
        if (
          !STABLECOIN_MINTS.has(quoteMint) &&
          quoteMint !== SOL_MINT &&
          quoteMint !== CBBTC_MINT &&
          quoteMint !== WBTC_MINT
        ) {
          continue;
        }

        let volumeUsd = 0;
        if (STABLECOIN_MINTS.has(quoteMint)) {
          volumeUsd = ticker.target_volume || 0;
        } else if (quoteMint === SOL_MINT && solPrice > 0) {
          // Convert SOL volume to USD
          volumeUsd = (ticker.target_volume || 0) * solPrice;
        } else if (
          (quoteMint === CBBTC_MINT || quoteMint === WBTC_MINT) &&
          cbbtcPrice > 0
        ) {
          // Convert CBBTC/WBTC volume to USD (both use CBBTC price)
          volumeUsd = (ticker.target_volume || 0) * cbbtcPrice;
        }

        volumeMap.set(ticker.ticker_id, volumeUsd);
      }

      console.log(
        `Fetched volumes: ${volumeMap.size} markets, SOL price: $${solPrice}, CBBTC price: $${cbbtcPrice}`,
      );
      this.lastMarketVolumes = new Map(volumeMap);
      if (solPrice > 0) {
        this.lastSolPriceUsd = solPrice;
      }
      return volumeMap;
    } catch (error) {
      console.error('Error fetching market volumes:', error);
      if (this.lastMarketVolumes.size > 0) {
        console.warn('Using the last successfully fetched market volumes');
        return new Map(this.lastMarketVolumes);
      }
      throw error;
    }
  }

  private async fetchSolPriceUsd(): Promise<SolPriceResult> {
    try {
      const response: Response = await fetch(
        'https://mfx-stats-mainnet.fly.dev/tickers',
      );
      if (!response.ok) {
        throw new Error(`Ticker request failed with HTTP ${response.status}`);
      }
      const tickers: TickerResponse[] =
        (await response.json()) as TickerResponse[];
      const solUsdcTicker: TickerResponse | undefined = tickers.find(
        (ticker: TickerResponse): boolean =>
          ticker.ticker_id === 'ENhU8LsaR7vDD2G1CsWcsuSGNrih9Cv5WZEk7q9kPapQ',
      );
      const priceUsd: number = solUsdcTicker?.last_price ?? 0;
      if (!Number.isFinite(priceUsd) || priceUsd <= 0) {
        throw new Error('Ticker response did not contain a valid SOL price');
      }
      this.lastSolPriceUsd = priceUsd;
      return { priceUsd, warning: null };
    } catch (error: unknown) {
      const reason: string =
        error instanceof Error ? error.message : String(error);
      const warning: string =
        this.lastSolPriceUsd > 0
          ? `Ticker refresh failed; using cached SOL price: ${reason}`
          : `Ticker refresh failed; SOL-denominated depth is unpriced: ${reason}`;
      console.warn(warning);
      return { priceUsd: this.lastSolPriceUsd, warning };
    }
  }

  /**
   * Load eligible markets (>$1k 24hr volume and USDC quote)
   */
  async loadEligibleMarkets(): Promise<void> {
    console.log('Loading eligible markets...');

    const volumeMap = await this.fetchMarketVolumes();
    const marketProgramAccounts: GetProgramAccountsResponse =
      await ManifestClient.getMarketProgramAccounts(this.connection);

    this.markets.clear();
    this.marketInfo.clear();

    for (const account of marketProgramAccounts) {
      const marketPk = account.pubkey.toBase58();
      const volume24h = volumeMap.get(marketPk) || 0;

      if (volume24h >= MIN_VOLUME_THRESHOLD_USD) {
        try {
          const market = Market.loadFromBuffer({
            buffer: account.account.data,
            address: account.pubkey,
          });

          // Skip markets that have never traded
          if (Number(market.quoteVolume()) === 0) {
            continue;
          }

          // Only include stablecoin, SOL, CBBTC, and WBTC quote markets
          const quoteMint = market.quoteMint().toBase58();
          if (
            !STABLECOIN_MINTS.has(quoteMint) &&
            quoteMint !== SOL_MINT &&
            quoteMint !== CBBTC_MINT &&
            quoteMint !== WBTC_MINT
          ) {
            continue;
          }

          this.markets.set(marketPk, market);

          this.marketInfo.set(marketPk, {
            address: marketPk,
            baseMint: market.baseMint().toBase58(),
            quoteMint: quoteMint,
            volume24hUsd: volume24h,
            lastPrice: 0, // Will be updated during monitoring
            baseDecimals: market.baseDecimals(),
            quoteDecimals: market.quoteDecimals(),
          });

          const marketType = STABLECOIN_MINTS.has(quoteMint)
            ? 'Stablecoin'
            : quoteMint === SOL_MINT
              ? 'SOL'
              : quoteMint === CBBTC_MINT
                ? 'CBBTC'
                : 'WBTC';
          console.log(
            `Added ${marketType} market ${marketPk} with $${volume24h.toLocaleString()} 24h volume`,
          );
        } catch (error) {
          console.error(`Error loading market ${marketPk}:`, error);
        }
      }
    }

    console.log(
      `Loaded ${this.markets.size} eligible markets (Stablecoin + SOL + CBBTC + WBTC)`,
    );
  }

  /**
   * Calculate market maker depths at various spreads
   */
  calculateMarketMakerDepths(
    market: Market,
    timestamp: Date,
    solPriceUsd: number = 0,
  ): MarketMakerStats[] {
    const bids = market.bids();
    const asks = market.asks();

    console.log(
      `Market ${market.address.toBase58()}: ${bids.length} bids, ${asks.length} asks`,
    );

    if (bids.length === 0 || asks.length === 0) {
      console.log(
        `Skipping market ${market.address.toBase58()}: no bids or asks`,
      );
      return [];
    }

    // Calculate mid price
    const bestBid = bids[bids.length - 1].tokenPrice;
    const bestAsk = asks[asks.length - 1].tokenPrice;
    const midPrice = (bestBid + bestAsk) / 2;

    console.log(
      `Market ${market.address.toBase58()}: bestBid=${bestBid}, bestAsk=${bestAsk}, midPrice=${midPrice}`,
    );

    const ordersByTrader: Map<
      string,
      { bids: typeof bids; asks: typeof asks }
    > = new Map();
    for (const order of bids) {
      const trader: string = order.trader.toBase58();
      const grouped = ordersByTrader.get(trader) ?? { bids: [], asks: [] };
      grouped.bids.push(order);
      ordersByTrader.set(trader, grouped);
    }
    for (const order of asks) {
      const trader: string = order.trader.toBase58();
      const grouped = ordersByTrader.get(trader) ?? { bids: [], asks: [] };
      grouped.asks.push(order);
      ordersByTrader.set(trader, grouped);
    }

    console.log(
      `Market ${market.address.toBase58()}: ${ordersByTrader.size} unique traders found`,
    );

    const stats: MarketMakerStats[] = [];

    for (const [trader, traderOrders] of ordersByTrader) {
      const traderBids: typeof bids = traderOrders.bids;
      const traderAsks: typeof asks = traderOrders.asks;

      const bidDepth: { [spreadBps: number]: number } = {};
      const askDepth: { [spreadBps: number]: number } = {};

      // Calculate depth at each spread level
      for (const spreadBps of SPREAD_BPS) {
        const spreadMultiplier = spreadBps / 10000; // Convert bps to decimal
        const bidThreshold = midPrice * (1 - spreadMultiplier);
        const askThreshold = midPrice * (1 + spreadMultiplier);

        // Calculate bid depth (orders above threshold)
        bidDepth[spreadBps] = traderBids
          .filter((order) => order.tokenPrice >= bidThreshold)
          .reduce((sum, order) => sum + Number(order.numBaseTokens), 0);

        // Calculate ask depth (orders below threshold)
        askDepth[spreadBps] = traderAsks
          .filter((order) => order.tokenPrice <= askThreshold)
          .reduce((sum, order) => sum + Number(order.numBaseTokens), 0);
      }

      // Calculate total notional in USD
      const totalBaseTokens = (bidDepth[100] || 0) + (askDepth[100] || 0);
      const quoteMint = market.quoteMint().toBase58();
      let totalNotionalUsd = totalBaseTokens * midPrice;

      // Convert to USD if this is a SOL market
      if (quoteMint === SOL_MINT && solPriceUsd > 0) {
        totalNotionalUsd = totalNotionalUsd * solPriceUsd;
      }

      // Only include if they meet minimum notional threshold
      if (totalNotionalUsd < MIN_NOTIONAL_USD) {
        continue;
      }

      const isActive = traderBids.length > 0 || traderAsks.length > 0;

      stats.push({
        trader,
        market: market.address.toBase58(),
        bidDepth,
        askDepth,
        totalNotionalUsd,
        isActive,
        timestamp: timestamp,
      });
    }

    return stats;
  }

  /**
   * Monitor all eligible markets
   */
  async monitorMarkets(): Promise<void> {
    if (this.isMonitoring) {
      console.log('Previous monitoring cycle still running, skipping...');
      return;
    }

    this.isMonitoring = true;

    try {
      const cycleTimestamp = new Date();
      console.log('Starting market monitoring cycle...', cycleTimestamp);

      // A transient ticker outage must not discard the whole monitoring
      // cycle. Reuse the last good price (or zero when none exists yet) and
      // expose the reduced fidelity as degraded health metadata.
      const solPriceResult: SolPriceResult = await this.fetchSolPriceUsd();
      const solPriceUsd: number = solPriceResult.priceUsd;

      console.log(`Using SOL price: $${solPriceUsd} for depth calculations`);

      const allStats: MarketMakerStats[] = [];
      const failedMarkets: string[] = [];
      const completedMarkets: Set<string> = new Set();

      for (const [marketPk, market] of this.markets) {
        try {
          const configuredMarket: MarketInfo | undefined =
            this.marketInfo.get(marketPk);
          if (configuredMarket?.quoteMint === SOL_MINT && solPriceUsd <= 0) {
            throw new Error(
              'No valid SOL/USD price is available; refusing to persist unpriced USD depth',
            );
          }

          // Reload market data
          await market.reload(this.connection);

          // Calculate market maker stats
          const marketStats = this.calculateMarketMakerDepths(
            market,
            cycleTimestamp,
            solPriceUsd,
          );

          allStats.push(...marketStats);

          // Update last price in market info
          const bids = market.bids();
          const asks = market.asks();
          if (bids.length > 0 && asks.length > 0) {
            const bestBid = bids[bids.length - 1].tokenPrice;
            const bestAsk = asks[asks.length - 1].tokenPrice;
            const lastPrice = (bestBid + bestAsk) / 2;

            const marketInfo = this.marketInfo.get(marketPk);
            if (marketInfo) {
              marketInfo.lastPrice = lastPrice;
              marketVolume24h.set(
                { market: marketPk },
                marketInfo.volume24hUsd,
              );
            }
          }

          // Update Prometheus metrics for market makers
          for (const stat of marketStats) {
            for (const spreadBps of SPREAD_BPS) {
              marketMakerDepth.set(
                {
                  market: marketPk,
                  trader: stat.trader,
                  side: 'bid',
                  spread_bps: spreadBps.toString(),
                },
                stat.bidDepth[spreadBps] || 0,
              );
              marketMakerDepth.set(
                {
                  market: marketPk,
                  trader: stat.trader,
                  side: 'ask',
                  spread_bps: spreadBps.toString(),
                },
                stat.askDepth[spreadBps] || 0,
              );
            }
          }

          console.log(
            `Processed ${marketStats.length} market makers for market ${marketPk}`,
          );
          completedMarkets.add(marketPk);
        } catch (error) {
          console.error(`Error monitoring market ${marketPk}:`, error);
          failedMarkets.push(marketPk);
        }
      }

      if (this.markets.size > 0 && completedMarkets.size === 0) {
        throw new Error(
          'Monitoring cycle did not complete any eligible market',
        );
      }

      // Remove duplicates before saving
      const uniqueStats = allStats.filter((stat, index, array) => {
        return (
          index ===
          array.findIndex(
            (s) =>
              s.market === stat.market &&
              s.trader === stat.trader &&
              s.timestamp.getTime() === stat.timestamp.getTime(),
          )
        );
      });

      // Save stats to database
      await this.saveStatsToDatabase(
        uniqueStats,
        cycleTimestamp,
        completedMarkets,
      );

      const warnings: string[] = [];

      // Collection and durable persistence define a successful monitoring
      // cycle. Prometheus refresh is best-effort and should degrade health
      // metadata rather than make an orchestrator restart a healthy process.
      try {
        await this.updatePrometheusMetrics();
      } catch (error: unknown) {
        const reason: string =
          error instanceof Error ? error.message : String(error);
        warnings.push(`Prometheus refresh failed: ${reason}`);
      }

      console.log(
        `Monitoring cycle complete. Processed ${uniqueStats.length} market maker entries.`,
      );
      this.lastSuccessfulMonitoringAtMs = Date.now();
      this.lastMonitoringError = null;
      this.lastFailedMarkets = failedMarkets;
      if (solPriceResult.warning !== null) {
        warnings.push(solPriceResult.warning);
      }
      if (failedMarkets.length > 0) {
        warnings.push(
          `Incomplete for ${failedMarkets.length} market(s): ${failedMarkets.join(', ')}`,
        );
      }
      this.lastMonitoringWarning =
        warnings.length === 0 ? null : warnings.join('; ');
    } catch (error) {
      this.lastMonitoringError =
        error instanceof Error ? error.message : String(error);
      throw error;
    } finally {
      this.isMonitoring = false;
    }
  }

  /**
   * Save market maker stats to database
   */
  async saveStatsToDatabase(
    stats: MarketMakerStats[],
    timestamp: Date,
    completedMarkets: ReadonlySet<string> = new Set(this.marketInfo.keys()),
  ): Promise<void> {
    if (stats.length === 0) return;

    try {
      console.log('Saving market maker stats to database...');

      // Batch insert market maker stats with UPSERT
      const batchSize = 50;
      for (let i = 0; i < stats.length; i += batchSize) {
        const batch = stats.slice(i, i + batchSize);

        const values = batch.flatMap((stat) => [
          stat.market,
          stat.trader,
          stat.timestamp,
          stat.isActive,
          stat.totalNotionalUsd,
          stat.bidDepth[10] || 0,
          stat.bidDepth[50] || 0,
          stat.bidDepth[100] || 0,
          stat.bidDepth[200] || 0,
          stat.askDepth[10] || 0,
          stat.askDepth[50] || 0,
          stat.askDepth[100] || 0,
          stat.askDepth[200] || 0,
        ]);

        const placeholders = batch
          .map((_, index) => {
            const offset = index * 13;
            return `($${offset + 1}, $${offset + 2}, $${offset + 3}, $${offset + 4}, $${offset + 5}, $${offset + 6}, $${offset + 7}, $${offset + 8}, $${offset + 9}, $${offset + 10}, $${offset + 11}, $${offset + 12}, $${offset + 13})`;
          })
          .join(', ');

        const query = `
          INSERT INTO market_maker_stats (
            market, trader, timestamp, is_active, total_notional_usd,
            bid_depth_10_bps, bid_depth_50_bps, bid_depth_100_bps, bid_depth_200_bps,
            ask_depth_10_bps, ask_depth_50_bps, ask_depth_100_bps, ask_depth_200_bps
          ) VALUES ${placeholders}
          ON CONFLICT (market, trader, timestamp) 
          DO UPDATE SET 
            is_active = EXCLUDED.is_active,
            total_notional_usd = EXCLUDED.total_notional_usd,
            bid_depth_10_bps = EXCLUDED.bid_depth_10_bps,
            bid_depth_50_bps = EXCLUDED.bid_depth_50_bps,
            bid_depth_100_bps = EXCLUDED.bid_depth_100_bps,
            bid_depth_200_bps = EXCLUDED.bid_depth_200_bps,
            ask_depth_10_bps = EXCLUDED.ask_depth_10_bps,
            ask_depth_50_bps = EXCLUDED.ask_depth_50_bps,
            ask_depth_100_bps = EXCLUDED.ask_depth_100_bps,
            ask_depth_200_bps = EXCLUDED.ask_depth_200_bps
        `;

        await this.pool.query(query, values);

        if (i + batchSize < stats.length) {
          await new Promise((resolve) => setTimeout(resolve, 200));
        }
      }

      // Save market info snapshots
      const completedMarketInfos: MarketInfo[] = Array.from(
        this.marketInfo.values(),
      ).filter((info: MarketInfo): boolean =>
        completedMarkets.has(info.address),
      );
      const marketInfoValues: (string | number | Date)[] =
        completedMarketInfos.flatMap((info: MarketInfo) => [
          info.address,
          timestamp,
          info.volume24hUsd,
          info.lastPrice,
        ]);

      if (marketInfoValues.length > 0) {
        const marketInfoPlaceholders: string = completedMarketInfos
          .map((_, index) => {
            const offset = index * 4;
            return `($${offset + 1}, $${offset + 2}, $${offset + 3}, $${offset + 4})`;
          })
          .join(', ');

        const marketInfoQuery = `
          INSERT INTO market_info_snapshots (market, timestamp, volume_24h_usd, last_price)
          VALUES ${marketInfoPlaceholders}
        `;

        await this.pool.query(marketInfoQuery, marketInfoValues);
      }

      console.log('Successfully saved stats to database');
    } catch (error) {
      console.error('Error saving stats to database:', error);
      throw error;
    }
  }

  /**
   * Update Prometheus metrics
   */
  async updatePrometheusMetrics(): Promise<void> {
    try {
      const uptimeQuery = `
        WITH recent_stats AS (
          SELECT 
            market,
            trader,
            is_active
          FROM market_maker_stats
          WHERE timestamp > NOW() - INTERVAL '24 hours'
            AND total_notional_usd >= ${MIN_NOTIONAL_USD}
        )
        SELECT 
          market,
          trader,
          CASE 
            WHEN COUNT(*) > 0 THEN 
              (COUNT(*) FILTER (WHERE is_active)::NUMERIC / COUNT(*)) * 100
            ELSE 0 
          END as uptime_24h
        FROM recent_stats
        GROUP BY market, trader
        HAVING COUNT(*) > 0
      `;

      const uptimeResult = await this.pool.query(uptimeQuery);
      for (const row of uptimeResult.rows) {
        marketMakerUptime.set(
          { market: row.market, trader: row.trader },
          Number(row.uptime_24h),
        );
      }

      console.log('Successfully updated Prometheus metrics');
    } catch (error) {
      console.error('Error updating Prometheus metrics:', error);
      throw error;
    }
  }

  getHealthStatus(): {
    healthy: boolean;
    starting: boolean;
    degraded: boolean;
    lastSuccessfulMonitoringAt: string | null;
    error: string | null;
    warning: string | null;
    failedMarkets: string[];
  } {
    const staleAfterMs: number = MONITORING_INTERVAL_MS * 2;
    const nowMs: number = Date.now();
    const starting: boolean =
      this.lastSuccessfulMonitoringAtMs === null &&
      this.monitoringStartedAtMs !== null &&
      nowMs - this.monitoringStartedAtMs <= staleAfterMs;
    const completedRecently: boolean =
      this.lastSuccessfulMonitoringAtMs !== null &&
      nowMs - this.lastSuccessfulMonitoringAtMs <= staleAfterMs;
    const healthy: boolean = starting || completedRecently;
    const degraded: boolean =
      completedRecently &&
      (this.lastMonitoringWarning !== null ||
        this.lastMonitoringError !== null);
    return {
      healthy,
      starting,
      degraded,
      lastSuccessfulMonitoringAt: this.lastSuccessfulMonitoringAtMs
        ? new Date(this.lastSuccessfulMonitoringAtMs).toISOString()
        : null,
      error: this.lastMonitoringError,
      warning: this.lastMonitoringWarning,
      failedMarkets: [...this.lastFailedMarkets],
    };
  }

  /**
   * Get market maker statistics for a specific time period
   */
  async getMarketMakerStats(
    options: {
      market?: string;
      trader?: string;
      hours?: number; // How many hours back to look
      startTimestamp?: number; // Unix timestamp (seconds)
      endTimestamp?: number; // Unix timestamp (seconds)
      limit?: number;
    } = {},
  ): Promise<any[]> {
    try {
      const {
        market,
        trader,
        hours = 24,
        startTimestamp,
        endTimestamp,
        limit = 100,
      } = options;

      // Build time filter - prioritize timestamps over hours
      let timeFilter = '';
      if (startTimestamp && endTimestamp) {
        timeFilter = `timestamp >= to_timestamp(${startTimestamp}) AND timestamp <= to_timestamp(${endTimestamp})`;
      } else if (startTimestamp) {
        timeFilter = `timestamp >= to_timestamp(${startTimestamp})`;
      } else if (endTimestamp) {
        timeFilter = `timestamp <= to_timestamp(${endTimestamp})`;
      } else {
        timeFilter = `timestamp > NOW() - INTERVAL '${hours} hours'`;
      }

      let query = `
        WITH total_cycles_per_market AS (
          SELECT 
            market,
            COUNT(DISTINCT timestamp) as total_possible_cycles
          FROM market_maker_stats
          WHERE ${timeFilter}
          GROUP BY market
        ),
        recent_stats AS (
          SELECT 
            market,
            trader,
            timestamp,
            is_active,
            total_notional_usd,
            -- Include ALL spread levels
            bid_depth_10_bps,
            bid_depth_50_bps,
            bid_depth_100_bps,
            bid_depth_200_bps,
            ask_depth_10_bps,
            ask_depth_50_bps,
            ask_depth_100_bps,
            ask_depth_200_bps,
            -- Track first and last seen times
            MIN(timestamp) OVER (PARTITION BY market, trader) as first_seen,
            MAX(timestamp) OVER (PARTITION BY market, trader) as last_seen
          FROM market_maker_stats
          WHERE ${timeFilter}
            AND total_notional_usd >= ${MIN_NOTIONAL_USD}
      `;

      const params: any[] = [];
      let paramIndex = 1;

      if (market) {
        query += ` AND market = $${paramIndex}`;
        params.push(market);
        paramIndex++;
      }

      if (trader) {
        query += ` AND trader = $${paramIndex}`;
        params.push(trader);
        paramIndex++;
      }

      query += `
        ),
        summary_stats AS (
          SELECT 
            rs.market,
            rs.trader,
            MAX(rs.last_seen) as last_active,
            MIN(rs.first_seen) as first_seen,
            COUNT(*) as total_samples,
            COUNT(*) FILTER (WHERE rs.is_active) as active_samples,
            -- TRUE UPTIME: active samples / total possible cycles for this market
            tcpm.total_possible_cycles,
            CASE 
              WHEN tcpm.total_possible_cycles > 0 THEN 
                (COUNT(*) FILTER (WHERE rs.is_active)::NUMERIC / tcpm.total_possible_cycles) * 100
              ELSE 0 
            END as uptime_percentage,
            -- PRESENCE: how often they appeared / total possible cycles
            CASE 
              WHEN tcpm.total_possible_cycles > 0 THEN 
                (COUNT(*)::NUMERIC / tcpm.total_possible_cycles) * 100
              ELSE 0 
            END as presence_percentage,
            -- Calculate tracking period in hours
            EXTRACT(EPOCH FROM (MAX(rs.last_seen) - MIN(rs.first_seen))) / 3600 as tracking_hours,
            -- Average depths when active for ALL spread levels
            AVG(rs.bid_depth_10_bps) FILTER (WHERE rs.is_active AND rs.bid_depth_10_bps > 0) as avg_bid_depth_10_bps,
            AVG(rs.bid_depth_50_bps) FILTER (WHERE rs.is_active AND rs.bid_depth_50_bps > 0) as avg_bid_depth_50_bps,
            AVG(rs.bid_depth_100_bps) FILTER (WHERE rs.is_active AND rs.bid_depth_100_bps > 0) as avg_bid_depth_100_bps,
            AVG(rs.bid_depth_200_bps) FILTER (WHERE rs.is_active AND rs.bid_depth_200_bps > 0) as avg_bid_depth_200_bps,
            AVG(rs.ask_depth_10_bps) FILTER (WHERE rs.is_active AND rs.ask_depth_10_bps > 0) as avg_ask_depth_10_bps,
            AVG(rs.ask_depth_50_bps) FILTER (WHERE rs.is_active AND rs.ask_depth_50_bps > 0) as avg_ask_depth_50_bps,
            AVG(rs.ask_depth_100_bps) FILTER (WHERE rs.is_active AND rs.ask_depth_100_bps > 0) as avg_ask_depth_100_bps,
            AVG(rs.ask_depth_200_bps) FILTER (WHERE rs.is_active AND rs.ask_depth_200_bps > 0) as avg_ask_depth_200_bps,
            AVG(rs.total_notional_usd) FILTER (WHERE rs.is_active) as avg_notional_usd
          FROM recent_stats rs
          JOIN total_cycles_per_market tcpm ON rs.market = tcpm.market
          GROUP BY rs.market, rs.trader, tcpm.total_possible_cycles
        )
        SELECT 
          ss.*,
          -- Include all spread levels with COALESCE for null handling
          COALESCE(ss.avg_bid_depth_10_bps, 0) as avg_bid_depth_10_bps,
          COALESCE(ss.avg_bid_depth_50_bps, 0) as avg_bid_depth_50_bps,
          COALESCE(ss.avg_bid_depth_100_bps, 0) as avg_bid_depth_100_bps,
          COALESCE(ss.avg_bid_depth_200_bps, 0) as avg_bid_depth_200_bps,
          COALESCE(ss.avg_ask_depth_10_bps, 0) as avg_ask_depth_10_bps,
          COALESCE(ss.avg_ask_depth_50_bps, 0) as avg_ask_depth_50_bps,
          COALESCE(ss.avg_ask_depth_100_bps, 0) as avg_ask_depth_100_bps,
          COALESCE(ss.avg_ask_depth_200_bps, 0) as avg_ask_depth_200_bps,
          -- Legacy fields for backward compatibility
          COALESCE(ss.avg_bid_depth_100_bps, 0) as avg_bid_depth,
          COALESCE(ss.avg_ask_depth_100_bps, 0) as avg_ask_depth,
          COALESCE(ss.avg_bid_depth_100_bps, 0) + COALESCE(ss.avg_ask_depth_100_bps, 0) as total_avg_depth,
          -- Market info
          mis.volume_24h_usd,
          mis.last_price,
          -- Helpful display fields
          CASE 
            WHEN ss.tracking_hours < 1 THEN 'Less than 1 hour'
            WHEN ss.tracking_hours < 24 THEN ROUND(ss.tracking_hours, 1) || ' hours'
            ELSE ROUND(ss.tracking_hours / 24, 1) || ' days'
          END as tracking_period,
          ROUND(ss.uptime_percentage, 1) as uptime_percent,
          ROUND(ss.presence_percentage, 1) as presence_percent,
          -- Add timestamps for reference
          EXTRACT(EPOCH FROM ss.first_seen) as first_seen_timestamp,
          EXTRACT(EPOCH FROM ss.last_active) as last_active_timestamp
        FROM summary_stats ss
        LEFT JOIN LATERAL (
          SELECT volume_24h_usd, last_price
          FROM market_info_snapshots mis_inner
          WHERE mis_inner.market = ss.market
          ORDER BY timestamp DESC
          LIMIT 1
        ) mis ON true
        ORDER BY 
          ss.uptime_percentage DESC,
          (COALESCE(ss.avg_bid_depth_100_bps, 0) + COALESCE(ss.avg_ask_depth_100_bps, 0)) DESC
        LIMIT $${paramIndex}
      `;

      params.push(limit);

      const result = await this.pool.query(query, params);
      return result.rows;
    } catch (error) {
      console.error('Error getting market maker stats:', error);
      return [];
    }
  }

  /**
   * Get market statistics
   */
  async getMarketStats(): Promise<any[]> {
    const nowMs: number = Date.now();
    if (this.marketStatsCache && this.marketStatsCache.expiresAtMs > nowMs) {
      return this.marketStatsCache.rows;
    }

    try {
      const query = `
        WITH latest_snapshots AS (
          SELECT DISTINCT ON (market)
            market, volume_24h_usd, last_price, timestamp
          FROM market_info_snapshots
          WHERE timestamp > NOW() - INTERVAL '1 hour'
          ORDER BY market, timestamp DESC
        ), makers_24h AS (
          SELECT market, COUNT(DISTINCT trader) AS unique_makers_24h
          FROM market_maker_stats
          WHERE timestamp > NOW() - INTERVAL '24 hours'
            AND total_notional_usd >= ${MIN_NOTIONAL_USD}
          GROUP BY market
        ), makers_current AS (
          SELECT market, COUNT(DISTINCT trader) AS unique_makers_current
          FROM market_maker_stats
          WHERE timestamp > NOW() - INTERVAL '1 hour'
            AND total_notional_usd >= ${MIN_NOTIONAL_USD}
          GROUP BY market
        )
        SELECT
          latest.market,
          latest.volume_24h_usd,
          latest.last_price,
          latest.timestamp,
          COALESCE(day.unique_makers_24h, 0) AS unique_makers_24h,
          COALESCE(current.unique_makers_current, 0) AS unique_makers_current
        FROM latest_snapshots latest
        LEFT JOIN makers_24h day ON day.market = latest.market
        LEFT JOIN makers_current current ON current.market = latest.market
        ORDER BY latest.market
      `;

      const result = await this.pool.query(query);
      this.marketStatsCache = {
        expiresAtMs: nowMs + 30_000,
        rows: result.rows,
      };
      return result.rows;
    } catch (error) {
      console.error('Error getting market stats:', error);
      throw error;
    }
  }

  /**
   * Start the monitoring loop
   */
  async startMonitoring(): Promise<void> {
    console.log('Starting liquidity monitoring...');
    this.monitoringStartedAtMs = Date.now();

    // Arm retries before the initial dependency calls so a transient startup
    // failure cannot permanently disarm monitoring.
    setInterval(async () => {
      try {
        if (this.markets.size === 0) {
          await this.loadEligibleMarkets();
        }
        await this.monitorMarkets();
      } catch (error) {
        console.error('Error in monitoring cycle:', error);
      }
    }, MONITORING_INTERVAL_MS);

    // Reload eligible markets every hour
    setInterval(
      async () => {
        try {
          await this.loadEligibleMarkets();
        } catch (error) {
          console.error('Error reloading markets:', error);
        }
      },
      60 * 60 * 1000,
    );

    try {
      await this.loadEligibleMarkets();
      await this.monitorMarkets();
    } catch (error) {
      this.lastMonitoringError =
        error instanceof Error ? error.message : String(error);
      console.error('Initial liquidity monitoring cycle failed:', error);
    }
  }
}

// API Setup
const setupAPI = (monitor: LiquidityMonitor) => {
  const app = express();
  const boundedQueryInt = (
    value: unknown,
    fallback: number,
    maximum: number,
    name: string,
  ): number => {
    if (value === undefined) return fallback;
    const parsed = Number(value);
    if (!Number.isSafeInteger(parsed) || parsed < 1 || parsed > maximum) {
      throw new Error(`${name} must be an integer between 1 and ${maximum}`);
    }
    return parsed;
  };
  app.use(cors());
  app.use(express.json());
  app.use(
    createExpensiveQueryAdmission({
      maxConcurrent: 2,
      maxRequestsPerMinute: 20,
    }),
  );

  // Market maker statistics with flexible time periods
  app.get('/market-makers', async (req, res) => {
    try {
      const market = req.query.market as string;
      const trader = req.query.trader as string;
      const hours = boundedQueryInt(req.query.hours, 24, 24 * 31, 'hours');
      const startTimestamp: number | undefined = parseOptionalUnixTimestamp(
        req.query.start,
        'start',
      );
      const endTimestamp: number | undefined = parseOptionalUnixTimestamp(
        req.query.end,
        'end',
      );
      validateUnixTimestampRange(
        startTimestamp,
        endTimestamp,
        24 * 31 * 60 * 60,
      );
      const limit = boundedQueryInt(req.query.limit, 100, 1_000, 'limit');

      const stats = await monitor.getMarketMakerStats({
        market,
        trader,
        hours,
        startTimestamp,
        endTimestamp,
        limit,
      });

      res.json({
        data: stats,
        meta: {
          timeframe_hours: hours,
          start_timestamp: startTimestamp,
          end_timestamp: endTimestamp,
          total_results: stats.length,
          query_timestamp: new Date().toISOString(),
        },
      });
    } catch (error) {
      console.error('Error getting market maker stats:', error);
      res.status(500).json({ error: 'Internal server error' });
    }
  });

  // Market statistics
  app.get('/markets', async (req, res) => {
    try {
      const stats = await monitor.getMarketStats();
      res.json(stats);
    } catch (error) {
      console.error('Error getting market stats:', error);
      res.status(500).json({ error: 'Internal server error' });
    }
  });

  // Raw market maker data for custom queries
  app.get('/market-makers/raw', async (req, res) => {
    try {
      const market = req.query.market as string;
      const trader = req.query.trader as string;
      const hours = boundedQueryInt(req.query.hours, 24, 24 * 31, 'hours');
      const startTimestamp: number | undefined = parseOptionalUnixTimestamp(
        req.query.start,
        'start',
      );
      const endTimestamp: number | undefined = parseOptionalUnixTimestamp(
        req.query.end,
        'end',
      );
      validateUnixTimestampRange(
        startTimestamp,
        endTimestamp,
        24 * 31 * 60 * 60,
      );
      const limit = boundedQueryInt(req.query.limit, 1_000, 5_000, 'limit');

      // Build time filter - prioritize timestamps over hours
      let timeFilter = '';
      if (startTimestamp && endTimestamp) {
        timeFilter = `timestamp >= to_timestamp(${startTimestamp}) AND timestamp <= to_timestamp(${endTimestamp})`;
      } else if (startTimestamp) {
        timeFilter = `timestamp >= to_timestamp(${startTimestamp})`;
      } else if (endTimestamp) {
        timeFilter = `timestamp <= to_timestamp(${endTimestamp})`;
      } else {
        timeFilter = `timestamp > NOW() - INTERVAL '${hours} hours'`;
      }

      let query = `
        SELECT *,
          EXTRACT(EPOCH FROM timestamp) as timestamp_unix
        FROM market_maker_stats
        WHERE ${timeFilter}
          AND total_notional_usd >= ${MIN_NOTIONAL_USD}
      `;

      const params: any[] = [];
      let paramIndex = 1;

      if (market) {
        query += ` AND market = $${paramIndex}`;
        params.push(market);
        paramIndex++;
      }

      if (trader) {
        query += ` AND trader = $${paramIndex}`;
        params.push(trader);
        paramIndex++;
      }

      query += ` ORDER BY timestamp DESC LIMIT $${paramIndex}`;
      params.push(limit);

      const result = await monitor.pool.query(query, params);

      res.json({
        data: result.rows,
        meta: {
          timeframe_hours: hours,
          start_timestamp: startTimestamp,
          end_timestamp: endTimestamp,
          total_results: result.rows.length,
          query_timestamp: new Date().toISOString(),
        },
      });
    } catch (error) {
      console.error('Error getting raw market maker data:', error);
      res.status(500).json({ error: 'Internal server error' });
    }
  });

  // Health check
  app.get('/health', (req, res) => {
    const health = monitor.getHealthStatus();
    res.status(health.healthy ? 200 : 503).json({
      status: health.starting
        ? 'starting'
        : !health.healthy
          ? 'unhealthy'
          : health.degraded
            ? 'degraded'
            : 'healthy',
      timestamp: new Date(),
      last_successful_monitoring_at: health.lastSuccessfulMonitoringAt,
      error: health.error,
      warning: health.warning,
      failed_markets: health.failedMarkets,
    });
  });

  return app;
};

// Main execution
const main = async () => {
  // Set up Prometheus metrics
  promClient.collectDefaultMetrics({
    labels: { app: 'liquidity-monitor' },
  });

  const metricsApp = express();
  metricsApp.listen(9090);

  const promMetrics = promBundle({
    includeMethod: true,
    metricsApp,
    autoregister: false,
  });
  metricsApp.use(promMetrics);

  // Initialize monitor
  const monitor = new LiquidityMonitor();
  await monitor.initDatabase();

  // Start API server
  const app = setupAPI(monitor);
  app.listen(PORT, () => {
    console.log(`Liquidity monitor API running on port ${PORT}`);
  });

  // Start monitoring
  await monitor.startMonitoring();

  // Graceful shutdown
  const gracefulShutdown = async (signal: string) => {
    console.log(`Received ${signal}, shutting down gracefully...`);
    process.exit(0);
  };

  process.on('SIGINT', () => gracefulShutdown('SIGINT'));
  process.on('SIGTERM', () => gracefulShutdown('SIGTERM'));
};

// Error handling
process.on('unhandledRejection', (reason, promise) => {
  console.error('Unhandled Rejection at:', promise, 'reason:', reason);
});

process.on('uncaughtException', (error) => {
  console.error('Uncaught Exception:', error);
  process.exit(1);
});

main().catch((error) => {
  console.error('Fatal error:', error);
  process.exit(1);
});
