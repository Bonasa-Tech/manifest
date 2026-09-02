use hypertree::get_mut_helper;
use manifest::validation::next_account_info;
use manifest::validation::AccountInfoExt;
use pinocchio::ProgramResult;
use manifest::validation::{Program, Signer};
use pinocchio::account_info::AccountInfo;
use solana_program::{
    pubkey::Pubkey,
    system_program,
};

use crate::{loader::WrapperStateAccountInfo, wrapper_state::ManifestWrapperStateFixed};

use super::shared::expand_wrapper_if_needed;

pub(crate) fn process_create_wrapper(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    _data: &[u8],
) -> ProgramResult {
    let account_iter: &mut std::slice::Iter<AccountInfo> = &mut accounts.iter();
    let owner: Signer = Signer::new(next_account_info(account_iter)?)?;
    let system_program: Program =
        Program::new(next_account_info(account_iter)?, &system_program::id())?;
    let wrapper_state: WrapperStateAccountInfo =
        WrapperStateAccountInfo::new_init(next_account_info(account_iter)?)?;

    {
        // Initialize wrapper state
        let empty_wrapper_state_fixed: ManifestWrapperStateFixed =
            ManifestWrapperStateFixed::new_empty(owner.pubkey());
        let wrapper_bytes: &mut [u8] = &mut wrapper_state.try_borrow_mut_data()?[..];
        *get_mut_helper::<ManifestWrapperStateFixed>(wrapper_bytes, 0_u32) =
            empty_wrapper_state_fixed;

        // Drop the reference to wrapper_state so it can be borrowed in expand
        // wrapper if needed.
    }

    // Expand wrapper so there is an initial block available.
    expand_wrapper_if_needed(&wrapper_state, &owner, &system_program)?;

    Ok(())
}
