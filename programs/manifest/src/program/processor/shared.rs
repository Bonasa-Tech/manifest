use std::{
    cell::{Ref, RefMut},
    mem::size_of,
};

use crate::{
    require,
    state::{
        claimed_seat::ClaimedSeat, constants::MARKET_BLOCK_SIZE, DynamicAccount, GlobalFixed,
        MarketFixed, MarketRefMut, GLOBAL_BLOCK_SIZE,
    },
    validation::{ManifestAccount, ManifestAccountInfo, Signer},
};
use bytemuck::Pod;
use hypertree::{get_helper, get_mut_helper, DataIndex, Get, RBNode};
#[cfg(not(feature = "certora"))]
use solana_program::sysvar::Sysvar;
use solana_program::{
    account_info::AccountInfo, entrypoint::ProgramResult, instruction::Instruction,
    sysvar::slot_history::ProgramError,
};

use super::batch_update::MarketDataTreeNodeType;

pub(crate) fn expand_market_if_needed<'a, 'info, T: ManifestAccount + Pod + Clone>(
    payer: &AccountInfo<'info>,
    market_account_info: &ManifestAccountInfo<'a, 'info, T>,
) -> ProgramResult {
    let need_expand: bool = {
        let market_data: &Ref<&mut [u8]> = &market_account_info.try_borrow_data()?;
        let fixed: &MarketFixed = get_helper::<MarketFixed>(market_data, 0_u32);
        !fixed.has_free_block()
    };

    if !need_expand {
        return Ok(());
    }
    // Convert the AccountInfo into a signer. The checks for writable and signer
    // are done this late so that in the case where it is not required, it can
    // work.
    expand_market(&Signer::new_payer(payer)?, market_account_info)
}

pub(crate) fn expand_market<'a, 'info, T: ManifestAccount + Pod + Clone>(
    payer: &Signer<'a, 'info>,
    manifest_account: &ManifestAccountInfo<'a, 'info, T>,
) -> ProgramResult {
    expand_dynamic(payer, manifest_account, MARKET_BLOCK_SIZE)?;
    expand_market_fixed(manifest_account.info)?;
    Ok(())
}

pub(crate) fn batch_expand_market<'a, 'info, T: ManifestAccount + Pod + Clone>(
    payer: &Signer<'a, 'info>,
    manifest_account: &ManifestAccountInfo<'a, 'info, T>,
    num_blocks: u32,
) -> ProgramResult {
    expand_dynamic(
        payer,
        manifest_account,
        num_blocks as usize * MARKET_BLOCK_SIZE,
    )?;
    expand_market_fixed_n(manifest_account.info, num_blocks)?;
    Ok(())
}

// Expand is always needed because global doesnt free bytes ever.
pub(crate) fn expand_global<'a, 'info, T: ManifestAccount + Pod + Clone>(
    payer: &Signer<'a, 'info>,
    manifest_account: &ManifestAccountInfo<'a, 'info, T>,
) -> ProgramResult {
    // Expand twice because of two trees at once.
    expand_dynamic(payer, manifest_account, 2 * GLOBAL_BLOCK_SIZE)?;
    expand_global_fixed(manifest_account.info)?;
    Ok(())
}

#[cfg(feature = "certora")]
fn expand_dynamic<'a, 'info, T: ManifestAccount + Pod + Clone>(
    _payer: &Signer<'a, 'info>,
    _manifest_account: &ManifestAccountInfo<'a, 'info, T>,
    _block_size: usize,
) -> ProgramResult {
    Ok(())
}
#[cfg(not(feature = "certora"))]
fn expand_dynamic<'a, 'info, T: ManifestAccount + Pod + Clone>(
    payer: &Signer<'a, 'info>,
    manifest_account: &ManifestAccountInfo<'a, 'info, T>,
    block_size: usize,
) -> ProgramResult {
    // Account types were already validated, so do not need to reverify that the
    // accounts are in order: payer, expandable_account, ...
    let expandable_account: &AccountInfo = manifest_account.info;
    let new_size: usize = expandable_account.data_len() + block_size;

    let rent: solana_program::rent::Rent = solana_program::rent::Rent::get()?;
    let new_minimum_balance: u64 = rent.minimum_balance(new_size);
    let old_minimum_balance: u64 = rent.minimum_balance(expandable_account.data_len());
    let lamports_diff: u64 = new_minimum_balance.saturating_sub(old_minimum_balance);

    let payer: &AccountInfo = payer.info;

    invoke(
        &solana_program::system_instruction::transfer(
            payer.key,
            expandable_account.key,
            lamports_diff,
        ),
        &[payer.clone(), expandable_account.clone()],
    )?;

    #[cfg(feature = "fuzz")]
    {
        solana_program::program::invoke(
            &solana_program::system_instruction::allocate(expandable_account.key, new_size as u64),
            &[expandable_account.clone()],
        )?;
    }
    #[cfg(not(feature = "fuzz"))]
    {
        #[allow(deprecated)]
        expandable_account.realloc(new_size, false)?;
    }
    Ok(())
}

fn expand_market_fixed(expandable_account: &AccountInfo) -> ProgramResult {
    let market_data: &mut RefMut<&mut [u8]> = &mut expandable_account.try_borrow_mut_data()?;
    let mut dynamic_account: DynamicAccount<&mut MarketFixed, &mut [u8]> =
        get_mut_dynamic_account(market_data);
    dynamic_account.market_expand()?;
    Ok(())
}

