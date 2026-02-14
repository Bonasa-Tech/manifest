use num_enum::TryFromPrimitive;
use shank::ShankInstruction;

/// Instructions available for the Manifest program
#[repr(u8)]
#[derive(TryFromPrimitive, Debug, Copy, Clone, ShankInstruction, PartialEq, Eq)]
#[rustfmt::skip]
pub enum ManifestInstruction {
    /// Create a market
    #[account(0, writable, signer, name = "payer", desc = "Payer")]
    #[account(1, writable, name = "market", desc = "Account holding all market state")]
    #[account(2, name = "system_program", desc = "System program")]
    #[account(3, name = "base_mint", desc = "Base mint")]
    #[account(4, name = "quote_mint", desc = "Quote mint")]
    #[account(5, writable, name = "base_vault", desc = "Base vault PDA, seeds are [b'vault', market, base_mint]")]
    #[account(6, writable, name = "quote_vault", desc = "Quote vault PDA, seeds are [b'vault', market, quote_mint]")]
    #[account(7, name = "token_program", desc = "Token program")]
    // Always include both token programs so we can initialize both types of token vaults if needed.
    #[account(8, name = "token_program_22", desc = "Token program 22")]
    CreateMarket = 0,

    /// Allocate a seat
    #[account(0, writable, signer, name = "payer", desc = "Payer")]
    #[account(1, writable, name = "market", desc = "Account holding all market state")]
    #[account(2, name = "system_program", desc = "System program")]
    ClaimSeat = 1,

    /// Deposit
    #[account(0, writable, signer, name = "payer", desc = "Payer")]
    #[account(1, writable, name = "market", desc = "Account holding all market state")]
    #[account(2, writable, name = "trader_token", desc = "Trader token account")]
    #[account(3, writable, name = "vault", desc = "Vault PDA, seeds are [b'vault', market, mint]")]
    #[account(4, name = "token_program", desc = "Token program(22), should be the version that aligns with the token being used")]
    #[account(5, name = "mint", desc = "Required for token22 transfer_checked")]
    Deposit = 2,

    /// Withdraw
    #[account(0, writable, signer, name = "payer", desc = "Payer")]
    #[account(1, writable, name = "market", desc = "Account holding all market state")]
    #[account(2, writable, name = "trader_token", desc = "Trader token account")]
    #[account(3, writable, name = "vault", desc = "Vault PDA, seeds are [b'vault', market, mint]")]
    #[account(4, name = "token_program", desc = "Token program(22), should be the version that aligns with the token being used")]
    #[account(5, name = "mint", desc = "Required for token22 transfer_checked")]
    Withdraw = 3,

    /// Places an order using funds in a wallet instead of on deposit
    #[account(0, writable, signer, name = "payer", desc = "Payer")]
    #[account(1, writable, name = "market", desc = "Account holding all market state")]
    #[account(2, name = "system_program", desc = "System program")]
    #[account(3, writable, name = "trader_base", desc = "Trader base token account")]
    #[account(4, writable, name = "trader_quote", desc = "Trader quote token account")]
    #[account(5, writable, name = "base_vault", desc = "Base vault PDA, seeds are [b'vault', market_address, base_mint]")]
    #[account(6, writable, name = "quote_vault", desc = "Quote vault PDA, seeds are [b'vault', market_address, quote_mint]")]
    #[account(7, name = "token_program_base", desc = "Token program(22) base")]
    #[account(8, optional, name = "base_mint", desc = "Base mint, only included if base is Token22, otherwise not required")]
    #[account(9, optional, name = "token_program_quote", desc = "Token program(22) quote. Optional. Only include if different from base")]
    #[account(10, optional, name = "quote_mint", desc = "Quote mint, only included if base is Token22, otherwise not required")]
    #[account(11, writable, optional, name = "global", desc = "Global account")]
    #[account(12, writable, optional, name = "global_vault", desc = "Global vault")]
    Swap = 4,

    /// Expand a market.
    /// 
    /// This is not used in normal operations because expansion happens within
    /// instructions that could require it.
    /// This is useful for when rent payer != transaction signer.
    #[account(0, writable, signer, name = "payer", desc = "Payer")]
    #[account(1, writable, name = "market", desc = "Account holding all market state")]
    #[account(2, name = "system_program", desc = "System program")]
    Expand = 5,

