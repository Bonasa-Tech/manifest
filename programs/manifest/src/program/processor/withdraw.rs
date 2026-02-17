use std::cell::RefMut;

use super::get_trader_index_with_hint;
use crate::{
    logs::{emit_stack, WithdrawLog},
    program::get_mut_dynamic_account,
    state::MarketRefMut,
    validation::{
        loaders::WithdrawContext,
        TokenAccountInfo, TokenProgram,
    },
};
use borsh::{BorshDeserialize, BorshSerialize};
use hypertree::DataIndex;
use solana_program::{account_info::AccountInfo, entrypoint::ProgramResult, pubkey::Pubkey};

#[cfg(not(feature = "certora"))]
use {crate::market_vault_seeds_with_bump, solana_program::program::invoke_signed};

#[cfg(feature = "certora")]
use {
    early_panic::early_panic,
    solana_cvt::token::spl_token_transfer,
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
    let params = WithdrawParams::try_from_slice(data)?;
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
        mint: _,
    } = withdraw_context;

    let market_data: &mut RefMut<&mut [u8]> = &mut market.try_borrow_mut_data()?;
    let mut dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);

    // Perps: only quote (USDC) withdrawals allowed — base is virtual.
    // The loader already validates that the trader_token is for the quote mint.

    let mint_key: &Pubkey = dynamic_account.get_quote_mint();

    // Derive vault bump on-the-fly for PDA signing
    let (_, bump) = crate::validation::get_vault_address(market.key, mint_key);

    // CPI to token program (ephemeral-spl-token on ER)
    spl_token_transfer_from_vault_to_trader(
        &token_program,
        &vault,
        &trader_token,
        amount_atoms,
        market.key,
        bump,
        mint_key,
    )?;

    let trader_index: DataIndex =
        get_trader_index_with_hint(trader_index_hint, &dynamic_account, &payer)?;

    // Lazy funding settlement before withdrawal + equity check.
    // This ensures margin reflects accumulated funding accurately.
    dynamic_account.settle_funding_for_trader(trader_index)?;

    // is_base = false: always withdrawing quote in perps
    dynamic_account.withdraw(trader_index, amount_atoms, false)?;

    // Verify remaining margin covers maintenance requirement
    {
        use crate::quantities::{BaseAtoms, WrapperU64};
        use crate::state::claimed_seat::ClaimedSeat;
        use hypertree::{get_helper, RBNode};

        let claimed_seat: &ClaimedSeat = get_helper::<RBNode<ClaimedSeat>>(
            &dynamic_account.dynamic,
            trader_index,
        )
        .get_value();

        let position_size: i64 = claimed_seat.get_position_size();
        if position_size != 0 {
            let abs_position: u64 = position_size.unsigned_abs();
            let mark_price =
                super::liquidate::compute_mark_price(&dynamic_account)?;
            let current_value: u64 = mark_price
                .checked_quote_for_base(BaseAtoms::new(abs_position), false)?
                .as_u64();

            let quote_cost_basis: u64 = claimed_seat.get_quote_cost_basis();
            // Use i128 to avoid overflow on large u64 values cast to i64
            let unrealized_pnl: i128 = if position_size > 0 {
                (current_value as i128) - (quote_cost_basis as i128)
            } else {
                (quote_cost_basis as i128) - (current_value as i128)
            };

            let remaining_margin: u64 = claimed_seat.quote_withdrawable_balance.as_u64();
            let equity: i128 = (remaining_margin as i128) + unrealized_pnl;

            let maintenance_margin_bps: u64 =
                dynamic_account.fixed.get_maintenance_margin_bps();
            let required_maintenance: u64 = current_value
                .checked_mul(maintenance_margin_bps)
                .unwrap_or(u64::MAX)
                / 10000;

            crate::require!(
                equity >= required_maintenance as i128,
                crate::program::ManifestError::InsufficientMargin,
                "Withdrawal would bring equity {} below maintenance margin {}",
                equity,
                required_maintenance,
            )?;
        }
    }

    // Store current global cumulative funding checkpoint.
    dynamic_account.store_cumulative_for_trader(trader_index);

    emit_stack(WithdrawLog {
        market: *market.key,
        trader: *payer.key,
        mint: *dynamic_account.get_quote_mint(),
        amount_atoms,
    })?;

    Ok(())
}

/** Transfer from base (quote) vault to base (quote) trader using SPL Token **/
#[cfg(not(feature = "certora"))]
fn spl_token_transfer_from_vault_to_trader<'a, 'info>(
    token_program: &TokenProgram<'a, 'info>,
    vault: &TokenAccountInfo<'a, 'info>,
    trader_account: &TokenAccountInfo<'a, 'info>,
    amount: u64,
    market_key: &Pubkey,
    vault_bump: u8,
    mint_pubkey: &Pubkey,
) -> ProgramResult {
    invoke_signed(
        &spl_token::instruction::transfer(
            token_program.key,
            vault.key,
            trader_account.key,
            vault.key,
            &[],
            amount,
        )?,
        &[
            token_program.as_ref().clone(),
            vault.as_ref().clone(),
            trader_account.as_ref().clone(),
        ],
        market_vault_seeds_with_bump!(market_key, mint_pubkey, vault_bump),
    )
}

#[cfg(feature = "certora")]
/** (Summary) Transfer from base (quote) vault to base (quote) trader using SPL Token **/
fn spl_token_transfer_from_vault_to_trader<'a, 'info>(
    _token_program: &TokenProgram<'a, 'info>,
    vault: &TokenAccountInfo<'a, 'info>,
    trader_account: &TokenAccountInfo<'a, 'info>,
    amount: u64,
    _market_key: &Pubkey,
    _vault_bump: u8,
    _mint_pubkey: &Pubkey,
) -> ProgramResult {
    spl_token_transfer(vault.info, trader_account.info, vault.info, amount)
}

