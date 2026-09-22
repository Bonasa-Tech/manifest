use crate::{wrapper_state::ManifestWrapperStateFixed, ManifestWrapperInstruction};
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    sysvar::{rent::Rent, slot_history::ProgramError},
};
use solana_sdk_ids::system_program;
use solana_system_interface::instruction as system_instruction;

pub fn create_wrapper_instructions(
    owner: &Pubkey,
    wrapper_state: &Pubkey,
) -> Result<Vec<Instruction>, ProgramError> {
    let space: usize = std::mem::size_of::<ManifestWrapperStateFixed>();
    Ok(vec![
        system_instruction::create_account(
            owner,
            wrapper_state,
            Rent::default().minimum_balance(space),
            space as u64,
            &crate::id(),
        ),
        create_wrapper_instruction(owner, wrapper_state),
    ])
}

fn create_wrapper_instruction(owner: &Pubkey, wrapper_state: &Pubkey) -> Instruction {
    Instruction {
        program_id: crate::id(),
        accounts: vec![
            AccountMeta::new(*owner, true),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new(*wrapper_state, true),
        ],
        data: [ManifestWrapperInstruction::CreateWrapper.to_vec()].concat(),
    }
}
