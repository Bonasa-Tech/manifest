import 'dotenv/config';

import { Connection, GetProgramAccountsResponse } from '@solana/web3.js';
import { ManifestClient, Market } from '@bonasa-tech/manifest-sdk';
import { Pool } from 'pg';
import express from 'express';
import cors from 'cors';
import { USDC_MINT, STABLECOIN_MINTS } from './stats_utils/constants';

// Configuration constants
const SNAPSHOT_INTERVAL_MS = 5 * 60 * 1000; // 5 minutes
const MIN_VOLUME_THRESHOLD_USD = 1; // $1 minimum 24hr volume
const GUARANTEED_ORDERS_COUNT = 10; // Always include first 10 orders regardless of spread
const MAX_SPREAD_FROM_REFERENCE = 0.25; // 25% max distance from reference price (applied after first 10)
const PORT = 3001;

// Environment variables
const { RPC_URL, DATABASE_URL } = process.env;

if (!RPC_URL || !DATABASE_URL) {
  throw new Error('RPC_URL and DATABASE_URL are required');
}

interface OrderbookSnapshot {
  id?: number;
  market: string;
  timestamp: Date;
  bids: OrderData[];
  asks: OrderData[];
  midPrice?: number;
  bestBid?: number;
  bestAsk?: number;
  volume24hUsd: number;
}

interface OrderData {
  price: number;
  quantity: number;
  trader: string;
}

export class MarketMakerLeaderboard {
  private connection: Connection;
  public pool: Pool;
  public markets: Map<string, Market> = new Map();
  private marketVolumes: Map<string, number> = new Map();
  private isSnapshotting = false;

  constructor() {
    this.connection = new Connection(RPC_URL!);
    this.pool = new Pool({
      connectionString: DATABASE_URL!,
      ssl: { rejectUnauthorized: false },
      max: 10,
      min: 2,
      idleTimeoutMillis: 30000,
      connectionTimeoutMillis: 10000,
    });

    this.pool.on('error', (err) => {
      console.error('Database pool error:', err);
    });
  }

  /**
   * Initialize database schema
   */
  async initDatabase(): Promise<void> {
    try {
      // Raw orderbook snapshots table
      await this.pool.query(`
        CREATE TABLE IF NOT EXISTS orderbook_snapshots (
          id SERIAL PRIMARY KEY,
          market TEXT NOT NULL,
          timestamp TIMESTAMP WITH TIME ZONE NOT NULL,
          mid_price NUMERIC,
          best_bid NUMERIC,
          best_ask NUMERIC,
          volume_24h_usd NUMERIC DEFAULT 0,
          created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
        )
      `);

      // Individual orders table
      await this.pool.query(`
        CREATE TABLE IF NOT EXISTS orders (
          id SERIAL PRIMARY KEY,
          snapshot_id INTEGER NOT NULL REFERENCES orderbook_snapshots(id) ON DELETE CASCADE,
          side TEXT NOT NULL CHECK (side IN ('bid', 'ask')),
          price NUMERIC NOT NULL,
          quantity NUMERIC NOT NULL,
          trader TEXT NOT NULL,
          value_usd NUMERIC GENERATED ALWAYS AS (price * quantity) STORED
        )
      `);

      // Create indexes for performance
      await this.pool.query(`
        CREATE INDEX IF NOT EXISTS idx_orderbook_snapshots_market_timestamp 
        ON orderbook_snapshots(market, timestamp DESC)
      `);

      await this.pool.query(`
        CREATE INDEX IF NOT EXISTS idx_orders_snapshot_trader 
        ON orders(snapshot_id, trader)
      `);

      await this.pool.query(`
        CREATE INDEX IF NOT EXISTS idx_orders_trader_side 
        ON orders(trader, side)
      `);

      console.log('✅ Database schema initialized');
    } catch (error) {
      console.error('❌ Error initializing database:', error);
      throw error;
    }
  }

