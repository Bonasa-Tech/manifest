# Manifest TypeScript Client

TypeScript SDK for interacting with the Manifest decentralized exchange on Solana.

## Installation

```bash
yarn add @bonasa-tech/manifest-sdk
```

## Overview

The SDK provides these main classes:

- **ManifestClient** - Primary class for building transactions and reading market data
- **Market** - Deserializes and queries market state (orderbook, balances, seats)
- **Wrapper** - Caches a trader's open orders across markets
- **Global** - Manages global account state for cross-market liquidity

---

## Quick Start

### For UI Applications (Read-Only)

```typescript
import { Connection, PublicKey } from '@solana/web3.js';
import { ManifestClient, Market } from '@bonasa-tech/manifest-sdk';

const connection = new Connection('https://api.mainnet-beta.solana.com');
const marketAddress = new PublicKey('YOUR_MARKET_ADDRESS');

// Load market data (no wallet needed)
const market = await Market.loadFromAddress({
  connection,
  address: marketAddress,
});

// Read orderbook
const bids = market.bids();
const asks = market.asks();

console.log('Best bid:', market.bestBidPrice());
console.log('Best ask:', market.bestAskPrice());
```

### For Trading Bots (Full Access)

```typescript
import { Connection, Keypair, PublicKey } from '@solana/web3.js';
import { ManifestClient, OrderType } from '@bonasa-tech/manifest-sdk';

const connection = new Connection('https://api.mainnet-beta.solana.com');
const trader = Keypair.fromSecretKey(/* your keypair */);
const marketAddress = new PublicKey('YOUR_MARKET_ADDRESS');

// Creates wrapper + claims seat automatically if needed
const client = await ManifestClient.getClientForMarket(
  connection,
  marketAddress,
  trader,
);

// Now ready to trade
const placeOrderIx = client.placeOrderIx({
  numBaseTokens: 1.0,
  tokenPrice: 100.0,
  isBid: true,
  lastValidSlot: 0, // No expiration
  orderType: OrderType.Limit,
  clientOrderId: 1,
});
```

---

## UI Integration Examples

### Finding Markets

```typescript
import { ManifestClient } from '@bonasa-tech/manifest-sdk';

// List all market addresses
const marketPubkeys = await ManifestClient.listMarketPublicKeys(connection);

// Find markets for specific token pair
const baseMint = new PublicKey('So11111111111111111111111111111111111111112'); // SOL
const quoteMint = new PublicKey('EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v'); // USDC

const markets = await ManifestClient.listMarketsForMints(
  connection,
  baseMint,
  quoteMint,
);
```

### Loading Market Data

```typescript
import { Market } from '@bonasa-tech/manifest-sdk';

// Method 1: Load from address (fetches from chain)
const market = await Market.loadFromAddress({
  connection,
  address: marketAddress,
});

// Method 2: Load from existing buffer (for subscriptions)
const accountInfo = await connection.getAccountInfo(marketAddress);
const market = Market.loadFromBuffer({
  address: marketAddress,
  buffer: accountInfo.data,
});

// Access market info
console.log('Base mint:', market.baseMint().toBase58());
console.log('Quote mint:', market.quoteMint().toBase58());
console.log('Base decimals:', market.baseDecimals());
console.log('Quote decimals:', market.quoteDecimals());
```

### Reading the Orderbook

```typescript
// Get all orders
const bids = market.bids(); // Sorted by price descending
const asks = market.asks(); // Sorted by price ascending

// Get best prices
const bestBid = market.bestBidPrice();
const bestAsk = market.bestAskPrice();

// Each order contains:
// - price: number (in quote tokens per base token)
// - numBaseTokens: number
// - clientOrderId: number
// - trader: PublicKey
// - sequenceNumber: number
// - lastValidSlot: number
```

### Subscribing to Market Updates

```typescript
// Subscribe to real-time market changes
connection.onAccountChange(marketAddress, (accountInfo) => {
  const market = Market.loadFromBuffer({
    address: marketAddress,
    buffer: accountInfo.data,
  });

  // Update your UI
  updateOrderbook(market.bids(), market.asks());
  updateBestPrices(market.bestBidPrice(), market.bestAskPrice());
});
```

### Reading Trader Balances

```typescript
// Get a trader's balance on a specific market
const traderPubkey = new PublicKey('TRADER_ADDRESS');

// Withdrawable balance (deposited - locked in orders)
const baseBalance = market.getWithdrawableBalanceTokens(traderPubkey, true);
const quoteBalance = market.getWithdrawableBalanceTokens(traderPubkey, false);

// Check if trader has a seat
const hasSeat = market.hasSeat(traderPubkey);
```

### Wallet Integration (No Private Key)

For browser wallets like Phantom, use `getSetupIxs` and `getClientForMarketNoPrivateKey`:

