use crate::{
    program::{create_market::CreateMarketParams, ManifestInstruction},
    validation::{get_market_address, get_vault_address},
};
use borsh::BorshSerialize;
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    system_program,
};

/// Creates a market at the PDA derived from base_mint_index and quote mint.
/// The market account is created inside the program via invoke_signed.
pub fn create_market_instructions(
    base_mint_index: u8,
    base_mint_decimals: u8,
    quote_mint: &Pubkey,
    market_creator: &Pubkey,
    initial_margin_bps: u64,
    maintenance_margin_bps: u64,
    pyth_feed_account: Pubkey,
) -> Vec<Instruction> {
    let (market, _) = get_market_address(base_mint_index, quote_mint);
    vec![create_market_instruction(
        &market,
        quote_mint,
        market_creator,
        base_mint_index,
        base_mint_decimals,
        initial_margin_bps,
        maintenance_margin_bps,
        pyth_feed_account,
    )]
}

#[allow(clippy::too_many_arguments)]
pub fn create_market_instruction(
    market: &Pubkey,
    quote_mint: &Pubkey,
    market_creator: &Pubkey,
    base_mint_index: u8,
    base_mint_decimals: u8,
    initial_margin_bps: u64,
    maintenance_margin_bps: u64,
    pyth_feed_account: Pubkey,
) -> Instruction {
    let (quote_vault, _) = get_vault_address(market, quote_mint);
    Instruction {
        program_id: crate::id(),
        accounts: vec![
            AccountMeta::new(*market_creator, true),
            AccountMeta::new(*market, false),
            AccountMeta::new_readonly(*quote_mint, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(spl_token_2022::id(), false),
        ],
        data: [
            ManifestInstruction::CreateMarket.to_vec(),
            CreateMarketParams::new(
                base_mint_index,
                base_mint_decimals,
                initial_margin_bps,
                maintenance_margin_bps,
                pyth_feed_account,
            )
            .try_to_vec()
            .unwrap(),
        ]
        .concat(),
    }
}
