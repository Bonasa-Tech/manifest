export interface MarketTicker {
  ticker_id: string;
  base_currency: string;
  target_currency: string;
  last_price: number;
  base_volume: number;
  target_volume: number;
  pool_id: string;
  liquidity_in_usd: number;
  trade_url?: string;
  bid?: number;
  ask?: number;
}

/**
 * One CoinGecko ticker per ordered base/quote mint pair. Volumes are summed
 * across markets; all other fields belong to the market with the greatest
 * individual 24h quote volume. Mint addresses avoid symbol collisions, and
 * keeping base/quote order avoids mixing volumes in different token units.
 */
export function aggregateTickers(tickers: MarketTicker[]): MarketTicker[] {
  const pairs = new Map<
    string,
    { representative: MarketTicker; baseVolume: number; quoteVolume: number }
  >();

  for (const ticker of tickers) {
    const key = `${ticker.base_currency}_${ticker.target_currency}`;
    const pair = pairs.get(key);
    if (!pair) {
      pairs.set(key, {
        representative: ticker,
        baseVolume: ticker.base_volume,
        quoteVolume: ticker.target_volume,
      });
      continue;
    }

    pair.baseVolume += ticker.base_volume;
    pair.quoteVolume += ticker.target_volume;
    const best = pair.representative;
    if (
      ticker.target_volume > best.target_volume ||
      (ticker.target_volume === best.target_volume &&
        ticker.ticker_id < best.ticker_id)
    ) {
      pair.representative = ticker;
    }
  }

  return Array.from(pairs.values(), (pair) => ({
    ...pair.representative,
    trade_url: `https://manifest.trade/market/${pair.representative.ticker_id}`,
    base_volume: pair.baseVolume,
    target_volume: pair.quoteVolume,
  }));
}
