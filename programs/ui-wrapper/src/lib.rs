//! UI-Wrapper program for Manifest
//!

pub mod error;
pub mod instruction;
pub mod instruction_builders;
pub mod logs;
pub mod market_info;
pub mod open_order;
pub mod processors;
pub mod wrapper_user;

use hypertree::trace;
use instruction::ManifestWrapperInstruction;
use processors::{
    cancel_order::process_cancel_order, create_wrapper::process_create_wrapper,
    edit_order::process_edit_order, place_order::process_place_order,
    settle_funds::process_settle_funds,
};
use solana_program::{
    account_info::AccountInfo, declare_id, entrypoint::ProgramResult, program_error::ProgramError,
    pubkey::Pubkey,
};

#[cfg(not(feature = "no-entrypoint"))]
use solana_security_txt::security_txt;

#[cfg(not(feature = "no-entrypoint"))]
security_txt! {
    name: "manifest-ui-wrapper",
    project_url: "",
    contacts: "email:dev@manifest.trade",
    policy: "",
    preferred_languages: "en",
    source_code: "https://github.com/Bonasa-Tech/manifest",
    auditors: ""
}

declare_id!("UMnFStVeG1ecZFc2gc5K3vFy3sMpotq8C91mXBQDGwh");

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
        ManifestWrapperInstruction::ClaimSeatUnused => {
            unimplemented!("ClaimSeat has been removed and is handled on-demand in PlaceOrder")
        }
        ManifestWrapperInstruction::PlaceOrder => {
            process_place_order(program_id, accounts, data)?;
        }
        ManifestWrapperInstruction::EditOrder => {
            process_edit_order(program_id, accounts, data)?;
        }
        ManifestWrapperInstruction::CancelOrder => {
            process_cancel_order(program_id, accounts, data)?;
        }
        ManifestWrapperInstruction::SettleFunds => {
            process_settle_funds(program_id, accounts, data)?;
        }
    }

    Ok(())
}
