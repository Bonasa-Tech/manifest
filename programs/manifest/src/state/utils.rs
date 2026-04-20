use std::cell::RefMut;

use crate::{
    global_vault_seeds_with_bump,
    logs::{emit_stack, GlobalCleanupLog},
    program::{batch_update::PlaceOrderParams, get_mut_dynamic_account, invoke},
    quantities::{GlobalAtoms, WrapperU64},
    require,
    validation::{loaders::GlobalTradeAccounts, MintAccountInfo, TokenAccountInfo, TokenProgram},
};
use hypertree::{DataIndex, NIL};
#[cfg(not(feature = "no-clock"))]
use solana_program::sysvar::Sysvar;
use solana_program::{
    entrypoint::ProgramResult, program::invoke_signed, program_error::ProgramError, pubkey::Pubkey,
};
use spl_token_2022::{
    extension::{
        transfer_fee::TransferFeeConfig, transfer_hook::TransferHook, BaseStateWithExtensions,
        StateWithExtensions,
    },
    state::Mint,
};

use super::{
    order_type_can_take, GlobalRefMut, OrderType, RestingOrder, GAS_DEPOSIT_LAMPORTS,
    NO_EXPIRATION_LAST_VALID_SLOT,
};

pub fn get_now_slot() -> u32 {
    // If we cannot get the clock (happens in tests, then only match with
    // orders without expiration). We assume that the clock cannot be
    // maliciously manipulated to clear all orders with expirations on the
    // orderbook.
    #[cfg(feature = "no-clock")]
    let now_slot: u64 = 0;
    #[cfg(not(feature = "no-clock"))]
    let now_slot: u64 = solana_program::clock::Clock::get()
        .unwrap_or(solana_program::clock::Clock {
            slot: u64::MAX,
            epoch_start_timestamp: i64::MAX,
            epoch: u64::MAX,
            leader_schedule_epoch: u64::MAX,
            unix_timestamp: i64::MAX,
        })
        .slot;
    now_slot as u32
}

pub(crate) fn get_now_epoch() -> u64 {
    #[cfg(feature = "no-clock")]
    let now_epoch: u64 = 0;
    #[cfg(not(feature = "no-clock"))]
    let now_epoch: u64 = solana_program::clock::Clock::get()
        .unwrap_or(solana_program::clock::Clock {
            slot: u64::MAX,
            epoch_start_timestamp: i64::MAX,
            epoch: u64::MAX,
            leader_schedule_epoch: u64::MAX,
            unix_timestamp: i64::MAX,
        })
        .epoch;
    now_epoch
}

#[inline(always)]
pub(crate) fn remove_from_global(
    global_trade_accounts_opt: &Option<GlobalTradeAccounts>,
) -> ProgramResult {
    if global_trade_accounts_opt.is_none() {
        // Payer is forfeiting the right to claim the gas prepayment. This
        // results in a stranded gas prepayment on the global account.
        return Ok(());
    }
    let global_trade_accounts: &GlobalTradeAccounts = &global_trade_accounts_opt.as_ref().unwrap();
    let GlobalTradeAccounts {
        global,
        gas_receiver_opt,
        ..
    } = global_trade_accounts;

    // The simple implementation gets
    //
    //     **receiver.lamports.borrow_mut() += GAS_DEPOSIT_LAMPORTS;
    //     **global.lamports.borrow_mut() -= GAS_DEPOSIT_LAMPORTS;
    //
    // failed: sum of account balances before and after instruction do not match
    //
    // doesnt make sense, but thats the solana runtime.
    //
    // Done here instead of inside the object because the borrow checker needs
    // to get the data on global which it cannot while there is a mut self
    // reference. Note that if it isnt claimed here, then nobody does and it is
    // lost to the global account.
    //
    // Then we tried to do a CPI, but that fails because
    //
    // `from` must not carry data
    //
    // if let Some(system_program) = &global_trade_accounts.system_program {
    //     solana_program::program::invoke_signed(
    //         &solana_program::system_instruction::transfer(
    //             &global.key,
    //             &trader.info.key,
    //             GAS_DEPOSIT_LAMPORTS,
    //         ),
    //         &[global.info.clone(), trader.info.clone(), system_program.info.clone()],
    //         global_seeds_with_bump!(mint, global_bump),
    //     )?;
    // }
    //
    // Somehow, a hybrid works. Dont know why, but it does.
    //
    if global_trade_accounts.system_program.is_some() {
        **global.lamports.borrow_mut() -= GAS_DEPOSIT_LAMPORTS;
        **gas_receiver_opt.as_ref().unwrap().lamports.borrow_mut() += GAS_DEPOSIT_LAMPORTS;
    }

    Ok(())
}

pub(crate) fn try_to_add_to_global(
    global_trade_accounts: &GlobalTradeAccounts,
    resting_order: &RestingOrder,
) -> ProgramResult {
    let GlobalTradeAccounts {
        global,
        gas_payer_opt,
        ..
    } = global_trade_accounts;

    let global_data: &mut RefMut<&mut [u8]> = &mut global.try_borrow_mut_data()?;
    let mut global_dynamic_account: GlobalRefMut = get_mut_dynamic_account(global_data);
    global_dynamic_account.add_order(resting_order, gas_payer_opt.as_ref().unwrap().key)
}

