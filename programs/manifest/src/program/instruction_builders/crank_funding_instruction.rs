use crate::program::{crank_funding::CrankFundingParams, ManifestInstruction};
use borsh::BorshSerialize;
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

pub fn crank_funding_instruction(
    market: &Pubkey,
    payer: &Pubkey,
    pyth_price_feed: &Pubkey,
) -> Instruction {
    Instruction {
        program_id: crate::id(),
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(*market, false),
            AccountMeta::new_readonly(*pyth_price_feed, false),
        ],
        data: [
            ManifestInstruction::CrankFunding.to_vec(),
            CrankFundingParams::new().try_to_vec().unwrap(),
        ]
        .concat(),
    }
}
