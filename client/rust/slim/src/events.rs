//! Event types emitted by the Manifest program.
//!
//! These are the log events that can be parsed from transaction logs.
//! Each event has an 8-byte discriminant prefix followed by the struct data.

use crate::constants::OrderType;
use crate::Pubkey;

/// Boolean type that is Pod-compatible (1 byte).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct PodBool(pub u8);

impl PodBool {
    pub fn is_true(&self) -> bool {
        self.0 != 0
    }
}

/// Base token atoms (smallest unit). Wrapper around u64.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct BaseAtoms(pub u64);

/// Quote token atoms (smallest unit). Wrapper around u64.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct QuoteAtoms(pub u64);

/// Global token atoms. Wrapper around u64.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct GlobalAtoms(pub u64);

/// Price represented as quote atoms per base atom.
/// Stored as a 128-bit fixed point number in two u64s.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct QuoteAtomsPerBaseAtom {
    pub inner: [u64; 2],
}

// Event discriminants (8-byte prefixes)
pub const CREATE_MARKET_LOG_DISCRIMINANT: [u8; 8] = [33, 31, 11, 6, 133, 143, 39, 71];
pub const CLAIM_SEAT_LOG_DISCRIMINANT: [u8; 8] = [129, 77, 152, 210, 218, 144, 163, 56];
pub const DEPOSIT_LOG_DISCRIMINANT: [u8; 8] = [23, 214, 24, 34, 52, 104, 109, 188];
pub const WITHDRAW_LOG_DISCRIMINANT: [u8; 8] = [112, 218, 111, 63, 18, 95, 136, 35];
pub const FILL_LOG_DISCRIMINANT: [u8; 8] = [58, 230, 242, 3, 75, 113, 4, 169];
pub const PLACE_ORDER_LOG_DISCRIMINANT: [u8; 8] = [157, 118, 247, 213, 47, 19, 164, 120];
pub const PLACE_ORDER_LOG_V2_DISCRIMINANT: [u8; 8] = [189, 97, 159, 235, 136, 5, 1, 141];
pub const CANCEL_ORDER_LOG_DISCRIMINANT: [u8; 8] = [22, 65, 71, 33, 244, 235, 255, 215];
pub const GLOBAL_CREATE_LOG_DISCRIMINANT: [u8; 8] = [188, 25, 199, 77, 26, 15, 142, 193];
pub const GLOBAL_ADD_TRADER_LOG_DISCRIMINANT: [u8; 8] = [129, 246, 90, 94, 87, 186, 242, 7];
pub const GLOBAL_CLAIM_SEAT_LOG_DISCRIMINANT: [u8; 8] = [164, 46, 227, 175, 3, 143, 73, 86];
pub const GLOBAL_DEPOSIT_LOG_DISCRIMINANT: [u8; 8] = [16, 26, 72, 1, 145, 232, 182, 71];
pub const GLOBAL_WITHDRAW_LOG_DISCRIMINANT: [u8; 8] = [206, 118, 67, 64, 124, 109, 157, 201];
pub const GLOBAL_EVICT_LOG_DISCRIMINANT: [u8; 8] = [250, 180, 155, 38, 98, 223, 82, 223];
pub const GLOBAL_CLEANUP_LOG_DISCRIMINANT: [u8; 8] = [193, 249, 115, 186, 42, 126, 196, 82];

/// Emitted when a new market is created.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct CreateMarketLog {
    pub market: Pubkey,
    pub creator: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
}

/// Emitted when a trader claims a seat on a market.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct ClaimSeatLog {
    pub market: Pubkey,
    pub trader: Pubkey,
}

/// Emitted when tokens are deposited to a market.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct DepositLog {
    pub market: Pubkey,
    pub trader: Pubkey,
    pub mint: Pubkey,
    pub amount_atoms: u64,
}

/// Emitted when tokens are withdrawn from a market.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct WithdrawLog {
    pub market: Pubkey,
    pub trader: Pubkey,
    pub mint: Pubkey,
    pub amount_atoms: u64,
}

/// Emitted when an order is filled (partial or complete).
/// This is the most important event for tracking trades.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct FillLog {
    pub market: Pubkey,
    pub maker: Pubkey,
    pub taker: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub price: QuoteAtomsPerBaseAtom,
    pub base_atoms: BaseAtoms,
    pub quote_atoms: QuoteAtoms,
    pub maker_sequence_number: u64,
    pub taker_sequence_number: u64,
    pub taker_is_buy: PodBool,
    pub is_maker_global: PodBool,
    pub _padding: [u8; 14],
}

/// Emitted when an order is placed on the book.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct PlaceOrderLog {
    pub market: Pubkey,
    pub trader: Pubkey,
    pub price: QuoteAtomsPerBaseAtom,
    pub base_atoms: BaseAtoms,
    pub order_sequence_number: u64,
    pub order_index: u32,
    pub last_valid_slot: u32,
    pub order_type: OrderType,
    pub is_bid: PodBool,
    pub _padding: [u8; 6],
}

/// Emitted when an order is placed on the book (v2 with payer).
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct PlaceOrderLogV2 {
    pub market: Pubkey,
    pub trader: Pubkey,
    pub payer: Pubkey,
    pub price: QuoteAtomsPerBaseAtom,
    pub base_atoms: BaseAtoms,
    pub order_sequence_number: u64,
    pub order_index: u32,
    pub last_valid_slot: u32,
    pub order_type: OrderType,
    pub is_bid: PodBool,
    pub _padding: [u8; 6],
}

/// Emitted when an order is cancelled.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct CancelOrderLog {
    pub market: Pubkey,
    pub trader: Pubkey,
    pub order_sequence_number: u64,
}

/// Emitted when a global account is created.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct GlobalCreateLog {
    pub global: Pubkey,
    pub creator: Pubkey,
}

/// Emitted when a trader is added to a global account.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct GlobalAddTraderLog {
    pub global: Pubkey,
    pub trader: Pubkey,
}

/// Emitted when a trader claims a seat via global account.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct GlobalClaimSeatLog {
    pub global: Pubkey,
    pub market: Pubkey,
    pub trader: Pubkey,
}

/// Emitted when tokens are deposited to a global account.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct GlobalDepositLog {
    pub global: Pubkey,
    pub trader: Pubkey,
    pub global_atoms: GlobalAtoms,
}

/// Emitted when tokens are withdrawn from a global account.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct GlobalWithdrawLog {
    pub global: Pubkey,
    pub trader: Pubkey,
    pub global_atoms: GlobalAtoms,
}

/// Emitted when a trader is evicted from a global account.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct GlobalEvictLog {
    pub evictor: Pubkey,
    pub evictee: Pubkey,
    pub evictor_atoms: GlobalAtoms,
    pub evictee_atoms: GlobalAtoms,
}

/// Emitted when global orders are cleaned up.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct GlobalCleanupLog {
    pub cleaner: Pubkey,
    pub maker: Pubkey,
    pub amount_desired: GlobalAtoms,
    pub amount_deposited: GlobalAtoms,
}
