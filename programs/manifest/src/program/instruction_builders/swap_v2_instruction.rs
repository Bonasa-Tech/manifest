use crate::{
    program::{swap::SwapParams, ManifestInstruction},
    validation::{get_global_address, get_global_vault_address, get_vault_address},
};
use borsh::BorshSerialize;
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

#[allow(clippy::too_many_arguments)]
pub fn swap_v2_instruction(
    market: &Pubkey,
    payer: &Pubkey,
    owner: &Pubkey,
    base_mint: &Pubkey,
    quote_mint: &Pubkey,
    trader_base_account: &Pubkey,
    trader_quote_account: &Pubkey,
    in_atoms: u64,
    out_atoms: u64,
    is_base_in: bool,
    is_exact_in: bool,
    token_program_base: Pubkey,
    token_program_quote: Pubkey,
    include_global: bool,
) -> Instruction {
    let (vault_base_account, _) = get_vault_address(market, base_mint);
    let (vault_quote_account, _) = get_vault_address(market, quote_mint);
    let mut account_metas: Vec<AccountMeta> = vec![
        AccountMeta::new_readonly(*payer, true),
        AccountMeta::new_readonly(*owner, true),
        AccountMeta::new(*market, false),
        AccountMeta::new_readonly(solana_sdk_ids::system_program::id(), false),
        AccountMeta::new(*trader_base_account, false),
        AccountMeta::new(*trader_quote_account, false),
        AccountMeta::new(vault_base_account, false),
        AccountMeta::new(vault_quote_account, false),
        AccountMeta::new_readonly(token_program_base, false),
    ];
    if token_program_base == spl_token_2022::id() {
        account_metas.push(AccountMeta::new_readonly(*base_mint, false))
    }
    if token_program_base != token_program_quote {
        account_metas.push(AccountMeta::new_readonly(token_program_quote, false))
    }
    if token_program_quote == spl_token_2022::id() {
        account_metas.push(AccountMeta::new(*quote_mint, false))
    }
    if include_global {
        let global_mint: &Pubkey = if is_base_in { quote_mint } else { base_mint };
        let (global, _) = get_global_address(global_mint);
        let (global_vault, _) = get_global_vault_address(global_mint);
        account_metas.push(AccountMeta::new(global, false));
        account_metas.push(AccountMeta::new(global_vault, false));
    }

    Instruction {
        program_id: crate::id(),
        accounts: account_metas,
        data: [
            ManifestInstruction::Swap.to_vec(),
            SwapParams::new(in_atoms, out_atoms, is_base_in, is_exact_in)
                .try_to_vec()
                .unwrap(),
        ]
        .concat(),
    }
}
