import { assert } from 'chai';
import { PublicKey } from '@solana/web3.js';
import * as promClient from 'prom-client';
import { ManifestStatsServer } from '../../../scripts/stats_utils/manifestStatsServer';
import {
  MARKET_EVICTION_HIGH_WATER,
  MARKET_EVICTION_LOW_WATER,
} from '../../../scripts/stats_utils/marketFetcher';

const USDC_MINT: string = 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v';
const ONE_DAY_SEC: number = 24 * 60 * 60;

// Deterministic, distinct market pubkeys.
function marketPk(index: number): string {
  const bytes: Uint8Array = new Uint8Array(32);
  bytes[0] = index & 0xff;
  bytes[1] = (index >> 8) & 0xff;
  bytes[2] = (index >> 16) & 0xff;
  bytes[3] = 1;
  return new PublicKey(bytes).toBase58();
}

function makeServer(): ManifestStatsServer {
  // Separate registry per server so repeated construction in one mocha
  // process never collides on metric names.
  const registry: promClient.Registry = new promClient.Registry();
  return new ManifestStatsServer(
    'http://localhost:8899',
    true, // read-only: skips database schema initialization
    undefined,
    {
      fills: new promClient.Counter({
        name: 'fills',
        help: 'fills',
        labelNames: ['market'] as const,
        registers: [registry],
      }),
      reconnects: new promClient.Counter({
        name: 'reconnects',
        help: 'reconnects',
        registers: [registry],
      }),
      volume: new promClient.Gauge({
        name: 'volume',
        help: 'volume',
        labelNames: ['market', 'mint', 'side'] as const,
        registers: [registry],
      }),
      lastPrice: new promClient.Gauge({
        name: 'last_price',
        help: 'last price',
        labelNames: ['market'] as const,
        registers: [registry],
      }),
      depth: new promClient.Gauge({
        name: 'depth',
        help: 'depth',
        labelNames: ['depth_bps', 'market', 'trader'] as const,
        registers: [registry],
      }),
      aggregatorQuoteVolume: new promClient.Counter({
        name: 'aggregator_quote_volume',
        help: 'aggregator quote volume',
        labelNames: ['aggregator', 'quote'] as const,
        registers: [registry],
      }),
      originatingProtocolQuoteVolume: new promClient.Counter({
        name: 'originating_protocol_quote_volume',
        help: 'originating protocol quote volume',
        labelNames: ['protocol', 'quote'] as const,
        registers: [registry],
      }),
      dbQueryCount: new promClient.Counter({
        name: 'db_query_count',
        help: 'db query count',
        labelNames: ['query_type', 'status'] as const,
        registers: [registry],
      }),
      dbQueryDuration: new promClient.Histogram({
        name: 'db_query_duration',
        help: 'db query duration',
        labelNames: ['query_type'] as const,
        registers: [registry],
      }),
    },
  );
}

// Minimal stand-in for a Market: eviction only touches mints and address.
function fakeMarket(pk: string, baseMint: string) {
  return {
    address: new PublicKey(pk),
    baseMint: (): PublicKey => new PublicKey(baseMint),
    quoteMint: (): PublicKey => new PublicKey(USDC_MINT),
  };
}

// Install a market into every per-market map the way loadNewMarket does.
// checkpoint arrays are abbreviated: isMarketEvictable never assumes a fixed
// length. tracked=false leaves it out of `markets` (an orphan restored from
// the database whose on-chain account no longer exists).
function installMarket(
  server: ManifestStatsServer,
  pk: string,
  options: {
    tracked: boolean;
    timestamps: number[];
    baseCheckpoints?: number[];
    pendingVolume?: number;
    baseMint?: string;
  },
): void {
  const s = server as any;
  if (options.tracked) {
    const baseMint: string = options.baseMint ?? marketPk(0xf00d);
    const market = fakeMarket(pk, baseMint);
    s.markets.set(pk, market);
    s.indexMarketForPnL(pk, market);
    s.tickers.set(pk, ['BASE', 'USDC']);
    s.lastPriceByMarket.set(pk, 1);
  }
  s.baseVolumeAtomsSinceLastCheckpoint.set(pk, options.pendingVolume ?? 0);
  s.quoteVolumeAtomsSinceLastCheckpoint.set(pk, 0);
  s.baseVolumeAtomsCheckpoints.set(
    pk,
    options.baseCheckpoints ?? options.timestamps.map(() => 0),
  );
  s.quoteVolumeAtomsCheckpoints.set(
    pk,
    options.timestamps.map(() => 0),
  );
  s.checkpointTimestamps.set(pk, [...options.timestamps]);
}

