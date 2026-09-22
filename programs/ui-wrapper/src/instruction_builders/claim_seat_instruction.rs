use crate::ManifestWrapperInstruction;
use solana_program::{instruction::{AccountMeta, Instruction}, pubkey::Pubkey};
use solana_sdk_ids::system_program;

pub fn claim_seat_instruction(
    market: &Pubkey,
    payer: &Pubkey,
    owner: &Pubkey,
    wrapper_state: &Pubkey,
) -> Instruction {
    Instruction {
        program_id: crate::id(),
        accounts: vec![
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(*owner, true),
            AccountMeta::new(*market, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new(*payer, true),
            AccountMeta::new(*wrapper_state, false),
        ],
        data: ManifestWrapperInstruction::ClaimSeat.to_vec(),
    }
}
