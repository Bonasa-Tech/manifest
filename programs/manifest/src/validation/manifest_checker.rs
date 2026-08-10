use bytemuck::Pod;
use hypertree::{get_helper, Get};
use solana_program::{
    account_info::AccountInfo, entrypoint::ProgramResult, program_error::ProgramError,
    pubkey::Pubkey,
};
use std::{cell::Ref, mem::size_of, ops::Deref};

use crate::require;

/// Validation for manifest accounts.
#[derive(Clone)]
pub struct ManifestAccountInfo<'a, 'info, T: ManifestAccount + Pod + Clone> {
    pub info: &'a AccountInfo<'info>,

    phantom: std::marker::PhantomData<T>,
}

impl<'a, 'info, T: ManifestAccount + Get + Pod + Clone> ManifestAccountInfo<'a, 'info, T> {
    #[cfg_attr(
        all(feature = "certora", not(feature = "certora-test")),
        early_panic::early_panic
    )]
    pub fn new(
        info: &'a AccountInfo<'info>,
    ) -> Result<ManifestAccountInfo<'a, 'info, T>, ProgramError> {
        verify_owned_by_manifest(info.owner)?;

        let bytes: Ref<&mut [u8]> = info.try_borrow_data()?;
        let (header_bytes, _) = bytes.split_at(size_of::<T>());
        let header: &T = get_helper::<T>(header_bytes, 0_u32);
        header.verify_discriminant()?;

        Ok(Self {
            info,
            phantom: std::marker::PhantomData,
        })
    }

    pub fn new_init(
        info: &'a AccountInfo<'info>,
    ) -> Result<ManifestAccountInfo<'a, 'info, T>, ProgramError> {
        verify_owned_by_manifest(info.owner)?;
        verify_uninitialized::<T>(info)?;
        Ok(Self {
            info,
            phantom: std::marker::PhantomData,
        })
    }

    pub fn get_fixed(&self) -> Result<Ref<'_, T>, ProgramError> {
        let data: Ref<&mut [u8]> = self.info.try_borrow_data()?;
        Ok(Ref::map(data, |data| {
            return get_helper::<T>(data, 0_u32);
        }))
    }
}

impl<'a, 'info, T: ManifestAccount + Pod + Clone> Deref for ManifestAccountInfo<'a, 'info, T> {
    type Target = AccountInfo<'info>;

    fn deref(&self) -> &Self::Target {
        self.info
    }
}

pub trait ManifestAccount {
    fn verify_discriminant(&self) -> ProgramResult;
}

fn verify_owned_by_manifest(owner: &Pubkey) -> ProgramResult {
    require!(
        owner == &crate::ID,
        ProgramError::IllegalOwner,
        "Account must be owned by the Manifest program expected:{} actual:{}",
        crate::ID,
        owner
    )?;
    Ok(())
}

fn verify_uninitialized<T: Pod + ManifestAccount>(info: &AccountInfo) -> ProgramResult {
    let bytes: Ref<&mut [u8]> = info.try_borrow_data()?;
    require!(
        size_of::<T>() == bytes.len(),
        ProgramError::InvalidAccountData,
        "Incorrect length for uninitialized header expected: {} actual: {}",
        size_of::<T>(),
        bytes.len()
    )?;

    // This can't happen because for Market, we increase the size of the account
    // with a free block when it gets init, so the first check fails. For
    // global, we dont use new_init because the account is a PDA, so it is not
    // at an existing account. Keep the check for thoroughness in case a new
    // type is ever added.
    require!(
        bytes.iter().all(|&byte| byte == 0),
        ProgramError::InvalidAccountData,
        "Expected zeroed",
    )?;
    Ok(())
}

#[cfg(test)]
mod test {
    use crate::state::{
        GlobalFixed, MarketFixed, GLOBAL_FIXED_DISCRIMINANT, MARKET_FIXED_DISCRIMINANT,
    };

    #[test]
    fn test_market_fixed_discriminant() {
        let discriminant: u64 = crate::utils::get_discriminant::<MarketFixed>().unwrap();
        assert_eq!(discriminant, MARKET_FIXED_DISCRIMINANT);
    }

    #[test]
    fn test_global_fixed_discriminant() {
        let discriminant: u64 = crate::utils::get_discriminant::<GlobalFixed>().unwrap();
        assert_eq!(discriminant, GLOBAL_FIXED_DISCRIMINANT);
    }
}

macro_rules! global_seeds {
    ( $mint:expr ) => {
        &[b"global", $mint.as_ref()]
    };
}

#[macro_export]
macro_rules! global_seeds_with_bump {
    ( $mint:expr, $bump:expr ) => {
        &[&[b"global", $mint.as_ref(), &[$bump]]]
    };
}

pub fn get_global_address(mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(global_seeds!(mint), &crate::ID)
}
