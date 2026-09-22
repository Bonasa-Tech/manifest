use crate::program::ManifestInstruction;
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use solana_sdk_ids::system_program;

pub fn expand_market_instruction(market: &Pubkey, payer: &Pubkey) -> Instruction {
    Instruction {
        program_id: crate::id(),
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(*market, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data: [ManifestInstruction::Expand.to_vec()].concat(),
    }
}
