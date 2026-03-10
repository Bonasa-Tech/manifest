# Manifest SDK Quick Map

Canonical public SDK docs live in the Bonasa-Tech `manifest` repository TypeScript client README.

## Core Types

- `ManifestClient`: builds transaction instructions and trader actions.
- `Market`: deserializes market state and exposes orderbook/balance queries.
- `Wrapper`: tracks trader state/open orders across markets.
- `Global`: global account operations for cross-market liquidity.

## Common API Surface

### Discovery

- `ManifestClient.listMarketPublicKeys(connection)`
- `ManifestClient.listMarketsForMints(connection, baseMint, quoteMint)`

### Client Construction

- `ManifestClient.getClientForMarket(connection, marketPk, keypair)`
- `ManifestClient.getClientForMarketNoPrivateKey(connection, marketPk, traderPk)`
- `ManifestClient.getClientReadOnly(connection, marketPk, traderPk?)`
- `ManifestClient.getClientsReadOnlyForAllTraderSeats(connection, traderPk)`
- `ManifestClient.getSetupIxs(connection, marketPk, traderPk)`

### Trader Instructions

- `client.depositIx(traderPk, mintPk, amountTokens)`
- `client.placeOrderIx({...})`
- `client.cancelOrderIx(clientOrderId)`
- `client.cancelAllIx()`
- `client.withdrawIx(traderPk, mintPk, amountTokens)`
- `client.withdrawAllIx()`

### Batch and Advanced

- `client.batchUpdateIx({ cancels, orders })`
- `client.placeOrderWithRequiredDepositIxs({...})`
- `client.cancelAllOnCoreIx()`

### Global Account Management

- `ManifestClient.createGlobalAddTraderIx(traderPk, mintPk)`
- `ManifestClient.globalDepositIx(connection, traderPk, mintPk, amountTokens)`
- `ManifestClient.globalWithdrawIx(connection, traderPk, mintPk, amountTokens)`

### Market Reads

- `Market.loadFromAddress({ connection, address })`
- `Market.loadFromBuffer({ address, buffer })`
- `market.bids()`, `market.asks()`
- `market.bidsL2()`, `market.asksL2()`
- `market.bestBidPrice()`, `market.bestAskPrice()`
- `market.getWithdrawableBalanceTokens(traderPk, isBase)`
- `market.hasSeat(traderPk)`

### Global Reads

- `Global.findGlobalAddress(mintPk)`
- `Global.loadFromAddress({ connection, address })`
- `global.getGlobalBalanceTokens(connection, traderPk)`
- `global.hasSeat(traderPk)`

## Order Types

- `OrderType.Limit`: standard order. It may match immediately or rest on the book.
- `OrderType.PostOnly`: maker-only order. Rejects if it would cross and take liquidity.
- `OrderType.ImmediateOrCancel`: taker-style order. Executes what it can immediately and cancels any remainder.
- `OrderType.Global`: uses token-level global balances rather than only market-local deposited balances. This is the capital-efficient path for liquidity shared across multiple markets.
- `OrderType.Reverse`: order that flips direction after fills. In the external SDK params this uses `spreadBps` instead of `lastValidSlot`.
- `OrderType.ReverseTight`: same reversed-order lifecycle, but with tighter spread precision than `Reverse`.

Practical reverse-order notes:

- Reverse orders are stateful resting orders intended for recurring two-sided liquidity rather than one-shot execution.
- The SDK converts `spreadBps` internally when building the onchain params.
- `Reverse` uses coarser spread precision.
- `ReverseTight` uses finer spread precision and is the better fit when tight quoting matters.
- `cancelAllIx()` is wrapper-based and does not fully cover reversed orders after they flip. Use `cancelAllOnCoreIx()` when you need core-level cleanup, including reverse/global edge cases.

## Account Model

- `Market`: the core orderbook account. It holds market state, orderbook data, and seat data for the market.
- `Wrapper`: trader-associated account used by wrapper-driven flows. It tracks market info and powers most user-friendly client methods like deposits, standard order placement, cancels, and withdrawals.
- Market seat: trader presence on a specific market. Needed for market-local balances and local order management.
- Global account: token-level account, separate from wrapper state, used to support global balances and global orders across markets.

Account-management flow:

1. Market-local wallet flow:
- call `ManifestClient.getSetupIxs(...)`
- if setup is needed, create wrapper state and/or claim the seat
- then use `getClientForMarketNoPrivateKey(...)` or `getClientForMarket(...)`

2. Signer-controlled bot flow:
- use `getClientForMarket(...)`
- the SDK can auto-create wrapper state and claim the seat

3. Global liquidity flow:
- call `createGlobalAddTraderIx(...)` once per token
- deposit with `globalDepositIx(...)`
- place `OrderType.Global` orders on relevant markets
- read balances back through `Global`

## Validation

If working inside the Bonasa-Tech `manifest` repository:

- Local TS flow: `sh local-validator-test.sh`
- Program tests: `cargo test-sbf`
