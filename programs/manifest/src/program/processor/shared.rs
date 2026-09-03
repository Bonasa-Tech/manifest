use pinocchio::account::{AccountView, Ref, RefMut};
use std::mem::size_of;

use crate::{
    require,
    state::{
        claimed_seat::ClaimedSeat, constants::MARKET_BLOCK_SIZE, DynamicAccount, GlobalFixed,
        MarketFixed, MarketRefMut, GLOBAL_BLOCK_SIZE,
    },
    validation::{ManifestAccount, ManifestAccountInfo, Signer},
};
use bytemuck::Pod;
use core::mem::MaybeUninit;
use pinocchio::instruction::{
    cpi::{Seed, Signer as PinocchioSigner},
    InstructionAccount as PinocchioAccountMeta, InstructionView as PinocchioInstruction,
};

use crate::validation::{as_raw_key, AccountViewExt};
use hypertree::{get_helper, get_mut_helper, DataIndex, Get, RBNode};
#[cfg(not(feature = "certora"))]
use pinocchio::sysvars::Sysvar;
use pinocchio::{error::ProgramError, ProgramResult};
use solana_program::instruction::Instruction;

use super::batch_update::MarketDataTreeNodeType;

pub(crate) fn expand_market_if_needed<'a, T: ManifestAccount + Pod + Clone>(
    payer: &AccountView,
    market_account_info: &ManifestAccountInfo<'a, T>,
) -> ProgramResult {
    let need_expand: bool = {
        let market_data: &Ref<[u8]> = &market_account_info.try_borrow()?;
        let fixed: &MarketFixed = get_helper::<MarketFixed>(market_data, 0_u32);
        !fixed.has_free_block()
    };

    if !need_expand {
        return Ok(());
    }
    // Convert the AccountView into a signer. The checks for writable and signer
    // are done this late so that in the case where it is not required, it can
    // work.
    expand_market(&Signer::new_payer(payer)?, market_account_info)
}

pub(crate) fn expand_market<'a, T: ManifestAccount + Pod + Clone>(
    payer: &Signer<'a>,
    manifest_account: &ManifestAccountInfo<'a, T>,
) -> ProgramResult {
    expand_dynamic(payer, manifest_account, MARKET_BLOCK_SIZE)?;
    expand_market_fixed(manifest_account.info)?;
    Ok(())
}

pub(crate) fn batch_expand_market<'a, T: ManifestAccount + Pod + Clone>(
    payer: &Signer<'a>,
    manifest_account: &ManifestAccountInfo<'a, T>,
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
pub(crate) fn expand_global<'a, T: ManifestAccount + Pod + Clone>(
    payer: &Signer<'a>,
    manifest_account: &ManifestAccountInfo<'a, T>,
) -> ProgramResult {
    // Expand twice because of two trees at once.
    expand_dynamic(payer, manifest_account, 2 * GLOBAL_BLOCK_SIZE)?;
    expand_global_fixed(manifest_account.info)?;
    Ok(())
}

#[cfg(feature = "certora")]
fn expand_dynamic<'a, T: ManifestAccount + Pod + Clone>(
    _payer: &Signer<'a>,
    _manifest_account: &ManifestAccountInfo<'a, T>,
    _block_size: usize,
) -> ProgramResult {
    Ok(())
}
#[cfg(not(feature = "certora"))]
fn expand_dynamic<'a, T: ManifestAccount + Pod + Clone>(
    payer: &Signer<'a>,
    manifest_account: &ManifestAccountInfo<'a, T>,
    block_size: usize,
) -> ProgramResult {
    // Account types were already validated, so do not need to reverify that the
    // accounts are in order: payer, expandable_account, ...
    let expandable_account: &AccountView = manifest_account.info;
    let new_size: usize = expandable_account.data_len() + block_size;

    let rent: pinocchio::sysvars::rent::Rent = pinocchio::sysvars::rent::Rent::get()?;
    let new_minimum_balance: u64 = rent.try_minimum_balance(new_size)?;
    let old_minimum_balance: u64 = rent.try_minimum_balance(expandable_account.data_len())?;
    let lamports_diff: u64 = new_minimum_balance.saturating_sub(old_minimum_balance);

    let payer: &AccountView = payer.info;

    invoke(
        &solana_program::system_instruction::transfer(
            payer.pubkey(),
            expandable_account.pubkey(),
            lamports_diff,
        ),
        &[payer, expandable_account],
    )?;

    #[cfg(feature = "fuzz")]
    {
        solana_program::program::invoke(
            &solana_program::system_instruction::allocate(
                expandable_account.pubkey(),
                new_size as u64,
            ),
            &[expandable_account.clone()],
        )?;
    }
    #[cfg(not(feature = "fuzz"))]
    {
        #[allow(deprecated)]
        expandable_account.resize(new_size)?;
    }
    Ok(())
}

fn expand_market_fixed(expandable_account: &AccountView) -> ProgramResult {
    let market_data: &mut RefMut<[u8]> = &mut expandable_account.try_borrow_mut()?;
    let mut dynamic_account: DynamicAccount<&mut MarketFixed, &mut [u8]> =
        get_mut_dynamic_account(market_data);
    dynamic_account.market_expand()?;
    Ok(())
}

