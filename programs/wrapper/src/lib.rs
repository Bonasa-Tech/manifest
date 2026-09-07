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

#[cfg(not(feature = "no-entrypoint"))]
pinocchio::program_entrypoint!(process_instruction, {
    manifest::state::constants::MAX_ACCOUNTS
});

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
