use crate::validation::AccountViewExt;
use pinocchio::{account::RefMut, error::ProgramError, ProgramResult};
// Only the certora build names the type; the rest reaches it through the
// account wrappers.
#[cfg(feature = "certora")]
use pinocchio::account::AccountView;

#[cfg(not(feature = "certora"))]
use crate::program::invoke_signed;
#[cfg(not(feature = "certora"))]
use crate::{
    global_vault_seeds_with_bump,
    program::invoke,
    validation::{to_program_error, MintAccountInfo, TokenProgram},
};
use crate::{
    logs::{emit_stack, GlobalCleanupLog},
    program::{batch_update::PlaceOrderParams, get_mut_dynamic_account},
    quantities::{GlobalAtoms, WrapperU64},
    require,
    validation::{loaders::GlobalTradeAccounts, TokenAccountInfo},
};
use hypertree::{DataIndex, NIL};
#[cfg(not(feature = "no-clock"))]
use pinocchio::sysvars::Sysvar;
use solana_program::pubkey::Pubkey;
#[cfg(not(feature = "certora"))]
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

// Most distinct trader addresses differ in their first eight bytes. A
// big-endian word comparison preserves Pubkey's lexicographic byte order;
// matching prefixes still use the full comparison, including equality.
#[inline(always)]
pub(super) fn compare_trader_keys(left: &Pubkey, right: &Pubkey) -> std::cmp::Ordering {
    let lhs = u64::from_be_bytes(left.to_bytes()[..8].try_into().unwrap());
    let rhs = u64::from_be_bytes(right.to_bytes()[..8].try_into().unwrap());
    match lhs.cmp(&rhs) {
        std::cmp::Ordering::Equal => left.cmp(right),
        order => order,
    }
}

pub fn get_now_slot() -> u32 {
    // If we cannot get the clock (happens in tests, then only match with
    // orders without expiration). We assume that the clock cannot be
    // maliciously manipulated to clear all orders with expirations on the
    // orderbook.
    #[cfg(feature = "no-clock")]
    let now_slot: u64 = 0;
    #[cfg(not(feature = "no-clock"))]
    let now_slot: u64 = pinocchio::sysvars::clock::Clock::get()
        .unwrap_or(pinocchio::sysvars::clock::Clock {
            slot: u64::MAX,
            epoch_start_timestamp: i64::MAX,
            epoch: u64::MAX,
            leader_schedule_epoch: u64::MAX,
            unix_timestamp: i64::MAX,
        })
        .slot;
    now_slot as u32
}