pub(crate) fn try_to_pay_all_global_gas_prepayment(
    orders: &Vec<PlaceOrderParams>,
    global_trade_accounts_opts: &[Option<GlobalTradeAccounts>; 2],
) -> ProgramResult {
    for (is_bid, account_idx) in [(true, 1), (false, 0)] {
        let global_order_count: usize = orders
            .iter()
            .filter(|o| o.order_type() == OrderType::Global && o.is_bid() == is_bid)
            .count();
        if global_order_count > 0 {
            pay_global_gas_prepayment(
                global_trade_accounts_opts[account_idx].as_ref().unwrap(),
                global_order_count as u64,
            )?;
        }
    }
    Ok(())
}

fn pay_global_gas_prepayment(
    global_trade_accounts: &GlobalTradeAccounts,
    num_gas_prepayments: u64,
) -> ProgramResult {
    let GlobalTradeAccounts {
        global,
        gas_payer_opt,
        ..
    } = global_trade_accounts;

    // Need to CPI because otherwise we get:
    //
    // instruction spent from the balance of an account it does not own
    //
    // Done here instead of inside the object because the borrow checker needs
    // to get the data on global which it cannot while there is a mut self
    // reference.
    invoke(
        &solana_program::system_instruction::transfer(
            &gas_payer_opt.as_ref().unwrap().info.key,
            &global.key,
            GAS_DEPOSIT_LAMPORTS
                .checked_mul(num_gas_prepayments)
                .unwrap(),
        ),
        &[
            gas_payer_opt.as_ref().unwrap().info.clone(),
            global.info.clone(),
        ],
    )?;

    Ok(())
}

pub(crate) fn assert_can_take(order_type: OrderType) -> ProgramResult {
    require!(
        order_type_can_take(order_type),
        crate::program::ManifestError::PostOnlyCrosses,
        "Post only order would cross",
    )?;
    Ok(())
}

pub(crate) fn assert_not_already_expired(last_valid_slot: u32, now_slot: u32) -> ProgramResult {
    require!(
        last_valid_slot == NO_EXPIRATION_LAST_VALID_SLOT || last_valid_slot > now_slot,
        crate::program::ManifestError::AlreadyExpired,
        "Placing an already expired order. now: {} last_valid: {}",
        now_slot,
        last_valid_slot
    )?;
    Ok(())
}

pub(crate) fn assert_already_has_seat(trader_index: DataIndex) -> ProgramResult {
    require!(
        trader_index != NIL,
        crate::program::ManifestError::AlreadyClaimedSeat,
        "Need to claim a seat first",
    )?;
    Ok(())
}

pub(crate) fn can_back_order<'a, 'info>(
    global_trade_accounts_opt: &'a Option<GlobalTradeAccounts<'a, 'info>>,
    resting_order_trader: &Pubkey,
    desired_global_atoms: GlobalAtoms,
) -> bool {
    if global_trade_accounts_opt.is_none() {
        return false;
    }
    let global_trade_accounts: &GlobalTradeAccounts = &global_trade_accounts_opt.as_ref().unwrap();
    let GlobalTradeAccounts { global, .. } = global_trade_accounts;

    let global_data: &mut RefMut<&mut [u8]> = &mut global.try_borrow_mut_data().unwrap();
    let global_dynamic_account: GlobalRefMut = get_mut_dynamic_account(global_data);

    let num_deposited_atoms: GlobalAtoms =
        global_dynamic_account.get_balance_atoms(resting_order_trader);
    return desired_global_atoms <= num_deposited_atoms;
}

