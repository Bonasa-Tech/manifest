import { assert } from 'chai';
import {
  aggregateTickers,
  MarketTicker,
} from '../../../scripts/stats_utils/tickers';

function ticker(id: string, volume: number): MarketTicker {
  return {
    ticker_id: id,
    pool_id: id,
    trade_url: `https://manifest.trade/market/${id}`,
    base_currency: 'base-mint',
    target_currency: 'quote-mint',
    base_volume: volume / 2,
    target_volume: volume,
    last_price: 2,
    liquidity_in_usd: 0,
  };
}

describe('CoinGecko ticker aggregation', () => {
  it('sums volumes while retaining the highest individual volume market', () => {
    const inputs = [
      ticker('small-a', 60),
      ticker('small-b', 60),
      { ...ticker('largest', 100), last_price: 3, bid: 2.9, ask: 3.1 },
    ];
    const original = JSON.parse(JSON.stringify(inputs));
    assert.deepEqual(aggregateTickers(inputs), [
      { ...inputs[2], base_volume: 110, target_volume: 220 },
    ]);
    assert.deepEqual(inputs, original, 'input tickers must remain unchanged');
  });

  it('ranks by quote volume and keeps optional fields from only the winner', () => {
    const inputs = [
      { ...ticker('lower-quote', 10), base_volume: 100, bid: 1, ask: 2 },
      ticker('higher-quote', 20),
    ];
    const [result] = aggregateTickers(inputs);
    assert.equal(result.ticker_id, 'higher-quote');
    assert.equal(result.base_volume, 110);
    assert.notProperty(result, 'bid');
    assert.notProperty(result, 'ask');
  });

  it('groups by mint addresses and preserves base/quote orientation', () => {
    const inputs = [
      ticker('first', 10),
      { ...ticker('other-base', 20), base_currency: 'other-base-mint' },
      { ...ticker('other-quote', 30), target_currency: 'other-quote-mint' },
      {
        ...ticker('reversed', 40),
        base_currency: 'quote-mint',
        target_currency: 'base-mint',
      },
    ];
    assert.deepEqual(aggregateTickers(inputs), inputs);
    assert.deepEqual(aggregateTickers([]), []);
  });

  it('breaks equal-volume and zero-volume ties consistently by market ID', () => {
    for (const volume of [0, 100]) {
      const inputs = [ticker('b', volume), ticker('a', volume)];
      assert.deepEqual(
        aggregateTickers(inputs),
        aggregateTickers([...inputs].reverse()),
      );
      assert.equal(aggregateTickers(inputs)[0].ticker_id, 'a');
    }
  });
});
