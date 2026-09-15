//! Wrapper program for Manifest
//!

pub mod instruction;
pub mod instruction_builders;
pub mod loader;
pub mod market_info;
pub mod open_order;
pub mod processors;
pub mod wrapper_state;

use hypertree::trace;
use instruction::ManifestWrapperInstruction;
use pinocchio::{account::AccountView, error::ProgramError, ProgramResult};
use processors::{
    batch_upate::process_batch_update, claim_seat::process_claim_seat, collect::process_collect,
    create_wrapper::process_create_wrapper, deposit::process_deposit, withdraw::process_withdraw,
};
use solana_program::{declare_id, pubkey::Pubkey};

#[cfg(not(feature = "no-entrypoint"))]
use solana_security_txt::security_txt;

#[cfg(not(feature = "no-entrypoint"))]
security_txt! {
    name: "manifest-wrapper",
    project_url: "",
    contacts: "email:dev@manifest.trade",
    policy: "",
    preferred_languages: "en",
    source_code: "https://github.com/Bonasa-Tech/manifest",
    auditors: ""
}

declare_id!("wMNFSTkir3HgyZTsB7uqu3i7FA73grFCptPXgrZjksL");

/// Fee charged per batch update, in lamports, transferred from the payer to
/// the wrapper state.
///
/// This used to borrow manifest's GAS_DEPOSIT_LAMPORTS, which happened to be
/// the same number but means something else entirely: that constant is the
/// core program's gas prepayment and refund for a global order. The two are
/// independent knobs, so the wrapper fee gets its own.
pub const WRAPPER_FEE_LAMPORTS: u64 = 10_000;

#[cfg(not(feature = "no-entrypoint"))]
pinocchio::program_entrypoint!(process_instruction, {
    manifest::state::constants::MAX_ACCOUNTS
});
// program_entrypoint! does not install a global allocator. Without one the
// toolchain links the deprecated Solana allocator, which calls sol_alloc_free_,
// a syscall the runtime does not register for SBPF v2. The resulting binary is
// rejected at deploy time.
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

    let instruction: ManifestWrapperInstruction =
        ManifestWrapperInstruction::try_from(*tag).or(Err(ProgramError::InvalidInstructionData))?;

    trace!("Instruction: {:?}", instruction);

    match instruction {
        ManifestWrapperInstruction::CreateWrapper => {
            process_create_wrapper(program_id, accounts, data)?;
        }
        ManifestWrapperInstruction::ClaimSeat => {
            process_claim_seat(program_id, accounts, data)?;
        }
        ManifestWrapperInstruction::Collect => {
            process_collect(program_id, accounts, data)?;
        }
        ManifestWrapperInstruction::Deposit => {
            process_deposit(program_id, accounts, data)?;
        }
        ManifestWrapperInstruction::Withdraw => {
            process_withdraw(program_id, accounts, data)?;
        }
        ManifestWrapperInstruction::BatchUpdate => {
            process_batch_update(program_id, accounts, data)?;
        }
        ManifestWrapperInstruction::BatchUpdateBaseGlobal => {
            process_batch_update(program_id, accounts, data)?;
        }
        ManifestWrapperInstruction::BatchUpdateQuoteGlobal => {
            process_batch_update(program_id, accounts, data)?;
        }
    }

    Ok(())
}
