//! Constants for the Manifest client.

use solana_pubkey::Pubkey;

// Re-export types from hypertree
pub use hypertree::DataIndex;
pub use hypertree::NIL;

/// Order type enum for placing and parsing orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum OrderType {
    #[default]
    Limit = 0,
    ImmediateOrCancel = 1,
    PostOnly = 2,
    Global = 3,
    Reverse = 4,
    ReverseTight = 5,
}

impl OrderType {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(OrderType::Limit),
            1 => Some(OrderType::ImmediateOrCancel),
            2 => Some(OrderType::PostOnly),
            3 => Some(OrderType::Global),
            4 => Some(OrderType::Reverse),
            5 => Some(OrderType::ReverseTight),
            _ => None,
        }
    }

    pub fn is_reversible(&self) -> bool {
        matches!(self, OrderType::Reverse | OrderType::ReverseTight)
    }
}

/// Manifest program ID: MNFSTqtC93rEfYHB6hF82sKdZpUDFWkViLByLd1k1Ms
pub const MANIFEST_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("MNFSTqtC93rEfYHB6hF82sKdZpUDFWkViLByLd1k1Ms");

/// System program ID
pub const SYSTEM_PROGRAM_ID: Pubkey = Pubkey::from_str_const("11111111111111111111111111111111");

/// SPL Token program ID
pub const TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

/// SPL Token-2022 program ID
pub const TOKEN_2022_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

/// Market discriminant value.
pub const MARKET_FIXED_DISCRIMINANT: u64 = 4859840929024028656;

/// Size of MarketFixed header in bytes.
pub const MARKET_FIXED_SIZE: usize = 256;

/// Size of each block in the market dynamic data.
pub const MARKET_BLOCK_SIZE: usize = 80;

/// Resting order size in bytes.
pub const RESTING_ORDER_SIZE: usize = 64;

/// Claimed seat size in bytes.
pub const CLAIMED_SEAT_SIZE: usize = 64;

/// No expiration sentinel for orders.
pub const NO_EXPIRATION_LAST_VALID_SLOT: u32 = 0;
