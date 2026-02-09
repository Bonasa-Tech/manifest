use std::cell::RefMut;

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{account_info::AccountInfo, entrypoint::ProgramResult, pubkey::Pubkey};

use crate::{
    logs::{emit_stack, GlobalDepositLog},
    program::get_mut_dynamic_account,
    quantities::{GlobalAtoms, WrapperU64},
    state::GlobalRefMut,
    validation::loaders::GlobalDepositContext,
};

use super::invoke;

#[derive(BorshDeserialize, BorshSerialize)]
pub struct GlobalDepositParams {
    pub amount_atoms: u64,
    // No trader index hint because global account is small so there is not much
    // benefit from hinted indices, unlike the market which can get large. Also,
    // seats are not permanent like on a market due to eviction, so it is more
    // likely that a client could send a bad request. Just look it up for them.
}

impl GlobalDepositParams {
    pub fn new(amount_atoms: u64) -> Self {
        GlobalDepositParams { amount_atoms }
    }
}

pub(crate) fn process_global_deposit(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let global_deposit_context: GlobalDepositContext = GlobalDepositContext::load(accounts)?;
    let GlobalDepositParams { amount_atoms } = GlobalDepositParams::try_from_slice(data)?;

    // Due to transfer fees, this might not be what you expect.
    let mut deposited_amount_atoms: u64 = amount_atoms;

    let GlobalDepositContext {
        payer,
        global,
        mint,
        global_vault,
        trader_token: trader_token_account,
        token_program,
    } = global_deposit_context;

    // Do the token transfer first to determine actual received amount
    if *global_vault.owner == spl_token_2022::id() {
        let before_vault_balance_atoms: u64 = global_vault.get_balance_atoms();
        invoke(
            &spl_token_2022::instruction::transfer_checked(
                token_program.key,
                trader_token_account.key,
                mint.info.key,
                global_vault.key,
                payer.key,
                &[],
                amount_atoms,
                mint.mint.decimals,
            )?,
            &[
                token_program.as_ref().clone(),
                trader_token_account.as_ref().clone(),
                mint.as_ref().clone(),
                global_vault.as_ref().clone(),
                payer.as_ref().clone(),
            ],
        )?;

        let after_vault_balance_atoms: u64 = global_vault.get_balance_atoms();
        deposited_amount_atoms = after_vault_balance_atoms
            .checked_sub(before_vault_balance_atoms)
            .unwrap();
    } else {
        invoke(
            &spl_token::instruction::transfer(
                token_program.key,
                trader_token_account.key,
                global_vault.key,
                payer.key,
                &[],
                amount_atoms,
            )?,
            &[
                token_program.as_ref().clone(),
                trader_token_account.as_ref().clone(),
                global_vault.as_ref().clone(),
                payer.as_ref().clone(),
            ],
        )?;
    }

    // Now deposit the actual received amount (which may be less than requested due to transfer fees)
    let global_data: &mut RefMut<&mut [u8]> = &mut global.try_borrow_mut_data()?;
    let mut global_dynamic_account: GlobalRefMut = get_mut_dynamic_account(global_data);
    global_dynamic_account.deposit_global(payer.key, GlobalAtoms::new(deposited_amount_atoms))?;

    emit_stack(GlobalDepositLog {
        global: *global.key,
        trader: *payer.key,
        global_atoms: GlobalAtoms::new(deposited_amount_atoms),
    })?;

    Ok(())
}
