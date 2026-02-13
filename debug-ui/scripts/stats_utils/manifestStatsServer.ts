import { Mutex } from 'async-mutex';
import * as promClient from 'prom-client';
import {
  AccountInfo,
  Connection,
  GetProgramAccountsResponse,
  PublicKey,
} from '@solana/web3.js';

const WRAPPER_PROGRAM_ID = new PublicKey(
  'wMNFSTkir3HgyZTsB7uqu3i7FA73grFCptPXgrZjksL',
);
import {
  FillLogResult,
  ManifestClient,
  Market,
  RestingOrder,
} from '@cks-systems/manifest-sdk';
import { Pool, PoolClient, QueryResult } from 'pg';
import {
  VOLUME_CHECKPOINT_DURATION_SEC,
  ONE_DAY_SEC,
  DEPTHS_BPS,
  SOL_USDC_MARKET,
  CBBTC_USDC_MARKET,
  WBTC_USDC_MARKET,
  USDC_MINT,
  SOL_MINT,
  CBBTC_MINT,
  WBTC_MINT,
  STABLECOIN_MINTS,
} from './constants';
import {
  resolveActualTrader,
  chunks,
  fetchBtcPriceFromCoinGecko,
  getLifetimeVolumeForMarkets,
  sendDiscordNotification,
} from './utils';
import * as queries from './queries';
import { lookupMintTicker } from './mint';
import { fetchMarketProgramAccounts } from './marketFetcher';
import { calculateTraderPnL } from './pnl';
import { CompleteFillsQueryOptions, CompleteFillsQueryResult } from './types';
import { withRetry } from './utils';
import { WebSocketManager } from './websocketManager';
import { parseTransactionForFills } from './backfill';

// Memory management constants
const MAX_TRADERS = 50000; // Maximum number of traders to track in memory
const MAX_FILL_LOG_MARKETS = 500; // Maximum number of markets to track fills for

export class ManifestStatsServer {
  private connection: Connection;
  private wsManager: WebSocketManager | null = null;
  // Base and quote volume
  private baseVolumeAtomsSinceLastCheckpoint: Map<string, number> = new Map();
  private quoteVolumeAtomsSinceLastCheckpoint: Map<string, number> = new Map();

  // Hourly checkpoints
  private baseVolumeAtomsCheckpoints: Map<string, number[]> = new Map();
  private quoteVolumeAtomsCheckpoints: Map<string, number[]> = new Map();

  // Unix timestamps for each checkpoint (in seconds)
  private checkpointTimestamps: Map<string, number[]> = new Map();

  // Last price by market. Price is in atoms per atom.
  private lastPriceByMarket: Map<string, number> = new Map();

  // Pubkey to the number of taker & maker trades.
  private traderNumTakerTrades: Map<string, number> = new Map();
  private traderNumMakerTrades: Map<string, number> = new Map();

  private traderPositions: Map<string, Map<string, number>> = new Map();
  private traderAcquisitionValue: Map<string, Map<string, number>> = new Map();

  // Market objects used for mints and decimals.
  private markets: Map<string, Market> = new Map();

  // Tickers. Ticker from metaplex metadata with a fallback to spl token
  // registry for old stuff like wsol.
  private tickers: Map<string, [string, string]> = new Map();

  private lastFillSlot: number = 0;

  // Recent fill log results. This is not saved to database. So on a refresh,
  // the recentFills is not complete. The completeFills query will be correct
  // however.
  private fillLogResults: Map<string, FillLogResult[]> = new Map();

  // Mutex to guard all the recent fills, volume, ... Most important for recent
  // fills when a fill spills over to multiple maker orders and bursts in fill
  // logs.
  private fillMutex: Mutex = new Mutex();

  private traderTakerNotionalVolume: Map<string, number> = new Map();
  private traderMakerNotionalVolume: Map<string, number> = new Map();
  private pool: Pool;
  private isReadOnly: boolean;
  private startTime: number;

  // Prometheus metrics
  private fills: promClient.Counter<'market'>;
  private reconnects: promClient.Counter<string>;
  private volume: promClient.Gauge<'market' | 'mint' | 'side'>;
  private lastPrice: promClient.Gauge<'market'>;
  private depth: promClient.Gauge<'depth_bps' | 'market' | 'trader'>;
  private dbQueryCount: promClient.Counter<'query_type' | 'status'>;
  private dbQueryDuration: promClient.Histogram<'query_type'>;

  // Discord alerting
  private discordWebhookUrl: string | undefined;
  private lastErrorAlertSentTimeMs: number = Date.now();

  // Allquiet alerting
  private allquietWebhookUrl: string | undefined;

  // Wrapper cache: owner pubkey -> wrapper pubkey
  private wrapperCache: Map<string, string> = new Map();

  constructor(
    rpcUrl: string,
    isReadOnly: boolean,
    databaseUrl: string | undefined,
    metrics: {
      fills: promClient.Counter<'market'>;
      reconnects: promClient.Counter<string>;
      volume: promClient.Gauge<'market' | 'mint' | 'side'>;
      lastPrice: promClient.Gauge<'market'>;
      depth: promClient.Gauge<'depth_bps' | 'market' | 'trader'>;
      dbQueryCount: promClient.Counter<'query_type' | 'status'>;
      dbQueryDuration: promClient.Histogram<'query_type'>;
    },
  ) {
    this.isReadOnly = isReadOnly;
    this.startTime = Date.now();
    this.connection = new Connection(rpcUrl);
    this.fills = metrics.fills;
    this.reconnects = metrics.reconnects;
    this.volume = metrics.volume;
    this.lastPrice = metrics.lastPrice;
    this.depth = metrics.depth;
    this.dbQueryCount = metrics.dbQueryCount;
    this.dbQueryDuration = metrics.dbQueryDuration;
    this.discordWebhookUrl = process.env.DISCORD_WEBHOOK_URL;
    this.allquietWebhookUrl = process.env.ALLQUIET_WEBHOOK_URL;

    this.pool = new Pool({
      connectionString: databaseUrl,
      ssl: { rejectUnauthorized: false }, // May be needed depending on Fly Postgres configuration
    });

    this.pool.on('error', (err) => {
      console.error('Unexpected database pool error:', err);
      // Continue operation - don't let DB errors crash the server
    });

    this.initWebSocket();

    // Only initialize database schema if not in read-only mode
    if (!this.isReadOnly) {
      this.initDatabase();
    }
  }

  private initTraderPositionTracking(trader: string): void {
    if (!this.traderPositions.has(trader)) {
      this.traderPositions.set(trader, new Map<string, number>());
    }
    if (!this.traderAcquisitionValue.has(trader)) {
      this.traderAcquisitionValue.set(trader, new Map<string, number>());
    }
  }

  private updateTraderPosition(
    trader: string,
    baseMint: string,
    baseAtomsDelta: number,
    quoteAtoms: number,
    market: Market,
  ): void {
    const positions = this.traderPositions.get(trader)!;
    const acquisitionValues = this.traderAcquisitionValue.get(trader)!;

    // Get current position
    const currentPosition = positions.get(baseMint) || 0;
    const newPosition = currentPosition + baseAtomsDelta;

    // Update position
    positions.set(baseMint, newPosition);

    // Get current acquisition value
    const currentValue = acquisitionValues.get(baseMint) || 0;
    const usdcValue = Number(quoteAtoms) / 10 ** market.quoteDecimals();

    if (baseAtomsDelta > 0) {
      acquisitionValues.set(baseMint, currentValue + usdcValue);
    } else {
      acquisitionValues.set(baseMint, currentValue - usdcValue);
    }
  }