    /// Batch update with multiple place orders and cancels.
    #[account(0, writable, signer, name = "payer", desc = "Payer")]
    #[account(1, writable, name = "market", desc = "Account holding all market state")]
    #[account(2, name = "system_program", desc = "System program")]
    #[account(3, optional, name = "base_mint", desc = "Mint for the base global account")]
    #[account(4, optional, writable, name = "base_global", desc = "Base global account")]
    #[account(5, optional, name = "base_global_vault", desc = "Base global vault")]
    #[account(6, optional, name = "base_market_vault", desc = "Base market vault")]
    #[account(7, optional, name = "base_token_program", desc = "Token program(22)")]
    #[account(8, optional, name = "quote_mint", desc = "Mint for this global account")]
    #[account(9, optional, writable, name = "quote_global", desc = "Quote global account")]
    #[account(10, optional, name = "quote_global_vault", desc = "Quote global vault")]
    #[account(11, optional, name = "quote_market_vault", desc = "Quote market vault")]
    #[account(12, optional, name = "quote_token_program", desc = "Token program(22)")]
    BatchUpdate = 6,

    /// Create global account for a given token.
    #[account(0, writable, signer, name = "payer", desc = "Payer")]
    #[account(1, writable, name = "global", desc = "Global account")]
    #[account(2, name = "system_program", desc = "System program")]
    #[account(3, name = "mint", desc = "Mint for this global account")]
    #[account(4, writable, name = "global_vault", desc = "Global vault")]
    #[account(5, name = "token_program", desc = "Token program(22)")]
    GlobalCreate = 7,

    /// Add a trader to the global account.
    #[account(0, writable, signer, name = "payer", desc = "Payer")]
    #[account(1, writable, name = "global", desc = "Global account")]
    #[account(2, name = "system_program", desc = "System program")]
    GlobalAddTrader = 8,

    /// Deposit into global account for a given token.
    #[account(0, writable, signer, name = "payer", desc = "Payer")]
    #[account(1, writable, name = "global", desc = "Global account")]
    #[account(2, name = "mint", desc = "Mint for this global account")]
    #[account(3, writable, name = "global_vault", desc = "Global vault")]
    #[account(4, writable, name = "trader_token", desc = "Trader token account")]
    #[account(5, name = "token_program", desc = "Token program(22)")]
    GlobalDeposit = 9,

    /// Withdraw from global account for a given token.
    #[account(0, writable, signer, name = "payer", desc = "Payer")]
    #[account(1, writable, name = "global", desc = "Global account")]
    #[account(2, name = "mint", desc = "Mint for this global account")]
    #[account(3, writable, name = "global_vault", desc = "Global vault")]
    #[account(4, writable, name = "trader_token", desc = "Trader token account")]
    #[account(5, name = "token_program", desc = "Token program(22)")]
    GlobalWithdraw = 10,

    /// Evict another trader from the global account.
    #[account(0, writable, signer, name = "payer", desc = "Payer")]
    #[account(1, writable, name = "global", desc = "Global account")]
    #[account(2, name = "mint", desc = "Mint for this global account")]
    #[account(3, writable, name = "global_vault", desc = "Global vault")]
    #[account(4, name = "trader_token", desc = "Trader token account")]
    #[account(5, name = "evictee_token", desc = "Evictee token account")]
    #[account(6, name = "token_program", desc = "Token program(22)")]
    GlobalEvict = 11,