#[cfg(not(feature = "certora"))]
pub(crate) fn get_now_epoch() -> u64 {
    #[cfg(feature = "no-clock")]
    let now_epoch: u64 = 0;
    #[cfg(not(feature = "no-clock"))]
    let now_epoch: u64 = pinocchio::sysvars::clock::Clock::get()
        .unwrap_or(pinocchio::sysvars::clock::Clock {
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
        // Expected behavior: omitting the refund-capable global account bundle
        // forfeits the payer's right to the cleanup gas prepayment. Removing
        // the order still succeeds and strands the prepayment in the global
        // account; callers that require a refund must supply the full bundle.
        return Ok(());
    }
    let global_trade_accounts: &GlobalTradeAccounts = global_trade_accounts_opt.as_ref().unwrap();

    // The refund cannot be a CPI because the global account carries data
    // (`from` must not carry data), so it has to be a direct lamport
    // manipulation. Direct lamport manipulation is only synced back to the
    // runtime for accounts included in a later CPI, so if we moved the
    // lamports here and a later CPI in the same instruction included only one
    // of the two accounts (e.g. the gas prepayment transfer for the other
    // side's global, or a market expand), the runtime lamport sum check fails
    // with
    //
    // failed: sum of account balances before and after instruction do not match
    //
    // Instead, just count the refund here and move the lamports in
    // settle_global_gas_refunds after the last CPI of the instruction.
    if global_trade_accounts.system_program.is_some() {
        let num_deferred_gas_refunds: &std::cell::Cell<u64> =
            &global_trade_accounts.num_deferred_gas_refunds;
        num_deferred_gas_refunds.set(num_deferred_gas_refunds.get().checked_add(1).unwrap());
    }

    Ok(())
}

/// Pays out the gas prepayment refunds accumulated by remove_from_global.
/// Must be called after the last CPI in the instruction because the direct
/// lamport manipulation is not synced into a later CPI unless that CPI
/// includes both accounts, which would fail the runtime lamport sum check.
pub(crate) fn settle_global_gas_refunds(
    global_trade_accounts_opts: &[Option<GlobalTradeAccounts>; 2],
) -> ProgramResult {
    for global_trade_accounts_opt in global_trade_accounts_opts.iter() {
        if global_trade_accounts_opt.is_none() {
            continue;
        }
        let global_trade_accounts: &GlobalTradeAccounts =
            global_trade_accounts_opt.as_ref().unwrap();
        let num_refunds: u64 = global_trade_accounts.num_deferred_gas_refunds.take();
        if num_refunds == 0 {
            continue;
        }
        let GlobalTradeAccounts {
            global,
            gas_receiver_opt,
            ..
        } = global_trade_accounts;

        // The simple implementation gets
        //
        //     receiver.set_lamports(receiver.lamports() + GAS_DEPOSIT_LAMPORTS);
        //     global.set_lamports(global.lamports() - GAS_DEPOSIT_LAMPORTS);
        //
        // failed: sum of account balances before and after instruction do not match
        //
        // when a CPI that includes only one of the two accounts follows in the
        // same instruction, because the runtime re-checks the lamport sum at
        // every CPI boundary and only syncs direct lamport manipulation for
        // accounts passed into the CPI. Thats why this is deferred until after
        // the last CPI of the instruction.
        //
        // Done here instead of inside the object because the borrow checker
        // needs to get the data on global which it cannot while there is a mut
        // self reference. Note that if it isnt claimed here, then nobody does
        // and it is lost to the global account.
        //
        // We also tried to do a CPI, but that fails because
        //
        // `from` must not carry data
        //
        // if let Some(system_program) = &global_trade_accounts.system_program {
        //     crate::program::invoke_signed(
        //         &solana_system_interface::instruction::transfer(
        //             &global.pubkey(),
        //             &trader.info.pubkey(),
        //             GAS_DEPOSIT_LAMPORTS,
        //         ),
        //         &[global.info, trader.info, system_program.info],
        //         global_seeds_with_bump!(mint, global_bump),
        //     )?;
        // }
        let refund_lamports: u64 = GAS_DEPOSIT_LAMPORTS.checked_mul(num_refunds).unwrap();
        // Both accounts belong to this program here, so the lamports move by
        // writing the balances rather than asking the system program.
        global.set_lamports(global.lamports() - refund_lamports);
        {
            let receiver = gas_receiver_opt.as_ref().unwrap();
            receiver.set_lamports(receiver.lamports() + refund_lamports);
        }
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

    let global_data: &mut RefMut<[u8]> = &mut global.try_borrow_mut()?;
    let mut global_dynamic_account: GlobalRefMut = get_mut_dynamic_account(global_data);
    global_dynamic_account.add_order(resting_order, gas_payer_opt.as_ref().unwrap().pubkey())
}

// Takes a slice so both `Vec` (production) and `NoResizableVec` (certora, via
// Deref) can be passed.
pub(crate) fn try_to_pay_all_global_gas_prepayment(
    orders: &[PlaceOrderParams],
    global_trade_accounts_opts: &[Option<GlobalTradeAccounts>; 2],
) -> ProgramResult {
    if orders.is_empty() {
        return Ok(());
    }
    let mut global_order_counts = [0usize; 2];
    for order in orders {
        if order.order_type() == OrderType::Global {
            global_order_counts[usize::from(order.is_bid())] += 1;
        }
    }
    // Keep the existing quote-global, then base-global payment order.
    for account_idx in [1, 0] {
        let global_order_count = global_order_counts[account_idx];
        if global_order_count > 0 {
            pay_global_gas_prepayment(
                global_trade_accounts_opts[account_idx]
                    .as_ref()
                    .ok_or(crate::program::ManifestError::MissingGlobal)?,
                global_order_count as u64,
            )?;
        }
    }
    Ok(())
}

#[cfg(not(feature = "certora"))]
pub(crate) fn pay_global_gas_prepayment(
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
        &solana_system_interface::instruction::transfer(
            gas_payer_opt.as_ref().unwrap().info.pubkey(),
            &global.pubkey(),
            GAS_DEPOSIT_LAMPORTS
                .checked_mul(num_gas_prepayments)
                .unwrap(),
        ),
        &[gas_payer_opt.as_ref().unwrap().info, global.info],
    )?;

    Ok(())
}

/// (Summary) The system-program transfer of the gas prepayment, modeled as a
/// direct lamport move so the prover can track it. The system program fails a
/// transfer that the payer cannot cover, hence the assume.
#[cfg(feature = "certora")]
pub(crate) fn pay_global_gas_prepayment(
    global_trade_accounts: &GlobalTradeAccounts,
    num_gas_prepayments: u64,
) -> ProgramResult {
    let GlobalTradeAccounts {
        global,
        gas_payer_opt,
        ..
    } = global_trade_accounts;
    let payer_info: &AccountView = gas_payer_opt.as_ref().unwrap().info;

    let lamports: u64 = GAS_DEPOSIT_LAMPORTS
        .checked_mul(num_gas_prepayments)
        .unwrap();
    cvt::cvt_assume!(payer_info.lamports() >= lamports);
    cvt::cvt_assume!(global.lamports() <= u64::MAX - lamports);
    payer_info.set_lamports(payer_info.lamports() - lamports);
    global.set_lamports(global.lamports() + lamports);

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

pub(crate) fn can_back_order<'a>(
    global_trade_accounts_opt: &'a Option<GlobalTradeAccounts<'a>>,
    resting_order_trader: &Pubkey,
    desired_global_atoms: GlobalAtoms,
) -> bool {
    if global_trade_accounts_opt.is_none() {
        return false;
    }
    let global_trade_accounts: &GlobalTradeAccounts = global_trade_accounts_opt.as_ref().unwrap();
    let GlobalTradeAccounts { global, .. } = global_trade_accounts;

    let global_data: &mut RefMut<[u8]> = &mut global.try_borrow_mut().unwrap();
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
/// missing deposit, insufficient balance, or transfer would fail (fee/hook),
/// Err on other errors.
pub(crate) fn try_to_reduce_global_tokens<'a>(
    global_trade_accounts_opt: &'a Option<GlobalTradeAccounts<'a>>,
    resting_order_trader: &Pubkey,
    desired_global_atoms: GlobalAtoms,
) -> Result<bool, ProgramError> {
    require!(
        global_trade_accounts_opt.is_some(),
        crate::program::ManifestError::MissingGlobal,
        "Missing global accounts when adding a global",
    )?;
    let global_trade_accounts: &GlobalTradeAccounts = global_trade_accounts_opt.as_ref().unwrap();
    let GlobalTradeAccounts {
        global,
        gas_receiver_opt,
        ..
    } = global_trade_accounts;
    #[cfg(not(feature = "certora"))]
    let GlobalTradeAccounts {
        mint_opt,
        token_program_opt,
        ..
    } = global_trade_accounts;

    let global_data: &mut RefMut<[u8]> = &mut global.try_borrow_mut()?;
    let mut global_dynamic_account: GlobalRefMut = get_mut_dynamic_account(global_data);

    let (num_deposited_atoms, deposit_index) =
        global_dynamic_account.get_balance_atoms_with_index(resting_order_trader);
    // Cleanup is advisory logging; a non-signer SwapV2 has no gas receiver.
    // Never let that optional account turn an unbacked maker into a panic.
    let cleaner: Pubkey = gas_receiver_opt
        .as_ref()
        .map(|receiver| *receiver.pubkey())
        .unwrap_or(*resting_order_trader);
    // Intentionally does not allow partial fills against a global order. The
    // reason for this is to punish global orders that are not backed. There is
    // no technical blocker for supporting partial fills against a global. It is
    // just because of the mechanism design where we want global to only be used
    // when needed, not just for all orders.
    // An evicted maker has no deposit, even when rounding makes the required
    // amount zero. Treat the order as unbacked instead of trying to reduce NIL.
    if deposit_index == NIL || desired_global_atoms > num_deposited_atoms {
        emit_stack(GlobalCleanupLog {
            cleaner,
            maker: *resting_order_trader,
            amount_desired: desired_global_atoms,
            amount_deposited: num_deposited_atoms,
        })?;
        return Ok(false);
    }

    // (Summary) Whether the mint carries a token-2022 transfer fee or transfer
    // hook depends on mint state the prover does not model, so the decision
    // "treat this global order as unbacked" is a nondeterministic choice. This
    // over-approximates the production branches below: both return Ok(false)
    // at exactly this point, before the balance is reduced, which is the
    // property that matters (a rejected transfer must not eat the deposit).
    #[cfg(feature = "certora")]
    if ::nondet::nondet::<bool>() {
        return Ok(false);
    }

    #[cfg(not(feature = "certora"))]
    let token_program: &TokenProgram<'a> = token_program_opt.as_ref().unwrap();

    // Check transfer fee/hook BEFORE reducing balance to avoid permanent
    // balance loss when the transfer is rejected.
    #[cfg(not(feature = "certora"))]
    if *token_program.pubkey() == spl_token_2022::id() {
        require!(
            mint_opt.is_some(),
            crate::program::ManifestError::MissingGlobal,
            "Missing global mint",
        )?;

        // Prevent transfer from global to market vault if a token has a non-zero fee.
        let mint_account_info: &MintAccountInfo = mint_opt.as_ref().unwrap();
        let mint_data = mint_account_info.info.try_borrow()?;
        let mint = StateWithExtensions::<Mint>::unpack(&mint_data).map_err(to_program_error)?;
        if mint
            .get_extension::<TransferFeeConfig>()
            .is_ok_and(|f| f.get_epoch_fee(get_now_epoch()).transfer_fee_basis_points != 0.into())
        {
            solana_program::msg!("Treating global order as unbacked because it has a transfer fee");
            emit_stack(GlobalCleanupLog {
                cleaner,
                maker: *resting_order_trader,
                amount_desired: desired_global_atoms,
                amount_deposited: num_deposited_atoms,
            })?;
            return Ok(false);
        }
        if mint
            .get_extension::<TransferHook>()
            .is_ok_and(|f| f.program_id.get().is_some())
        {
            solana_program::msg!(
                "Treating global order as unbacked because it has a transfer hook"
            );
            emit_stack(GlobalCleanupLog {
                cleaner,
                maker: *resting_order_trader,
                amount_desired: desired_global_atoms,
                amount_deposited: num_deposited_atoms,
            })?;
            return Ok(false);
        }
    }

    // Reduce balance only after confirming the transfer can proceed.
    // The mutable global borrow is held throughout the checks above. They
    // only read the mint and emit logs, so the resolved deposit has not moved.
    global_dynamic_account.reduce_at_deposit_index(
        resting_order_trader,
        desired_global_atoms,
        deposit_index,
    )?;

    Ok(true)
}

/// (Summary) Transfers tokens from global vault to market vault.
///
/// The prover models SPL transfers as a direct move of the token account
/// amounts, which is what makes the global vault visible to the funds
/// invariants.
#[cfg(feature = "certora")]
pub(crate) fn transfer_global_tokens<'a>(
    global_trade_accounts_opt: &'a Option<GlobalTradeAccounts<'a>>,
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
    let global_trade_accounts: &GlobalTradeAccounts = global_trade_accounts_opt.as_ref().unwrap();
    let GlobalTradeAccounts {
        global_vault_opt,
        market_vault_opt,
        ..
    } = global_trade_accounts;

    let global_vault: &TokenAccountInfo<'a> = global_vault_opt.as_ref().unwrap();
    let market_vault: &TokenAccountInfo<'a> = market_vault_opt.as_ref().unwrap();

    solana_cvt::token::spl_token_transfer(
        global_vault.info,
        market_vault.info,
        global_vault.info,
        total_atoms.as_u64(),
    )
}

