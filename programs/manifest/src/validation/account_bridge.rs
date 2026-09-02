//! Bridge between pinocchio's account type and the rest of this program.
//!
//! The entrypoint hands out [`pinocchio::account_info::AccountInfo`], which is
//! a pointer into the runtime's input buffer: reading a field costs a load
//! rather than the two `Rc<RefCell<_>>` allocations `solana_program`'s
//! equivalent needs per account. Its keys are bare `[u8; 32]` where everything
//! stored in a market, logged, or handed to a client is a
//! [`solana_program::pubkey::Pubkey`].
//!
//! Those are the same 32 bytes: `Pubkey` is `#[repr(transparent)]` over
//! `[u8; 32]`, so a reference to one is a reference to the other. This trait
//! does that reinterpretation in one place, so call sites keep reading in
//! terms of `Pubkey` and no bytes are copied to get there.

use pinocchio::{account_info::AccountInfo, program_error::ProgramError, pubkey::Pubkey as RawKey};
use solana_program::pubkey::Pubkey;

/// Takes the next account from the runtime's slice.
///
/// pinocchio has no equivalent of `solana_program`'s `next_account_info`; the
/// loaders walk the slice in order the same way they always did.
pub fn next_account_info<'a>(
    iter: &mut core::slice::Iter<'a, AccountInfo>,
) -> Result<&'a AccountInfo, ProgramError> {
    iter.next().ok_or(ProgramError::NotEnoughAccountKeys)
}

/// Reinterprets a raw runtime key as a `Pubkey`.
///
/// Sound because `Pubkey` is a `#[repr(transparent)]` newtype over the same
/// `[u8; 32]`, which is checked below.
#[inline(always)]
pub fn as_pubkey(key: &RawKey) -> &Pubkey {
    // SAFETY: `Pubkey` is `#[repr(transparent)]` over `[u8; 32]`, so the two
    // have the same size, alignment and layout.
    unsafe { &*(key as *const RawKey as *const Pubkey) }
}

/// The reverse, for handing a `Pubkey` to pinocchio.
#[inline(always)]
pub fn as_raw_key(key: &Pubkey) -> &RawKey {
    // SAFETY: as above.
    unsafe { &*(key as *const Pubkey as *const RawKey) }
}

/// Account fields in the terms the rest of the program is written in.
pub trait AccountInfoExt {
    /// The account's address.
    fn pubkey(&self) -> &Pubkey;
    /// The program that owns the account.
    fn owner_pubkey(&self) -> &Pubkey;
    /// Whether the account is owned by `program`.
    fn owned_by(&self, program: &Pubkey) -> bool;
}

impl AccountInfoExt for AccountInfo {
    #[inline(always)]
    fn pubkey(&self) -> &Pubkey {
        as_pubkey(self.key())
    }

    #[inline(always)]
    fn owner_pubkey(&self) -> &Pubkey {
        // SAFETY: pinocchio marks this unsafe because the owner must not be
        // read while the account is mutably borrowed by this program. Every
        // caller here reads it during account validation, before any borrow of
        // that account is taken.
        as_pubkey(unsafe { self.owner() })
    }

    #[inline(always)]
    fn owned_by(&self, program: &Pubkey) -> bool {
        self.is_owned_by(as_raw_key(program))
    }
}

/// A `solana_program` error in pinocchio's terms.
///
/// The two crates each define their own `ProgramError` with the same wire
/// representation, a `u64` the runtime understands, and neither is ours to
/// implement `From` between. Anything this program calls that still returns
/// the `solana_program` one, mostly the SPL token account parsing, comes back
/// through here.
#[inline(always)]
pub fn to_program_error(error: solana_program::program_error::ProgramError) -> ProgramError {
    ProgramError::from(u64::from(error))
}

/// Borsh and the other `std::io::Error` producers.
#[inline(always)]
pub fn io_to_program_error(_error: std::io::Error) -> ProgramError {
    ProgramError::BorshIoError
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, size_of};

    #[test]
    fn the_two_key_types_are_the_same_bytes() {
        assert_eq!(size_of::<Pubkey>(), size_of::<RawKey>());
        assert_eq!(align_of::<Pubkey>(), align_of::<RawKey>());
        let raw: RawKey = [7u8; 32];
        assert_eq!(as_pubkey(&raw).to_bytes(), raw);
        let key: Pubkey = Pubkey::new_unique();
        assert_eq!(*as_raw_key(&key), key.to_bytes());
    }
}