  /**
   * Fetch market volume data
   */
  async fetchMarketVolumes(): Promise<Map<string, number>> {
    try {
      const response = await fetch('https://mfx-stats-mainnet.fly.dev/tickers');
      const tickers = await response.json();

      const volumeMap = new Map<string, number>();

      for (const ticker of tickers) {
        if (!STABLECOIN_MINTS.has(ticker.target_currency)) continue;
        const volumeUsd = ticker.target_volume || 0;
        volumeMap.set(ticker.ticker_id, volumeUsd);
      }

      return volumeMap;
    } catch (error) {
      console.error('Error fetching market volumes:', error);
      return new Map();
    }
  }

  /**
   * Load eligible markets
   */
  async loadEligibleMarkets(): Promise<void> {
    console.log('🔄 Loading eligible markets...');

    const volumeMap = await this.fetchMarketVolumes();
    const marketProgramAccounts: GetProgramAccountsResponse =
      await ManifestClient.getMarketProgramAccounts(this.connection);

    this.markets.clear();
    this.marketVolumes.clear();

    for (const account of marketProgramAccounts) {
      const marketPk = account.pubkey.toBase58();
      const volume24h = volumeMap.get(marketPk) || 0;

      if (volume24h >= MIN_VOLUME_THRESHOLD_USD) {
        try {
          const market = Market.loadFromBuffer({
            buffer: account.account.data,
            address: account.pubkey,
          });

          // Only USDC quote markets
          const quoteMint = market.quoteMint().toBase58();
          if (quoteMint !== USDC_MINT) continue;

          // Skip markets that have never traded
          if (Number(market.quoteVolume()) === 0) continue;

          this.markets.set(marketPk, market);
          this.marketVolumes.set(marketPk, volume24h);

          console.log(
            `✅ ${marketPk.slice(-8)} - $${volume24h.toLocaleString()}`,
          );
        } catch (error) {
          console.error(`❌ Error loading market ${marketPk}:`, error);
        }
      }
    }

    console.log(`📊 Loaded ${this.markets.size} eligible markets`);
  }

  /**
   * Take orderbook snapshot for a single market
   */
  async snapshotMarket(
    marketPk: string,
    market: Market,
    timestamp: Date,
  ): Promise<OrderbookSnapshot | null> {
    try {
      await market.reload(this.connection);

      const bids = market.bids();
      const asks = market.asks();

      // Always include markets even with no orders (empty orderbook is still data)
      let referencePrice: number | undefined;
      let midPrice: number | undefined;
      let bestBid: number | undefined;
      let bestAsk: number | undefined;

      if (bids.length > 0) {
        bestBid = bids[bids.length - 1].tokenPrice;
      }
      if (asks.length > 0) {
        bestAsk = asks[asks.length - 1].tokenPrice;
      }

      // Determine reference price for filtering (only used beyond first 10 orders)
      if (bestBid && bestAsk) {
        midPrice = (bestBid + bestAsk) / 2;
        referencePrice = midPrice;
      } else if (bestBid) {
        referencePrice = bestBid;
      } else if (bestAsk) {
        referencePrice = bestAsk;
      }

      const volume24hUsd = this.marketVolumes.get(marketPk) || 0;

      // Filter bids: always include first 10, then apply 25% filter
      const filteredBids = bids
        .map((order, index) => ({
          order,
          index,
          price: order.tokenPrice,
          quantity: Number(order.numBaseTokens),
          trader: order.trader.toBase58(),
        }))
        .filter(({ index, price }) => {
          // Always include first 10 orders
          if (index < GUARANTEED_ORDERS_COUNT) return true;
          // Beyond first 10, apply 25% filter if we have a reference price
          if (referencePrice) {
            return price >= referencePrice * (1 - MAX_SPREAD_FROM_REFERENCE);
          }
          // If no reference price, include all orders
          return true;
        })
        .map(({ price, quantity, trader }) => ({ price, quantity, trader }));

      // Filter asks: always include first 10, then apply 25% filter
      const filteredAsks = asks
        .map((order, index) => ({
          order,
          index,
          price: order.tokenPrice,
          quantity: Number(order.numBaseTokens),
          trader: order.trader.toBase58(),
        }))
        .filter(({ index, price }) => {
          // Always include first 10 orders
          if (index < GUARANTEED_ORDERS_COUNT) return true;
          // Beyond first 10, apply 25% filter if we have a reference price
          if (referencePrice) {
            return price <= referencePrice * (1 + MAX_SPREAD_FROM_REFERENCE);
          }
          // If no reference price, include all orders
          return true;
        })
        .map(({ price, quantity, trader }) => ({ price, quantity, trader }));

      const snapshot: OrderbookSnapshot = {
        market: marketPk,
        timestamp,
        bids: filteredBids,
        asks: filteredAsks,
        midPrice,
        bestBid,
        bestAsk,
        volume24hUsd,
      };

      return snapshot;
    } catch (error) {
      console.error(`❌ Error snapshotting market ${marketPk}:`, error);
      return null;
    }
  }

