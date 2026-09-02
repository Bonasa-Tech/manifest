use pinocchio::program_error::ProgramError;
use thiserror::Error;

#[derive(Debug, Error)]
#[repr(u32)]
pub enum ManifestWrapperError {
    #[error("Invalid deposit accounts error")]
    InvalidDepositAccounts = 0,
}

impl From<ManifestWrapperError> for ProgramError {
    fn from(e: ManifestWrapperError) -> Self {
        ProgramError::Custom(e as u32)
    }
}