/// Transfers tokens from global vault to market vault.
/// Should be called after matching is complete with the accumulated total.
#[cfg(not(feature = "certora"))]
pub(crate) fn transfer_global_tokens<'a>(
    global_trade_accounts_opt: &'a Option<GlobalTradeAccounts<'a>>,
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
    let global_trade_accounts: &GlobalTradeAccounts = global_trade_accounts_opt.as_ref().unwrap();
    let GlobalTradeAccounts {
        global,
        mint_opt,
        global_vault_opt,
        market_vault_opt,
        token_program_opt,
        ..
    } = global_trade_accounts;

    let global_data: &mut RefMut<[u8]> = &mut global.try_borrow_mut()?;
    let global_dynamic_account: GlobalRefMut = get_mut_dynamic_account(global_data);

    let mint_key: Pubkey = *global_dynamic_account.fixed.get_mint();
    let global_vault_bump: u8 = global_dynamic_account.fixed.get_vault_bump();

    let global_vault: &TokenAccountInfo<'a> = global_vault_opt.as_ref().unwrap();
    let market_vault: &TokenAccountInfo<'a> = market_vault_opt.as_ref().unwrap();
    let token_program: &TokenProgram<'a> = token_program_opt.as_ref().unwrap();

    if *token_program.pubkey() == spl_token_2022::id() {
        let mint_account_info: &MintAccountInfo = mint_opt.as_ref().unwrap();
        invoke_signed(
            &spl_token_2022::instruction::transfer_checked(
                token_program.pubkey(),
                global_vault.pubkey(),
                mint_account_info.info.pubkey(),
                market_vault.pubkey(),
                global_vault.pubkey(),
                &[],
                total_atoms.as_u64(),
                mint_account_info.mint.decimals,
            )
            .map_err(to_program_error)?,
            // source, mint, destination, authority: the vault signs for itself.
            &[
                global_vault.as_ref(),
                mint_account_info.as_ref(),
                market_vault.as_ref(),
            ],
            global_vault_seeds_with_bump!(&mint_key, global_vault_bump),
        )?;
    } else {
        invoke_signed(
            &spl_token::instruction::transfer(
                token_program.pubkey(),
                global_vault.pubkey(),
                market_vault.pubkey(),
                global_vault.pubkey(),
                &[],
                total_atoms.as_u64(),
            )
            .map_err(to_program_error)?,
            // source, destination, authority: the vault signs for itself.
            &[global_vault.as_ref(), market_vault.as_ref()],
            global_vault_seeds_with_bump!(&mint_key, global_vault_bump),
        )?;
    }

    Ok(())
}

