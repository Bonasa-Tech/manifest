import { assert } from 'chai';
import { ManifestStatsServer } from '../../../scripts/stats_utils/manifestStatsServer';

describe('CoinGecko ticker snapshots', () => {
  it('uses 24h token volumes and caches both aggregate and per-market views', () => {
    // Bypass the constructor so this test needs no RPC, database, or credentials.
    const now = Math.floor(Date.now() / 1000);
    const market = {
      baseMint: () => ({ toBase58: () => 'base-mint' }),
      quoteMint: () => ({ toBase58: () => 'quote-mint' }),
      baseDecimals: () => 6,
      quoteDecimals: () => 9,
      bestBidPrice: () => 1.9,
      bestAskPrice: () => 2.1,
    };
    const state = {
      markets: new Map([
        ['smaller', market],
        ['largest', market],
      ]),
      checkpointTimestamps: new Map([
        ['smaller', [now - 90_000, now - 60]],
        ['largest', [now - 60]],
      ]),
      baseVolumeAtomsCheckpoints: new Map([
        ['smaller', [999_000_000, 1_000_000]],
        ['largest', [3_000_000]],
      ]),
      quoteVolumeAtomsCheckpoints: new Map([
        ['smaller', [999_000_000_000, 2_000_000_000]],
        ['largest', [6_000_000_000]],
      ]),
      lastPriceByMarket: new Map([
        ['smaller', 2000],
        ['largest', 2000],
      ]),
      cachedTickersJson: null,
      cachedAggregatedTickersJson: null,
    };
    const server: ManifestStatsServer = Object.assign(
      Object.create(ManifestStatsServer.prototype),
      state,
    );

    const raw = JSON.parse(server.getTickersJson());
    assert.deepEqual(
      raw,
      ['smaller', 'largest'].map((id, index) => ({
        ticker_id: id,
        base_currency: 'base-mint',
        target_currency: 'quote-mint',
        last_price: 2,
        base_volume: index === 0 ? 1 : 3,
        target_volume: index === 0 ? 2 : 6,
        pool_id: id,
        liquidity_in_usd: 0,
        bid: 1.9,
        ask: 2.1,
      })),
    );
    assert.deepEqual(JSON.parse(server.getTickersJson(false)), raw);
    assert.deepEqual(server.getTickers(), raw);
    assert.deepEqual(server.getTickers(false), raw);

    const [pair] = JSON.parse(server.getTickersJson(true));
    assert.equal(pair.base_volume, 4);
    assert.equal(pair.target_volume, 8);
    assert.equal(pair.last_price, 2);
    assert.equal(pair.ticker_id, 'largest');
    assert.equal(pair.pool_id, 'largest');
    assert.equal(pair.trade_url, 'https://manifest.trade/market/largest');
    assert.equal(pair.bid, 1.9);
    assert.equal(pair.ask, 2.1);
    assert.deepEqual(server.getTickers(true), [pair]);

    state.quoteVolumeAtomsCheckpoints.set('smaller', [0, 10_000_000_000]);
    assert.deepEqual(JSON.parse(server.getTickersJson(true)), [pair]);
    // Checkpoint advancement calls this same refresh method.
    (server as any).refreshTickersCache();
    const [refreshed] = JSON.parse(server.getTickersJson(true));
    assert.equal(refreshed.ticker_id, 'smaller');
    assert.equal(refreshed.trade_url, 'https://manifest.trade/market/smaller');
    assert.equal(refreshed.target_volume, 16);
    assert.equal(JSON.parse(server.getTickersJson())[0].target_volume, 10);
  });
});