describe('stats market eviction', () => {
  const nowSec: number = Math.floor(Date.now() / 1000);
  const staleTs: number = nowSec - 2 * ONE_DAY_SEC;
  const freshTs: number = nowSec - 60;

  it('judges evictability from the 24h window', () => {
    const server = makeServer();
    const s = server as any;

    // Tracked, full window, no volume anywhere: dead.
    installMarket(server, marketPk(1), {
      tracked: true,
      timestamps: [staleTs, freshTs],
    });
    assert.isTrue(s.isMarketEvictable(marketPk(1), nowSec));

    // Tracked with volume inside the window: alive.
    installMarket(server, marketPk(2), {
      tracked: true,
      timestamps: [staleTs, freshTs],
      baseCheckpoints: [0, 5],
    });
    assert.isFalse(s.isMarketEvictable(marketPk(2), nowSec));

    // Volume only in a bucket that aged out of the window: dead.
    installMarket(server, marketPk(3), {
      tracked: true,
      timestamps: [staleTs, freshTs],
      baseCheckpoints: [5, 0],
    });
    assert.isTrue(s.isMarketEvictable(marketPk(3), nowSec));

    // Un-checkpointed volume pending since the last advance: alive.
    installMarket(server, marketPk(4), {
      tracked: true,
      timestamps: [staleTs, freshTs],
      pendingVolume: 7,
    });
    assert.isFalse(s.isMarketEvictable(marketPk(4), nowSec));

    // Tracked but observed for less than the full window (leading zero
    // timestamp): too new to judge.
    installMarket(server, marketPk(5), {
      tracked: true,
      timestamps: [0, freshTs],
    });
    assert.isFalse(s.isMarketEvictable(marketPk(5), nowSec));

    // Orphan (not in markets) with the same partial window: evictable, it
    // can never advance to a full window.
    installMarket(server, marketPk(6), {
      tracked: false,
      timestamps: [0, staleTs],
    });
    assert.isTrue(s.isMarketEvictable(marketPk(6), nowSec));

    // Orphan whose buckets still fall inside the window: not yet.
    installMarket(server, marketPk(7), {
      tracked: false,
      timestamps: [staleTs, freshTs],
      baseCheckpoints: [0, 3],
    });
    assert.isFalse(s.isMarketEvictable(marketPk(7), nowSec));
  });

  it('evictMarket clears every per-market map and the PnL index', () => {
    const server = makeServer();
    const s = server as any;
    const pk: string = marketPk(10);
    const baseMint: string = marketPk(0xbeef);
    installMarket(server, pk, {
      tracked: true,
      timestamps: [staleTs, freshTs],
      baseMint,
    });
    assert.deepEqual(s.baseMintToStablecoinMarkets.get(baseMint), [pk]);

    s.evictMarket(pk);

    for (const map of [
      s.markets,
      s.baseVolumeAtomsSinceLastCheckpoint,
      s.quoteVolumeAtomsSinceLastCheckpoint,
      s.baseVolumeAtomsCheckpoints,
      s.quoteVolumeAtomsCheckpoints,
      s.checkpointTimestamps,
      s.lastPriceByMarket,
      s.tickers,
    ]) {
      assert.isFalse(map.has(pk));
    }
    assert.isFalse(s.baseMintToStablecoinMarkets.has(baseMint));
  });

  it('evicts down to the low water mark, orphans first, live markets last', () => {
    const server = makeServer();
    const s = server as any;

    // Two live tracked markets that must survive any eviction.
    const livePks: string[] = [marketPk(100_001), marketPk(100_002)];
    for (const pk of livePks) {
      installMarket(server, pk, {
        tracked: true,
        timestamps: [staleTs, freshTs],
        baseCheckpoints: [0, 9],
      });
    }
    // Enough dead orphans to push past the high water mark.
    const orphanCount: number = MARKET_EVICTION_HIGH_WATER + 25;
    for (let i = 0; i < orphanCount; i++) {
      installMarket(server, marketPk(i + 1), {
        tracked: false,
        timestamps: [staleTs],
      });
    }

    s.evictDeadMarkets();

    assert.equal(s.baseVolumeAtomsCheckpoints.size, MARKET_EVICTION_LOW_WATER);
    for (const pk of livePks) {
      assert.isTrue(s.markets.has(pk));
      assert.isTrue(s.baseVolumeAtomsCheckpoints.has(pk));
    }

    // Below the high water mark nothing further is evicted.
    s.evictDeadMarkets();
    assert.equal(s.baseVolumeAtomsCheckpoints.size, MARKET_EVICTION_LOW_WATER);
  });
});