#[cfg(all(test, not(feature = "certora")))]
mod tests {
    use super::*;
    use crate::{
        state::{GlobalFixed, GLOBAL_BLOCK_SIZE, GLOBAL_FIXED_SIZE},
        validation::{ManifestAccountInfo, OwnedAccount},
    };
    use std::cell::Cell;

    #[test]
    fn global_reduction_requires_a_seat_even_for_zero_atoms() {
        for (has_seat, desired_atoms, expected_backed) in [
            (false, 0, false),
            (false, 1, false),
            (true, 0, true),
            (true, 1, false),
        ] {
            let maker = Pubkey::new_unique();
            let fixed = GlobalFixed::new_empty(&Pubkey::new_unique());
            let mut data = bytemuck::bytes_of(&fixed).to_vec();
            data.resize(GLOBAL_FIXED_SIZE + 2 * GLOBAL_BLOCK_SIZE, 0);
            let account = OwnedAccount::new(&Pubkey::new_unique(), &crate::ID, 0, &data);
            let token_account = OwnedAccount::new(&spl_token::id(), &Pubkey::default(), 0, &[]);
            // SAFETY: both owned accounts outlive all views and borrows below.
            let global_view = unsafe { account.view() };
            let token_view = unsafe { token_account.view() };
            if has_seat {
                let mut bytes = global_view.try_borrow_mut().unwrap();
                let mut global: GlobalRefMut = get_mut_dynamic_account(&mut bytes);
                global.global_expand().unwrap();
                global.add_trader(&maker).unwrap();
            }
            let before = global_view.try_borrow().unwrap().to_vec();
            let accounts = Some(GlobalTradeAccounts {
                global: ManifestAccountInfo::<GlobalFixed>::new(&global_view).unwrap(),
                token_program_opt: Some(TokenProgram::new(&token_view).unwrap()),
                mint_opt: None,
                global_vault_opt: None,
                market_vault_opt: None,
                system_program: None,
                gas_payer_opt: None,
                gas_receiver_opt: None,
                market: Pubkey::new_unique(),
                num_deferred_gas_refunds: Cell::new(0),
            });

            assert_eq!(
                try_to_reduce_global_tokens(&accounts, &maker, GlobalAtoms::new(desired_atoms)),
                Ok(expected_backed),
                "has_seat={has_seat}, desired_atoms={desired_atoms}",
            );
            assert_eq!(&*global_view.try_borrow().unwrap(), before.as_slice());
        }
    }
}

