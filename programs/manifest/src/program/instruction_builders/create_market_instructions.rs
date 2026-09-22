use crate::{
    program::ManifestInstruction, state::MarketFixed, validation::get_vault_address, ProgramError,
};
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    sysvar::rent::Rent,
};
use solana_sdk_ids::system_program;
use solana_system_interface::instruction as system_instruction;

/// Creates the account and populates it with rent.
pub fn create_market_instructions(
    market: &Pubkey,
    base_mint: &Pubkey,
    quote_mint: &Pubkey,
    market_creator: &Pubkey,
) -> Result<Vec<Instruction>, ProgramError> {
    let space: usize = std::mem::size_of::<MarketFixed>();
    Ok(vec![
        system_instruction::create_account(
            market_creator,
            market,
            Rent::default().minimum_balance(space),
            space as u64,
            &crate::id(),
        ),
        create_market_instruction(market, base_mint, quote_mint, market_creator),
    ])
}

pub fn create_market_instruction(
    market: &Pubkey,
    base_mint: &Pubkey,
    quote_mint: &Pubkey,
    market_creator: &Pubkey,
) -> Instruction {
    let (base_vault, _) = get_vault_address(market, base_mint);
    let (quote_vault, _) = get_vault_address(market, quote_mint);
    Instruction {
        program_id: crate::id(),
        accounts: vec![
            AccountMeta::new(*market_creator, true),
            AccountMeta::new(*market, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(*base_mint, false),
            AccountMeta::new_readonly(*quote_mint, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(spl_token_2022::id(), false),
        ],
        data: [ManifestInstruction::CreateMarket.to_vec()].concat(),
    }
}