fn expand_market_fixed_n(expandable_account: &AccountInfo, n: u32) -> ProgramResult {
    let market_data: &mut RefMut<&mut [u8]> = &mut expandable_account.try_borrow_mut_data()?;
    let mut dynamic_account: DynamicAccount<&mut MarketFixed, &mut [u8]> =
        get_mut_dynamic_account(market_data);
    dynamic_account.market_expand_n(n)?;
    Ok(())
}

fn expand_global_fixed(expandable_account: &AccountInfo) -> ProgramResult {
    let global_data: &mut RefMut<&mut [u8]> = &mut expandable_account.try_borrow_mut_data()?;
    let mut dynamic_account: DynamicAccount<&mut GlobalFixed, &mut [u8]> =
        get_mut_dynamic_account(global_data);
    dynamic_account.global_expand()?;
    Ok(())
}

/// Generic get dynamic account from the data bytes of the account.
pub fn get_dynamic_account<'a, T: Get>(
    data: &'a Ref<'a, &'a mut [u8]>,
) -> DynamicAccount<&'a T, &'a [u8]> {
    let (fixed_data, dynamic) = data.split_at(size_of::<T>());
    let fixed: &T = get_helper::<T>(fixed_data, 0_u32);

    let dynamic_account: DynamicAccount<&'a T, &'a [u8]> = DynamicAccount { fixed, dynamic };
    dynamic_account
}

/// Generic get mutable dynamic account from the data bytes of the account.
pub fn get_mut_dynamic_account<'a, T: Get>(
    data: &'a mut RefMut<'_, &mut [u8]>,
) -> DynamicAccount<&'a mut T, &'a mut [u8]> {
    let (fixed_data, dynamic) = data.split_at_mut(size_of::<T>());
    let fixed: &mut T = get_mut_helper::<T>(fixed_data, 0_u32);

    let dynamic_account: DynamicAccount<&'a mut T, &'a mut [u8]> =
        DynamicAccount { fixed, dynamic };
    dynamic_account
}

pub fn get_dynamic_ref<T: Get>(data: &[u8]) -> DynamicAccount<&'_ T, &'_ [u8]> {
    let (fixed_data, dynamic_data) = data.split_at(size_of::<T>());
    let market_fixed: &T = get_helper::<T>(fixed_data, 0_u32);

    let dynamic_account: DynamicAccount<&T, &[u8]> = DynamicAccount {
        fixed: market_fixed,
        dynamic: dynamic_data,
    };
    dynamic_account
}

/// Generic get owned dynamic account from the data bytes of the account.
pub fn get_dynamic_value_or<T: Get>(
    data: &[u8],
) -> Result<DynamicAccount<T, Vec<u8>>, ProgramError> {
    if data.len() < size_of::<T>() {
        return Err(ProgramError::InvalidAccountData);
    }
    let (fixed_data, dynamic_data) = data.split_at(size_of::<T>());
    let market_fixed: &T = get_helper::<T>(fixed_data, 0_u32);

    Ok(DynamicAccount {
        fixed: *market_fixed,
        dynamic: (dynamic_data).to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_value_rejects_truncated_fixed_data() {
        for len in 0..size_of::<MarketFixed>() {
            assert!(matches!(
                get_dynamic_value_or::<MarketFixed>(&vec![0; len]),
                Err(ProgramError::InvalidAccountData)
            ));
        }
    }
}

// Uses a MarketRefMut instead of a MarketRef because callers will have mutable data.
pub(crate) fn get_trader_index_with_hint(
    trader_index_hint: Option<DataIndex>,
    dynamic_account: &MarketRefMut,
    payer: &Signer,
) -> Result<DataIndex, ProgramError> {
    let trader_index: DataIndex = match trader_index_hint {
        None => dynamic_account.get_trader_index(payer.key),
        Some(hinted_index) => {
            verify_trader_index_hint(hinted_index, &dynamic_account, &payer)?;
            hinted_index
        }
    };
    Ok(trader_index)
}

fn verify_trader_index_hint(
    hinted_index: DataIndex,
    dynamic_account: &MarketRefMut,
    payer: &Signer,
) -> ProgramResult {
    require!(
        hinted_index % (MARKET_BLOCK_SIZE as DataIndex) == 0,
        crate::program::ManifestError::WrongIndexHintParams,
        "Invalid trader hint index {} did not align",
        hinted_index,
    )?;
    require!(
        get_helper::<RBNode<ClaimedSeat>>(&dynamic_account.dynamic, hinted_index)
            .get_payload_type()
            == MarketDataTreeNodeType::ClaimedSeat as u8,
        crate::program::ManifestError::WrongIndexHintParams,
        "Invalid trader hint index {} is not a ClaimedSeat",
        hinted_index,
    )?;
    require!(
        payer
            .key
            .eq(dynamic_account.get_trader_key_by_index(hinted_index)),
        crate::program::ManifestError::WrongIndexHintParams,
        "Invalid trader hint index {} did not match payer",
        hinted_index
    )?;
    Ok(())
}

// TODO: Same for invoke_signed

pub fn invoke(ix: &Instruction, account_infos: &[AccountInfo<'_>]) -> ProgramResult {
    #[cfg(target_os = "solana")]
    {
        solana_invoke::invoke_unchecked(ix, account_infos)
    }
    #[cfg(not(target_os = "solana"))]
    {
        solana_program::program::invoke(ix, account_infos)
    }
}
