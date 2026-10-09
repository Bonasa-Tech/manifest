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
    // Nodes relocated in this run. `None` is unbounded. Reclaiming empty
    // seats is never capped; it costs no tree work.
    limit: Option<u32>,
) -> Instruction {
    let mut data: Vec<u8> = vec![ManifestInstruction::Defrag as u8];
    if let Some(limit) = limit {
        data.extend_from_slice(&limit.to_le_bytes());
    }
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
        data,
    }
}
