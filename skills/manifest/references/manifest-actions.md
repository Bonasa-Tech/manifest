# Manifest Actions

## Read-Only Market Data (No Wallet)

Use when you need orderbook state, best bid/ask, balances, or seat checks.

- Read-only client: `ManifestClient.getClientReadOnly(connection, marketPk, traderPk?)`
- Load market from chain: `Market.loadFromAddress(...)`
- Load market from account data/subscriptions: `Market.loadFromBuffer(...)`
- Read orderbook: `market.bids()`, `market.asks()`
- Read UI-friendly orderbook levels: `market.bidsL2()`, `market.asksL2()`
- Best prices: `market.bestBidPrice()`, `market.bestAskPrice()`
- Trader view: `market.hasSeat(trader)`, `market.getWithdrawableBalanceTokens(trader, isBase)`

## Trading Flow (Wallet / Signer)

Use `ManifestClient` when building instructions for deposits, orders, cancels, and withdrawals.

- Client setup with signer: `ManifestClient.getClientForMarket(...)`
- Client setup for wallet adapters: `ManifestClient.getSetupIxs(...)` + `getClientForMarketNoPrivateKey(...)`
- Deposit: `client.depositIx(...)`
- Place order: `client.placeOrderIx(...)`
- Cancel single/all: `client.cancelOrderIx(...)`, `client.cancelAllIx()`
- Withdraw: `client.withdrawIx(...)`, `client.withdrawAllIx()`

## Order Strategy Helpers

- Batch operations: `client.batchUpdateIx(...)`
- Fill stream: `FillFeed` for processing fills/events

## Order Types

- `Limit`: standard resting order. Can take immediately or rest on the book.
- `PostOnly`: must rest. Reject if it would immediately match.
- `ImmediateOrCancel`: fills what it can immediately and cancels any remainder.
- `Global`: rests against a trader's global balance for the supporting token instead of market-local deposited balance. Useful when the same capital should support liquidity across multiple markets.
- `Reverse`: a resting order that flips sides after being filled. It uses `spreadBps` instead of `lastValidSlot`, with lower spread precision than `ReverseTight`.
- `ReverseTight`: same reverse-order behavior, but with tighter spread precision. The SDK encodes reverse spread more precisely for this type than for normal `Reverse`.

## Account Management

- Market-local trading requires wrapper state plus a seat on that market.
- `ManifestClient.getSetupIxs(...)` is the safe preflight for wallet adapters. It tells you whether wrapper creation and/or seat claim instructions are still required.
- `ManifestClient.getClientForMarket(...)` is the signer path that auto-creates wrapper state and claims a seat if needed.
- `ManifestClient.getClientForMarketNoPrivateKey(...)` assumes setup is already complete and is the wallet-adapter path after preflight/setup.
- `ManifestClient.getClientReadOnly(...)` is the anonymous or pre-setup inspection path. It can still load wrapper-linked state when available.
- A market seat is what makes a trader present on a given market for local balances and orders.
- Global accounts are separate from wrapper state. They are token-level accounts used for cross-market liquidity and global order support.
- Global account setup is one-time per token per trader: `ManifestClient.createGlobalAddTraderIx(...)`
- Fund and defund global balances with `ManifestClient.globalDepositIx(...)` and `ManifestClient.globalWithdrawIx(...)`
- Portfolio-style reads across all occupied seats: `ManifestClient.getClientsReadOnlyForAllTraderSeats(...)`

## Selection Guide

- UI orderbook display only: `Market` APIs only.
- Read-only market page or pre-setup wallet flow: `getClientReadOnly(...)`
- Bot or transaction executor: `ManifestClient` path.
- Wallet adapter flow: `getSetupIxs(...)` first, then `getClientForMarketNoPrivateKey(...)`
- Multi-market/global operations: load `Global`, use global deposit/withdraw methods, and prefer `OrderType.Global` when shared collateral across markets is intended.

## Source Repo Pointers

Use these only when working inside the Bonasa-Tech `manifest` repository:

- TypeScript SDK source: `client/ts/src`
- TypeScript examples/tests: `client/ts/tests`
- TS client docs: `client/ts/README.md`
- Rust AMM client: `client/rust/src`
