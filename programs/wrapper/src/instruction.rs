use num_enum::TryFromPrimitive;
use shank::ShankInstruction;

/// Instructions available for the Manifest wrapper program
#[repr(u8)]
#[derive(TryFromPrimitive, Debug, Copy, Clone, ShankInstruction, PartialEq, Eq)]
#[rustfmt::skip]
pub enum ManifestWrapperInstruction {
    // Create a market is not needed for the wrapper

    /// Create a wrapper for owner.
    #[account(0, writable, signer, name = "owner", desc = "Owner of the Manifest account")]
    #[account(1, name = "system_program", desc = "System program")]
    #[account(2, writable, name = "wrapper_state", desc = "Wrapper state")]
    CreateWrapper= 0,

    /// Allocate a seat. Also initializes this wrapper state
    #[account(0, name = "manifest_program", desc = "Manifest program")]
    #[account(1, writable, signer, name = "owner", desc = "Owner of the Manifest account")]
    #[account(2, writable, name = "market", desc = "Account holding all market state")]
    #[account(3, name = "system_program", desc = "System program")]
    #[account(4, writable, name = "wrapper_state", desc = "Wrapper state")]
    ClaimSeat = 1,

    /// Deposit
    #[account(0, name = "manifest_program", desc = "Manifest program")]
    #[account(1, writable, signer, name = "owner", desc = "Owner of the Manifest account")]
    #[account(2, writable, name = "market", desc = "Account holding all market state")]
    #[account(3, writable, name = "trader_token_account", desc = "Trader token account")]
    #[account(4, writable, name = "vault", desc = "Vault PDA, seeds are [b'vault', market_address, mint_address]")]
    #[account(5, name = "token_program", desc = "Token program")]
    #[account(6, writable, name = "wrapper_state", desc = "Wrapper state")]
    #[account(7, name = "mint", desc = "Mint, needed for token 22")]
    Deposit = 2,

    /// Withdraw
    #[account(0, name = "manifest_program", desc = "Manifest program")]
    #[account(1, writable, signer, name = "owner", desc = "Owner of the Manifest account")]
    #[account(2, writable, name = "market", desc = "Account holding all market state")]
    #[account(3, writable, name = "trader_token_account", desc = "Trader token account")]
    #[account(4, writable, name = "vault", desc = "Vault PDA, seeds are [b'vault', market_address, mint_address]")]
    #[account(5, name = "token_program", desc = "Token program")]
    #[account(6, writable, name = "wrapper_state", desc = "Wrapper state")]
    #[account(7, name = "mint", desc = "Mint, needed for token 22")]
    Withdraw = 3,
    
    /// All global or not, optional. Only reason the separate instructions exist
    /// is so that solita can work without intermediate optional accounts.
    #[account(0, writable, name = "wrapper_state", desc = "Wrapper state")]
    #[account(1, name = "manifest_program", desc = "Manifest program")]
    #[account(2, writable, signer, name = "owner", desc = "Owner of the Manifest account")]
    #[account(3, writable, name = "market", desc = "Account holding all market state")]
    #[account(4, name = "system_program", desc = "System program")]
    #[account(5, optional, name = "base_mint", desc = "Mint for the base global account")]
    #[account(6, optional, writable, name = "base_global", desc = "Base global account")]
    #[account(7, optional, name = "base_global_vault", desc = "Base global vault")]
    #[account(8, optional, name = "base_market_vault", desc = "Base market vault")]
    #[account(9, optional, name = "base_token_program", desc = "Token program(22)")]
    #[account(10, optional, name = "quote_mint", desc = "Mint for this global account")]
    #[account(11, optional, writable, name = "quote_global", desc = "Quote global account")]
    #[account(12, optional, name = "quote_global_vault", desc = "Quote global vault")]
    #[account(13, optional, name = "quote_market_vault", desc = "Quote market vault")]
    #[account(14, optional, name = "quote_token_program", desc = "Token program(22)")]
    BatchUpdate = 4,

    /// BatchUpdate base global.
    #[account(0, writable, name = "wrapper_state", desc = "Wrapper state")]
    #[account(1, name = "manifest_program", desc = "Manifest program")]
    #[account(2, writable, signer, name = "owner", desc = "Owner of the Manifest account")]
    #[account(3, writable, name = "market", desc = "Account holding all market state")]
    #[account(4, name = "system_program", desc = "System program")]
    #[account(5, name = "base_mint", desc = "Mint for the base global account")]
    #[account(6, writable, name = "base_global", desc = "Base global account")]
    #[account(7, name = "base_global_vault", desc = "Base global vault")]
    #[account(8, name = "base_market_vault", desc = "Base market vault")]
    #[account(9, name = "base_token_program", desc = "Token program(22)")]
    BatchUpdateBaseGlobal = 5,

    /// BatchUpdate quote global.
    #[account(0, writable, name = "wrapper_state", desc = "Wrapper state")]
    #[account(1, name = "manifest_program", desc = "Manifest program")]
    #[account(2, writable, signer, name = "owner", desc = "Owner of the Manifest account")]
    #[account(3, writable, name = "market", desc = "Account holding all market state")]
    #[account(4, name = "system_program", desc = "System program")]
    #[account(5, name = "quote_mint", desc = "Mint for the quote global account")]
    #[account(6, writable, name = "quote_global", desc = "Quote global account")]
    #[account(7, name = "quote_global_vault", desc = "Quote global vault")]
    #[account(8, name = "quote_market_vault", desc = "Quote market vault")]
    #[account(9, name = "quote_token_program", desc = "Token program(22)")]
    BatchUpdateQuoteGlobal = 6,

    /// Collect.
    #[account(0, writable, name = "wrapper_state", desc = "Wrapper state")]
    #[account(1, name = "system_program", desc = "System program")]
    #[account(2, writable, signer, name = "collector", desc = "Fee collector")]
    Collect = 7,
}

impl ManifestWrapperInstruction {
    pub fn to_vec(&self) -> Vec<u8> {
        vec![*self as u8]
    }
}
