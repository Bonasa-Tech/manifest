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

/// Whether `key` is the global account address for `mint` derived with
/// `bump`, i.e. whether
/// `Pubkey::create_program_address(&[b"global", mint, &[bump]], &crate::ID)`
/// would return it.
///
/// Program derived addresses are what make the global account checks
/// meaningful: only this program can sign for such an address, so only its
/// own init could have created an account there. Deriving with the syscall
/// costs 1,500 CU (and `find_program_address` that much per bump tried), so
/// this does the same computation, the seed hash followed by rejecting hashes
/// that are on the ed25519 curve, with the sha256 and curve syscalls
/// directly for about 350 CU. Exhaustively checked against
/// `create_program_address` in the tests below.
#[cfg(not(feature = "certora"))]
pub fn is_global_address(key: &Pubkey, mint: &Pubkey, bump: u8) -> bool {
    let hash: [u8; 32] = solana_program::hash::hashv(&[
        b"global",
        mint.as_ref(),
        &[bump],
        crate::ID.as_ref(),
        PDA_MARKER,
    ])
    .to_bytes();
    hash == key.to_bytes()
        && !solana_curve25519::edwards::validate_edwards(
            &solana_curve25519::edwards::PodEdwardsPoint(hash),
        )
}

/// Formal verification models `create_program_address` but not the hash and
/// curve syscalls, so it keeps the derivation.
#[cfg(feature = "certora")]
pub fn is_global_address(key: &Pubkey, mint: &Pubkey, bump: u8) -> bool {
    Pubkey::create_program_address(&[b"global", mint.as_ref(), &[bump]], &crate::ID) == Ok(*key)
}

/// Domain separator the runtime appends when hashing program derived address
/// seeds. Only the hand rolled derivation above needs it, and that is not the
/// one the `certora` build uses.
#[cfg(not(feature = "certora"))]
const PDA_MARKER: &[u8; 21] = b"ProgramDerivedAddress";

/// Exhaustive check that the hand rolled derivation agrees with
/// `create_program_address`. It covers the non certora `is_global_address`,
/// which is the one the prover never sees, so this is where that path is
/// pinned down.
#[cfg(all(test, not(feature = "certora")))]
mod is_global_address_test {
    use super::*;

    #[test]
    fn matches_create_program_address_for_every_bump() {
        for _ in 0..32 {
            let mint: Pubkey = Pubkey::new_unique();
            let (global, bump) = get_global_address(&mint);
            assert!(is_global_address(&global, &mint, bump));
            assert!(!is_global_address(&Pubkey::new_unique(), &mint, bump));
            assert!(!is_global_address(&global, &Pubkey::new_unique(), bump));

            for candidate_bump in 0..=u8::MAX {
                let seeds: [&[u8]; 3] = [b"global", mint.as_ref(), &[candidate_bump]];
                match Pubkey::create_program_address(&seeds, &crate::ID) {
                    Ok(address) => {
                        assert!(is_global_address(&address, &mint, candidate_bump));
                        assert_eq!(candidate_bump == bump, address == global);
                    }
                    Err(_) => {
                        // The seed hash is on the curve, so somebody could
                        // hold its private key. It must be rejected even for
                        // an account that sits exactly at that key.
                        let on_curve: Pubkey = Pubkey::from(
                            solana_program::hash::hashv(&[
                                b"global",
                                mint.as_ref(),
                                &[candidate_bump],
                                crate::ID.as_ref(),
                                PDA_MARKER,
                            ])
                            .to_bytes(),
                        );
                        assert!(!is_global_address(&on_curve, &mint, candidate_bump));
                    }
                }
            }
        }
    }
}
