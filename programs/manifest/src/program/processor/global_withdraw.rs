use crate::validation::{io_to_program_error, to_program_error, AccountViewExt};
use pinocchio::{account::RefMut, ProgramResult};

use borsh::{BorshDeserialize, BorshSerialize};
use pinocchio::account::AccountView;
use solana_program::pubkey::Pubkey;

use crate::{
    logs::{emit_stack, GlobalWithdrawLog},
    program::get_mut_dynamic_account,
    quantities::{GlobalAtoms, WrapperU64},
    state::GlobalRefMut,
    validation::{loaders::GlobalWithdrawContext, MintAccountInfo, TokenAccountInfo, TokenProgram},
};

#[cfg(not(feature = "certora"))]
use crate::{global_vault_seeds_with_bump, program::invoke_signed};

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
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let params: GlobalWithdrawParams =
        GlobalWithdrawParams::try_from_slice(data).map_err(io_to_program_error)?;
    process_global_withdraw_core(program_id, accounts, params)
}

#[cfg_attr(all(feature = "certora", not(feature = "certora-test")), early_panic)]
pub(crate) fn process_global_withdraw_core(
    _program_id: &Pubkey,
    accounts: &[AccountView],
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

    let global_data: &mut RefMut<[u8]> = &mut global.try_borrow_mut()?;
    let mut global_dynamic_account: GlobalRefMut = get_mut_dynamic_account(global_data);
    global_dynamic_account.withdraw_global(payer.pubkey(), GlobalAtoms::new(amount_atoms))?;

    // Sign with the bump stored at creation instead of searching for it.
    let bump: u8 = global_dynamic_account.fixed.get_vault_bump();

    // Do the token transfer
    if global_vault.owner_pubkey() == spl_token_2022::id() {
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
        global: *global.pubkey(),
        trader: *payer.pubkey(),
        global_atoms: GlobalAtoms::new(amount_atoms),
    })?;

    Ok(())
}

/** Transfer from global vault to trader using SPL Token **/
#[cfg(not(feature = "certora"))]
fn spl_token_transfer_from_global_vault_to_trader<'a>(
    token_program: &TokenProgram<'a>,
    mint: &MintAccountInfo<'a>,
    global_vault: &TokenAccountInfo<'a>,
    trader_token: &TokenAccountInfo<'a>,
    amount_atoms: u64,
    bump: u8,
) -> ProgramResult {
    invoke_signed(
        &spl_token::instruction::transfer(
            token_program.pubkey(),
            global_vault.pubkey(),
            trader_token.pubkey(),
            global_vault.pubkey(),
            &[],
            amount_atoms,
        )
        .map_err(to_program_error)?,
        // source, destination, authority: the vault signs for itself.
        &[
            global_vault.as_ref(),
            trader_token.as_ref(),
            global_vault.as_ref(),
        ],
        global_vault_seeds_with_bump!(mint.info.pubkey(), bump),
    )
}

#[cfg(feature = "certora")]
/** (Summary) Transfer from global vault to trader using SPL Token **/
fn spl_token_transfer_from_global_vault_to_trader<'a>(
    _token_program: &TokenProgram<'a>,
    _mint: &MintAccountInfo<'a>,
    global_vault: &TokenAccountInfo<'a>,
    trader_token: &TokenAccountInfo<'a>,
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
fn spl_token_2022_transfer_from_global_vault_to_trader<'a>(
    token_program: &TokenProgram<'a>,
    mint: &MintAccountInfo<'a>,
    global_vault: &TokenAccountInfo<'a>,
    trader_token: &TokenAccountInfo<'a>,
    amount_atoms: u64,
    bump: u8,
) -> ProgramResult {
    invoke_signed(
        &spl_token_2022::instruction::transfer_checked(
            token_program.pubkey(),
            global_vault.pubkey(),
            mint.info.pubkey(),
            trader_token.pubkey(),
            global_vault.pubkey(),
            &[],
            amount_atoms,
            mint.mint.decimals,
        )
        .map_err(to_program_error)?,
        // source, mint, destination, authority: the vault signs for itself.
        &[
            global_vault.as_ref(),
            mint.as_ref(),
            trader_token.as_ref(),
            global_vault.as_ref(),
        ],
        global_vault_seeds_with_bump!(mint.info.pubkey(), bump),
    )
}

#[cfg(feature = "certora")]
/** (Summary) Transfer from global vault to trader using SPL Token 2022 **/
fn spl_token_2022_transfer_from_global_vault_to_trader<'a>(
    _token_program: &TokenProgram<'a>,
    _mint: &MintAccountInfo<'a>,
    global_vault: &TokenAccountInfo<'a>,
    trader_token: &TokenAccountInfo<'a>,
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
