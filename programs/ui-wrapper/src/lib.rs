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
use pinocchio::account_info::AccountInfo;
use pinocchio::ProgramResult;
use pinocchio::program_error::ProgramError;
use instruction::ManifestWrapperInstruction;
use processors::{
    cancel_order::process_cancel_order, create_wrapper::process_create_wrapper,
    place_order::process_place_order, settle_funds::process_settle_funds,
};
use solana_program::{
    declare_id, pubkey::Pubkey,
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
pinocchio::program_entrypoint!(process_instruction, { manifest::entrypoint::MAX_ACCOUNTS });

pub fn process_instruction(
    program_id_raw: &pinocchio::pubkey::Pubkey,
    accounts: &[AccountInfo],
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
        ManifestWrapperInstruction::ClaimSeatUnused => {
            unimplemented!("ClaimSeat has been removed and is handled on-demand in PlaceOrder")
        }
        ManifestWrapperInstruction::PlaceOrder => {
            process_place_order(program_id, accounts, data)?;
        }
        ManifestWrapperInstruction::EditOrder => {
            unimplemented!("todo");
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

