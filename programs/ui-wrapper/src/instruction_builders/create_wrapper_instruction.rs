use crate::{wrapper_user::ManifestWrapperUserFixed, ManifestWrapperInstruction};
use pinocchio::error::ProgramError;
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    rent::Rent,
    system_instruction, system_program,
};

pub fn create_wrapper_instructions(
    payer: &Pubkey,
    owner: &Pubkey,
    wrapper_state: &Pubkey,
) -> Result<Vec<Instruction>, ProgramError> {
    let space: usize = std::mem::size_of::<ManifestWrapperUserFixed>();
    Ok(vec![
        system_instruction::create_account(
            payer,
            wrapper_state,
            Rent::default().minimum_balance(space),
            space as u64,
            &crate::id(),
        ),
        create_wrapper_instruction(payer, owner, wrapper_state),
    ])
}

fn create_wrapper_instruction(
    payer: &Pubkey,
    owner: &Pubkey,
    wrapper_state: &Pubkey,
) -> Instruction {
    Instruction {
        program_id: crate::id(),
        accounts: vec![
            AccountMeta::new(*owner, true),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new(*payer, true),
            AccountMeta::new(*wrapper_state, true),
        ],
        data: [ManifestWrapperInstruction::CreateWrapper.to_vec()].concat(),
    }
}
