use pinocchio::program_error::ProgramError;
use thiserror::Error;

#[derive(Debug, Error)]
#[repr(u32)]
pub enum ManifestError {
    #[error("Invalid market parameters error")]
    InvalidMarketParameters = 0,
    #[error("Invalid deposit accounts error")]
    InvalidDepositAccounts = 1,
    #[error("Invalid withdraw accounts error")]
    InvalidWithdrawAccounts = 2,
    #[error("Invalid cancel error")]
    InvalidCancel = 3,
    #[error("Internal free list corruption error")]
    InvalidFreeList = 4,
    #[error("Cannot claim a second seat for the same trader")]
    AlreadyClaimedSeat = 5,
    #[error("Matched on a post only order")]
    PostOnlyCrosses = 6,
    #[error("New order is already expired")]
    AlreadyExpired = 7,
    #[error("Less than minimum out amount")]
    InsufficientOut = 8,
    #[error("Invalid place order from wallet params")]
    InvalidPlaceOrderFromWalletParams = 9,
    #[error("Index hint did not match actual index")]
    WrongIndexHintParams = 10,
    #[error("Price is not positive")]
    PriceNotPositive = 11,
    #[error("Order settlement would overflow")]
    OrderWouldOverflow = 12,
    #[error("Order is too small to settle any value")]
    OrderTooSmall = 13,
    #[error("Overflow in token addition")]
    Overflow = 14,
    #[error("Missing Global account")]
    MissingGlobal = 15,
    #[error("Insufficient funds on global account to rest an order")]
    GlobalInsufficient = 16,
    #[error("Account key did not match expected")]
    IncorrectAccount = 17,
    #[error("Mint not allowed for market")]
    InvalidMint = 18,
    #[error("Cannot claim a new global seat, use evict")]
    TooManyGlobalSeats = 19,
    #[error("Can only evict the lowest depositor")]
    InvalidEvict = 20,
    #[error("Tried to clean order that was not eligible to be cleaned")]
    InvalidClean = 21,
}

impl From<ManifestError> for ProgramError {
    fn from(e: ManifestError) -> Self {
        ProgramError::Custom(e as u32)
    }
}

#[cfg(feature = "certora")]
#[macro_export]
macro_rules! require {
    ($test:expr, $err:expr, $($arg:tt)*) => {{
        ::cvt::cvt_assume!($test);
        Ok::<(), crate::ProgramError>(())
    }};
}

#[cfg(not(feature = "certora"))]
#[macro_export]
macro_rules! require {
  ($test:expr, $err:expr, $($arg:tt)*) => {
    if $test {
        Ok(())
    } else {
        #[cfg(target_os = "solana")]
        solana_program::msg!("[{}:{}] {}", std::file!(), std::line!(), std::format_args!($($arg)*));
        #[cfg(not(target_os = "solana"))]
        std::println!("[{}:{}] {}", std::file!(), std::line!(), std::format_args!($($arg)*));
        Err(($err))
    }
  };
}
