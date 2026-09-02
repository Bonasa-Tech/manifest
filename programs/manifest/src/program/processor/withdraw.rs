use pinocchio::account_info::RefMut;
use crate::validation::io_to_program_error;
use crate::validation::to_program_error;
use crate::validation::AccountInfoExt;
use pinocchio::ProgramResult;

use super::get_trader_index_with_hint;
use crate::{
    logs::{emit_stack, WithdrawLog},
    program::get_mut_dynamic_account,
    state::MarketRefMut,
    validation::{loaders::WithdrawContext, MintAccountInfo, TokenAccountInfo, TokenProgram},
};
use borsh::{BorshDeserialize, BorshSerialize};
use hypertree::DataIndex;
use pinocchio::account_info::AccountInfo;
use solana_program::pubkey::Pubkey;

#[cfg(not(feature = "certora"))]
use crate::{market_vault_seeds_with_bump, program::invoke_signed};

#[cfg(feature = "certora")]
use {
    early_panic::early_panic,
    solana_cvt::token::{spl_token_2022_transfer, spl_token_transfer},
};

#[derive(BorshDeserialize, BorshSerialize)]
pub struct WithdrawParams {
    pub amount_atoms: u64,
    pub trader_index_hint: Option<DataIndex>,
}

impl WithdrawParams {
    pub fn new(amount_atoms: u64, trader_index_hint: Option<DataIndex>) -> Self {
        WithdrawParams {
            amount_atoms,
            trader_index_hint,
        }
    }
}

pub(crate) fn process_withdraw(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let params = WithdrawParams::try_from_slice(data).map_err(io_to_program_error)?;
    process_withdraw_core(program_id, accounts, params)
}

#[cfg_attr(all(feature = "certora", not(feature = "certora-test")), early_panic)]
pub(crate) fn process_withdraw_core(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: WithdrawParams,
) -> ProgramResult {
    let withdraw_context: WithdrawContext = WithdrawContext::load(accounts)?;
    let WithdrawParams {
        amount_atoms,
        trader_index_hint,
    } = params;

    let WithdrawContext {
        market,
        payer,
        trader_token,
        vault,
        token_program,
        mint,
    } = withdraw_context;

    let market_data: &mut RefMut<[u8]> = &mut market.try_borrow_mut_data()?;
    let mut dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);

    // Validation verifies that the mint is either base or quote.
    let is_base: bool =
        &trader_token.try_borrow_data()?[0..32] == dynamic_account.get_base_mint().as_ref();

    let mint_key: &Pubkey = if is_base {
        dynamic_account.get_base_mint()
    } else {
        dynamic_account.get_quote_mint()
    };

    let bump: u8 = if is_base {
        dynamic_account.fixed.get_base_vault_bump()
    } else {
        dynamic_account.fixed.get_quote_vault_bump()
    };

    if *vault.owner_pubkey() == spl_token_2022::id() {
        spl_token_2022_transfer_from_vault_to_trader_fixed(
            &token_program,
            Some(mint),
            mint_key,
            &vault,
            &trader_token,
            amount_atoms,
            if is_base {
                dynamic_account.fixed.get_base_mint_decimals()
            } else {
                dynamic_account.fixed.get_quote_mint_decimals()
            },
            market.pubkey(),
            bump,
        )?;
    } else {
        spl_token_transfer_from_vault_to_trader(
            &token_program,
            &vault,
            &trader_token,
            amount_atoms,
            market.pubkey(),
            bump,
            mint_key,
        )?;
    }

    let trader_index: DataIndex =
        get_trader_index_with_hint(trader_index_hint, &dynamic_account, &payer)?;
    dynamic_account.withdraw(trader_index, amount_atoms, is_base)?;

    emit_stack(WithdrawLog {
        market: *market.pubkey(),
        trader: *payer.pubkey(),
        mint: if is_base {
            *dynamic_account.get_base_mint()
        } else {
            *dynamic_account.get_quote_mint()
        },
        amount_atoms,
    })?;

    Ok(())
}

/** Transfer from base (quote) vault to base (quote) trader using SPL Token **/
#[cfg(not(feature = "certora"))]
fn spl_token_transfer_from_vault_to_trader<'a>(
    token_program: &TokenProgram<'a>,
    vault: &TokenAccountInfo<'a>,
    trader_account: &TokenAccountInfo<'a>,
    amount: u64,
    market_key: &Pubkey,
    vault_bump: u8,
    mint_pubkey: &Pubkey,
) -> ProgramResult {
    invoke_signed(
        &spl_token::instruction::transfer(
            token_program.pubkey(),
            vault.pubkey(),
            trader_account.pubkey(),
            vault.pubkey(),
            &[],
            amount,
        ).map_err(to_program_error)?,
        &[
            token_program.as_ref(),
            vault.as_ref(),
            trader_account.as_ref(),
        ],
        market_vault_seeds_with_bump!(market_key, mint_pubkey, vault_bump),
    )
}

#[cfg(feature = "certora")]
/** (Summary) Transfer from base (quote) vault to base (quote) trader using SPL Token **/
fn spl_token_transfer_from_vault_to_trader<'a>(
    _token_program: &TokenProgram<'a>,
    vault: &TokenAccountInfo<'a>,
    trader_account: &TokenAccountInfo<'a>,
    amount: u64,
    _market_key: &Pubkey,
    _vault_bump: u8,
    _mint_pubkey: &Pubkey,
) -> ProgramResult {
    spl_token_transfer(vault.info, trader_account.info, vault.info, amount)
}

/** Transfer from base (quote) vault to base (quote) trader using SPL Token 2022 **/
#[cfg(not(feature = "certora"))]
fn spl_token_2022_transfer_from_vault_to_trader_fixed<'a>(
    token_program: &TokenProgram<'a>,
    mint: Option<MintAccountInfo<'a>>,
    mint_key: &Pubkey,
    vault: &TokenAccountInfo<'a>,
    trader_token: &TokenAccountInfo<'a>,
    amount_atoms: u64,
    decimals: u8,
    market_key: &Pubkey,
    bump: u8,
) -> ProgramResult {
    invoke_signed(
        &spl_token_2022::instruction::transfer_checked(
            token_program.pubkey(),
            vault.pubkey(),
            mint_key,
            trader_token.pubkey(),
            vault.pubkey(),
            &[],
            amount_atoms,
            decimals,
        ).map_err(to_program_error)?,
        &[
            token_program.as_ref(),
            vault.as_ref(),
            mint.unwrap().as_ref().clone(),
            trader_token.as_ref(),
        ],
        market_vault_seeds_with_bump!(market_key, mint_key, bump),
    )
}

// TODO: Share these with swap and deposit.
#[cfg(feature = "certora")]
/** (Summary) Transfer from base (quote) vault to base (quote) trader using SPL Token 2022 **/
fn spl_token_2022_transfer_from_vault_to_trader_fixed<'a>(
    _token_program: &TokenProgram<'a>,
    _mint: Option<MintAccountInfo<'a>>,
    _mint_key: &Pubkey,
    vault: &TokenAccountInfo<'a>,
    trader_token: &TokenAccountInfo<'a>,
    amount_atoms: u64,
    _decimals: u8,
    _market_key: &Pubkey,
    _bump: u8,
) -> ProgramResult {
    spl_token_2022_transfer(vault.info, trader_token.info, vault.info, amount_atoms)
}
