use crate::{program::ManifestInstruction, validation::get_vault_address};
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

pub fn defrag_instruction(
    market: &Pubkey,
    collector: &Pubkey,
    base_mint: &Pubkey,
    quote_mint: &Pubkey,
    base_token_program: Pubkey,
    quote_token_program: Pubkey,
) -> Instruction {
    Instruction {
        program_id: crate::id(),
        accounts: vec![
            AccountMeta::new(*collector, true),
            AccountMeta::new(*market, false),
            AccountMeta::new(get_vault_address(market, base_mint).0, false),
            AccountMeta::new(get_vault_address(market, quote_mint).0, false),
            AccountMeta::new_readonly(base_token_program, false),
            AccountMeta::new_readonly(quote_token_program, false),
            AccountMeta::new_readonly(solana_sdk_ids::system_program::id(), false),
        ],
        data: vec![ManifestInstruction::Defrag as u8],
    }
}
