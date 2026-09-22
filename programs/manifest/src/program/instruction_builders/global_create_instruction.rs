use crate::{
    program::ManifestInstruction,
    validation::{get_global_address, get_global_vault_address},
};
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use solana_sdk_ids::system_program;

pub fn create_global_instruction(
    mint: &Pubkey,
    payer: &Pubkey,
    token_program: &Pubkey,
) -> Instruction {
    let (global, _) = get_global_address(mint);
    let (global_vault, _) = get_global_vault_address(mint);
    Instruction {
        program_id: crate::id(),
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(global, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new(global_vault, false),
            AccountMeta::new_readonly(*token_program, false),
        ],
        data: [ManifestInstruction::GlobalCreate.to_vec()].concat(),
    }
}
