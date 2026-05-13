//! Minimal dependency Manifest client for instruction building and market parsing.
//!
//! This crate provides instruction builders and state parsing for the Manifest
//! exchange with minimal dependencies.

mod constants;
mod events;
mod instruction;
mod state;

pub use solana_instruction::{AccountMeta, Instruction};
pub use solana_pubkey::Pubkey;

pub use constants::{
    DataIndex, OrderType, CLAIMED_SEAT_SIZE, MANIFEST_PROGRAM_ID, MARKET_BLOCK_SIZE,
    MARKET_FIXED_DISCRIMINANT, MARKET_FIXED_SIZE, NIL, NO_EXPIRATION_LAST_VALID_SLOT,
    RESTING_ORDER_SIZE, SYSTEM_PROGRAM_ID, TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID,
};

pub use instruction::{
    batch_update_instruction, batch_update_with_global_instruction, claim_seat_instruction,
    create_market_instruction, deposit_instruction, expand_instruction, get_global_address,
    get_global_vault_address, get_vault_address, swap_instruction, withdraw_instruction,
    BatchUpdateParams, CancelOrderParams, DepositParams, ManifestInstruction, PlaceOrderParams,
    SwapParams, WithdrawParams,
};

pub use state::{ClaimedSeat, Market, MarketFixed, OrderIterator, RBNodeHeader, RestingOrder};

// Event types
pub use events::{BaseAtoms, GlobalAtoms, PodBool, QuoteAtoms, QuoteAtomsPerBaseAtom};
// Event discriminants
pub use events::{
    CANCEL_ORDER_LOG_DISCRIMINANT, CLAIM_SEAT_LOG_DISCRIMINANT, CREATE_MARKET_LOG_DISCRIMINANT,
    DEPOSIT_LOG_DISCRIMINANT, FILL_LOG_DISCRIMINANT, GLOBAL_ADD_TRADER_LOG_DISCRIMINANT,
    GLOBAL_CLAIM_SEAT_LOG_DISCRIMINANT, GLOBAL_CLEANUP_LOG_DISCRIMINANT,
    GLOBAL_CREATE_LOG_DISCRIMINANT, GLOBAL_DEPOSIT_LOG_DISCRIMINANT, GLOBAL_EVICT_LOG_DISCRIMINANT,
    GLOBAL_WITHDRAW_LOG_DISCRIMINANT, PLACE_ORDER_LOG_DISCRIMINANT,
    PLACE_ORDER_LOG_V2_DISCRIMINANT, WITHDRAW_LOG_DISCRIMINANT,
};
// Event structs
pub use events::{
    CancelOrderLog, ClaimSeatLog, CreateMarketLog, DepositLog, FillLog, GlobalAddTraderLog,
    GlobalClaimSeatLog, GlobalCleanupLog, GlobalCreateLog, GlobalDepositLog, GlobalEvictLog,
    GlobalWithdrawLog, PlaceOrderLog, PlaceOrderLogV2, WithdrawLog,
};

#[cfg(test)]
mod tests;
