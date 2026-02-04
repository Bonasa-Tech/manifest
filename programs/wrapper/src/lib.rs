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
use processors::{
    batch_upate::process_batch_update, claim_seat::process_claim_seat, collect::process_collect,
    create_wrapper::process_create_wrapper, deposit::process_deposit, withdraw::process_withdraw,
};
use solana_program::{
    account_info::AccountInfo, declare_id, entrypoint::ProgramResult, program_error::ProgramError,
    pubkey::Pubkey,
};

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
solana_program::entrypoint!(process_instruction);

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
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
