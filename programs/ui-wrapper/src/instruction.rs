use num_enum::TryFromPrimitive;
use shank::ShankInstruction;

/// Instructions available for the Manifest ui-wrapper program
#[repr(u8)]
#[derive(TryFromPrimitive, Debug, Copy, Clone, ShankInstruction, PartialEq, Eq)]
#[rustfmt::skip]
pub enum ManifestWrapperInstruction {
    // Create a market is not needed for the wrapper

    /// Create and initialize a wrapper for owner. Note that owner and payer
    /// are separate to enable PDA owners.
    #[account(0, name = "owner", desc = "Owner of the Manifest account")]
    #[account(1, name = "system_program", desc = "System program")]
    #[account(2, writable, signer, name = "payer", desc = "Payer of rent and gas")]
    #[account(3, writable, signer, name = "wrapper_state", desc = "Wrapper state")]
    CreateWrapper = 0,

    /// Unused
    /// Allocate a seat on a given market, this adds a market info to the given
    /// wrapper.
    #[account(0, name = "manifest_program", desc = "Manifest program")]
    #[account(1, writable, signer, name = "owner", desc = "Owner of the Manifest account")]
    #[account(2, writable, name = "market", desc = "Account holding all market state")]
    #[account(3, name = "system_program", desc = "System program")]
    #[account(4, writable, signer, name = "payer", desc = "Payer of rent and gas")]
    #[account(5, writable, name = "wrapper_state", desc = "Wrapper state")]
    ClaimSeatUnused = 1,

    /// Place order, deposits additional funds needed.
    /// Syncs both balances and open orders on the wrapper.
    /// TODO: document return data
    /// TODO: Remove the unneeded global accounts on the bookside that I am placing on.
    #[account(0, writable, name = "wrapper_state", desc = "Wrapper state")]
    #[account(1, signer, name = "owner", desc = "Owner of the Manifest account")]
    #[account(2, writable, name = "trader_token_account", desc = "Trader token account")]
    #[account(3, writable, name = "market", desc = "Account holding all market state")]
    #[account(4, writable, name = "vault", desc = "Vault PDA, seeds are [b'vault', market_address, mint_address]")]
    #[account(5, writable, name = "mint", desc = "Mint of trader token account")]
    #[account(6, name = "system_program", desc = "System program")]
    #[account(7, name = "token_program", desc = "Token program owning trader token account")]
    #[account(8, name = "manifest_program", desc = "Manifest program")]
    #[account(9, writable, signer, name = "payer", desc = "Payer of rent and gas")]
    #[account(10, name = "base_mint", desc = "Base mint")]
    #[account(11, writable, name = "base_global", desc = "Base global account")]
    #[account(12, writable, name = "base_global_vault", desc = "Base global vault")]
    #[account(13, writable, name = "base_market_vault", desc = "Base market vault")]
    #[account(14, writable, name = "base_token_program", desc = "Base token program")]
    #[account(15, name = "quote_mint", desc = "Quote mint")]
    #[account(16, writable, name = "quote_global", desc = "Quote global account")]
    #[account(17, writable, name = "quote_global_vault", desc = "Quote global vault")]
    #[account(18, writable, name = "quote_market_vault", desc = "Quote market vault")]
    #[account(19, writable, name = "quote_token_program", desc = "Quote token program")]
    PlaceOrder = 2,

