use hypertree::get_mut_helper;
use manifest::validation::{next_account_info, AccountInfoExt, Program, Signer};
use pinocchio::{account_info::AccountInfo, ProgramResult};
use solana_program::{pubkey::Pubkey, system_program};

use crate::wrapper_user::ManifestWrapperUserFixed;

use super::shared::{expand_wrapper_if_needed, WrapperStateAccountInfo};

pub(crate) fn process_create_wrapper(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    _data: &[u8],
) -> ProgramResult {
    // Load account infos
    let account_iter: &mut std::slice::Iter<AccountInfo> = &mut accounts.iter();
    let owner: &AccountInfo = next_account_info(account_iter)?;
    let system_program: Program =
        Program::new(next_account_info(account_iter)?, &system_program::id())?;
    let payer: Signer = Signer::new(next_account_info(account_iter)?)?;
    let wrapper_state: WrapperStateAccountInfo =
        WrapperStateAccountInfo::new_init(next_account_info(account_iter)?)?;

    // Initialize wrapper state.
    {
        let empty_market_fixed: ManifestWrapperUserFixed =
            ManifestWrapperUserFixed::new_empty(owner.pubkey());
        let market_bytes: &mut [u8] = &mut wrapper_state.try_borrow_mut_data()?[..];
        *get_mut_helper::<ManifestWrapperUserFixed>(market_bytes, 0_u32) = empty_market_fixed;
    }

    // Expand wrapper so there is an initial block available.
    expand_wrapper_if_needed(&wrapper_state, &payer, &system_program)?;

    Ok(())
}
