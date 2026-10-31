//! Liquidity wrapper for Manifest.
//!
//! Runs a ladder of reverse orders on one Manifest market as a position of its
//! own, and records what went in and what came out, so a strategy's return
//! can be compared with simply holding the opening mix.
//!
//! `CreatePosition` makes a PDA unique per (owner, market, creation unix
//! time) holding the opening price and the ladder, and a trader PDA (a plain
//! system account the program signs for) that claims its own seat on the
//! market. Its orders, balances and fills are therefore the position's alone.
//!
//! `PlaceOrders` lays the given orders of the ladder, computed on chain from
//! the stored ladder and opening price: it pulls exactly what they need from
//! the owner's wallet into the seat and adds that to the opening snapshot. A
//! bitmap marks each order placed, so none can be placed twice and a
//! placement cut short resumes from chain data. `ClosePosition` cancels every
//! order of the seat, withdraws its balances to the owner and records them as
//! the closing snapshot; after that the account never changes again.

pub mod instruction;
pub mod processor;
pub mod state;

use instruction::PositionInstruction;
use pinocchio::{account::AccountView, error::ProgramError, ProgramResult};
use solana_program::{declare_id, pubkey::Pubkey};

#[cfg(not(feature = "no-entrypoint"))]
use solana_security_txt::security_txt;

#[cfg(not(feature = "no-entrypoint"))]
security_txt! {
    name: "manifest-liquidity-wrapper",
    project_url: "",
    contacts: "email:dev@manifest.trade",
    policy: "",
    preferred_languages: "en",
    source_code: "https://github.com/Bonasa-Tech/manifest",
    auditors: ""
}

declare_id!("C49XDn78cDqgo9fhjXUuLJB1N5pDxVyCPWYHUe2FBjHo");

#[cfg(not(feature = "no-entrypoint"))]
pinocchio::program_entrypoint!(process_instruction, 20);
// See ui-wrapper: program_entrypoint! installs no allocator, and the default
// one calls a syscall SBPF v2 does not register.
#[cfg(not(feature = "no-entrypoint"))]
pinocchio::default_allocator!();

pub fn process_instruction(
    program_id_raw: &pinocchio::address::Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    let program_id: &Pubkey = manifest::validation::as_pubkey(program_id_raw);
    let (tag, data) = instruction_data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;

    match PositionInstruction::try_from(*tag)? {
        PositionInstruction::CreatePosition => {
            processor::process_create_position(program_id, accounts, data)
        }
        PositionInstruction::PlaceOrders => {
            processor::process_place_orders(program_id, accounts, data)
        }
        PositionInstruction::ClosePosition => {
            processor::process_close_position(program_id, accounts, data)
        }
        PositionInstruction::CancelOrders => {
            processor::process_cancel_orders(program_id, accounts, data)
        }
    }
}
