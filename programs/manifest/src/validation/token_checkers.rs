use crate::{require, validation::AccountInfoExt};
use pinocchio::program_error::ProgramError;
use pinocchio::account_info::AccountInfo;
use solana_program::{pubkey::Pubkey};
use spl_token_2022::{
    check_spl_token_program_account, extension::StateWithExtensions, state::Mint,
};
use std::ops::Deref;

#[derive(Clone)]
pub struct MintAccountInfo<'a> {
    pub mint: Mint,
    pub info: &'a AccountInfo,
}

impl<'a> MintAccountInfo<'a> {
    pub fn new(info: &'a AccountInfo) -> Result<MintAccountInfo<'a>, ProgramError> {
        check_spl_token_program_account(info.owner_pubkey())?;

        let mint: Mint = StateWithExtensions::<Mint>::unpack(&info.try_borrow_data()?)?.base;

        Ok(Self { mint, info })
    }
}

impl<'a> AsRef<AccountInfo> for MintAccountInfo<'a> {
    fn as_ref(&self) -> &AccountInfo {
        self.info
    }
}

#[derive(Clone)]
pub struct TokenAccountInfo<'a> {
    pub info: &'a AccountInfo,
}

impl<'a> TokenAccountInfo<'a> {
    pub fn new(
        info: &'a AccountInfo,
        mint: &Pubkey,
    ) -> Result<TokenAccountInfo<'a>, ProgramError> {
        require!(
            info.owned_by(&spl_token::id()) || info.owned_by(&spl_token_2022::id()),
            ProgramError::IllegalOwner,
            "Token account must be owned by the Token Program",
        )?;
        // The mint key is found at offset 0 of the token account
        require!(
            &info.try_borrow_data()?[0..32] == mint.as_ref(),
            ProgramError::InvalidAccountData,
            "Token account mint mismatch",
        )?;
        Ok(Self { info })
    }

    pub fn get_owner(&self) -> Pubkey {
        Pubkey::new_from_array(
            self.info.try_borrow_data().unwrap()[32..64]
                .try_into()
                .unwrap(),
        )
    }

    pub fn get_balance_atoms(&self) -> u64 {
        u64::from_le_bytes(
            self.info.try_borrow_data().unwrap()[64..72]
                .try_into()
                .unwrap(),
        )
    }

    pub fn new_with_owner(
        info: &'a AccountInfo,
        mint: &Pubkey,
        owner: &Pubkey,
    ) -> Result<TokenAccountInfo<'a>, ProgramError> {
        let token_account_info = Self::new(info, mint)?;
        // The owner key is found at offset 32 of the token account
        require!(
            &info.try_borrow_data()?[32..64] == owner.as_ref(),
            ProgramError::IllegalOwner,
            "Token account owner mismatch",
        )?;
        Ok(token_account_info)
    }

    pub fn new_with_owner_and_key(
        info: &'a AccountInfo,
        mint: &Pubkey,
        owner: &Pubkey,
        key: &Pubkey,
    ) -> Result<TokenAccountInfo<'a>, ProgramError> {
        require!(
            info.pubkey() == key,
            ProgramError::InvalidInstructionData,
            "Invalid pubkey for Token Account {:?}",
            info.pubkey()
        )?;
        Self::new_with_owner(info, mint, owner)
    }
}

impl<'a> AsRef<AccountInfo> for TokenAccountInfo<'a> {
    fn as_ref(&self) -> &AccountInfo {
        self.info
    }
}

impl<'a> Deref for TokenAccountInfo<'a> {
    type Target = AccountInfo;

    fn deref(&self) -> &Self::Target {
        self.info
    }
}

#[macro_export]
macro_rules! market_vault_seeds {
    ( $market:expr, $mint:expr ) => {
        &[b"vault", $market.as_ref(), $mint.as_ref()]
    };
}

#[macro_export]
macro_rules! market_vault_seeds_with_bump {
    ( $market:expr, $mint:expr, $bump:expr ) => {
        &[&[b"vault", $market.as_ref(), $mint.as_ref(), &[$bump]]]
    };
}

#[macro_export]
macro_rules! global_vault_seeds {
    ( $mint:expr ) => {
        &[b"global-vault", $mint.as_ref()]
    };
}

#[macro_export]
macro_rules! global_vault_seeds_with_bump {
    ( $mint:expr, $bump:expr ) => {
        &[&[b"global-vault", $mint.as_ref(), &[$bump]]]
    };
}

pub fn get_vault_address(market: &Pubkey, mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(market_vault_seeds!(market, mint), &crate::ID)
}

pub fn get_global_vault_address(mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(global_vault_seeds!(mint), &crate::ID)
}