    /// Removes an order from a market that cannot be filled. There are 3
    /// reasons. It is expired, the global trader got evicted, or the global
    /// trader no longer has deposited the funds to back the order. This
    /// function results in cleaner orderbooks which helps reduce variance and
    /// thus compute unit estimates for traders. It is incentivized by receiving
    /// gas prepayment deposits. This is not required for normal operation of
    /// market. It exists as a deterrent to unfillable and unmaintained global
    /// spam.
    #[account(0, writable, signer, name = "payer", desc = "Payer for this tx, receiver of rent deposit")]
    #[account(1, writable, name = "market", desc = "Account holding all market state")]
    #[account(2, name = "system_program", desc = "System program")]
    #[account(3, writable, name = "global", desc = "Global account")]
    GlobalClean = 12,

    
    /// Places an order using funds in a wallet instead of on deposit. Separates
    /// the owner of the token accounts and the payer. This allows routers to
    /// swap and have intermediate hops go through PDAs, rather than all token
    /// accounts owned by the user.
    #[account(0, writable, signer, name = "payer", desc = "Payer")]
    #[account(1, writable, signer, name = "owner", desc = "Owner")]
    #[account(2, writable, name = "market", desc = "Account holding all market state")]
    #[account(3, name = "system_program", desc = "System program")]
    #[account(4, writable, name = "trader_base", desc = "Trader base token account")]
    #[account(5, writable, name = "trader_quote", desc = "Trader quote token account")]
    #[account(6, writable, name = "base_vault", desc = "Base vault PDA, seeds are [b'vault', market_address, base_mint]")]
    #[account(7, writable, name = "quote_vault", desc = "Quote vault PDA, seeds are [b'vault', market_address, quote_mint]")]
    #[account(8, name = "token_program_base", desc = "Token program(22) base")]
    #[account(9, optional, name = "base_mint", desc = "Base mint, only included if base is Token22, otherwise not required")]
    #[account(10, optional, name = "token_program_quote", desc = "Token program(22) quote. Optional. Only include if different from base")]
    #[account(11, optional, name = "quote_mint", desc = "Quote mint, only included if base is Token22, otherwise not required")]
    #[account(12, writable, optional, name = "global", desc = "Global account")]
    #[account(13, writable, optional, name = "global_vault", desc = "Global vault")]
    SwapV2 = 13,

    /// Delegate market account to MagicBlock ephemeral rollups.
    #[account(0, writable, signer, name = "payer", desc = "Payer and market creator")]
    #[account(1, writable, name = "market", desc = "Market PDA to delegate")]
    #[account(2, name = "owner_program", desc = "Manifest program (owner of the PDA)")]
    #[account(3, name = "delegation_program", desc = "MagicBlock delegation program")]
    #[account(4, writable, name = "delegation_record", desc = "Delegation record PDA")]
    #[account(5, writable, name = "delegation_metadata", desc = "Delegation metadata PDA")]
    #[account(6, name = "system_program", desc = "System program")]
    #[account(7, writable, name = "buffer", desc = "Buffer account for delegation")]
    DelegateMarket = 14,

    /// Commit delegated market state back to mainnet.
    #[account(0, writable, signer, name = "payer", desc = "Payer")]
    #[account(1, writable, name = "market", desc = "Delegated market account")]
    #[account(2, name = "magic_program", desc = "MagicBlock magic program")]
    #[account(3, name = "magic_context", desc = "MagicBlock magic context")]
    CommitMarket = 15,

    /// Liquidate an underwater perps position.
    #[account(0, writable, signer, name = "liquidator", desc = "Liquidator")]
    #[account(1, writable, name = "market", desc = "Perps market account")]
    #[account(2, name = "system_program", desc = "System program")]
    Liquidate = 16,

    /// Crank funding rate using oracle price.
    #[account(0, writable, signer, name = "payer", desc = "Payer / cranker")]
    #[account(1, writable, name = "market", desc = "Perps market account")]
    #[account(2, name = "pyth_price_feed", desc = "Pyth price feed account")]
    CrankFunding = 17,
}

impl ManifestInstruction {
    pub fn to_vec(&self) -> Vec<u8> {
        vec![*self as u8]
    }
}

#[test]
fn test_instruction_serialization() {
    let num_instructions: u8 = 17;
    for i in 0..=255 {
        let instruction: ManifestInstruction = match ManifestInstruction::try_from(i) {
            Ok(j) => {
                assert!(i <= num_instructions);
                j
            }
            Err(_) => {
                assert!(i > num_instructions);
                continue;
            }
        };
        assert_eq!(instruction as u8, i);
    }
}

#[test]
fn test_to_vec() {
    let create_market_ix = ManifestInstruction::CreateMarket;
    let vec = create_market_ix.to_vec();
    assert_eq!(*vec.first().unwrap(), 0);
}
