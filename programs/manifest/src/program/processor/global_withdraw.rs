use std::cell::RefMut;

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{account_info::AccountInfo, entrypoint::ProgramResult, pubkey::Pubkey};

use crate::{
    logs::{emit_stack, GlobalWithdrawLog},
    program::get_mut_dynamic_account,
    quantities::{GlobalAtoms, WrapperU64},
    state::GlobalRefMut,
    validation::{
        get_global_vault_address, loaders::GlobalWithdrawContext, MintAccountInfo,
        TokenAccountInfo, TokenProgram,
    },
};

#[cfg(not(feature = "certora"))]
use {crate::global_vault_seeds_with_bump, solana_program::program::invoke_signed};

#[cfg(feature = "certora")]
use {
    early_panic::early_panic,
    solana_cvt::token::{spl_token_2022_transfer, spl_token_transfer},
};

#[derive(BorshDeserialize, BorshSerialize)]
pub struct GlobalWithdrawParams {
    pub amount_atoms: u64,
    // No trader index hint because global account is small so there is not much
    // benefit from hinted indices, unlike the market which can get large. Also,
    // seats are not permanent like on a market due to eviction, so it is more
    // likely that a client could send a bad request. Just look it up for them.
}

impl GlobalWithdrawParams {
    pub fn new(amount_atoms: u64) -> Self {
        GlobalWithdrawParams { amount_atoms }
    }
}

pub(crate) fn process_global_withdraw(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let params: GlobalWithdrawParams = GlobalWithdrawParams::try_from_slice(data)?;
    process_global_withdraw_core(program_id, accounts, params)
}

#[cfg_attr(all(feature = "certora", not(feature = "certora-test")), early_panic)]
pub(crate) fn process_global_withdraw_core(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: GlobalWithdrawParams,
) -> ProgramResult {
    let global_withdraw_context: GlobalWithdrawContext = GlobalWithdrawContext::load(accounts)?;
    let GlobalWithdrawParams { amount_atoms } = params;

    let GlobalWithdrawContext {
        payer,
        global,
        mint,
        global_vault,
        trader_token,
        token_program,
    } = global_withdraw_context;

    let global_data: &mut RefMut<&mut [u8]> = &mut global.try_borrow_mut_data()?;
    let mut global_dynamic_account: GlobalRefMut = get_mut_dynamic_account(global_data);
    global_dynamic_account.withdraw_global(payer.key, GlobalAtoms::new(amount_atoms))?;

    let (_, bump) = get_global_vault_address(mint.info.key);

    // Do the token transfer
    if *global_vault.owner == spl_token_2022::id() {
        spl_token_2022_transfer_from_global_vault_to_trader(
            &token_program,
            &mint,
            &global_vault,
            &trader_token,
            amount_atoms,
            bump,
        )?;
    } else {
        spl_token_transfer_from_global_vault_to_trader(
            &token_program,
            &mint,
            &global_vault,
            &trader_token,
            amount_atoms,
            bump,
        )?;
    }

    emit_stack(GlobalWithdrawLog {
        global: *global.key,
        trader: *payer.key,
        global_atoms: GlobalAtoms::new(amount_atoms),
    })?;

    Ok(())
}

/** Transfer from global vault to trader using SPL Token **/
#[cfg(not(feature = "certora"))]
fn spl_token_transfer_from_global_vault_to_trader<'a, 'info>(
    token_program: &TokenProgram<'a, 'info>,
    mint: &MintAccountInfo<'a, 'info>,
    global_vault: &TokenAccountInfo<'a, 'info>,
    trader_token: &TokenAccountInfo<'a, 'info>,
    amount_atoms: u64,
    bump: u8,
) -> ProgramResult {
    invoke_signed(
        &spl_token::instruction::transfer(
            token_program.key,
            global_vault.key,
            trader_token.key,
            global_vault.key,
            &[],
            amount_atoms,
        )?,
        &[
            token_program.as_ref().clone(),
            global_vault.as_ref().clone(),
            trader_token.as_ref().clone(),
        ],
        global_vault_seeds_with_bump!(mint.info.key, bump),
    )
}

#[cfg(feature = "certora")]
/** (Summary) Transfer from global vault to trader using SPL Token **/
fn spl_token_transfer_from_global_vault_to_trader<'a, 'info>(
    _token_program: &TokenProgram<'a, 'info>,
    _mint: &MintAccountInfo<'a, 'info>,
    global_vault: &TokenAccountInfo<'a, 'info>,
    trader_token: &TokenAccountInfo<'a, 'info>,
    amount_atoms: u64,
    _bump: u8,
) -> ProgramResult {
    spl_token_transfer(
        global_vault.info,
        trader_token.info,
        global_vault.info,
        amount_atoms,
    )
}

/** Transfer from global vault to trader using SPL Token 2022 **/
#[cfg(not(feature = "certora"))]
fn spl_token_2022_transfer_from_global_vault_to_trader<'a, 'info>(
    token_program: &TokenProgram<'a, 'info>,
    mint: &MintAccountInfo<'a, 'info>,
    global_vault: &TokenAccountInfo<'a, 'info>,
    trader_token: &TokenAccountInfo<'a, 'info>,
    amount_atoms: u64,
    bump: u8,
) -> ProgramResult {
    invoke_signed(
        &spl_token_2022::instruction::transfer_checked(
            token_program.key,
            global_vault.key,
            mint.info.key,
            trader_token.key,
            global_vault.key,
            &[],
            amount_atoms,
            mint.mint.decimals,
        )?,
        &[
            token_program.as_ref().clone(),
            trader_token.as_ref().clone(),
            mint.as_ref().clone(),
            global_vault.as_ref().clone(),
        ],
        global_vault_seeds_with_bump!(mint.info.key, bump),
    )
}

#[cfg(feature = "certora")]
/** (Summary) Transfer from global vault to trader using SPL Token 2022 **/
fn spl_token_2022_transfer_from_global_vault_to_trader<'a, 'info>(
    _token_program: &TokenProgram<'a, 'info>,
    _mint: &MintAccountInfo<'a, 'info>,
    global_vault: &TokenAccountInfo<'a, 'info>,
    trader_token: &TokenAccountInfo<'a, 'info>,
    amount_atoms: u64,
    _bump: u8,
) -> ProgramResult {
    spl_token_2022_transfer(
        global_vault.info,
        trader_token.info,
        global_vault.info,
        amount_atoms,
    )
}
