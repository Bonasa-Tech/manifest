# Manifest Wallet Integration Guide

## For DeFi Wallets

This guide helps wallet teams integrate Manifest's limit order functionality to provide users with true on-chain limit orders—not triggered taker orders.

---

## Table of Contents

1. [Why Limit Orders on Manifest?](#why-limit-orders-on-manifest)
2. [Quick Start](#quick-start)
3. [Core Concepts](#core-concepts)
4. [Integration Architecture](#integration-architecture)
5. [Implementation Guide](#implementation-guide)
6. [Fee Structure & Monetization](#fee-structure--monetization)
7. [Advanced Features](#advanced-features)
8. [Code Examples](#code-examples)
9. [Testing & Support](#testing--support)

---

## Why Limit Orders on Manifest?

### The Problem with Traditional Swap-Based Limit Orders

Most wallet "limit orders" are actually **triggered market orders** that execute swaps when a price is reached. These:
- Consume precious compute units at execution time
- Are vulnerable to slippage and MEV
- Don't provide liquidity to the ecosystem
- Often have hidden execution costs

### Why Manifest?

Real on-chain limit orders that actually add liquidity to the ecosystem:

- **Free to use** - Zero protocol fees, forever
- **Only formally verified DEX on Solana** - Mathematically proven security
- **Monetization ready** - Add your own fees via wrapper programs
- **Trade anything** - 0.007 SOL market creation (350x cheaper than competitors)
- **Better performance** - Lower CU consumption
- **Capital efficient** - Global orders support multiple markets

### Why This Matters for Your Wallet

Integrating Manifest gives you a real competitive edge. Most wallets still use triggered swap orders disguised as "limit orders" - they're just market orders that execute when price is hit. That means slippage, MEV, and poor fills.

With Manifest, your users get:
- Orders that rest on the orderbook and fill at limit price
- Open market competition to fill orders
- No slippage, no MEV extraction
- Ability to trade exotic pairs that don't exist elsewhere

For your business:
- New revenue from custom trading fees (still cheaper than competitors at 0.1-0.2%)
- Lower integration risk (formal verification = no exploits)
- Advanced features that keep power users in your ecosystem

---

## Quick Start

### Install

```bash
yarn add @bonasa-tech/manifest-sdk
```

### Your First Limit Order

```typescript
import { ManifestClient, OrderType } from '@bonasa-tech/manifest-sdk';
import { Connection, PublicKey, Transaction } from '@solana/web3.js';

// Initialize client for a market
const connection = new Connection('https://api.mainnet-beta.solana.com');
const marketPubkey = new PublicKey('YOUR_MARKET_ADDRESS');
const wallet = getWalletAdapter(); // Your wallet integration

const client = await ManifestClient.getClientForMarket(
  connection,
  marketPubkey,
  wallet
);

// Claim seat (one-time per market per user)
if (!client.market.hasSeat(wallet.publicKey)) {
  await wallet.sendTransaction(
    new Transaction().add(client.claimSeatIx()),
    connection
  );
}

// Place a limit order
const orderIx = client.placeOrderIx({
  numBaseTokens: 1.5,
  tokenPrice: 100.0,
  isBid: true,
  orderType: OrderType.Limit
});

await wallet.sendTransaction(new Transaction().add(orderIx), connection);
```

Done. Real limit orders in ~20 lines of code.

---

## Core Concepts

### Order Types

Manifest supports multiple order types to match various trading strategies:

| Order Type | Description | Use Case |
|------------|-------------|----------|
| `Limit` | Standard limit order that rests on the book | Normal trading |
| `PostOnly` | Fails if it would match immediately | Market making, avoid taker fees |
| `ImmediateOrCancel` | Takes liquidity but never rests | Market orders with price protection |
| `Global` | Uses global account capital (can support multiple markets) | Capital-efficient market making |
| `Reverse` | Auto-flips to opposite side when filled | AMM-like behavior, LP positions |

### Market Structure

- **Market account**: Contains full orderbook (~0.007 SOL rent)
- **Base/Quote vaults**: Hold deposited tokens
- **Seats**: One-time claim per trader per market (required to rest a limit order, seats not required to swap)

### Two Ways to Trade

**Direct (simpler)**
Claim seat → Place order with wallet tokens → Done

**Deposit first (for active traders)**
Claim seat → Deposit to market → Place many orders → Withdraw when done

---

## Integration Architecture

### Recommended Architecture for Wallets

```
┌─────────────────────────────────────────────────────────┐
│                    Your Wallet UI                       │
│  - Market selection                                      │
│  - Order entry form                                      │
│  - Open orders management                                │
│  - Trade history                                         │
└────────────────────┬────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────┐
│              Your Backend (Optional)                     │
│  - Market discovery/indexing                             │
│  - Price feeds                                           │
│  - Transaction building                                  │
│  - Fee collection tracking                               │
└────────────────────┬────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────┐
│            Manifest TypeScript SDK                       │
│  @bonasa-tech/manifest-sdk                              │
└────────────────────┬────────────────────────────────────┘
                     │
      ┌──────────────┴──────────────┐
      ▼                             ▼
┌──────────────┐          ┌──────────────────┐
│ Core Program │          │ Wrapper Program  │
│ (Feeless)    │          │ (Custom Fees)    │
└──────────────┘          └──────────────────┘
```

### Integration Layers

1. **UI Layer**: Your wallet interface
   - Order entry
   - Market data display
   - Order management

2. **SDK Layer**: Manifest TypeScript SDK
   - Instruction building
   - Market state parsing
   - Account management

3. **Protocol Layer**: Choose your approach
   - **Core Program**: Direct interaction, no fees
   - **Wrapper Program**: Custom fee support, client order ID tracking

---

## Implementation Guide

### Step 1: Market Discovery

Find markets for token pairs your users want to trade:

```typescript
import { ManifestClient } from '@bonasa-tech/manifest-sdk';

// List all markets
const allMarkets = await ManifestClient.listMarketPublicKeys(connection);

// Find market for specific token pair
const markets = await ManifestClient.listMarketsForMints(
  connection,
  baseMint,  // e.g., SOL mint
  quoteMint  // e.g., USDC mint
);

// Use the first market (or let user choose if multiple)
const marketPubkey = markets[0];
```

**No Market Exists?** Create one! It only costs **0.007 SOL** in rent:

```typescript
// Market creation (advanced - see SDK docs)
const createMarketIx = createMarketInstruction({
  payer: walletPubkey,
  baseMint,
  quoteMint,
  baseDecimals,
  quoteDecimals,
});
```

### Step 2: Initialize Client

```typescript
const client = await ManifestClient.getClientForMarket(
  connection,
  marketPubkey,
  wallet // Signer with privateKey access
);

// Or without automatic seat claiming:
const client = await ManifestClient.getClientForMarketNoPrivateKey(
  connection,
  marketPubkey,
  walletPubkey
);
```

### Step 3: Claim Seat

One-time per trader per market:

```typescript
if (!client.market.hasSeat(wallet.publicKey)) {
  const tx = new Transaction().add(
    ComputeBudgetProgram.setComputeUnitLimit({ units: 70_000 }),
    client.claimSeatIx()
  );
  await wallet.sendTransaction(tx, connection);
}
```

### Step 4: Get Orderbook

```typescript
const bids = client.market.bids();
const asks = client.market.asks();

const spread = asks[0]?.price - bids[0]?.price;
```

### Step 5: Place an Order

```typescript
const tx = new Transaction().add(
  ComputeBudgetProgram.setComputeUnitLimit({ units: 100_000 }),
  client.placeOrderIx({
    numBaseTokens: 10,
    tokenPrice: 25.50,
    isBid: true,
    orderType: OrderType.Limit
  })
);

await wallet.sendTransaction(tx, connection);
```

### Step 6: Cancel & Withdraw

```typescript
// Cancel all
await wallet.sendTransaction(
  new Transaction().add(client.cancelAllIx()),
  connection
);

// Withdraw all
await wallet.sendTransaction(
  new Transaction().add(...client.withdrawAllIx()),
  connection
);
```

---

## Fee Structure & Monetization

The core protocol is free (zero fees, forever). But you can add your own fees via wrapper programs.

### How to Monetize

Build a custom wrapper program that:
1. Takes orders from your UI
2. Calls Manifest via CPI
3. Collects fees on settlement

Reference implementation: `wMNFSTkir3HgyZTsB7uqu3i7FA73grFCptPXgrZjksL`

### Fee Calculation

```rust
fee_atoms = quote_volume * fee_mantissa / 1_000_000_000
```

Common fee tiers:
- 0.05-0.1% - Competitive with CEXs
- 0.2-0.5% - Premium tier
- Still cheaper than most DEXs at 0.25-0.3%

---

## Advanced Features

### Global Orders
Same capital supports orders across multiple markets. Good for market makers and power users who are capital sensitive. For example, a user with 100 USDC in their wallet could set a global buy order for 100 USDC to purchase SOL, BONK or any other token without need more than 100 USDC.

```typescript
orderType: OrderType.Global
```

### Reverse Orders
Auto-flip when filled. Buy at $100 → auto-sells at $102 (with spread). Like concentrated liquidity AMM positions.

### Batch Operations
Cancel + place + withdraw in one transaction.

### Token-22
Fully supported, auto-detected by SDK.

### Long-Tail Pairs
0.007 SOL market creation means you can list exotic pairs that don't exist anywhere else.

---

## Code Examples

### Complete Wallet Integration Example

```typescript
import {
  ManifestClient,
  OrderType,
  Market,
} from '@bonasa-tech/manifest-sdk';
import {
  Connection,
  PublicKey,
  Transaction,
  ComputeBudgetProgram,
} from '@solana/web3.js';
import { getAssociatedTokenAddressSync } from '@solana/spl-token';

class ManifestWalletIntegration {
  private connection: Connection;
  private wallet: any; // Your wallet adapter
  private clients: Map<string, ManifestClient>;

  constructor(connection: Connection, wallet: any) {
    this.connection = connection;
    this.wallet = wallet;
    this.clients = new Map();
  }

  /**
   * Get or create client for a market
   */
  async getClient(marketPubkey: PublicKey): Promise<ManifestClient> {
    const key = marketPubkey.toBase58();

    if (!this.clients.has(key)) {
      const client = await ManifestClient.getClientForMarket(
        this.connection,
        marketPubkey,
        this.wallet
      );
      this.clients.set(key, client);
    }

    return this.clients.get(key)!;
  }

  /**
   * Find or create market for token pair
   */
  async findMarket(
    baseMint: PublicKey,
    quoteMint: PublicKey
  ): Promise<PublicKey> {
    const markets = await ManifestClient.listMarketsForMints(
      this.connection,
      baseMint,
      quoteMint
    );

    if (markets.length > 0) {
      return markets[0]; // Use first available market
    }

    // Market doesn't exist - could create here
    // For now, throw error
    throw new Error(
      `No market found for ${baseMint.toBase58()} / ${quoteMint.toBase58()}`
    );
  }

  /**
   * Ensure user has a seat on the market
   */
  async ensureSeat(marketPubkey: PublicKey): Promise<void> {
    const client = await this.getClient(marketPubkey);
    const hasSeat = client.market.hasSeat(this.wallet.publicKey);

    if (!hasSeat) {
      const tx = new Transaction();

      tx.add(
        ComputeBudgetProgram.setComputeUnitLimit({ units: 70_000 }),
        ComputeBudgetProgram.setComputeUnitPrice({ microLamports: 10_000 })
      );

      const claimSeatIx = client.claimSeatIx();
      tx.add(claimSeatIx);

      const signature = await this.wallet.sendTransaction(tx, this.connection);
      await this.connection.confirmTransaction(signature);
    }
  }

  /**
   * Get orderbook for display
   */
  async getOrderbook(marketPubkey: PublicKey) {
    const client = await this.getClient(marketPubkey);
    await client.reload();

    const bids = client.market.bids();
    const asks = client.market.asks();

    return {
      bids: bids.map(order => ({
        price: order.tokenPrice,
        size: order.numBaseTokens,
        total: order.tokenPrice * order.numBaseTokens,
      })),
      asks: asks.map(order => ({
        price: order.tokenPrice,
        size: order.numBaseTokens,
        total: order.tokenPrice * order.numBaseTokens,
      })),
      spread: asks[0]?.tokenPrice - bids[0]?.tokenPrice,
      midPrice: (asks[0]?.tokenPrice + bids[0]?.tokenPrice) / 2,
    };
  }

  /**
   * Place a limit order
   */
  async placeLimitOrder(
    marketPubkey: PublicKey,
    params: {
      side: 'buy' | 'sell';
      amount: number;      // In base tokens
      price: number;       // Limit price
      orderType?: OrderType;
    }
  ): Promise<string> {
    // Ensure seat is claimed
    await this.ensureSeat(marketPubkey);

    const client = await this.getClient(marketPubkey);

    const orderParams = {
      numBaseTokens: params.amount,
      tokenPrice: params.price,
      isBid: params.side === 'buy',
      orderType: params.orderType || OrderType.Limit,
      lastValidSlot: 0,
      clientOrderId: Date.now(), // Use timestamp as simple ID
      minOutTokens: 0,
    };

    const tx = new Transaction();

    // Add compute budget
    tx.add(
      ComputeBudgetProgram.setComputeUnitLimit({ units: 100_000 }),
      ComputeBudgetProgram.setComputeUnitPrice({ microLamports: 10_000 })
    );

    // Add order instruction
    const placeOrderIx = client.placeOrderIx(orderParams);
    tx.add(placeOrderIx);

    // Send transaction
    const signature = await this.wallet.sendTransaction(tx, this.connection);

    // Wait for confirmation
    await this.connection.confirmTransaction(signature, 'confirmed');

    return signature;
  }

  /**
   * Cancel all orders
   */
  async cancelAllOrders(marketPubkey: PublicKey): Promise<string> {
    const client = await this.getClient(marketPubkey);

    const tx = new Transaction();

    tx.add(
      ComputeBudgetProgram.setComputeUnitLimit({ units: 80_000 }),
      ComputeBudgetProgram.setComputeUnitPrice({ microLamports: 10_000 })
    );

    const cancelAllIx = client.cancelAllIx();
    tx.add(cancelAllIx);

    const signature = await this.wallet.sendTransaction(tx, this.connection);
    await this.connection.confirmTransaction(signature, 'confirmed');

    return signature;
  }

  /**
   * Get user's open orders
   */
  async getOpenOrders(marketPubkey: PublicKey) {
    const client = await this.getClient(marketPubkey);
    await client.reload();

    return client.wrapper.openOrdersForMarket(marketPubkey);
  }

  /**
   * Withdraw all funds from market
   */
  async withdrawAll(marketPubkey: PublicKey): Promise<string> {
    const client = await this.getClient(marketPubkey);

    const tx = new Transaction();

    tx.add(
      ComputeBudgetProgram.setComputeUnitLimit({ units: 100_000 }),
      ComputeBudgetProgram.setComputeUnitPrice({ microLamports: 10_000 })
    );

    const withdrawIxs = client.withdrawAllIx();
    tx.add(...withdrawIxs);

    const signature = await this.wallet.sendTransaction(tx, this.connection);
    await this.connection.confirmTransaction(signature, 'confirmed');

    return signature;
  }

  /**
   * Get market balances for user
   */
  async getMarketBalances(marketPubkey: PublicKey) {
    const client = await this.getClient(marketPubkey);
    await client.reload();

    const marketInfo = client.wrapper.marketInfoForMarket(marketPubkey);

    return {
      base: marketInfo?.baseBalance || 0,
      quote: marketInfo?.quoteBalance || 0,
    };
  }
}

// Usage example:
async function main() {
  const connection = new Connection('https://api.mainnet-beta.solana.com');
  const wallet = getYourWallet(); // Your wallet adapter

  const integration = new ManifestWalletIntegration(connection, wallet);

  // Find SOL/USDC market
  const SOL_MINT = new PublicKey('So11111111111111111111111111111111111111112');
  const USDC_MINT = new PublicKey('EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v');

  const marketPubkey = await integration.findMarket(SOL_MINT, USDC_MINT);
  console.log('Market found:', marketPubkey.toBase58());

  // Get orderbook
  const orderbook = await integration.getOrderbook(marketPubkey);
  console.log('Best bid:', orderbook.bids[0]);
  console.log('Best ask:', orderbook.asks[0]);
  console.log('Spread:', orderbook.spread);

  // Place buy limit order: 1 SOL at $100
  const signature = await integration.placeLimitOrder(marketPubkey, {
    side: 'buy',
    amount: 1,
    price: 100,
  });
  console.log('Order placed:', signature);

  // Check open orders
  const openOrders = await integration.getOpenOrders(marketPubkey);
  console.log('Open orders:', openOrders);

  // Cancel all orders
  const cancelSig = await integration.cancelAllOrders(marketPubkey);
  console.log('Orders cancelled:', cancelSig);
}
```

### UI Component Example (React)

```typescript
import React, { useState, useEffect } from 'react';
import { ManifestWalletIntegration } from './manifest-integration';

export function LimitOrderForm({ integration, marketPubkey }) {
  const [side, setSide] = useState<'buy' | 'sell'>('buy');
  const [amount, setAmount] = useState('');
  const [price, setPrice] = useState('');
  const [orderbook, setOrderbook] = useState(null);
  const [loading, setLoading] = useState(false);

  // Load orderbook on mount
  useEffect(() => {
    loadOrderbook();
    const interval = setInterval(loadOrderbook, 5000); // Refresh every 5s
    return () => clearInterval(interval);
  }, [marketPubkey]);

  async function loadOrderbook() {
    const data = await integration.getOrderbook(marketPubkey);
    setOrderbook(data);

    // Auto-fill price with best bid/ask
    if (side === 'buy' && data.bids[0]) {
      setPrice(data.bids[0].price.toString());
    } else if (side === 'sell' && data.asks[0]) {
      setPrice(data.asks[0].price.toString());
    }
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setLoading(true);

    try {
      const signature = await integration.placeLimitOrder(marketPubkey, {
        side,
        amount: parseFloat(amount),
        price: parseFloat(price),
      });

      alert(`Order placed! Tx: ${signature}`);
      setAmount('');
      await loadOrderbook();
    } catch (error) {
      alert(`Error: ${error.message}`);
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="limit-order-form">
      <h2>Limit Order</h2>

      {orderbook && (
        <div className="market-info">
          <div>Best Bid: ${orderbook.bids[0]?.price.toFixed(2)}</div>
          <div>Best Ask: ${orderbook.asks[0]?.price.toFixed(2)}</div>
          <div>Spread: ${orderbook.spread?.toFixed(2)}</div>
        </div>
      )}

      <form onSubmit={handleSubmit}>
        <div className="side-selector">
          <button
            type="button"
            className={side === 'buy' ? 'active' : ''}
            onClick={() => setSide('buy')}
          >
            Buy
          </button>
          <button
            type="button"
            className={side === 'sell' ? 'active' : ''}
            onClick={() => setSide('sell')}
          >
            Sell
          </button>
        </div>

        <div className="form-field">
          <label>Amount</label>
          <input
            type="number"
            value={amount}
            onChange={e => setAmount(e.target.value)}
            placeholder="0.00"
            step="0.000001"
            required
          />
        </div>

        <div className="form-field">
          <label>Price</label>
          <input
            type="number"
            value={price}
            onChange={e => setPrice(e.target.value)}
            placeholder="0.00"
            step="0.01"
            required
          />
        </div>

        <div className="form-field">
          <label>Total</label>
          <div className="readonly">
            {amount && price
              ? `$${(parseFloat(amount) * parseFloat(price)).toFixed(2)}`
              : '$0.00'}
          </div>
        </div>

        <button
          type="submit"
          disabled={loading || !amount || !price}
          className={`submit-btn ${side}`}
        >
          {loading ? 'Placing Order...' : `Place ${side} Order`}
        </button>
      </form>
    </div>
  );
}
```

---

## Testing & Support

### Testing Approach

For initial testing and development:

```typescript
// Start with mainnet markets using small amounts
const connection = new Connection('https://api.mainnet-beta.solana.com');

// Use existing liquid markets for testing
// e.g., SOL/USDC, BONK/USDC, etc.

// Contact Manifest team for recommended test market addresses
// and integration support
```

### Compute Unit Estimation

Typical CU usage:
- Claim seat (one-time): 50,000 - 70,000 CU
- Simple order placement: 40,000 - 60,000 CU
- Order placement + auto seat claim: 80,000 - 100,000 CU
- Cancel all + withdraw: 80,000 - 120,000 CU
- Batch operations: 100,000 - 150,000 CU

Always add compute budget instructions:

```typescript
tx.add(
  ComputeBudgetProgram.setComputeUnitLimit({ units: 100_000 }),
  ComputeBudgetProgram.setComputeUnitPrice({ microLamports: 10_000 })
);
```

### Debugging

Manifest provides a custom explorer at [explorer.manifest.trade](https://explorer.manifest.trade/) with:
- Instruction decoding
- Fill log parsing
- Market state inspection

### Formal Verification

Manifest is the **only DEX on Solana with formal verification** - mathematical proof that the code works correctly.

**What this means:**
Unlike traditional audits that just look for bugs, formal verification mathematically proves entire classes of bugs cannot exist. Certora prover runs daily against the codebase.

**Verified properties:**
- No loss of funds (mathematically impossible)
- Users can always withdraw/cancel
- Matching logic correctly attributes funds
- Data structures maintain integrity

**Why it matters for you:**
DEX exploits have cost users billions (Wormhole $320M, Mango $110M, Crema $8.8M). Integrating an unverified DEX exposes you to reputation damage, user flight, and legal liability.

Formal verification eliminates these risks. It's the highest security standard in software and Manifest is the only Solana DEX that has it.

### Documentation & Resources

- **NPM Package**: [@bonasa-tech/manifest-sdk](https://www.npmjs.com/package/@bonasa-tech/manifest-sdk)
- **Whitepaper**: [The Orderbook Manifesto](https://manifest.trade/whitepaper.pdf)
- **Audit Report**: [manifest.trade/audit.pdf](https://www.manifest.trade/audit.pdf)
- **Formal Verification**: Detailed in [Certora_README](https://github.com/Bonasa-Tech/manifest/blob/main/Certora_README.md)
- **GitHub**: [github.com/Bonasa-Tech/manifest](https://github.com/Bonasa-Tech/manifest)
- **Explorer**: [explorer.manifest.trade](https://explorer.manifest.trade/)

### Getting Help

- **GitHub Issues**: Report bugs or request features
- **Email**: dev@manifest.trade for integration support

---

## Summary: Why Choose Manifest?

### For Users
- **Better Prices**: True limit orders fill at specified price or better
- **Better Execution**: No slippage, no MEV extraction
- **Lower Costs**: Zero protocol fees (possible wallet fees are still lower than alternatives)
- **More Options**: Trade exotic pairs that don't exist elsewhere

### For Wallets
- Offer real DeFi features competitors don't have
- New revenue from trading fees (still cheaper than alternatives)
- Simple SDK integration
- Only formally verified DEX on Solana = no exploit risk

### Technical
- **Formally verified** - only DEX on Solana with mathematical proof of security
- **Zero fees** - Core protocol is free forever
- **Capital efficient** - Global orders across markets
- **Cheap markets** - 0.007 SOL creation (350x cheaper)
- **Better CU** - Lower consumption than alternatives

---

## Next Steps

1. **Install SDK**: `yarn add @bonasa-tech/manifest-sdk`
2. **Review Examples**: Study code samples in this guide
3. **Build POC**: Test integration with mainnet markets using small amounts
4. **Define Fee Structure**: Decide on monetization approach
5. **Implement UI**: Build order entry and management interface
6. **Test with Users**: Beta test with small user group
7. **Launch**: Deploy to production and promote to users

**Questions?** Reach out to the Manifest team—we're here to help you succeed.

---

*Built with Manifest - The Unlimited Orderbook*
