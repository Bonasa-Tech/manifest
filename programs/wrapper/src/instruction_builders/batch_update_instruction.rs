use crate::{
    instruction::ManifestWrapperInstruction,
    processors::batch_upate::{
        WrapperBatchUpdateParams, WrapperCancelOrderParams, WrapperPlaceOrderParams,
    },
};
use borsh::BorshSerialize;
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use solana_sdk_ids::system_program;

pub fn batch_update_instruction(
    market: &Pubkey,
    owner: &Pubkey,
    wrapper_state: &Pubkey,
    cancels: Vec<WrapperCancelOrderParams>,
    cancel_all: bool,
    orders: Vec<WrapperPlaceOrderParams>,
) -> Instruction {
    Instruction {
        program_id: crate::id(),
        accounts: vec![
            AccountMeta::new(*wrapper_state, false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(*owner, true),
            AccountMeta::new(*market, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data: [
            ManifestWrapperInstruction::BatchUpdate.to_vec(),
            WrapperBatchUpdateParams::new(cancels, cancel_all, orders)
                .try_to_vec()
                .unwrap(),
        ]
        .concat(),
    }
}