```typescript
import { Transaction } from '@solana/web3.js';
import { ManifestClient } from '@bonasa-tech/manifest-sdk';

async function setupAndGetClient(
  connection: Connection,
  marketAddress: PublicKey,
  walletPubkey: PublicKey,
  sendTransaction: (tx: Transaction) => Promise<string>,
) {
  // Check if setup is needed (wrapper creation + seat claim)
  const { setupNeeded, instructions, wrapperKeypair } =
    await ManifestClient.getSetupIxs(connection, marketAddress, walletPubkey);

  if (setupNeeded) {
    const tx = new Transaction().add(...instructions);
    tx.recentBlockhash = (await connection.getLatestBlockhash()).blockhash;
    tx.feePayer = walletPubkey;

    // Sign with wrapper keypair if creating new wrapper
    if (wrapperKeypair) {
      tx.partialSign(wrapperKeypair);
    }

    // Send via wallet adapter
    await sendTransaction(tx);

    // Wait for confirmation
    await new Promise((resolve) => setTimeout(resolve, 5000));
  }

  // Create client (read-only, instructions must be signed by wallet)
  const client = await ManifestClient.getClientForMarketNoPrivateKey(
    connection,
    marketAddress,
    walletPubkey,
  );

  return client;
}
```

---

## Trading Bot Examples

### Complete Bot Setup Flow

```typescript
import {
  Connection,
  Keypair,
  PublicKey,
  Transaction,
  sendAndConfirmTransaction,
  ComputeBudgetProgram,
} from '@solana/web3.js';
import { ManifestClient, OrderType } from '@bonasa-tech/manifest-sdk';

const connection = new Connection('https://api.mainnet-beta.solana.com');
const trader = Keypair.fromSecretKey(Uint8Array.from(/* your key */));
const marketAddress = new PublicKey('YOUR_MARKET_ADDRESS');

// Step 1: Initialize client (auto-creates wrapper + claims seat)
const client = await ManifestClient.getClientForMarket(
  connection,
  marketAddress,
  trader,
);

console.log('Market loaded:', client.market.address.toBase58());
console.log('Base mint:', client.market.baseMint().toBase58());
console.log('Quote mint:', client.market.quoteMint().toBase58());
```

### Depositing Funds

```typescript
// Deposit base tokens (e.g., SOL)
const depositBaseIx = client.depositIx(
  trader.publicKey,
  client.market.baseMint(),
  10.0, // Amount in tokens (not atoms)
);

// Deposit quote tokens (e.g., USDC)
const depositQuoteIx = client.depositIx(
  trader.publicKey,
  client.market.quoteMint(),
  1000.0, // Amount in tokens
);

// Send deposit transaction
const depositTx = new Transaction().add(depositBaseIx, depositQuoteIx);
const depositSig = await sendAndConfirmTransaction(connection, depositTx, [
  trader,
]);
console.log('Deposited:', depositSig);
```

### Placing Orders

```typescript
// Place a limit bid
const bidIx = client.placeOrderIx({
  numBaseTokens: 1.0,
  tokenPrice: 95.0, // Price in quote per base
  isBid: true,
  lastValidSlot: 0, // 0 = no expiration
  orderType: OrderType.Limit,
  clientOrderId: 1, // Your reference ID
});

// Place a limit ask
const askIx = client.placeOrderIx({
  numBaseTokens: 1.0,
  tokenPrice: 105.0,
  isBid: false,
  lastValidSlot: 0,
  orderType: OrderType.Limit,
  clientOrderId: 2,
});

// Send orders
const orderTx = new Transaction().add(bidIx, askIx);
const orderSig = await sendAndConfirmTransaction(connection, orderTx, [trader]);
```

### Order Types

```typescript
// Limit Order - standard order that can take or provide liquidity
const limitOrder = client.placeOrderIx({
  numBaseTokens: 1.0,
  tokenPrice: 100.0,
  isBid: true,
  lastValidSlot: 0,
  orderType: OrderType.Limit,
  clientOrderId: 1,
});

// Post-Only Order - rejected if it would immediately match
const postOnlyOrder = client.placeOrderIx({
  numBaseTokens: 1.0,
  tokenPrice: 100.0,
  isBid: true,
  lastValidSlot: 0,
  orderType: OrderType.PostOnly,
  clientOrderId: 2,
});

// Immediate-or-Cancel - fills what it can, cancels the rest
const iocOrder = client.placeOrderIx({
  numBaseTokens: 1.0,
  tokenPrice: 100.0,
  isBid: true,
  lastValidSlot: 0,
  orderType: OrderType.ImmediateOrCancel,
  clientOrderId: 3,
});

// Global Order - uses global account for cross-market liquidity
const globalOrder = client.placeOrderIx({
  numBaseTokens: 1.0,
  tokenPrice: 100.0,
  isBid: true,
  lastValidSlot: 0,
  orderType: OrderType.Global,
  clientOrderId: 4,
});
```