  /**
   * Save orderbook snapshot to database
   */
  async saveSnapshot(snapshot: OrderbookSnapshot): Promise<void> {
    const client = await this.pool.connect();

    try {
      await client.query('BEGIN');

      // Insert snapshot record
      const snapshotResult = await client.query(
        `
        INSERT INTO orderbook_snapshots (market, timestamp, mid_price, best_bid, best_ask, volume_24h_usd)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
      `,
        [
          snapshot.market,
          snapshot.timestamp,
          snapshot.midPrice,
          snapshot.bestBid,
          snapshot.bestAsk,
          snapshot.volume24hUsd,
        ],
      );

      const snapshotId = snapshotResult.rows[0].id;

      // Batch insert orders
      const allOrders = [
        ...snapshot.bids.map((order) => [
          'bid',
          order.price,
          order.quantity,
          order.trader,
        ]),
        ...snapshot.asks.map((order) => [
          'ask',
          order.price,
          order.quantity,
          order.trader,
        ]),
      ];

      if (allOrders.length > 0) {
        // Create proper parameterized query for batch insert
        const orderValues = allOrders
          .map((_, index) => {
            const offset = index * 5; // 5 parameters per order (snapshot_id + 4 order fields)
            return `($${offset + 1}, $${offset + 2}, $${offset + 3}, $${offset + 4}, $${offset + 5})`;
          })
          .join(', ');

        // Include snapshotId for each order in the parameters array
        const orderParams = allOrders.flatMap((order) => [
          snapshotId,
          ...order,
        ]);

        await client.query(
          `
          INSERT INTO orders (snapshot_id, side, price, quantity, trader)
          VALUES ${orderValues}
        `,
          orderParams,
        );
      }

      await client.query('COMMIT');
    } catch (error) {
      await client.query('ROLLBACK');
      throw error;
    } finally {
      client.release();
    }
  }

  /**
   * Take snapshots of all markets
   */
  async takeSnapshots(): Promise<void> {
    if (this.isSnapshotting) {
      console.log('⏸️ Previous snapshot cycle still running, skipping...');
      return;
    }

    this.isSnapshotting = true;

    try {
      const timestamp = new Date();
      console.log(`📸 Taking snapshots at ${timestamp.toISOString()}`);

      const snapshots: OrderbookSnapshot[] = [];

      for (const [marketPk, market] of this.markets) {
        const snapshot = await this.snapshotMarket(marketPk, market, timestamp);
        if (snapshot) {
          snapshots.push(snapshot);
        }
      }

      console.log(`💾 Saving ${snapshots.length} snapshots to database`);

      // Save all snapshots
      for (const snapshot of snapshots) {
        await this.saveSnapshot(snapshot);
      }

      console.log(
        `✅ Snapshot cycle complete - ${snapshots.length} markets processed`,
      );
    } finally {
      this.isSnapshotting = false;
    }
  }

  /**
   * Start the monitoring system
   */
  async start(): Promise<void> {
    console.log('🚀 Starting Orderbook Snapshots');

    // Load markets initially
    await this.loadEligibleMarkets();

    // Take initial snapshot
    await this.takeSnapshots();

    // Set up periodic snapshots
    setInterval(async () => {
      try {
        await this.takeSnapshots();
      } catch (error) {
        console.error('Error in snapshot cycle:', error);
      }
    }, SNAPSHOT_INTERVAL_MS);

    // Reload markets every hour
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

    console.log('✅ Orderbook Snapshots started');
  }
}

