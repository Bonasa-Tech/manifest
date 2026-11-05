# CoinGecko tickers

`GET /tickers` preserves the existing response: one ticker per individual market,
with the same fields and per-market volumes. Existing clients, SDKs, and monitoring
scripts do not need to change. `aggregate=false` also returns this view.

`GET /tickers?aggregate=true` opts into one ticker per ordered base/quote token mint pair.
`base_volume` and `target_volume` sum the last 24 hours of checkpointed volume
across all markets for that pair, in token units.

The market with the highest individual 24-hour `target_volume` supplies
`ticker_id`, `pool_id`, `trade_url`, `last_price`, and optional `bid`/`ask`.
Equal volumes use the lexicographically smallest market address as a stable
tie-breaker. The market IDs remain valid for `GET /orderbook?ticker_id=...`.
Different quote tokens and reversed base/quote orientations stay separate.

Both views share the same cached checkpoint snapshot, refreshed about every five
minutes.

After deploying the server, ask CoinGecko to change its Manifest ticker feed URL
to `https://mfx-stats-mainnet.fly.dev/tickers?aggregate=true`. This returns summed
24-hour base and quote volumes, with the highest-volume individual market's ID
and trading link. The orderbook endpoint and its market-ID parameter stay the same.

Run the aggregation and server snapshot tests with:

```sh
node_modules/.bin/mocha -r ts-node/register client/ts/tests/tickers.ts client/ts/tests/tickerSnapshots.ts
```

These tests use in-memory fixtures without starting the server or connecting to
external services.