fn expand_market_fixed_n(expandable_account: &AccountView, n: u32) -> ProgramResult {
    let market_data: &mut RefMut<[u8]> = &mut expandable_account.try_borrow_mut()?;
    let mut dynamic_account: DynamicAccount<&mut MarketFixed, &mut [u8]> =
        get_mut_dynamic_account(market_data);
    dynamic_account.market_expand_n(n)?;
    Ok(())
}

fn expand_global_fixed(expandable_account: &AccountView) -> ProgramResult {
    let global_data: &mut RefMut<[u8]> = &mut expandable_account.try_borrow_mut()?;
    let mut dynamic_account: DynamicAccount<&mut GlobalFixed, &mut [u8]> =
        get_mut_dynamic_account(global_data);
    dynamic_account.global_expand()?;
    Ok(())
}

/// Generic get dynamic account from the data bytes of the account.
pub fn get_dynamic_account<'a, T: Get>(data: &'a Ref<'a, [u8]>) -> DynamicAccount<&'a T, &'a [u8]> {
    let (fixed_data, dynamic) = data.split_at(size_of::<T>());
    let fixed: &T = get_helper::<T>(fixed_data, 0_u32);

    let dynamic_account: DynamicAccount<&'a T, &'a [u8]> = DynamicAccount { fixed, dynamic };
    dynamic_account
}

/// Generic get mutable dynamic account from the data bytes of the account.
pub fn get_mut_dynamic_account<'a, T: Get>(
    data: &'a mut RefMut<'_, [u8]>,
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
        None => dynamic_account.get_trader_index(payer.pubkey()),
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
            .pubkey()
            .eq(dynamic_account.get_trader_key_by_index(hinted_index)),
        crate::program::ManifestError::WrongIndexHintParams,
        "Invalid trader hint index {} did not match payer",
        hinted_index
    )?;
    Ok(())
}

/// Builds pinocchio's borrowed instruction from a `solana_program` one and
/// hands it, with the accounts it names, to `call`.
///
/// Two things have to be translated. The instruction itself: pinocchio's
/// points at the caller's key, metas and data rather than owning them, so this
/// is one stack array of metas and no copy of the data. And the accounts:
/// `solana_program`'s `invoke` takes any superset of the instruction's
/// accounts in any order and matches them up by key, while pinocchio's takes
/// exactly the instruction's accounts, in the instruction's order. Call sites
/// here pass what they have, so the matching happens here.
fn with_pinocchio_instruction<R>(
    ix: &Instruction,
    account_infos: &[&AccountView],
    call: impl FnOnce(&PinocchioInstruction, &[&AccountView]) -> R,
) -> Result<R, ProgramError> {
    const MAX_CPI_ACCOUNTS: usize = 16;
    require!(
        ix.accounts.len() <= MAX_CPI_ACCOUNTS,
        ProgramError::InvalidArgument,
        "CPI with {} accounts, more than the {} supported",
        ix.accounts.len(),
        MAX_CPI_ACCOUNTS,
    )?;

    let mut metas: [MaybeUninit<PinocchioAccountMeta>; MAX_CPI_ACCOUNTS] =
        [const { MaybeUninit::uninit() }; MAX_CPI_ACCOUNTS];
    let mut ordered: [MaybeUninit<&AccountView>; MAX_CPI_ACCOUNTS] =
        [const { MaybeUninit::uninit() }; MAX_CPI_ACCOUNTS];

    // Call sites that already pass exactly the instruction's accounts, in its
    // order, skip the matching below: that is pinocchio's own contract, and
    // the token and system CPIs here are written to it.
    let positional: bool = account_infos.len() == ix.accounts.len()
        && account_infos
            .iter()
            .zip(ix.accounts.iter())
            .all(|(info, account)| info.address() == as_raw_key(&account.pubkey));

    for (index, account) in ix.accounts.iter().enumerate() {
        metas[index].write(PinocchioAccountMeta {
            address: as_raw_key(&account.pubkey),
            is_writable: account.is_writable,
            is_signer: account.is_signer,
        });
        // Keys are compared eight bytes at a time, and almost always differ in
        // the first eight, so this is one integer compare per candidate rather
        // than a 32 byte one. Worth it: this runs for every account of every
        // CPI, and the whole point of the account type below it is that
        // reading a field is a load.
        if positional {
            ordered[index].write(&account_infos[index]);
            continue;
        }
        let wanted: &[u8; 32] = as_raw_key(&account.pubkey).as_array();
        let wanted_head: u64 = u64::from_le_bytes(wanted[..8].try_into().unwrap());
        let found: &&AccountView = account_infos
            .iter()
            .find(|info| {
                let key: &[u8; 32] = info.address().as_array();
                u64::from_le_bytes(key[..8].try_into().unwrap()) == wanted_head && key == wanted
            })
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        ordered[index].write(found);
    }

    // SAFETY: the first `ix.accounts.len()` entries of both arrays were just
    // written, and that length is within the arrays by the check above.
    let metas: &[PinocchioAccountMeta] =
        unsafe { core::slice::from_raw_parts(metas.as_ptr().cast(), ix.accounts.len()) };
    let ordered: &[&AccountView] =
        unsafe { core::slice::from_raw_parts(ordered.as_ptr().cast(), ix.accounts.len()) };

    Ok(call(
        &PinocchioInstruction {
            program_id: as_raw_key(&ix.program_id),
            accounts: metas,
            data: &ix.data,
        },
        ordered,
    ))
}