// API Setup
const setupAPI = (monitor: MarketMakerLeaderboard) => {
  const app = express();
  app.use(cors());
  app.use(express.json());

  /**
   * Get orderbook snapshots in compact format
   * Query params:
   * - hours: number of hours back (default: 24)
   * - start: start timestamp (unix seconds)
   * - end: end timestamp (unix seconds)
   * - market: specific market address
   * - trader: specific trader address
   * - limit: max snapshots (default: 100)
   * - page: page number for pagination (default: 1)
   */
  app.get('/snapshots', async (req, res) => {
    try {
      const hours = req.query.hours ? parseInt(req.query.hours as string) : 24;
      const snapshotLimit = req.query.limit
        ? parseInt(req.query.limit as string)
        : 100;
      const page = req.query.page ? parseInt(req.query.page as string) : 1;
      const offset = (page - 1) * snapshotLimit;
      const startTime = req.query.start
        ? new Date(parseInt(req.query.start as string) * 1000)
        : undefined;
      const endTime = req.query.end
        ? new Date(parseInt(req.query.end as string) * 1000)
        : undefined;
      const market = req.query.market as string;
      const trader = req.query.trader as string;

      // Build time filter
      let timeFilter = '';
      const params: any[] = [];
      let paramIndex = 1;

      if (startTime && endTime) {
        timeFilter = `os.timestamp >= $${paramIndex} AND os.timestamp <= $${paramIndex + 1}`;
        params.push(startTime, endTime);
        paramIndex += 2;
      } else {
        timeFilter = `os.timestamp > NOW() - INTERVAL '${hours} hours'`;
      }

      let additionalFilters = '';
      if (market) {
        additionalFilters += ` AND os.market = $${paramIndex}`;
        params.push(market);
        paramIndex++;
      }
      if (trader) {
        additionalFilters += ` AND o.trader = $${paramIndex}`;
        params.push(trader);
        paramIndex++;
      }

      // First, get the snapshot IDs we want (limited by number of snapshots)
      const snapshotQuery = `
            SELECT DISTINCT os.id, os.timestamp
            FROM orderbook_snapshots os
            ${trader ? 'JOIN orders o ON os.id = o.snapshot_id' : ''}
            WHERE ${timeFilter} ${additionalFilters}
            ORDER BY os.timestamp DESC
            LIMIT $${paramIndex} OFFSET $${paramIndex + 1}
            `;
      params.push(snapshotLimit, offset);

      const snapshotResult = await monitor.pool.query(
        snapshotQuery,
        params.slice(0, paramIndex + 1),
      );
      const snapshotIds = snapshotResult.rows.map((row) => row.id);

      if (snapshotIds.length === 0) {
        return res.json({
          snapshots: [],
          meta: {
            timeframe_hours: hours,
            filters: { market, trader },
            pagination: {
              page,
              limit: snapshotLimit,
              total_snapshots: 0,
            },
            total_orders: 0,
            query_timestamp: new Date().toISOString(),
          },
        });
      }

      // Get total count for pagination
      let countQuery = `
    SELECT COUNT(DISTINCT os.id) as total
    FROM orderbook_snapshots os
    ${trader ? 'JOIN orders o ON os.id = o.snapshot_id' : ''}
    WHERE ${timeFilter}
    `;

      // Build count query parameters (exclude limit and offset)
      const countParams = [];
      let countParamIndex = 1;

      // Add time parameters
      if (startTime && endTime) {
        countParams.push(startTime, endTime);
        countParamIndex += 2;
      }

      // Add market filter if present
      if (market) {
        countQuery += ` AND os.market = $${countParamIndex}`;
        countParams.push(market);
        countParamIndex++;
      }

      // Add trader filter if present
      if (trader) {
        countQuery += ` AND o.trader = $${countParamIndex}`;
        countParams.push(trader);
        countParamIndex++;
      }

      const countResult = await monitor.pool.query(countQuery, countParams);
      const totalSnapshots = parseInt(countResult.rows[0].total);

      // Then get all orders for those snapshots
      const orderQuery = `
        SELECT 
          os.id as snapshot_id,
          os.market,
          EXTRACT(EPOCH FROM os.timestamp) as timestamp,
          os.mid_price,
          os.best_bid,
          os.best_ask,
          os.volume_24h_usd,
          o.side,
          o.price,
          o.quantity,
          o.trader,
          o.value_usd
        FROM orderbook_snapshots os
        JOIN orders o ON os.id = o.snapshot_id
        WHERE os.id = ANY($1)
        ORDER BY os.timestamp DESC, os.market, o.side DESC, o.price DESC
      `;

      const result = await monitor.pool.query(orderQuery, [snapshotIds]);

      // Transform flat results into compact nested structure
      const snapshotsMap = new Map();

      for (const row of result.rows) {
        const snapshotKey = `${row.snapshot_id}-${row.market}`;

        if (!snapshotsMap.has(snapshotKey)) {
          snapshotsMap.set(snapshotKey, {
            snapshot_id: row.snapshot_id,
            market: row.market,
            timestamp: parseFloat(row.timestamp),
            mid_price: row.mid_price ? parseFloat(row.mid_price) : null,
            best_bid: row.best_bid ? parseFloat(row.best_bid) : null,
            best_ask: row.best_ask ? parseFloat(row.best_ask) : null,
            volume_24h_usd: parseFloat(row.volume_24h_usd),
            bids: [],
            asks: [],
          });
        }

        const snapshot = snapshotsMap.get(snapshotKey);
        const order = {
          price: parseFloat(row.price),
          quantity: parseFloat(row.quantity),
          trader: row.trader,
          value_usd: parseFloat(row.value_usd),
        };

        if (row.side === 'bid') {
          snapshot.bids.push(order);
        } else {
          snapshot.asks.push(order);
        }
      }

      const snapshots = Array.from(snapshotsMap.values());

      res.json({
        snapshots,
        meta: {
          timeframe_hours: hours,
          filters: { market, trader },
          pagination: {
            page,
            limit: snapshotLimit,
            total_snapshots: totalSnapshots,
            total_pages: Math.ceil(totalSnapshots / snapshotLimit),
            has_next: page * snapshotLimit < totalSnapshots,
            has_prev: page > 1,
          },
          total_orders: result.rows.length,
          query_timestamp: new Date().toISOString(),
        },
      });
    } catch (error) {
      console.error('Error getting snapshots:', error);
      res.status(500).json({ error: 'Internal server error' });
    }
  });

  /**
   * Health check and system status
   */
  app.get('/health', async (req, res) => {
    try {
      const dbResult = await monitor.pool.query(
        "SELECT COUNT(*) FROM orderbook_snapshots WHERE timestamp > NOW() - INTERVAL '1 hour'",
      );
      const recentSnapshots = parseInt(dbResult.rows[0].count);

      const marketsResult = await monitor.pool.query(
        "SELECT COUNT(DISTINCT market) FROM orderbook_snapshots WHERE timestamp > NOW() - INTERVAL '1 hour'",
      );
      const activeMarkets = parseInt(marketsResult.rows[0].count);

      res.json({
        status: 'healthy',
        timestamp: new Date().toISOString(),
        metrics: {
          recent_snapshots_1h: recentSnapshots,
          active_markets_1h: activeMarkets,
          total_markets_monitored: monitor.markets.size,
          expected_snapshots_per_hour:
            Math.ceil(60 / (SNAPSHOT_INTERVAL_MS / 60000)) *
            monitor.markets.size,
        },
      });
    } catch (error) {
      console.error('Health check error:', error);
      res.status(500).json({ status: 'unhealthy', error });
    }
  });

  return app;
};

// Main execution
const main = async () => {
  // Initialize monitor
  const monitor = new MarketMakerLeaderboard();
  await monitor.initDatabase();

  // Start API server
  const app = setupAPI(monitor);
  app.listen(PORT, () => {
    console.log(`📊 Orderbook Snapshots API running on port ${PORT}`);
  });

  // Start monitoring
  await monitor.start();

  // Graceful shutdown
  const gracefulShutdown = async (signal: string) => {
    console.log(`Received ${signal}, shutting down gracefully...`);
    await monitor.pool.end();
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