    /// Edit order, deposits additional funds needed. TODO: Not implemented yet
    /// programs/ui-wrapper/src/TODO: document return data
    /// TODO: Remove the unneeded global accounts on the bookside that I am placing on.
    #[account(0, writable, name = "wrapper_state", desc = "Wrapper state")]
    #[account(1, signer, name = "owner", desc = "Owner of the Manifest account")]
    #[account(2, writable, name = "trader_token_account", desc = "Trader token account")]
    #[account(3, writable, name = "market", desc = "Account holding all market state")]
    #[account(4, writable, name = "vault", desc = "Vault PDA, seeds are [b'vault', market_address, mint_address]")]
    #[account(5, writable, name = "mint", desc = "Mint of trader token account")]
    #[account(6, name = "system_program", desc = "System program")]
    #[account(7, name = "token_program", desc = "Token program owning trader token account")]
    #[account(8, name = "manifest_program", desc = "Manifest program")]
    #[account(9, writable, signer, name = "payer", desc = "Payer of rent and gas")]
    #[account(10, name = "base_mint", desc = "Base mint")]
    #[account(11, writable, name = "base_global", desc = "Base global account")]
    #[account(12, writable, name = "base_global_vault", desc = "Base global vault")]
    #[account(13, writable, name = "base_market_vault", desc = "Base market vault")]
    #[account(14, writable, name = "base_token_program", desc = "Base token program")]
    #[account(15, name = "quote_mint", desc = "Quote mint")]
    #[account(16, writable, name = "quote_global", desc = "Quote global account")]
    #[account(17, writable, name = "quote_global_vault", desc = "Quote global vault")]
    #[account(18, writable, name = "quote_market_vault", desc = "Quote market vault")]
    #[account(19, writable, name = "quote_token_program", desc = "Quote token program")]
    EditOrder = 3,

    /// Cancel order, no funds are transferred, but token accounts are passed
    /// writeable anyways as it cpis into manifest::BatchUpdate.
    /// Syncs the wrapper balances but not open orders.
    /// TODO: also sync open orders
    #[account(0, writable, name = "wrapper_state", desc = "Wrapper state")]
    #[account(1, signer, name = "owner", desc = "Owner of the Manifest account")]
    #[account(2, writable, name = "trader_token_account", desc = "Trader token account")]
    #[account(3, writable, name = "market", desc = "Account holding all market state")]
    #[account(4, writable, name = "vault", desc = "Vault PDA, seeds are [b'vault', market_address, mint_address]")]
    #[account(5, writable, name = "mint", desc = "Mint of trader token account")]
    #[account(6, name = "system_program", desc = "System program")]
    #[account(7, name = "token_program", desc = "Token program owning trader token account")]
    #[account(8, name = "manifest_program", desc = "Manifest program")]
    CancelOrder = 4,

    /// Settle withdrawable funds.
    /// Syncs both balances and open orders on the wrapper.
    /// Instruction also charges fees for UI platform and optional referral.
    /// Execution fails if the user can not pay the full amount of fees owed
    /// in quote currency.
    #[account(0, writable, name = "wrapper_state", desc = "Wrapper state")]
    #[account(1, signer, name = "owner", desc = "Owner of the Manifest account")]
    #[account(2, writable, name = "trader_token_account_base", desc = "Trader base token account")]
    #[account(3, writable, name = "trader_token_account_quote", desc = "Trader quote token account")]
    #[account(4, writable, name = "market", desc = "Account holding all market state")]
    #[account(5, writable, name = "vault_base", desc = "Base currency vault PDA, seeds are [b'vault', market_address, mint_address]")]
    #[account(6, writable, name = "vault_quote", desc = "Quote currency vault PDA, seeds are [b'vault', market_address, mint_address]")]
    #[account(7, writable, name = "mint_base", desc = "Mint of trader base token account")]
    #[account(8, writable, name = "mint_quote", desc = "Mint of trader quote token account")]
    #[account(9, name = "token_program_base", desc = "Token program for base token")]
    #[account(10, name = "token_program_quote", desc = "Token program for quote token")]
    #[account(11, name = "manifest_program", desc = "Manifest program")]
    #[account(12, signer, name = "fee_authority", desc = "Authority that authenticates fee policy and destinations")]
    #[account(13, writable, name = "platform_token_account", desc = "Platform fee token account")]
    #[account(14, writable, name = "referrer_token_account", desc = "Referrer fee token account", optional)]
    SettleFunds = 5,
}

impl ManifestWrapperInstruction {
    pub fn to_vec(&self) -> Vec<u8> {
        vec![*self as u8]
    }
}