/// Calls another program, signing for a program derived address.
///
/// `seeds` are the byte slices the address was derived from, bump included,
/// in the shape the `*_seeds_with_bump!` macros produce.
pub fn invoke_signed(
    ix: &Instruction,
    account_infos: &[&AccountView],
    seeds: &[&[&[u8]]],
) -> ProgramResult {
    const MAX_SEEDS: usize = 8;
    require!(
        seeds.len() == 1 && seeds[0].len() <= MAX_SEEDS,
        ProgramError::InvalidArgument,
        "CPI signing for {} addresses with up to {} seeds is not supported",
        seeds.len(),
        MAX_SEEDS,
    )?;

    let mut seed_array: [MaybeUninit<Seed>; MAX_SEEDS] =
        [const { MaybeUninit::uninit() }; MAX_SEEDS];
    for (slot, seed) in seed_array.iter_mut().zip(seeds[0].iter()) {
        slot.write(Seed::from(*seed));
    }
    // SAFETY: the first `seeds[0].len()` entries were just written, and that
    // length is within the array by the check above.
    let written: &[Seed] =
        unsafe { core::slice::from_raw_parts(seed_array.as_ptr().cast(), seeds[0].len()) };
    let signer: PinocchioSigner = PinocchioSigner::from(written);

    with_pinocchio_instruction(ix, account_infos, |pinocchio_ix, ordered| {
        pinocchio::cpi::invoke_signed_with_slice(pinocchio_ix, ordered, &[signer])
    })?
}

/// Calls another program.
///
/// The instruction is still built with the `solana_program` builders, which
/// are what the SPL and system program crates hand out, and is translated to
/// pinocchio's borrowed form here: its `Instruction` points at the caller's
/// key, metas and data rather than owning them, so the translation is one
/// stack array of metas and no copying of the instruction data.
///
/// Accounts arrive as `&[&AccountView]` because that is what pinocchio's CPI
/// takes; the slice form avoids the const generic count at every call site.
/// Calls `program_id` with `data`, passing exactly `accounts` in the order
/// given and taking each account's writable and signer flags from this
/// program's own view of it.
///
/// This is what forwarding an instruction to another program looks like when
/// the caller already holds the accounts it wants to pass. Going through a
/// `solana_program::Instruction` to say the same thing copies every address
/// into an `AccountMeta` only for [`with_pinocchio_instruction`] to borrow it
/// back out again; pinocchio's metas point at the addresses the accounts
/// already carry.
pub fn invoke_passthrough(
    program_id: &solana_program::pubkey::Pubkey,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    const MAX_CPI_ACCOUNTS: usize = 16;
    require!(
        accounts.len() <= MAX_CPI_ACCOUNTS,
        ProgramError::InvalidArgument,
        "CPI with {} accounts, more than the {} supported",
        accounts.len(),
        MAX_CPI_ACCOUNTS,
    )?;

    let mut metas: [MaybeUninit<PinocchioAccountMeta>; MAX_CPI_ACCOUNTS] =
        [const { MaybeUninit::uninit() }; MAX_CPI_ACCOUNTS];
    // pinocchio wants the accounts as a slice of references; this is that
    // slice, on the stack, rather than a heap vector of pointers.
    let mut borrowed: [MaybeUninit<&AccountView>; MAX_CPI_ACCOUNTS] =
        [const { MaybeUninit::uninit() }; MAX_CPI_ACCOUNTS];
    for ((meta, slot), account) in metas
        .iter_mut()
        .zip(borrowed.iter_mut())
        .zip(accounts.iter())
    {
        meta.write(PinocchioAccountMeta {
            address: account.address(),
            is_writable: account.is_writable(),
            is_signer: account.is_signer(),
        });
        slot.write(account);
    }
    // SAFETY: the first `accounts.len()` entries of both arrays were just
    // written, and that length is within them by the check above.
    let metas: &[PinocchioAccountMeta] =
        unsafe { core::slice::from_raw_parts(metas.as_ptr().cast(), accounts.len()) };
    let borrowed: &[&AccountView] =
        unsafe { core::slice::from_raw_parts(borrowed.as_ptr().cast(), accounts.len()) };

    pinocchio::cpi::invoke_with_slice(
        &PinocchioInstruction {
            program_id: as_raw_key(program_id),
            accounts: metas,
            data,
        },
        borrowed,
    )
}

pub fn invoke(ix: &Instruction, account_infos: &[&AccountView]) -> ProgramResult {
    with_pinocchio_instruction(ix, account_infos, |pinocchio_ix, ordered| {
        pinocchio::cpi::invoke_with_slice(pinocchio_ix, ordered)
    })?
}
