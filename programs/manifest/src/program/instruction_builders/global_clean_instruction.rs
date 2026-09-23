use crate::program::{global_clean::GlobalCleanParams, ManifestInstruction};
use borsh::BorshSerialize;
use hypertree::DataIndex;
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use solana_sdk_ids::system_program;

pub fn global_clean_instruction(
    global: &Pubkey,
    payer: &Pubkey,
    market: &Pubkey,
    order_index: DataIndex,
) -> Instruction {
    Instruction {
        program_id: crate::id(),
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(*market, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new(*global, false),
        ],
        data: [
            ManifestInstruction::GlobalClean.to_vec(),
            GlobalCleanParams::new(order_index).try_to_vec().unwrap(),
        ]
        .concat(),
    }
}