/// Checks if a global order has sufficient balance and reduces the balance.
/// Does NOT perform the token transfer - caller should accumulate amounts
/// and call transfer_global_tokens after matching is complete.
///
/// Returns Ok(true) if balance was reduced successfully, Ok(false) if
/// insufficient balance or transfer would fail (fee/hook), Err on other errors.
pub(crate) fn try_to_reduce_global_tokens<'a, 'info>(
    global_trade_accounts_opt: &'a Option<GlobalTradeAccounts<'a, 'info>>,
    resting_order_trader: &Pubkey,
    desired_global_atoms: GlobalAtoms,
) -> Result<bool, ProgramError> {
    require!(
        global_trade_accounts_opt.is_some(),
        crate::program::ManifestError::MissingGlobal,
        "Missing global accounts when adding a global",
    )?;
    let global_trade_accounts: &GlobalTradeAccounts = &global_trade_accounts_opt.as_ref().unwrap();
    let GlobalTradeAccounts {
        global,
        mint_opt,
        gas_receiver_opt,
        token_program_opt,
        ..
    } = global_trade_accounts;

    let global_data: &mut RefMut<&mut [u8]> = &mut global.try_borrow_mut_data()?;
    let mut global_dynamic_account: GlobalRefMut = get_mut_dynamic_account(global_data);

    let num_deposited_atoms: GlobalAtoms =
        global_dynamic_account.get_balance_atoms(resting_order_trader);
    // Intentionally does not allow partial fills against a global order. The
    // reason for this is to punish global orders that are not backed. There is
    // no technical blocker for supporting partial fills against a global. It is
    // just because of the mechanism design where we want global to only be used
    // when needed, not just for all orders.
    if desired_global_atoms > num_deposited_atoms {
        emit_stack(GlobalCleanupLog {
            cleaner: *gas_receiver_opt.as_ref().unwrap().key,
            maker: *resting_order_trader,
            amount_desired: desired_global_atoms,
            amount_deposited: num_deposited_atoms,
        })?;
        return Ok(false);
    }

    let token_program: &TokenProgram<'a, 'info> = token_program_opt.as_ref().unwrap();

    // Check transfer fee/hook BEFORE reducing balance to avoid permanent
    // balance loss when the transfer is rejected.
    if *token_program.key == spl_token_2022::id() {
        require!(
            mint_opt.is_some(),
            crate::program::ManifestError::MissingGlobal,
            "Missing global mint",
        )?;

        // Prevent transfer from global to market vault if a token has a non-zero fee.
        let mint_account_info: &MintAccountInfo = &mint_opt.as_ref().unwrap();
        if StateWithExtensions::<Mint>::unpack(&mint_account_info.info.data.borrow())?
            .get_extension::<TransferFeeConfig>()
            .is_ok_and(|f| f.get_epoch_fee(get_now_epoch()).transfer_fee_basis_points != 0.into())
        {
            solana_program::msg!("Treating global order as unbacked because it has a transfer fee");
            return Ok(false);
        }
        if StateWithExtensions::<Mint>::unpack(&mint_account_info.info.data.borrow())?
            .get_extension::<TransferHook>()
            .is_ok_and(|f| f.program_id.0 != Pubkey::default())
        {
            solana_program::msg!(
                "Treating global order as unbacked because it has a transfer hook"
            );
            return Ok(false);
        }
    }

    // Reduce balance only after confirming the transfer can proceed.
    global_dynamic_account.reduce(resting_order_trader, desired_global_atoms)?;

    Ok(true)
}

/// Transfers tokens from global vault to market vault.
/// Should be called after matching is complete with the accumulated total.
pub(crate) fn transfer_global_tokens<'a, 'info>(
    global_trade_accounts_opt: &'a Option<GlobalTradeAccounts<'a, 'info>>,
    total_atoms: GlobalAtoms,
) -> Result<(), ProgramError> {
    if total_atoms.as_u64() == 0 {
        return Ok(());
    }

    require!(
        global_trade_accounts_opt.is_some(),
        crate::program::ManifestError::MissingGlobal,
        "Missing global accounts when transferring",
    )?;
    let global_trade_accounts: &GlobalTradeAccounts = &global_trade_accounts_opt.as_ref().unwrap();
    let GlobalTradeAccounts {
        global,
        mint_opt,
        global_vault_opt,
        market_vault_opt,
        token_program_opt,
        ..
    } = global_trade_accounts;

    let global_data: &mut RefMut<&mut [u8]> = &mut global.try_borrow_mut_data()?;
    let global_dynamic_account: GlobalRefMut = get_mut_dynamic_account(global_data);

    let mint_key: Pubkey = *global_dynamic_account.fixed.get_mint();
    let global_vault_bump: u8 = global_dynamic_account.fixed.get_vault_bump();

    let global_vault: &TokenAccountInfo<'a, 'info> = global_vault_opt.as_ref().unwrap();
    let market_vault: &TokenAccountInfo<'a, 'info> = market_vault_opt.as_ref().unwrap();
    let token_program: &TokenProgram<'a, 'info> = token_program_opt.as_ref().unwrap();

    if *token_program.key == spl_token_2022::id() {
        let mint_account_info: &MintAccountInfo = &mint_opt.as_ref().unwrap();
        invoke_signed(
            &spl_token_2022::instruction::transfer_checked(
                token_program.key,
                global_vault.key,
                mint_account_info.info.key,
                market_vault.key,
                global_vault.key,
                &[],
                total_atoms.as_u64(),
                mint_account_info.mint.decimals,
            )?,
            &[
                token_program.as_ref().clone(),
                global_vault.as_ref().clone(),
                mint_account_info.as_ref().clone(),
                market_vault.as_ref().clone(),
            ],
            global_vault_seeds_with_bump!(&mint_key, global_vault_bump),
        )?;
    } else {
        invoke_signed(
            &spl_token::instruction::transfer(
                token_program.key,
                global_vault.key,
                market_vault.key,
                global_vault.key,
                &[],
                total_atoms.as_u64(),
            )?,
            &[
                token_program.as_ref().clone(),
                global_vault.as_ref().clone(),
                market_vault.as_ref().clone(),
            ],
            global_vault_seeds_with_bump!(&mint_key, global_vault_bump),
        )?;
    }

    Ok(())
}
