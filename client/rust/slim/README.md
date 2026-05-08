# manifest-client

A minimal dependency Rust client for the Manifest DEX. This crate provides instruction builders and market state parsing without requiring `solana-program` or `solana-sdk` as dependencies, making it suitable for lightweight integrations.

## Features

- Instruction builders for all Manifest operations
- Market state parsing (orders, seats, balances)
- PDA derivation helpers
- Minimal dependencies: only `solana-pubkey` and `solana-instruction`

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
manifest-client = { path = "path/to/client/rust/slim" }
```

## Usage

### Creating a Market

```rust
use manifest_client::{
    create_market_instruction,
    TOKEN_PROGRAM_ID,
    TOKEN_2022_PROGRAM_ID,
    Pubkey,
};

let ix = create_market_instruction(
    payer,
    market,
    base_mint,
    quote_mint,
    TOKEN_PROGRAM_ID,
    TOKEN_2022_PROGRAM_ID,
);
```

### Depositing Tokens

```rust
use manifest_client::{
    deposit_instruction,
    DepositParams,
    TOKEN_PROGRAM_ID,
};

let ix = deposit_instruction(
    payer,
    market,
    trader_token_account,
    mint,
    TOKEN_PROGRAM_ID,
    DepositParams::new(1_000_000_000), // amount in atoms
);
```

### Placing Orders

```rust
use manifest_client::{
    batch_update_instruction,
    BatchUpdateParams,
    PlaceOrderParams,
    OrderType,
};

let params = BatchUpdateParams::new()
    .add_order(PlaceOrderParams::new(
        1_000_000_000,  // base atoms
        150,            // price mantissa
        0,              // price exponent (price = mantissa * 10^exponent)
        true,           // is_bid
        OrderType::Limit,
    ));

let ix = batch_update_instruction(payer, market, params);
```

### Swapping

```rust
use manifest_client::{
    swap_instruction,
    SwapParams,
    TOKEN_PROGRAM_ID,
};

let ix = swap_instruction(
    payer,
    market,
    trader_base_account,
    trader_quote_account,
    base_mint,
    quote_mint,
    TOKEN_PROGRAM_ID,
    None,   // quote token program (if different)
    false,  // include_base_mint (for token-2022)
    false,  // include_quote_mint (for token-2022)
    SwapParams::new(
        100_000_000,  // in_atoms
        0,            // min out_atoms
        false,        // is_base_in
        true,         // is_exact_in
    ),
);
```

### Parsing Market State

```rust
use manifest_client::{Market, DataIndex};

let market = Market::try_from_bytes(&account_data).unwrap();

// Get best prices
let best_bid: Option<f64> = market.get_best_bid();
let best_ask: Option<f64> = market.get_best_ask();

// Iterate orders
for (index, order) in market.iter_bids() {
    println!("Bid at index {}: {} @ {}",
        index,
        order.num_base_atoms,
        order.get_price_float()
    );
}

// Find trader's seat
if let Some((index, seat)) = market.find_trader_seat(&trader_pubkey) {
    println!("Base balance: {}", seat.base_withdrawable_balance);
    println!("Quote balance: {}", seat.quote_withdrawable_balance);
}
```

## Running Tests

The test suite uses `solana-program-test` to verify instructions work correctly with the actual Manifest program.

```bash
cargo test -p manifest-client
```

## Types

### DataIndex

A `u32` type alias used for indices into the market's dynamic data. This is consistent with the Manifest program's internal representation.

### OrderType

```rust
pub enum OrderType {
    Limit,
    ImmediateOrCancel,
    PostOnly,
    Global,
    Reverse,
    ReverseTight,
}
```

### Constants

- `MANIFEST_PROGRAM_ID` - The Manifest program address
- `NIL` - Sentinel value for null indices (`DataIndex::MAX`)
- `MARKET_FIXED_SIZE` - Size of the market header (256 bytes)
- `NO_EXPIRATION_LAST_VALID_SLOT` - Sentinel for orders with no expiration

## License

See LICENSE file in the repository root.
