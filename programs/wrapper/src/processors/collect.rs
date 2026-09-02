use manifest::validation::{next_account_info, AccountViewExt, Program, Signer};
use pinocchio::{
    account::{AccountView, RefMut},
    error::ProgramError,
    sysvars::Sysvar,
    ProgramResult,
};
use solana_program::{pubkey, pubkey::Pubkey, system_program};

use crate::loader::WrapperStateAccountInfo;

pub(crate) fn process_collect(
    _program_id: &Pubkey,
    accounts: &[AccountView],
    _data: &[u8],
) -> ProgramResult {
    let account_iter: &mut std::slice::Iter<AccountView> = &mut accounts.iter();
    let wrapper_state: WrapperStateAccountInfo =
        WrapperStateAccountInfo::new(next_account_info(account_iter)?)?;
    let _system_program: Program =
        Program::new(next_account_info(account_iter)?, &system_program::id())?;
    let collector: Signer = Signer::new(next_account_info(account_iter)?)?;

    let rent: pinocchio::sysvars::rent::Rent = pinocchio::sysvars::rent::Rent::get()?;
    let minimum_balance: u64 = rent.try_minimum_balance(wrapper_state.data_len())?;
    let current_balance: u64 = wrapper_state.lamports();

    let lamports_diff: u64 = current_balance.saturating_sub(minimum_balance);

    // Program deployer of the wrapper is allowed to collect the extra rent.
    #[cfg(not(feature = "test"))]
    const COLLECTOR: Pubkey = pubkey!("B6dmr2UAn2wgjdm3T4N1Vjd8oPYRRTguByW7AEngkeL6");
    #[cfg(feature = "test")]
    const COLLECTOR: Pubkey = pubkey!("2iXtA8oeZqUU5pofxK971TCEvFGfems2AcDRaZHKD2pQ");
    if *collector.pubkey() != COLLECTOR {
        return Err(ProgramError::InvalidArgument);
    }

    // The System Program cannot debit a data-bearing account that it does not
    // own. This program owns wrapper_state, so it must move excess lamports by
    // mutating both balances directly while preserving the rent exemption.
    wrapper_state.info.set_lamports(
        current_balance
            .checked_sub(lamports_diff)
            .ok_or(ProgramError::ArithmeticOverflow)?,
    );
    collector.info.set_lamports(
        collector
            .info
            .lamports()
            .checked_add(lamports_diff)
            .ok_or(ProgramError::ArithmeticOverflow)?,
    );

    Ok(())
}
