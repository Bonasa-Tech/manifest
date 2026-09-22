use crate::program::ManifestInstruction;
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use solana_sdk_ids::system_program;

pub fn global_add_trader_instruction(global: &Pubkey, payer: &Pubkey) -> Instruction {
    Instruction {
        program_id: crate::id(),
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(*global, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data: [ManifestInstruction::GlobalAddTrader.to_vec()].concat(),
    }
}
