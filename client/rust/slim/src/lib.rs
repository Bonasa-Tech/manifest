//! Minimal dependency Manifest client for instruction building and market parsing.
//!
//! This crate provides instruction builders and state parsing for the Manifest
//! exchange with minimal dependencies.

mod constants;
mod instruction;
mod state;

pub use solana_instruction::AccountMeta;
pub use solana_instruction::Instruction;
pub use solana_pubkey::Pubkey;

pub use constants::DataIndex;
pub use constants::OrderType;
pub use constants::CLAIMED_SEAT_SIZE;
pub use constants::MANIFEST_PROGRAM_ID;
pub use constants::MARKET_BLOCK_SIZE;
pub use constants::MARKET_FIXED_DISCRIMINANT;
pub use constants::MARKET_FIXED_SIZE;
pub use constants::NIL;
pub use constants::NO_EXPIRATION_LAST_VALID_SLOT;
pub use constants::RESTING_ORDER_SIZE;
pub use constants::SYSTEM_PROGRAM_ID;
pub use constants::TOKEN_2022_PROGRAM_ID;
pub use constants::TOKEN_PROGRAM_ID;

pub use instruction::batch_update_instruction;
pub use instruction::batch_update_with_global_instruction;
pub use instruction::claim_seat_instruction;
pub use instruction::create_market_instruction;
pub use instruction::deposit_instruction;
pub use instruction::expand_instruction;
pub use instruction::get_global_address;
pub use instruction::get_global_vault_address;
pub use instruction::get_vault_address;
pub use instruction::swap_instruction;
pub use instruction::withdraw_instruction;
pub use instruction::BatchUpdateParams;
pub use instruction::CancelOrderParams;
pub use instruction::DepositParams;
pub use instruction::ManifestInstruction;
pub use instruction::PlaceOrderParams;
pub use instruction::SwapParams;
pub use instruction::WithdrawParams;

pub use state::ClaimedSeat;
pub use state::Market;
pub use state::MarketFixed;
pub use state::OrderIterator;
pub use state::RBNodeHeader;
pub use state::RestingOrder;

#[cfg(test)]
mod tests;