  /**
   * Send alert to allquiet.app for database error
   */
  private async sendAllquietAlert(error: any): Promise<void> {
    if (!this.allquietWebhookUrl) {
      return;
    }

    try {
      const response = await fetch(this.allquietWebhookUrl, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          alertName: 'Manifest Stats Server Database Error',
          alertStatus: 'firing',
          description: `Database error occurred: ${error.code || 'unknown'} - ${error.message || 'No message'}`,
        }),
      });

      if (!response.ok) {
        console.error(
          `Failed to send allquiet alert: ${response.status} ${response.statusText}`,
        );
      } else {
        console.log('Allquiet alert sent for database error');
      }
    } catch (alertError) {
      console.error('Error sending allquiet alert:', alertError);
    }
  }

  /**
   * Send alerts for database error (allquiet.app and Discord)
   * Rate limited to at most once per hour to avoid spam.
   */
  private async sendDatabaseErrorAlert(error: any): Promise<void> {
    // Only send alerts once an hour to avoid spam.
    const now_ms: number = Date.now();
    if (now_ms - this.lastErrorAlertSentTimeMs < 60 * 60 * 1_000) {
      return;
    }

    this.lastErrorAlertSentTimeMs = Date.now();

    // Send to allquiet.app (rate limited by check above)
    await this.sendAllquietAlert(error);

    // Send to Discord if configured (rate limited by check above)
    if (this.discordWebhookUrl) {
      const message = `**Database Alert**
**Error Details:**
- Code: ${error.code}
- Message: ${error.message}
- Time: ${new Date().toISOString()}
`;

      await sendDiscordNotification(this.discordWebhookUrl, message, {
        title: '🚨 Database Error Detected',
        timestamp: true,
      });

      console.error('Discord alert sent for database error');
    }
  }

  /**
   * Execute database query with metrics collection
   */
  private async executeQueryWithMetrics<T = QueryResult>(
    queryType: string,
    queryFn: () => Promise<T>,
  ): Promise<T> {
    const timer: () => number = this.dbQueryDuration.startTimer({
      query_type: queryType,
    });

    try {
      const result: T = await queryFn();
      this.dbQueryCount.inc({ query_type: queryType, status: 'success' });
      return result;
    } catch (error: any) {
      this.dbQueryCount.inc({ query_type: queryType, status: 'error' });
      throw error;
    } finally {
      timer();
    }
  }

  /**
   * Execute client query with metrics collection (for transactions)
   */
  private async executeClientQueryWithMetrics<T = QueryResult>(
    client: PoolClient,
    queryType: string,
    query: string,
    params?: any[],
  ): Promise<T> {
    const timer = this.dbQueryDuration.startTimer({ query_type: queryType });

    try {
      const result = await client.query(query, params);
      this.dbQueryCount.inc({ query_type: queryType, status: 'success' });
      return result as T;
    } catch (error) {
      this.dbQueryCount.inc({ query_type: queryType, status: 'error' });
      throw error;
    } finally {
      timer();
    }
  }

  /**
   * Save complete fill to database immediately (async, non-blocking)
   */
  private async saveCompleteFillToDatabase(fill: FillLogResult): Promise<void> {
    if (this.isReadOnly) {
      return; // Skip database writes in read-only mode
    }

    try {
      await withRetry(async () => {
        await this.executeQueryWithMetrics('INSERT_FILL_COMPLETE', async () =>
          this.pool.query(queries.INSERT_FILL_COMPLETE, [
            fill.slot,
            fill.market,
            fill.signature,
            fill.taker,
            fill.maker,
            fill.takerSequenceNumber,
            fill.makerSequenceNumber,
            JSON.stringify(fill),
          ]),
        );
      });
    } catch (error) {
      console.error('Error saving complete fill to database:', error);
      await this.sendDatabaseErrorAlert(error);
      // Don't throw - fire and forget
    }
  }

  private async processFillAsync(fill: FillLogResult): Promise<void> {
    try {
      const { market, baseAtoms, quoteAtoms, priceAtoms, taker, maker } = fill;

      // Use originalSigner if available (it's optional in FillLogResult)
      const originalSigner = (fill as any).originalSigner;
      const actualTaker = resolveActualTrader(taker, originalSigner);

      // Update trader counts
      this.traderNumTakerTrades.set(
        actualTaker,
        (this.traderNumTakerTrades.get(actualTaker) || 0) + 1,
      );
      this.traderNumMakerTrades.set(
        maker,
        (this.traderNumMakerTrades.get(maker) || 0) + 1,
      );

      // Initialize notional volumes if needed
      if (!this.traderTakerNotionalVolume.has(actualTaker)) {
        this.traderTakerNotionalVolume.set(actualTaker, 0);
      }
      if (!this.traderMakerNotionalVolume.has(maker)) {
        this.traderMakerNotionalVolume.set(maker, 0);
      }

      // Load market if needed (this is the slow part)
      let marketObject = this.markets.get(market);
      if (!marketObject) {
        marketObject = await this.loadNewMarket(market);
        if (!marketObject) {
          console.error('Failed to load market:', market);
          return;
        }
      }

      // Update price and volume
      this.lastPrice.set(
        { market },
        priceAtoms *
          10 ** (marketObject.baseDecimals() - marketObject.quoteDecimals()),
      );

      this.lastPriceByMarket.set(market, priceAtoms);
      this.baseVolumeAtomsSinceLastCheckpoint.set(
        market,
        (this.baseVolumeAtomsSinceLastCheckpoint.get(market) || 0) +
          Number(baseAtoms),
      );
      this.quoteVolumeAtomsSinceLastCheckpoint.set(
        market,
        (this.quoteVolumeAtomsSinceLastCheckpoint.get(market) || 0) +
          Number(quoteAtoms),
      );

      // Process notional volumes and positions
      await this.updateTradingMetrics(fill, marketObject, actualTaker);
    } catch (error) {
      console.error(
        'Error in background fill processing:',
        error,
        'Fill:',
        fill,
      );
      // Don't throw - this is fire-and-forget
    }
  }

  // Helper method for market loading
  private async loadNewMarket(market: string): Promise<Market | undefined> {
    try {
      this.baseVolumeAtomsSinceLastCheckpoint.set(market, 0);
      this.quoteVolumeAtomsSinceLastCheckpoint.set(market, 0);
      this.baseVolumeAtomsCheckpoints.set(
        market,
        new Array<number>(ONE_DAY_SEC / VOLUME_CHECKPOINT_DURATION_SEC).fill(0),
      );
      this.quoteVolumeAtomsCheckpoints.set(
        market,
        new Array<number>(ONE_DAY_SEC / VOLUME_CHECKPOINT_DURATION_SEC).fill(0),
      );
      this.checkpointTimestamps.set(
        market,
        new Array<number>(ONE_DAY_SEC / VOLUME_CHECKPOINT_DURATION_SEC).fill(0),
      );

      const marketPk = new PublicKey(market);
      const marketObject = await Market.loadFromAddress({
        connection: this.connection,
        address: marketPk,
      });

      this.markets.set(market, marketObject);
      const baseSymbol = await lookupMintTicker(
        this.connection,
        marketObject.baseMint(),
      );
      const quoteSymbol = await lookupMintTicker(
        this.connection,
        marketObject.quoteMint(),
      );

      this.tickers.set(market, [baseSymbol, quoteSymbol]);

      return marketObject;
    } catch (error) {
      console.error('Error loading market:', market, error);
      return undefined; // Changed from null to undefined
    }
  }

  // Helper method for trading metrics
  private async updateTradingMetrics(
    fill: FillLogResult,
    marketObject: Market,
    actualTaker: string,
  ): Promise<void> {
    const { baseAtoms, quoteAtoms, takerIsBuy, maker } = fill;
    const quoteMint = marketObject.quoteMint().toBase58();

    if (STABLECOIN_MINTS.has(quoteMint)) {
      const notionalVolume =
        Number(quoteAtoms) / 10 ** marketObject.quoteDecimals();

      this.traderTakerNotionalVolume.set(
        actualTaker,
        this.traderTakerNotionalVolume.get(actualTaker)! + notionalVolume,
      );
      this.traderMakerNotionalVolume.set(
        maker,
        this.traderMakerNotionalVolume.get(maker)! + notionalVolume,
      );

      const baseMint = marketObject.baseMint().toBase58();
      this.initTraderPositionTracking(actualTaker);
      this.initTraderPositionTracking(maker);

      this.updateTraderPosition(
        actualTaker,
        baseMint,
        takerIsBuy ? Number(baseAtoms) : -Number(baseAtoms),
        Number(quoteAtoms),
        marketObject,
      );

      this.updateTraderPosition(
        maker,
        baseMint,
        takerIsBuy ? -Number(baseAtoms) : Number(baseAtoms),
        Number(quoteAtoms),
        marketObject,
      );
    } else if (quoteMint === SOL_MINT) {
      const { solPrice } = this.getSolAndBtcPrices();
      if (solPrice > 0) {
        const notionalVolume =
          (Number(quoteAtoms) / 10 ** marketObject.quoteDecimals()) * solPrice;

        this.traderTakerNotionalVolume.set(
          actualTaker,
          this.traderTakerNotionalVolume.get(actualTaker)! + notionalVolume,
        );
        this.traderMakerNotionalVolume.set(
          maker,
          this.traderMakerNotionalVolume.get(maker)! + notionalVolume,
        );
      }
    } else if (quoteMint === CBBTC_MINT || quoteMint === WBTC_MINT) {
      const { cbbtcPrice } = this.getSolAndBtcPrices();
      if (cbbtcPrice > 0) {
        const notionalVolume =
          (Number(quoteAtoms) / 10 ** marketObject.quoteDecimals()) *
          cbbtcPrice;

        this.traderTakerNotionalVolume.set(
          actualTaker,
          this.traderTakerNotionalVolume.get(actualTaker)! + notionalVolume,
        );
        this.traderMakerNotionalVolume.set(
          maker,
          this.traderMakerNotionalVolume.get(maker)! + notionalVolume,
        );
      }
    }
  }

  private initWebSocket(): void {
    this.wsManager = new WebSocketManager({
      url: 'wss://mfx-feed-mainnet.fly.dev',
      reconnectDelay: 1000,
      maxReconnectDelay: 30000,
      heartbeatInterval: 30000,
      connectionTimeout: 10000,
      onMessage: (fill: FillLogResult) => {
        this.fillMutex.runExclusive(async () => {
          // Track slot for database persistence
          this.lastFillSlot = Math.max(this.lastFillSlot, fill.slot);

          // Immediately save to recent fill
          const { market } = fill;
          if (!this.fillLogResults.has(market)) {
            this.fillLogResults.set(market, []);
          }

          const prevFills = this.fillLogResults.get(market)!;
          prevFills.push(fill);

          const FILLS_TO_SAVE = 1000;
          if (prevFills.length > FILLS_TO_SAVE) {
            prevFills.splice(0, prevFills.length - FILLS_TO_SAVE);
          }
          this.fillLogResults.set(market, prevFills);

          this.fills.inc({ market });

          await this.processFillAsync(fill);

          // Queue for background processing to avoid waiting for db operation.
          setImmediate(() => this.saveCompleteFillToDatabase(fill));
        });
      },
      onConnect: () => {
        console.log('WebSocket connected to fill feed');
      },
      onDisconnect: (code, reason) => {
        console.log(
          `WebSocket disconnected from fill feed: ${code} - ${reason}`,
        );
        this.reconnects.inc();
      },
      onError: (error) => {
        console.error('WebSocket error:', error);
        this.reconnects.inc();
      },
      onReconnectAttempt: (attempt) => {
        console.log(
          `Attempting to reconnect to fill feed (attempt ${attempt})`,
        );
      },
    });

    this.wsManager.connect();
  }

  /**
   * Load all wrapper accounts and build the owner -> wrapper cache.
   * Wrapper account layout:
   *   - Bytes 0-7: Discriminant (8 bytes)
   *   - Bytes 8-39: Trader/owner pubkey (32 bytes)
   */
  private async loadWrapperCache(): Promise<void> {
    console.log('Loading wrapper accounts...');
    try {
      const wrapperAccounts = await this.connection.getProgramAccounts(
        WRAPPER_PROGRAM_ID,
        {
          commitment: 'confirmed',
        },
      );

      for (const { pubkey, account } of wrapperAccounts) {
        if (account.data.length >= 40) {
          // Extract trader pubkey from bytes 8-40
          const traderPubkey = new PublicKey(account.data.subarray(8, 40));
          this.wrapperCache.set(traderPubkey.toBase58(), pubkey.toBase58());
        }
      }

      console.log(`Loaded ${this.wrapperCache.size} wrapper accounts`);
    } catch (error) {
      console.error('Error loading wrapper accounts:', error);
      // Continue without wrapper cache - it's not critical
    }
  }

  /**
   * Get wrapper pubkey for a given owner pubkey.
   * Returns null if no wrapper found.
   */
  getWrapper(ownerPubkey: string): string | null {
    return this.wrapperCache.get(ownerPubkey) || null;
  }

  /**
   * Get all wrapper mappings (owner -> wrapper).
   */
  getAllWrappers(): { [owner: string]: string } {
    return Object.fromEntries(this.wrapperCache);
  }

  /**
   * Initialize at the start with a get program accounts.
   */
  async initialize(): Promise<void> {
    await this.loadState();
    await this.loadWrapperCache();

    const marketProgramAccounts: GetProgramAccountsResponse =
      await fetchMarketProgramAccounts(this.connection);

    marketProgramAccounts.forEach(
      (
        value: Readonly<{ account: AccountInfo<Buffer>; pubkey: PublicKey }>,
      ) => {
        const marketPk: string = value.pubkey.toBase58();

        // If we have account data, load the market and check volume
        if (value.account.data.length > 0) {
          try {
            const market: Market = Market.loadFromBuffer({
              buffer: value.account.data,
              address: new PublicKey(marketPk),
            });

            // Skip markets that have never traded to keep the amount of data
            // retention smaller.
            if (Number(market.quoteVolume()) == 0) {
              return;
            }

            this.markets.set(marketPk, market);
          } catch (err) {
            console.error(`Failed to load market ${marketPk}:`, err);
            // Continue with other markets
            return;
          }
        }

        // Initialize checkpoints regardless of whether we have market data
        if (!this.baseVolumeAtomsCheckpoints.has(marketPk)) {
          this.baseVolumeAtomsSinceLastCheckpoint.set(marketPk, 0);
          this.quoteVolumeAtomsSinceLastCheckpoint.set(marketPk, 0);
          this.baseVolumeAtomsCheckpoints.set(
            marketPk,
            new Array<number>(
              ONE_DAY_SEC / VOLUME_CHECKPOINT_DURATION_SEC,
            ).fill(0),
          );
          this.quoteVolumeAtomsCheckpoints.set(
            marketPk,
            new Array<number>(
              ONE_DAY_SEC / VOLUME_CHECKPOINT_DURATION_SEC,
            ).fill(0),
          );
          this.checkpointTimestamps.set(
            marketPk,
            new Array<number>(
              ONE_DAY_SEC / VOLUME_CHECKPOINT_DURATION_SEC,
            ).fill(0),
          );
        }
      },
    );

    const mintToSymbols: Map<string, string> = new Map();
    this.markets.forEach(async (market: Market) => {
      const baseMint: PublicKey = market.baseMint();
      const quoteMint: PublicKey = market.quoteMint();

      let baseSymbol = '';
      let quoteSymbol = '';
      if (mintToSymbols.has(baseMint.toBase58())) {
        baseSymbol = mintToSymbols.get(baseMint.toBase58())!;
      } else {
        // Sleep to backoff on RPC load.
        await new Promise((f) => setTimeout(f, 500));
        baseSymbol = await lookupMintTicker(this.connection, baseMint);
      }
      mintToSymbols.set(baseMint.toBase58(), baseSymbol);

      if (mintToSymbols.has(quoteMint.toBase58())) {
        quoteSymbol = mintToSymbols.get(quoteMint.toBase58())!;
      } else {
        quoteSymbol = await lookupMintTicker(this.connection, quoteMint);
      }
      mintToSymbols.set(quoteMint.toBase58(), quoteSymbol);

      this.tickers.set(market.address.toBase58(), [
        mintToSymbols.get(market.baseMint()!.toBase58())!,
        mintToSymbols.get(market.quoteMint()!.toBase58())!,
      ]);
    });
  }

  /**
   * Periodically save the volume so a 24 hour rolling volume can be calculated.
   */
  async advanceCheckpoints(): Promise<void> {
    this.fillMutex.runExclusive(async () => {
      console.log('Advancing checkpoints');

      // Run memory cleanup to prevent unbounded growth
      this.runMemoryCleanup();

      // Check websocket connection status - no need to reset every time
      if (this.wsManager && !this.wsManager.isConnected()) {
        console.log('WebSocket disconnected, reconnecting...');
        this.wsManager.connect();
      }

      const currentTimestamp = Math.floor(Date.now() / 1000); // Unix timestamp in seconds

      this.markets.forEach((value: Market, market: string) => {
        console.log(
          'Advancing checkpoints for market',
          market,
          'base since last',
          this.baseVolumeAtomsSinceLastCheckpoint.get(market),
        );
        this.baseVolumeAtomsCheckpoints.set(market, [
          ...this.baseVolumeAtomsCheckpoints.get(market)!.slice(1),
          this.baseVolumeAtomsSinceLastCheckpoint.get(market)!,
        ]);
        this.baseVolumeAtomsSinceLastCheckpoint.set(market, 0);

        this.quoteVolumeAtomsCheckpoints.set(market, [
          ...this.quoteVolumeAtomsCheckpoints.get(market)!.slice(1),
          this.quoteVolumeAtomsSinceLastCheckpoint.get(market)!,
        ]);
        this.quoteVolumeAtomsSinceLastCheckpoint.set(market, 0);

        // Replace any null values with 0 in the checkpoint arrays
        this.baseVolumeAtomsCheckpoints.set(
          market,
          this.baseVolumeAtomsCheckpoints.get(market)!.map((v) => v ?? 0),
        );
        this.quoteVolumeAtomsCheckpoints.set(
          market,
          this.quoteVolumeAtomsCheckpoints.get(market)!.map((v) => v ?? 0),
        );

        // Update checkpoint timestamps
        this.checkpointTimestamps.set(market, [
          ...this.checkpointTimestamps.get(market)!.slice(1),
          currentTimestamp,
        ]);

        const baseMint: string = value.baseMint().toBase58();
        const quoteMint: string = value.quoteMint().toBase58();

        // Calculate volume using only checkpoints from the last 24 hours
        const timestamps = this.checkpointTimestamps.get(market)!;
        const baseCheckpoints = this.baseVolumeAtomsCheckpoints.get(market)!;
        const quoteCheckpoints = this.quoteVolumeAtomsCheckpoints.get(market)!;
        const twentyFourHoursAgo = currentTimestamp - ONE_DAY_SEC;

        let baseVolume = 0;
        let quoteVolume = 0;

        for (let i = 0; i < timestamps.length; i++) {
          if (timestamps[i] >= twentyFourHoursAgo) {
            baseVolume += baseCheckpoints[i];
            quoteVolume += quoteCheckpoints[i];
          }
        }

        this.volume.set({ market, mint: baseMint, side: 'base' }, baseVolume);
        this.volume.set(
          { market, mint: quoteMint, side: 'quote' },
          quoteVolume,
        );
      });
    });
  }

  /**
   * Periodically save to prometheus the depths of different market makers. This
   * is expensive, so it will only be run every few minutes at most. If we
   * wanted more frequent, should subscribe to market accounts. Because the
   * number of markets is unbounded, that is not done here.
   */
  async depthProbe(): Promise<void> {
    console.log('Probing depths for market maker data');

    const marketKeys: PublicKey[] = Array.from(this.markets.keys()).map(
      (market: string) => {
        return new PublicKey(market);
      },
    );

    try {
      const marketKeysChunks: PublicKey[][] = chunks(marketKeys, 100);
      for (const marketKeysChunk of marketKeysChunks) {
        const accountInfos: (AccountInfo<Buffer> | null)[] =
          await this.connection.getMultipleAccountsInfo(marketKeysChunk);
        accountInfos.forEach(
          (accountInfo: AccountInfo<Buffer> | null, index: number) => {
            if (!accountInfo) {
              return;
            }
            const marketPk: PublicKey = marketKeys[index];
            const market: Market = Market.loadFromBuffer({
              buffer: accountInfo.data,
              address: marketPk,
            });
            const bids: RestingOrder[] = market.bids();
            const asks: RestingOrder[] = market.asks();
            if (bids.length == 0 || asks.length == 0) {
              return;
            }

            const midTokens: number =
              (bids[bids.length - 1].tokenPrice +
                asks[asks.length - 1].tokenPrice) /
              2;

            DEPTHS_BPS.forEach((depthBps: number) => {
              const bidsAtDepth: RestingOrder[] = bids.filter(
                (bid: RestingOrder) => {
                  return bid.tokenPrice > midTokens * (1 - depthBps * 0.0001);
                },
              );
              const asksAtDepth: RestingOrder[] = asks.filter(
                (ask: RestingOrder) => {
                  return ask.tokenPrice < midTokens * (1 + depthBps * 0.0001);
                },
              );

              const bidTraders: Set<string> = new Set(
                bidsAtDepth.map((bid: RestingOrder) => bid.trader.toBase58()),
              );

              bidTraders.forEach((trader: string) => {
                const bidTokensAtDepth: number = bidsAtDepth
                  .filter((bid: RestingOrder) => {
                    return bid.trader.toBase58() == trader;
                  })
                  .map((bid: RestingOrder) => {
                    return Number(bid.numBaseTokens);
                  })
                  .reduce((sum, num) => sum + num, 0);
                const askTokensAtDepth: number = asksAtDepth
                  .filter((ask: RestingOrder) => {
                    return ask.trader.toBase58() == trader;
                  })
                  .map((ask: RestingOrder) => {
                    return Number(ask.numBaseTokens);
                  })
                  .reduce((sum, num) => sum + num, 0);

                if (bidTokensAtDepth > 0 && askTokensAtDepth > 0) {
                  this.depth.set(
                    {
                      depth_bps: depthBps,
                      market: marketPk.toBase58(),
                      trader: trader,
                    },
                    Math.min(bidTokensAtDepth, askTokensAtDepth) * midTokens,
                  );
                }
              });
            });
          },
        );
      }
    } catch (err) {
      console.log('Unable to fetch depth probe', err);
    }
  }

  /**
   * Get Tickers
   *
   * https://docs.google.com/document/d/1v27QFoQq1SKT3Priq3aqPgB70Xd_PnDzbOCiuoCyixw/edit?tab=t.0#heading=h.pa64vhp5pbih
   */
  getTickers() {
    const tickers: any = [];
    const currentTimestamp = Math.floor(Date.now() / 1000);
    const twentyFourHoursAgo = currentTimestamp - ONE_DAY_SEC;

    this.markets.forEach((market: Market, marketPk: string) => {
      // Calculate volume using only checkpoints from the last 24 hours
      const timestamps = this.checkpointTimestamps.get(marketPk) || [];
      const baseCheckpoints =
        this.baseVolumeAtomsCheckpoints.get(marketPk) || [];
      const quoteCheckpoints =
        this.quoteVolumeAtomsCheckpoints.get(marketPk) || [];

      let baseVolumeAtoms = 0;
      let quoteVolumeAtoms = 0;

      for (let i = 0; i < timestamps.length; i++) {
        if (timestamps[i] >= twentyFourHoursAgo) {
          baseVolumeAtoms += baseCheckpoints[i];
          quoteVolumeAtoms += quoteCheckpoints[i];
        }
      }

      const bids = market.bids();
      const asks = market.asks();
      const bestBid = bids.length > 0 ? bids[bids.length - 1].tokenPrice : 0;
      const bestAsk = asks.length > 0 ? asks[asks.length - 1].tokenPrice : 0;

      tickers.push({
        ticker_id: marketPk,
        base_currency: market.baseMint().toBase58(),
        target_currency: market.quoteMint().toBase58(),
        last_price:
          this.lastPriceByMarket.get(marketPk)! *
          10 ** (market.baseDecimals() - market.quoteDecimals()),
        base_volume: baseVolumeAtoms / 10 ** market.baseDecimals(),
        target_volume: quoteVolumeAtoms / 10 ** market.quoteDecimals(),
        pool_id: marketPk,
        // Does not apply to orderbooks.
        liquidity_in_usd: 0,
        bid: bestBid,
        ask: bestAsk,
        // Optional: not yet implemented
        // high: 0,
        // low: 0,
      });
    });
    return tickers;
  }

  /**
   * Would be named tickers if that wasnt reserved for coingecko.
   *
   */
  getMetadata() {
    console.log(
      'Getting metadata request. Returning',
      this.tickers.size,
      'tickers',
    );
    return this.tickers;
  }

  /**
   * Get Orderbook
   *
   * https://docs.google.com/document/d/1v27QFoQq1SKT3Priq3aqPgB70Xd_PnDzbOCiuoCyixw/edit?tab=t.0#heading=h.vgzsfbx8rvps
   */
  async getOrderbook(tickerId: string, depth: number) {
    try {
      const market: Market = await Market.loadFromAddress({
        connection: this.connection,
        address: new PublicKey(tickerId),
      });
      const timestamp = Math.floor(Date.now() / 1000).toString();

      if (depth == 0) {
        return {
          ticker_id: tickerId,
          timestamp,
          bids: market
            .bids()
            .reverse()
            .map((restingOrder: RestingOrder) => {
              return [
                restingOrder.tokenPrice,
                Number(restingOrder.numBaseTokens),
              ];
            }),
          asks: market
            .asks()
            .reverse()
            .map((restingOrder: RestingOrder) => {
              return [
                restingOrder.tokenPrice,
                Number(restingOrder.numBaseTokens),
              ];
            }),
        };
      }
      const bids: RestingOrder[] = market.bids().reverse();
      const asks: RestingOrder[] = market.asks().reverse();

      // CoinGecko spec: depth = total orders, split evenly between bids/asks
      // depth=100 means 50 bids + 50 asks
      const ordersPerSide = Math.floor(depth / 2);
      const bidsUpToDepth = bids.slice(0, ordersPerSide);
      const asksUpToDepth = asks.slice(0, ordersPerSide);

      return {
        ticker_id: tickerId,
        timestamp,
        bids: bidsUpToDepth.map((restingOrder: RestingOrder) => {
          return [restingOrder.tokenPrice, Number(restingOrder.numBaseTokens)];
        }),
        asks: asksUpToDepth.reverse().map((restingOrder: RestingOrder) => {
          return [restingOrder.tokenPrice, Number(restingOrder.numBaseTokens)];
        }),
      };
    } catch (err) {
      console.log('Error getOrderbook', tickerId, depth, err);
      return {};
    }
  }

  /**
   * Get Checkpoints
   *
   * Returns all base and quote volume checkpoints for all markets
   */
  getCheckpoints(): {
    [market: string]: {
      baseCheckpoints: number[];
      quoteCheckpoints: number[];
      timestamps: number[];
    };
  } {
    const checkpointsByMarket: {
      [market: string]: {
        baseCheckpoints: number[];
        quoteCheckpoints: number[];
        timestamps: number[];
      };
    } = {};

    this.markets.forEach((_market: Market, marketPk: string) => {
      const baseCheckpoints =
        this.baseVolumeAtomsCheckpoints.get(marketPk) || [];
      const quoteCheckpoints =
        this.quoteVolumeAtomsCheckpoints.get(marketPk) || [];
      const timestamps = this.checkpointTimestamps.get(marketPk) || [];

      checkpointsByMarket[marketPk] = {
        baseCheckpoints,
        quoteCheckpoints,
        timestamps,
      };
    });

    return checkpointsByMarket;
  }

  /**
   * Get Notional Volume (USD) by Market
   *
   * Returns USD notional traded on each market in the last 24 hours
   */
  async getNotional(): Promise<{ [market: string]: number }> {
    const notionalByMarket: { [market: string]: number } = {};

    // Get SOL price for converting SOL-quoted volumes to USDC equivalent
    const solPriceAtoms = this.lastPriceByMarket.get(SOL_USDC_MARKET);
    const solUsdcMarket = this.markets.get(SOL_USDC_MARKET);
    let solPrice = 0;
    if (solPriceAtoms && solUsdcMarket) {
      solPrice =
        solPriceAtoms *
        10 ** (solUsdcMarket.baseDecimals() - solUsdcMarket.quoteDecimals());
    }

    // Get CBBTC price for converting CBBTC-quoted volumes to USDC equivalent
    const cbbtcPriceAtoms = this.lastPriceByMarket.get(CBBTC_USDC_MARKET);
    const cbbtcUsdcMarket = this.markets.get(CBBTC_USDC_MARKET);
    let cbbtcPrice = 0;
    if (cbbtcPriceAtoms && cbbtcUsdcMarket) {
      cbbtcPrice =
        cbbtcPriceAtoms *
        10 **
          (cbbtcUsdcMarket.baseDecimals() - cbbtcUsdcMarket.quoteDecimals());
    }

    // Get WBTC price as fallback for BTC conversion
    const wbtcPriceAtoms = this.lastPriceByMarket.get(WBTC_USDC_MARKET);
    const wbtcUsdcMarket = this.markets.get(WBTC_USDC_MARKET);
    let wbtcPrice = 0;
    if (wbtcPriceAtoms && wbtcUsdcMarket) {
      wbtcPrice =
        wbtcPriceAtoms *
        10 ** (wbtcUsdcMarket.baseDecimals() - wbtcUsdcMarket.quoteDecimals());
    }

    // Use whichever BTC price is available (prefer CBBTC, fallback to WBTC, then CoinGecko)
    let btcPrice = cbbtcPrice > 0 ? cbbtcPrice : wbtcPrice;

    // If no BTC price available from markets, fetch from CoinGecko
    if (btcPrice === 0) {
      btcPrice = await fetchBtcPriceFromCoinGecko();
    }

    const currentTimestamp = Math.floor(Date.now() / 1000);
    const twentyFourHoursAgo = currentTimestamp - ONE_DAY_SEC;

    this.markets.forEach((market: Market, marketPk: string) => {
      // Calculate volume using only checkpoints from the last 24 hours
      const timestamps = this.checkpointTimestamps.get(marketPk) || [];
      const quoteCheckpoints =
        this.quoteVolumeAtomsCheckpoints.get(marketPk) || [];

      let checkpointsVolume = 0;
      for (let i = 0; i < timestamps.length; i++) {
        if (timestamps[i] >= twentyFourHoursAgo) {
          checkpointsVolume += quoteCheckpoints[i];
        }
      }

      // Include both the checkpoints AND the volume since last checkpoint
      const currentPeriodVolume =
        this.quoteVolumeAtomsSinceLastCheckpoint.get(marketPk) || 0;
      const totalVolumeAtoms = checkpointsVolume + currentPeriodVolume;

      const quoteVolume: number =
        totalVolumeAtoms / 10 ** market.quoteDecimals();

      if (quoteVolume === 0) {
        return;
      }

      const quoteMint = market.quoteMint().toBase58();

      // Track stablecoin quote volume directly (USDC, USDT, PYUSD, USDS, USD1)
      if (STABLECOIN_MINTS.has(quoteMint)) {
        notionalByMarket[marketPk] = quoteVolume;
        return;
      }

      // Convert SOL quote volume to USDC equivalent
      if (quoteMint === SOL_MINT && solPrice > 0) {
        notionalByMarket[marketPk] = quoteVolume * solPrice;
        return;
      }

      // Convert CBBTC/WBTC quote volume to USDC equivalent
      if (
        (quoteMint === CBBTC_MINT || quoteMint === WBTC_MINT) &&
        btcPrice > 0
      ) {
        notionalByMarket[marketPk] = quoteVolume * btcPrice;
        return;
      }

      // If we can't convert to USD, don't include it
    });

    return notionalByMarket;
  }

  /**
   * Get normalized SOL and BTC prices
   * @returns Object containing solPrice and cbbtcPrice (both normalized to USDC)
   */
  private getSolAndBtcPrices(): { solPrice: number; cbbtcPrice: number } {
    let solPrice = 0;
    let cbbtcPrice = 0;

    // Get SOL price for converting SOL-quoted volumes to USDC equivalent
    const solPriceAtoms = this.lastPriceByMarket.get(SOL_USDC_MARKET);
    const solUsdcMarket = this.markets.get(SOL_USDC_MARKET);
    if (solPriceAtoms && solUsdcMarket) {
      solPrice =
        solPriceAtoms *
        10 ** (solUsdcMarket.baseDecimals() - solUsdcMarket.quoteDecimals());
    }

    // Get CBBTC price for converting CBBTC-quoted volumes to USDC equivalent
    const cbbtcPriceAtoms = this.lastPriceByMarket.get(CBBTC_USDC_MARKET);
    const cbbtcUsdcMarket = this.markets.get(CBBTC_USDC_MARKET);
    if (cbbtcPriceAtoms && cbbtcUsdcMarket) {
      cbbtcPrice =
        cbbtcPriceAtoms *
        10 **
          (cbbtcUsdcMarket.baseDecimals() - cbbtcUsdcMarket.quoteDecimals());
    }

    return { solPrice, cbbtcPrice };
  }

  /**
   * Get Volume
   *
   * https://docs.llama.fi/list-your-project/other-dashboards/dimensions
   */
  async getVolume() {
    let marketProgramAccounts: GetProgramAccountsResponse;
    let lifetimeVolume = 0;

    // Get normalized SOL and BTC prices
    const { solPrice, cbbtcPrice } = this.getSolAndBtcPrices();

    try {
      marketProgramAccounts = await ManifestClient.getMarketProgramAccounts(
        this.connection,
      );

      lifetimeVolume = getLifetimeVolumeForMarkets(
        marketProgramAccounts,
        solPrice,
        cbbtcPrice,
      );
    } catch (error) {
      console.error(
        'Failed to get market program accounts for volume calculation:',
        error,
      );
      // Return zero lifetime volume on error.
      lifetimeVolume = 0;
    }

    const dailyVolumesByToken: Map<string, number> = new Map();
    let dailyUsdcEquivalentVolume = 0;
    let dailyDirectUsdcVolume = 0;

    const currentTimestamp = Math.floor(Date.now() / 1000);
    const twentyFourHoursAgo = currentTimestamp - ONE_DAY_SEC;

    this.markets.forEach((market: Market, marketPk: string) => {
      // Calculate volume using only checkpoints from the last 24 hours
      const timestamps = this.checkpointTimestamps.get(marketPk) || [];
      const baseCheckpoints =
        this.baseVolumeAtomsCheckpoints.get(marketPk) || [];
      const quoteCheckpoints =
        this.quoteVolumeAtomsCheckpoints.get(marketPk) || [];

      let baseVolumeAtoms = 0;
      let quoteVolumeAtoms = 0;

      for (let i = 0; i < timestamps.length; i++) {
        if (timestamps[i] >= twentyFourHoursAgo) {
          baseVolumeAtoms += baseCheckpoints[i];
          quoteVolumeAtoms += quoteCheckpoints[i];
        }
      }

      const baseVolume: number = baseVolumeAtoms / 10 ** market.baseDecimals();
      const quoteVolume: number =
        quoteVolumeAtoms / 10 ** market.quoteDecimals();
      const baseMint: string = 'solana:' + market.baseMint().toBase58();
      const quoteMint: string = 'solana:' + market.quoteMint().toBase58();
      if (baseVolume == 0 || quoteVolume == 0) {
        return;
      }
      // Track individual token volumes (excluding USDC which we'll handle separately)
      if (!dailyVolumesByToken.has(baseMint)) {
        dailyVolumesByToken.set(baseMint, 0);
      }
      dailyVolumesByToken.set(
        baseMint,
        dailyVolumesByToken.get(baseMint)! + baseVolume,
      );

      // Handle quote volumes differently for USDC vs other tokens
      if (market.quoteMint().toBase58() != USDC_MINT) {
        if (!dailyVolumesByToken.has(quoteMint)) {
          dailyVolumesByToken.set(quoteMint, 0);
        }
        dailyVolumesByToken.set(
          quoteMint,
          dailyVolumesByToken.get(quoteMint)! + quoteVolume,
        );
      }

      // Calculate total USDC equivalent volume
      if (market.quoteMint().toBase58() == SOL_MINT && solPrice > 0) {
        dailyUsdcEquivalentVolume += quoteVolume * solPrice;
      } else if (market.quoteMint().toBase58() == USDC_MINT) {
        dailyDirectUsdcVolume += quoteVolume;
        dailyUsdcEquivalentVolume += quoteVolume;
      }
    });

    // Report direct USDC volume separately and combined volume under USDC key
    const usdcKey = 'solana:' + USDC_MINT;
    if (dailyDirectUsdcVolume > 0) {
      dailyVolumesByToken.set(
        'manifest:direct_usdc_volume',
        dailyDirectUsdcVolume,
      );
    }
    if (dailyUsdcEquivalentVolume > 0) {
      dailyVolumesByToken.set(usdcKey, dailyUsdcEquivalentVolume);
    }

    return {
      totalVolume: {
        [usdcKey]: lifetimeVolume,
      },
      dailyVolume: Object.fromEntries(dailyVolumesByToken),
    };
  }
  /**
   * Get Traders to be used in a leaderboard if a UI wants to.
   * Returns counts for taker/maker trades and volumes.
   */
  getTraders(
    includeDebug: boolean = false,
    limit: number = 500,
  ): {
    [key: string]: {
      taker: number;
      maker: number;
      takerNotionalVolume: number;
      makerNotionalVolume: number;
      pnl: number;
      _debug?: any;
    };
  } {
    const allTraders = new Set<string>([
      ...Array.from(this.traderNumTakerTrades.keys()),
      ...Array.from(this.traderNumMakerTrades.keys()),
    ]);

    // Sort traders by total volume to get the most active ones
    const tradersByVolume = Array.from(allTraders)
      .map((trader) => ({
        trader,
        totalVolume:
          (this.traderTakerNotionalVolume.get(trader) || 0) +
          (this.traderMakerNotionalVolume.get(trader) || 0),
      }))
      .sort((a, b) => b.totalVolume - a.totalVolume)
      .slice(0, limit); // Only process top N traders

    const traderData: {
      [key: string]: {
        taker: number;
        maker: number;
        takerNotionalVolume: number;
        makerNotionalVolume: number;
        pnl: number;
        _debug?: any;
      };
    } = {};

    tradersByVolume.forEach(({ trader }) => {
      const takerNotionalVolume =
        this.traderTakerNotionalVolume.get(trader) || 0;
      const makerNotionalVolume =
        this.traderMakerNotionalVolume.get(trader) || 0;

      const pnlResult = calculateTraderPnL(
        trader,
        this.traderPositions,
        this.traderAcquisitionValue,
        this.markets,
        this.lastPriceByMarket,
        includeDebug,
      );

      const pnl =
        typeof pnlResult === 'number' ? pnlResult : pnlResult.totalPnL;

      traderData[trader] = {
        taker: this.traderNumTakerTrades.get(trader) || 0,
        maker: this.traderNumMakerTrades.get(trader) || 0,
        takerNotionalVolume,
        makerNotionalVolume,
        pnl,
      };

      if (includeDebug && typeof pnlResult !== 'number') {
        traderData[trader]._debug = pnlResult;
      }
    });

    return traderData;
  }

  async getAlts(): Promise<{ alt: string; market: string }[]> {
    const response = await this.executeQueryWithMetrics(
      'SELECT_ALT_MARKETS',
      async () => this.pool.query(queries.SELECT_ALT_MARKETS),
    );
    return response.rows.map((r) => ({ alt: r.alt, market: r.market }));
  }

  /**
   * Get array of recent fills.
   */
  getRecentFills(market: string) {
    return { [market]: this.fillLogResults.get(market) };
  }

  async getCompleteFillsFromDatabase(
    options: CompleteFillsQueryOptions = {},
  ): Promise<CompleteFillsQueryResult> {
    const {
      market,
      taker,
      maker,
      signature,
      limit = 100,
      offset = 0,
      fromSlot,
      toSlot,
    } = options;

    try {
      const conditions: string[] = [];
      const params: any[] = [];
      let paramIndex = 1;

      if (market) {
        conditions.push(`market = $${paramIndex++}`);
        params.push(market);
      }

      if (taker) {
        conditions.push(`taker = $${paramIndex++}`);
        params.push(taker);
      }

      if (maker) {
        conditions.push(`maker = $${paramIndex++}`);
        params.push(maker);
      }

      if (signature) {
        conditions.push(`signature = $${paramIndex++}`);
        params.push(signature);
      }

      if (fromSlot) {
        conditions.push(`slot >= $${paramIndex++}`);
        params.push(fromSlot);
      }

      if (toSlot) {
        conditions.push(`slot <= $${paramIndex++}`);
        params.push(toSlot);
      }

      const whereClause =
        conditions.length > 0 ? `WHERE ${conditions.join(' AND ')}` : '';

      // No longer   Get complete fills count
      // const countResult = await this.executeQueryWithMetrics(
      //   'SELECT_FILLS_COUNT',
      //   async () =>
      //     this.pool.query(
      //       `${queries.SELECT_FILLS_COMPLETE_COUNT_BASE} ${whereClause}`,
      //       params,
      //     ),
      // );
      // const total = parseInt(countResult.rows[0].total);

      // Get data
      const dataQuery = `
      ${queries.SELECT_FILLS_COMPLETE_DATA_BASE}
      ${whereClause}
      ORDER BY slot DESC, timestamp DESC
      LIMIT $${paramIndex++} OFFSET $${paramIndex++}
    `;

      params.push(limit, offset);
      const dataResult = await this.executeQueryWithMetrics(
        'SELECT_FILLS_DATA',
        async () => this.pool.query(dataQuery, params),
      );

      const fills: FillLogResult[] = dataResult.rows.map(
        (row) => row.fill_data,
      );

      return {
        fills,
        total: 0,
        hasMore: true,
      };
    } catch (error) {
      console.error('Error querying complete fills:', error);
      throw error;
    }
  }

  /**
   * Backfill fills from a transaction signature.
   * Fetches the transaction from RPC, parses fills, and inserts any missing ones.
   * Returns the number of fills backfilled.
   */
  async backfillTransaction(
    signature: string,
  ): Promise<{ backfilled: number; alreadyExisted: number }> {
    if (this.isReadOnly) {
      throw new Error('Cannot backfill in read-only mode');
    }

    // Parse fills from the transaction
    const fills: FillLogResult[] = await parseTransactionForFills(
      this.connection,
      signature,
    );

    if (fills.length === 0) {
      return { backfilled: 0, alreadyExisted: 0 };
    }

    let backfilled = 0;
    let alreadyExisted = 0;

    for (const fill of fills) {
      try {
        const result: QueryResult = await this.pool.query(
          queries.INSERT_FILL_COMPLETE,
          [
            fill.slot,
            fill.market,
            fill.signature,
            fill.taker,
            fill.maker,
            fill.takerSequenceNumber,
            fill.makerSequenceNumber,
            JSON.stringify(fill),
          ],
        );

        // INSERT with ON CONFLICT DO NOTHING returns rowCount 0 if it already existed
        if (result.rowCount === 0) {
          alreadyExisted++;
        } else {
          backfilled++;
        }
      } catch (error) {
        console.error(`Error backfilling fill from ${signature}:`, error);
        throw error;
      }
    }

    console.log(
      `Backfill for ${signature}: ${backfilled} new, ${alreadyExisted} already existed`,
    );

    return { backfilled, alreadyExisted };
  }

  /**
   * Set up database schema if needed
   */
  async initDatabase(): Promise<void> {
    try {
      // Create tables if they don't exist

      // Market_volumes table is deprecated, volume data is now in market_checkpoints
      // Fill_log_result table is deprecated. fill_log_complete is now used.

      await this.executeQueryWithMetrics(
        'CREATE_STATE_CHECKPOINTS_TABLE',
        async () => this.pool.query(queries.CREATE_STATE_CHECKPOINTS_TABLE),
      );
      await this.executeQueryWithMetrics(
        'CREATE_MARKET_CHECKPOINTS_TABLE',
        async () => this.pool.query(queries.CREATE_MARKET_CHECKPOINTS_TABLE),
      );
      await this.executeQueryWithMetrics(
        'CREATE_TRADER_STATS_TABLE',
        async () => this.pool.query(queries.CREATE_TRADER_STATS_TABLE),
      );
      await this.executeQueryWithMetrics(
        'CREATE_TRADER_POSITIONS_TABLE',
        async () => this.pool.query(queries.CREATE_TRADER_POSITIONS_TABLE),
      );
      await this.executeQueryWithMetrics(
        'CREATE_FILLS_COMPLETE_TABLE',
        async () => this.pool.query(queries.CREATE_FILLS_COMPLETE_TABLE),
      );
      await this.executeQueryWithMetrics(
        'CREATE_FILLS_COMPLETE_INDEXES',
        async () => this.pool.query(queries.CREATE_FILLS_COMPLETE_INDEXES),
      );
      await this.executeQueryWithMetrics('CREATE_ALT_MARKETS_TABLE', async () =>
        this.pool.query(queries.CREATE_ALT_MARKETS_TABLE),
      );

      // Run migrations for existing tables
      await this.executeQueryWithMetrics(
        'ALTER_MARKET_CHECKPOINTS_ADD_TIMESTAMPS',
        async () =>
          this.pool.query(queries.ALTER_MARKET_CHECKPOINTS_ADD_TIMESTAMPS),
      );

      console.log('Database schema initialized');
    } catch (error) {
      console.error('Error initializing database:', error);
      throw error;
    }
  }

  /**
   * Save checkpoint and return checkpoint ID
   */
  private async saveLastFillSlot(): Promise<number> {
    if (this.isReadOnly) {
      throw new Error('Cannot save checkpoint in read-only mode');
    }

    console.log('Saving checkpoint to database...');
    let client: PoolClient | undefined;

    try {
      client = await this.pool.connect();
      client.on('error', (err: Error) => {
        console.error('Database client error during checkpoint save:', err);
      });

      await this.executeClientQueryWithMetrics(
        client,
        'BEGIN_TRANSACTION',
        queries.BEGIN_TRANSACTION,
      );

      const checkpointResult: QueryResult =
        await this.executeClientQueryWithMetrics(
          client,
          'INSERT_STATE_CHECKPOINT',
          queries.INSERT_STATE_CHECKPOINT,
          [this.lastFillSlot],
        );

      const checkpointId: number = checkpointResult.rows[0].id;

      await this.executeClientQueryWithMetrics(
        client,
        'COMMIT_TRANSACTION',
        queries.COMMIT_TRANSACTION,
      );
      console.log(`Checkpoint saved with ID: ${checkpointId}`);

      return checkpointId;
    } catch (error: any) {
      console.error('Error saving checkpoint:', error);
      await this.sendDatabaseErrorAlert(error);

      if (client) {
        try {
          await this.executeClientQueryWithMetrics(
            client,
            'ROLLBACK_TRANSACTION',
            queries.ROLLBACK_TRANSACTION,
          );
        } catch (rollbackError: any) {
          console.error('Error during checkpoint rollback:', rollbackError);
        }
      }
      throw error;
    } finally {
      if (client) {
        try {
          client.release();
        } catch (releaseError: any) {
          console.error(
            'Error releasing client after checkpoint save:',
            releaseError,
          );
        }
      }
    }
  }

  /**
   * Save volume and market data for a checkpoint
   */
  private async saveVolumeAndMarketData(checkpointId: number): Promise<void> {
    if (this.isReadOnly) {
      return; // Skip in read-only mode
    }

    console.log('Saving volume and market data to database...');
    let client: PoolClient | undefined;

    try {
      client = await this.pool.connect();
      client.on('error', (err: Error) => {
        console.error('Database client error during volume save:', err);
      });

      await this.executeClientQueryWithMetrics(
        client,
        'BEGIN_TRANSACTION',
        queries.BEGIN_TRANSACTION,
      );

      // Save market checkpoints
      const checkpointPromises: Promise<QueryResult>[] = [];
      for (const [
        market,
        baseCheckpoints,
      ] of this.baseVolumeAtomsCheckpoints.entries()) {
        const quoteCheckpoints: number[] =
          this.quoteVolumeAtomsCheckpoints.get(market) || [];
        const checkpointTimestamps: number[] =
          this.checkpointTimestamps.get(market) || [];
        const lastPrice: number = this.lastPriceByMarket.get(market) || 0;

        checkpointPromises.push(
          client.query(queries.INSERT_MARKET_CHECKPOINT, [
            checkpointId,
            market,
            JSON.stringify(baseCheckpoints),
            JSON.stringify(quoteCheckpoints),
            JSON.stringify(checkpointTimestamps),
            lastPrice,
          ]),
        );
      }

      console.log('Awaiting checkpoint inserts to complete');
      await Promise.all(checkpointPromises);

      await this.executeClientQueryWithMetrics(
        client,
        'COMMIT_TRANSACTION',
        queries.COMMIT_TRANSACTION,
      );
      console.log('Volume and market data saved successfully');
    } catch (error: any) {
      console.error('Error saving volume and market data:', error);
      await this.sendDatabaseErrorAlert(error);

      if (client) {
        try {
          await this.executeClientQueryWithMetrics(
            client,
            'ROLLBACK_TRANSACTION',
            queries.ROLLBACK_TRANSACTION,
          );
        } catch (rollbackError: any) {
          console.error('Error during volume data rollback:', rollbackError);
        }
      }
      throw error;
    } finally {
      if (client) {
        try {
          client.release();
        } catch (releaseError: any) {
          console.error(
            'Error releasing client after volume save:',
            releaseError,
          );
        }
      }
    }
  }

  /**
   * Save trader data for a checkpoint
   */
  private async saveTraderData(checkpointId: number): Promise<void> {
    if (this.isReadOnly) {
      return; // Skip in read-only mode
    }

    console.log('Saving trader data to database...');
    let client: PoolClient | undefined;

    try {
      client = await this.pool.connect();
      client.on('error', (err: Error) => {
        console.error('Database client error during trader data save:', err);
      });

      await this.executeClientQueryWithMetrics(
        client,
        'BEGIN_TRANSACTION',
        queries.BEGIN_TRANSACTION,
      );

      // Save trader stats in batches
      console.log('Saving trader stats in batches');
      const traderArray: string[] = Array.from(
        new Set([
          ...Array.from(this.traderNumTakerTrades.keys()),
          ...Array.from(this.traderNumMakerTrades.keys()),
        ]),
      );
      const TRADER_BATCH_SIZE: number = 20;

      for (let i = 0; i < traderArray.length; i += TRADER_BATCH_SIZE) {
        const batch: string[] = traderArray.slice(i, i + TRADER_BATCH_SIZE);
        const batchPromises: Promise<QueryResult>[] = [];

        for (const trader of batch) {
          const numTakerTrades: number =
            this.traderNumTakerTrades.get(trader) || 0;
          const numMakerTrades: number =
            this.traderNumMakerTrades.get(trader) || 0;
          const takerVolume: number =
            this.traderTakerNotionalVolume.get(trader) || 0;
          const makerVolume: number =
            this.traderMakerNotionalVolume.get(trader) || 0;

          batchPromises.push(
            client.query(queries.INSERT_TRADER_STATS, [
              checkpointId,
              trader,
              numTakerTrades,
              numMakerTrades,
              takerVolume,
              makerVolume,
            ]),
          );
        }

        await Promise.all(batchPromises);
      }

      // Save trader positions with filtering and batching
      console.log('Saving trader positions with filtering');
      const POSITION_THRESHOLD: number = 1;
      const BATCH_SIZE: number = 10;
      const DELAY_BETWEEN_BATCHES: number = 50;

      const delay = (ms: number | undefined): Promise<void> =>
        new Promise((resolve) => setTimeout(resolve, ms));

      let traderCount: number = 0;
      for (const [trader, positions] of this.traderPositions.entries()) {
        const acquisitionValues: Map<string, number> =
          this.traderAcquisitionValue.get(trader) || new Map();
        const positionBatchPromises: Promise<QueryResult>[] = [];

        for (const [mint, position] of positions.entries()) {
          const acquisitionValue: number = acquisitionValues.get(mint) || 0;

          if (
            Math.abs(position) === 0 ||
            Math.abs(acquisitionValue) < POSITION_THRESHOLD
          ) {
            continue;
          }

          positionBatchPromises.push(
            client.query(queries.INSERT_TRADER_POSITION, [
              checkpointId,
              trader,
              mint,
              position,
              acquisitionValue,
            ]),
          );
        }

        if (positionBatchPromises.length > 0) {
          await Promise.all(positionBatchPromises);
        }

        traderCount++;
        if (traderCount % BATCH_SIZE === 0) {
          await delay(DELAY_BETWEEN_BATCHES);
        }
      }

      await this.executeClientQueryWithMetrics(
        client,
        'COMMIT_TRANSACTION',
        queries.COMMIT_TRANSACTION,
      );
      console.log('Trader data saved successfully');
    } catch (error: any) {
      console.error('Error saving trader data:', error);
      await this.sendDatabaseErrorAlert(error);

      if (client) {
        try {
          await this.executeClientQueryWithMetrics(
            client,
            'ROLLBACK_TRANSACTION',
            queries.ROLLBACK_TRANSACTION,
          );
        } catch (rollbackError: any) {
          console.error('Error during trader data rollback:', rollbackError);
        }
      }
      throw error;
    } finally {
      if (client) {
        try {
          client.release();
        } catch (releaseError: any) {
          console.error(
            'Error releasing client after trader data save:',
            releaseError,
          );
        }
      }
    }
  }

  /**
   * Clean up old checkpoints. This deletes the state_checkpoint with the id as
   * well as the market checkpoints with the base and volume. This ensures that
   * these tables only have 1 save of data in them and are just acting as a
   * disk, not as a database.
   */
  private async cleanupOldCheckpoints(
    previousCheckpointId: number,
  ): Promise<void> {
    if (this.isReadOnly) {
      return; // Skip in read-only mode
    }

    console.log('Cleaning up old checkpoints...');
    let client: PoolClient | undefined;

    try {
      client = await this.pool.connect();
      client.on('error', (err: Error) => {
        console.error('Database client error during cleanup:', err);
      });

      await this.executeClientQueryWithMetrics(
        client,
        'BEGIN_TRANSACTION',
        queries.BEGIN_TRANSACTION,
      );

      await this.executeClientQueryWithMetrics(
        client,
        'DELETE_OLD_CHECKPOINTS',
        queries.DELETE_OLD_CHECKPOINTS,
        [previousCheckpointId],
      );

      await this.executeClientQueryWithMetrics(
        client,
        'DELETE_OLD_MARKET_VOLUMES',
        queries.DELETE_OLD_MARKET_VOLUMES,
        [previousCheckpointId],
      );

      await this.executeClientQueryWithMetrics(
        client,
        'COMMIT_TRANSACTION',
        queries.COMMIT_TRANSACTION,
      );
      console.log('Old checkpoints cleaned up successfully');
    } catch (error: any) {
      console.error('Error cleaning up old checkpoints:', error);
      await this.sendDatabaseErrorAlert(error);

      if (client) {
        try {
          await this.executeClientQueryWithMetrics(
            client,
            'ROLLBACK_TRANSACTION',
            queries.ROLLBACK_TRANSACTION,
          );
        } catch (rollbackError: any) {
          console.error('Error during cleanup rollback:', rollbackError);
        }
      }
      throw error;
    } finally {
      if (client) {
        try {
          client.release();
        } catch (releaseError: any) {
          console.error('Error releasing client after cleanup:', releaseError);
        }
      }
    }
  }

  /**
   * Save current state to database
   */
  async saveState(): Promise<void> {
    if (this.isReadOnly) {
      console.log('Skipping state save (read-only mode)');
      return;
    }

    console.log('Saving state to database...');

    try {
      // Save checkpoint first and get the ID. This is a different checkpoint from the volume checkpoints.
      const checkpointId: number = await this.saveLastFillSlot();

      // Save volume and market data in separate transaction
      await this.saveVolumeAndMarketData(checkpointId);

      // Save trader data in separate transaction
      await this.saveTraderData(checkpointId);

      // Clean up old checkpoints - this gets its own transaction
      await this.cleanupOldCheckpoints(checkpointId);

      console.log('All state saved successfully to database');
    } catch (error: any) {
      console.error('Error saving state to database:', error);
      await this.sendDatabaseErrorAlert(error);
      // Don't re-throw - we want to continue operation even after errors
    }
  }

  /**
   * Load state from database
   */
  async loadState(): Promise<boolean> {
    console.log('Loading state from database...');

    try {
      // Get the most recent checkpoint
      const checkpointResultRecent: QueryResult =
        await this.executeQueryWithMetrics(
          'SELECT_RECENT_CHECKPOINT',
          async () => this.pool.query(queries.SELECT_RECENT_CHECKPOINT),
        );

      if (checkpointResultRecent.rowCount === 0) {
        console.log('No saved lastFillSlot found in database');
        return false;
      }

      const checkpointId: number = checkpointResultRecent.rows[0].id;
      this.lastFillSlot = checkpointResultRecent.rows[0].last_fill_slot;

      // Load market checkpoints
      const marketCheckpointsResult: QueryResult =
        await this.executeQueryWithMetrics(
          'SELECT_MARKET_CHECKPOINTS',
          async () =>
            this.pool.query(queries.SELECT_MARKET_CHECKPOINTS, [checkpointId]),
        );

      for (const row of marketCheckpointsResult.rows) {
        const market: string = row.market;
        const baseCheckpoints: number[] = row.base_volume_checkpoints || [];
        const quoteCheckpoints: number[] = row.quote_volume_checkpoints || [];
        const timestamps: number[] = row.checkpoint_timestamps || [];

        this.baseVolumeAtomsCheckpoints.set(market, baseCheckpoints);
        this.quoteVolumeAtomsCheckpoints.set(market, quoteCheckpoints);
        this.checkpointTimestamps.set(market, timestamps);
        if (row.last_price) {
          this.lastPriceByMarket.set(market, Number(row.last_price));
        }
      }

      console.log('State loaded successfully from database');
      return true;
    } catch (error: unknown) {
      console.error('Error loading state from database:', error);
      return false;
    }
  }

  /**
   * Prune inactive traders to prevent memory leaks.
   * Keeps only the top traders by volume when the limit is exceeded.
   */
  private pruneInactiveTraders(): void {
    const allTraders = new Set<string>([
      ...Array.from(this.traderNumTakerTrades.keys()),
      ...Array.from(this.traderNumMakerTrades.keys()),
    ]);

    const traderCount = allTraders.size;
    if (traderCount <= MAX_TRADERS) {
      return;
    }

    console.log(
      `Pruning traders: ${traderCount} traders exceeds limit of ${MAX_TRADERS}`,
    );

    // Sort traders by total volume and keep top MAX_TRADERS
    const tradersByVolume = Array.from(allTraders)
      .map((trader) => ({
        trader,
        totalVolume:
          (this.traderTakerNotionalVolume.get(trader) || 0) +
          (this.traderMakerNotionalVolume.get(trader) || 0),
      }))
      .sort((a, b) => b.totalVolume - a.totalVolume);

    // Keep top MAX_TRADERS traders
    const tradersToKeep = new Set(
      tradersByVolume.slice(0, MAX_TRADERS).map((t) => t.trader),
    );

    // Remove traders not in the keep set
    let removedCount = 0;
    for (const trader of allTraders) {
      if (!tradersToKeep.has(trader)) {
        this.traderNumTakerTrades.delete(trader);
        this.traderNumMakerTrades.delete(trader);
        this.traderTakerNotionalVolume.delete(trader);
        this.traderMakerNotionalVolume.delete(trader);
        this.traderPositions.delete(trader);
        this.traderAcquisitionValue.delete(trader);
        removedCount++;
      }
    }

    console.log(`Pruned ${removedCount} inactive traders`);
  }

  /**
   * Prune old fill log results to prevent memory leaks.
   * Keeps only the most recent markets.
   */
  private pruneFillLogResults(): void {
    if (this.fillLogResults.size <= MAX_FILL_LOG_MARKETS) {
      return;
    }

    console.log(
      `Pruning fill log results: ${this.fillLogResults.size} markets exceeds limit of ${MAX_FILL_LOG_MARKETS}`,
    );

    // Get markets sorted by most recent fill (based on array length as proxy)
    const marketsByActivity = Array.from(this.fillLogResults.entries())
      .map(([market, fills]) => ({
        market,
        fillCount: fills.length,
      }))
      .sort((a, b) => b.fillCount - a.fillCount);

    // Keep top MAX_FILL_LOG_MARKETS markets
    const marketsToKeep = new Set(
      marketsByActivity.slice(0, MAX_FILL_LOG_MARKETS).map((m) => m.market),
    );

    // Remove markets not in the keep set
    let removedCount = 0;
    for (const market of this.fillLogResults.keys()) {
      if (!marketsToKeep.has(market)) {
        this.fillLogResults.delete(market);
        removedCount++;
      }
    }

    console.log(`Pruned ${removedCount} inactive fill log markets`);
  }

  /**
   * Log memory usage for debugging
   */
  private logMemoryUsage(): void {
    const used = process.memoryUsage();
    console.log('Memory usage:', {
      heapUsed: `${Math.round(used.heapUsed / 1024 / 1024)}MB`,
      heapTotal: `${Math.round(used.heapTotal / 1024 / 1024)}MB`,
      rss: `${Math.round(used.rss / 1024 / 1024)}MB`,
      external: `${Math.round(used.external / 1024 / 1024)}MB`,
      traders: this.traderNumTakerTrades.size,
      markets: this.markets.size,
      fillLogMarkets: this.fillLogResults.size,
    });
  }

  /**
   * Run all memory cleanup tasks
   */
  runMemoryCleanup(): void {
    this.logMemoryUsage();
    this.pruneInactiveTraders();
    this.pruneFillLogResults();
    this.logMemoryUsage();
  }

  /**
   * Clean shutdown of the server
   */
  async shutdown(): Promise<void> {
    console.log('Shutting down ManifestStatsServer...');

    // Close WebSocket manager
    if (this.wsManager) {
      this.wsManager.close();
      this.wsManager = null;
    }

    // Close database pool
    if (this.pool) {
      await this.pool.end();
    }

    console.log('ManifestStatsServer shutdown complete');
  }
}
