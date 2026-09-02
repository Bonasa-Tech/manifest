use crate::{require, validation::AccountInfoExt};
use pinocchio::{account_info::AccountInfo, program_error::ProgramError};
use solana_program::{pubkey::Pubkey, system_program};
use std::ops::Deref;

#[derive(Clone)]
pub struct Program<'a> {
    pub info: &'a AccountInfo,
}

impl<'a> Program<'a> {
    pub fn new(
        info: &'a AccountInfo,
        expected_program_id: &Pubkey,
    ) -> Result<Program<'a>, ProgramError> {
        require!(
            info.pubkey() == expected_program_id,
            ProgramError::IncorrectProgramId,
            "Incorrect program id expected {:?} actual {:?}",
            expected_program_id,
            info.pubkey()
        )?;
        Ok(Self { info })
    }
}

impl<'a> AsRef<AccountInfo> for Program<'a> {
    fn as_ref(&self) -> &AccountInfo {
        self.info
    }
}

#[derive(Clone)]
pub struct TokenProgram<'a> {
    pub info: &'a AccountInfo,
}

impl<'a> TokenProgram<'a> {
    pub fn new(info: &'a AccountInfo) -> Result<TokenProgram<'a>, ProgramError> {
        require!(
            *info.pubkey() == spl_token::id() || *info.pubkey() == spl_token_2022::id(),
            ProgramError::IncorrectProgramId,
            "Incorrect token program id: {:?}",
            info.pubkey()
        )?;
        Ok(Self { info })
    }
}

impl<'a> AsRef<AccountInfo> for TokenProgram<'a> {
    fn as_ref(&self) -> &AccountInfo {
        self.info
    }
}

impl<'a> Deref for TokenProgram<'a> {
    type Target = AccountInfo;

    fn deref(&self) -> &Self::Target {
        self.info
    }
}

#[derive(Clone)]
pub struct Signer<'a> {
    pub info: &'a AccountInfo,
}

impl<'a> Signer<'a> {
    pub fn new(info: &'a AccountInfo) -> Result<Signer<'a>, ProgramError> {
        require!(
            info.is_signer(),
            ProgramError::MissingRequiredSignature,
            "Missing required signature for {:?}",
            info.pubkey()
        )?;
        Ok(Self { info })
    }

    pub fn new_payer(info: &'a AccountInfo) -> Result<Signer<'a>, ProgramError> {
        require!(
            info.is_writable(),
            ProgramError::InvalidInstructionData,
            "Payer is not writable. Key {:?}",
            info.pubkey()
        )?;
        require!(
            info.is_signer(),
            ProgramError::MissingRequiredSignature,
            "Missing required signature for payer {:?}",
            info.pubkey()
        )?;
        Ok(Self { info })
    }
}

impl<'a> AsRef<AccountInfo> for Signer<'a> {
    fn as_ref(&self) -> &AccountInfo {
        self.info
    }
}

impl<'a> Deref for Signer<'a> {
    type Target = AccountInfo;

    fn deref(&self) -> &Self::Target {
        self.info
    }
}

#[derive(Clone)]
pub struct EmptyAccount<'a> {
    pub info: &'a AccountInfo,
}

impl<'a> EmptyAccount<'a> {
    pub fn new(info: &'a AccountInfo) -> Result<EmptyAccount<'a>, ProgramError> {
        require!(
            info.data_is_empty(),
            ProgramError::InvalidAccountData,
            "Account must be uninitialized {:?}",
            info.pubkey()
        )?;
        require!(
            info.owned_by(&system_program::id()),
            ProgramError::IllegalOwner,
            "Empty accounts must be owned by the system program {:?}",
            info.pubkey()
        )?;
        Ok(Self { info })
    }
}

impl<'a> AsRef<AccountInfo> for EmptyAccount<'a> {
    fn as_ref(&self) -> &AccountInfo {
        self.info
    }
}