#[cfg(test)]
mod trader_key_comparison_tests {
    use super::compare_trader_keys;
    use solana_program::pubkey::Pubkey;

    #[test]
    fn trader_key_order_matches_pubkey_order() {
        // Exercise every possible first-differing byte, including word edges.
        for first in 0..32 {
            for common in [0u8, 0x7f, 0x80, 0xff] {
                let mut left = [common; 32];
                let mut right = left;
                left[first] = 0x7f;
                right[first] = 0x80;
                left[first + 1..].fill(0xff);
                right[first + 1..].fill(0);
                let left = Pubkey::new_from_array(left);
                let right = Pubkey::new_from_array(right);
                assert_eq!(compare_trader_keys(&left, &right), left.cmp(&right));
                assert_eq!(compare_trader_keys(&right, &left), right.cmp(&left));
                assert_eq!(compare_trader_keys(&left, &left), left.cmp(&left));
            }
        }
        let mut random = 42u64;
        for _ in 0..4096 {
            let mut left = [0u8; 32];
            let mut right = left;
            for byte in left.iter_mut().chain(right.iter_mut()) {
                random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                *byte = (random >> 32) as u8;
            }
            let left = Pubkey::new_from_array(left);
            let right = Pubkey::new_from_array(right);
            assert_eq!(compare_trader_keys(&left, &right), left.cmp(&right));
        }
    }
}
