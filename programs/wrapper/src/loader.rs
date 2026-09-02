use std::{mem::size_of, ops::Deref};

use crate::wrapper_state::ManifestWrapperStateFixed;
use hypertree::get_helper;
use manifest::{require, validation::AccountInfoExt};
use pinocchio::{
    account_info::{AccountInfo, Ref, RefMut},
    program_error::ProgramError,
};
use solana_program::pubkey::Pubkey;

#[derive(Clone)]
pub struct WrapperStateAccountInfo<'a> {
    pub(crate) info: &'a AccountInfo,
}

pub const WRAPPER_STATE_DISCRIMINANT: u64 = 1;

impl<'a> WrapperStateAccountInfo<'a> {
    #[inline(always)]
    fn _new_unchecked(info: &'a AccountInfo) -> Result<WrapperStateAccountInfo<'a>, ProgramError> {
        require!(
            info.owner_pubkey() == &crate::ID,
            ProgramError::IllegalOwner,
            "Wrapper must be owned by the program",
        )?;
        Ok(Self { info })
    }

    pub fn new(info: &'a AccountInfo) -> Result<WrapperStateAccountInfo<'a>, ProgramError> {
        let wrapper_state: WrapperStateAccountInfo<'a> = Self::_new_unchecked(info)?;

        let wrapper_bytes: Ref<[u8]> = info.try_borrow_data()?;
        let (header_bytes, _) = wrapper_bytes.split_at(size_of::<ManifestWrapperStateFixed>());
        let header: &ManifestWrapperStateFixed =
            get_helper::<ManifestWrapperStateFixed>(header_bytes, 0_u32);

        require!(
            header.discriminant == WRAPPER_STATE_DISCRIMINANT,
            ProgramError::InvalidAccountData,
            "Invalid wrapper state discriminant",
        )?;

        Ok(wrapper_state)
    }

    pub fn new_init(info: &'a AccountInfo) -> Result<WrapperStateAccountInfo<'a>, ProgramError> {
        require!(
            info.is_signer(),
            ProgramError::MissingRequiredSignature,
            "Wrapper state must sign initialization",
        )?;
        let wrapper_bytes: Ref<[u8]> = info.try_borrow_data()?;
        let (header_bytes, _) = wrapper_bytes.split_at(size_of::<ManifestWrapperStateFixed>());
        let header: &ManifestWrapperStateFixed =
            get_helper::<ManifestWrapperStateFixed>(header_bytes, 0_u32);
        require!(
            info.owner_pubkey() == &crate::ID,
            ProgramError::IllegalOwner,
            "Wrapper must be owned by the Manifest program",
        )?;
        // On initialization, the discriminant is not set yet.
        require!(
            header.discriminant == 0,
            ProgramError::InvalidAccountData,
            "Expected uninitialized wrapper with discriminant 0",
        )?;
        Ok(Self { info })
    }
}

impl<'a> Deref for WrapperStateAccountInfo<'a> {
    type Target = AccountInfo;

    fn deref(&self) -> &Self::Target {
        self.info
    }
}

pub(crate) fn check_signer(wrapper_state: &WrapperStateAccountInfo, owner_key: &Pubkey) {
    let mut wrapper_data: RefMut<[u8]> = wrapper_state.info.try_borrow_mut_data().unwrap();
    let (header_bytes, _wrapper_dynamic_data) =
        wrapper_data.split_at_mut(size_of::<ManifestWrapperStateFixed>());
    let header: &ManifestWrapperStateFixed =
        get_helper::<ManifestWrapperStateFixed>(header_bytes, 0_u32);
    assert_eq!(header.trader, *owner_key);
}
