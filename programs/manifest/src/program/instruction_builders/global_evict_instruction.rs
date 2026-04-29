use crate::{
    program::{global_deposit::GlobalDepositParams, ManifestInstruction},
    validation::{get_global_address, get_global_vault_address},
};
use borsh::BorshSerialize;
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    system_program,
};

pub fn global_evict_instruction(
    mint: &Pubkey,
    payer: &Pubkey,
    trader_token_account: &Pubkey,
    evictee_token_account: &Pubkey,
    token_program: &Pubkey,
    num_atoms: u64,
) -> Instruction {
    let (global, _global_bump) = get_global_address(mint);
    let (global_vault, _global_vault_bump) = get_global_vault_address(mint);
    Instruction {
        program_id: crate::id(),
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(global, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new(global_vault, false),
            AccountMeta::new(*trader_token_account, false),
            AccountMeta::new(*evictee_token_account, false),
            AccountMeta::new_readonly(*token_program, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data: [
            ManifestInstruction::GlobalEvict.to_vec(),
            GlobalDepositParams::new(num_atoms).try_to_vec().unwrap(),
        ]
        .concat(),
    }
}