### Cancelling Orders

```typescript
// Cancel by client order ID
const cancelIx = client.cancelOrderIx(1); // clientOrderId

// Cancel all orders on this market
const cancelAllIx = client.cancelAllIx();

// Cancel all bids only
const cancelBidsIx = client.cancelBidsOnCoreIx();

// Cancel all asks only
const cancelAsksIx = client.cancelAsksOnCoreIx();
```

### Withdrawing Funds

```typescript
// Withdraw specific amount
const withdrawBaseIx = client.withdrawIx(
  trader.publicKey,
  client.market.baseMint(),
  5.0, // Amount in tokens
);

// Withdraw all funds from market
const withdrawAllIx = client.withdrawAllIx();

const withdrawTx = new Transaction().add(...withdrawAllIx);
await sendAndConfirmTransaction(connection, withdrawTx, [trader]);
```

### Place Order with Auto-Deposit

Automatically deposits required funds if balance is insufficient:

```typescript
const instructions = await client.placeOrderWithRequiredDepositIxs({
  numBaseTokens: 1.0,
  tokenPrice: 100.0,
  isBid: true,
  lastValidSlot: 0,
  orderType: OrderType.Limit,
  clientOrderId: 1,
});

// instructions array may include deposit ix before place order ix
const tx = new Transaction().add(...instructions);
await sendAndConfirmTransaction(connection, tx, [trader]);
```

### Complete Trading Loop Example

```typescript
async function tradingBot() {
  const client = await ManifestClient.getClientForMarket(
    connection,
    marketAddress,
    trader,
  );

  while (true) {
    // Reload market data
    await client.market.reload(connection);

    const bestBid = client.market.bestBidPrice();
    const bestAsk = client.market.bestAskPrice();
    const spread = bestAsk && bestBid ? (bestAsk - bestBid) / bestBid : null;

    console.log(
      `Best Bid: ${bestBid}, Best Ask: ${bestAsk}, Spread: ${spread}`,
    );

    // Check our balances
    const baseBalance = client.market.getWithdrawableBalanceTokens(
      trader.publicKey,
      true,
    );
    const quoteBalance = client.market.getWithdrawableBalanceTokens(
      trader.publicKey,
      false,
    );

    console.log(`Balances - Base: ${baseBalance}, Quote: ${quoteBalance}`);

    // Your trading logic here...

    await new Promise((resolve) => setTimeout(resolve, 1000));
  }
}
```

---

## Global Account Operations

Global accounts allow traders to share liquidity across multiple markets.

### Setup Global Account

```typescript
import { Global, ManifestClient } from '@bonasa-tech/manifest-sdk';

const mint = new PublicKey('TOKEN_MINT_ADDRESS');

// Add trader to global account (one-time setup)
const addTraderIx = await ManifestClient.createGlobalAddTraderIx(
  connection,
  trader.publicKey,
  mint,
);

const tx = new Transaction().add(addTraderIx);
await sendAndConfirmTransaction(connection, tx, [trader]);
```

### Deposit to Global Account

```typescript
// Deposit to global account (available across all markets for this token)
const globalDepositIx = await ManifestClient.globalDepositIx(
  connection,
  trader.publicKey,
  mint,
  100.0, // Amount in tokens
);

const tx = new Transaction().add(globalDepositIx);
await sendAndConfirmTransaction(connection, tx, [trader]);
```

### Withdraw from Global Account

```typescript
const globalWithdrawIx = await ManifestClient.globalWithdrawIx(
  connection,
  trader.publicKey,
  mint,
  50.0, // Amount in tokens
);

const tx = new Transaction().add(globalWithdrawIx);
await sendAndConfirmTransaction(connection, tx, [trader]);
```

### Reading Global Account State

```typescript
import { Global } from '@bonasa-tech/manifest-sdk';

// Load global account
const globalAddress = Global.findGlobalAddress(mint);
const global = await Global.loadFromAddress({
  connection,
  address: globalAddress,
});

// Check balances
const balance = await global.getGlobalBalanceTokens(
  connection,
  trader.publicKey,
);
console.log('Global balance:', balance);

// Check if trader has global seat
const hasSeat = global.hasSeat(trader.publicKey);
```

---

## Advanced Patterns

### Batch Order Updates

```typescript
// Cancel and replace multiple orders in one transaction
const batchIx = client.batchUpdateIx({
  cancels: [{ clientOrderId: 1 }, { clientOrderId: 2 }],
  orders: [
    {
      numBaseTokens: 1.0,
      tokenPrice: 96.0,
      isBid: true,
      lastValidSlot: 0,
      orderType: OrderType.Limit,
      clientOrderId: 3,
    },
    {
      numBaseTokens: 1.0,
      tokenPrice: 104.0,
      isBid: false,
      lastValidSlot: 0,
      orderType: OrderType.Limit,
      clientOrderId: 4,
    },
  ],
});
```

