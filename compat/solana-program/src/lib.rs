//! Source-compatibility facade for the Solana Program 3 module split.
//!
//! Solana Program 3 moved the system program ID and instruction constructors
//! into dedicated crates. Re-export them here so the audited on-chain source
//! can move to the v3-capable runtime without a broad import-only rewrite.

pub use solana_program_upstream::*;
pub use solana_sdk_ids::system_program;
pub use solana_system_interface::instruction as system_instruction;