### Loading All Markets for a Trader

```typescript
// Get clients for all markets where trader has a seat
const clients = await ManifestClient.getClientsReadOnlyForAllTraderSeats(
  connection,
  trader.publicKey,
);

for (const client of clients) {
  console.log('Market:', client.market.address.toBase58());
  const baseBalance = client.market.getWithdrawableBalanceTokens(
    trader.publicKey,
    true,
  );
  console.log('Base balance:', baseBalance);
}
```

### Fill Feed (Monitoring Trades)

```typescript
import { FillFeed } from '@bonasa-tech/manifest-sdk';

const fillFeed = new FillFeed(connection);

// Subscribe to fills (runs indefinitely)
fillFeed.on('fill', (fill) => {
  console.log('Fill:', {
    market: fill.market.toBase58(),
    maker: fill.maker.toBase58(),
    taker: fill.taker.toBase58(),
    baseTokens: fill.baseTokens,
    quoteTokens: fill.quoteTokens,
    price: fill.price,
    takerIsBuy: fill.takerIsBuy,
  });
});

await fillFeed.parseLogs(); // Starts monitoring
```

---

## Error Handling

```typescript
try {
  const tx = new Transaction().add(client.placeOrderIx(/* ... */));
  await sendAndConfirmTransaction(connection, tx, [trader]);
} catch (error) {
  if (error.message.includes('InsufficientFunds')) {
    console.log('Need to deposit more funds');
  } else if (error.message.includes('PostOnlyWouldTake')) {
    console.log('Post-only order would have crossed the spread');
  } else if (error.message.includes('InvalidOrderType')) {
    console.log('Order type not allowed');
  } else {
    throw error;
  }
}
```

---

## Token2022 Support

The SDK automatically handles Token2022 tokens. No special configuration needed:

```typescript
// Works the same for both SPL Token and Token2022
const client = await ManifestClient.getClientForMarket(
  connection,
  marketAddress, // Market with Token2022 tokens
  trader,
);

// Token program is detected automatically
console.log('Is base Token2022:', client.isBase22);
console.log('Is quote Token2022:', client.isQuote22);

// All operations work identically
const depositIx = client.depositIx(
  trader.publicKey,
  client.market.baseMint(),
  10.0,
);
```

---

## API Reference

### ManifestClient

| Method                                                         | Description                                   |
| -------------------------------------------------------------- | --------------------------------------------- |
| `getClientForMarket(connection, marketPk, keypair)`            | Create client with auto-setup                 |
| `getClientForMarketNoPrivateKey(connection, marketPk, trader)` | Create read-only client                       |
| `getSetupIxs(connection, marketPk, trader)`                    | Get setup instructions for wallet integration |
| `listMarketPublicKeys(connection)`                             | List all market addresses                     |
| `listMarketsForMints(connection, base, quote)`                 | Find markets for token pair                   |
| `depositIx(payer, mint, amount)`                               | Deposit tokens to market                      |
| `withdrawIx(payer, mint, amount)`                              | Withdraw tokens from market                   |
| `withdrawAllIx()`                                              | Withdraw all funds                            |
| `placeOrderIx(params)`                                         | Place an order                                |
| `cancelOrderIx(clientOrderId)`                                 | Cancel specific order                         |
| `cancelAllIx()`                                                | Cancel all orders                             |
| `batchUpdateIx(params)`                                        | Batch cancel and place orders                 |

### Market

| Method                                         | Description                    |
| ---------------------------------------------- | ------------------------------ |
| `loadFromAddress({connection, address})`       | Load market from chain         |
| `loadFromBuffer({address, buffer})`            | Load from account data         |
| `reload(connection)`                           | Refresh market data            |
| `bids()`                                       | Get bid orders                 |
| `asks()`                                       | Get ask orders                 |
| `bestBidPrice()`                               | Get best bid price             |
| `bestAskPrice()`                               | Get best ask price             |
| `getWithdrawableBalanceTokens(trader, isBase)` | Get trader's available balance |
| `hasSeat(trader)`                              | Check if trader has seat       |
| `baseMint()` / `quoteMint()`                   | Get token mints                |
| `baseDecimals()` / `quoteDecimals()`           | Get token decimals             |

### Global

| Method                                       | Description                     |
| -------------------------------------------- | ------------------------------- |
| `loadFromAddress({connection, address})`     | Load global account             |
| `findGlobalAddress(mint)`                    | Derive global account PDA       |
| `getGlobalBalanceTokens(connection, trader)` | Get trader's global balance     |
| `hasSeat(trader)`                            | Check if trader has global seat |
| `tokenMint()`                                | Get token mint                  |
