use std::cell::RefMut;

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{account_info::AccountInfo, entrypoint::ProgramResult, pubkey::Pubkey};

use crate::{
    global_vault_seeds_with_bump,
    logs::{emit_stack, GlobalDepositLog, GlobalEvictLog, GlobalWithdrawLog},
    program::get_mut_dynamic_account,
    quantities::{GlobalAtoms, WrapperU64},
    require,
    state::GlobalRefMut,
    validation::{get_global_vault_address, loaders::GlobalEvictContext},
};
use solana_program::program::invoke_signed;

use super::invoke;

#[derive(BorshDeserialize, BorshSerialize)]
pub struct GlobalEvictParams {
    // Deposit amount that must be greater than the evictee deposit amount
    amount_atoms: u64,
}

impl GlobalEvictParams {
    pub fn new(amount_atoms: u64) -> Self {
        GlobalEvictParams { amount_atoms }
    }
}

pub(crate) fn process_global_evict(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let global_evict_context: GlobalEvictContext = GlobalEvictContext::load(accounts)?;
    let GlobalEvictParams { amount_atoms } = GlobalEvictParams::try_from_slice(data)?;

    let GlobalEvictContext {
        payer,
        global,
        mint,
        global_vault,
        trader_token,
        evictee_token,
        token_program,
    } = global_evict_context;

    // 1. Withdraw for the evictee
    // 2. Evict the seat on the global account and claim
    // 3. Deposit for the evictor
    let global_data: &mut RefMut<&mut [u8]> = &mut global.try_borrow_mut_data()?;
    let mut global_dynamic_account: GlobalRefMut = get_mut_dynamic_account(global_data);
    let evictee_balance: GlobalAtoms =
        global_dynamic_account.get_balance_atoms(&evictee_token.get_owner());

    {
        // Do verifications that this is a valid eviction.
        require!(
            global_dynamic_account.fixed.needs_eviction(),
            crate::program::ManifestError::InvalidEvict,
            "Eviction is only allowed when global is at capacity",
        )?;
        require!(
            evictee_balance < GlobalAtoms::new(amount_atoms),
            crate::program::ManifestError::InvalidEvict,
            "Evictee balance {} is more than evictor wants to deposit",
            evictee_balance.as_u64(),
        )?;
        global_dynamic_account.verify_min_balance(&evictee_token.get_owner())?;
    }

    // Withdraw
    {
        let evictee_balance: GlobalAtoms =
            global_dynamic_account.get_balance_atoms(&evictee_token.get_owner());
        global_dynamic_account.withdraw_global(&evictee_token.get_owner(), evictee_balance)?;

        let (_, bump) = get_global_vault_address(mint.info.key);

        // Do the token transfer
        if *global_vault.owner == spl_token_2022::id() {
            invoke_signed(
                &spl_token_2022::instruction::transfer_checked(
                    token_program.key,
                    global_vault.key,
                    mint.info.key,
                    evictee_token.key,
                    global_vault.key,
                    &[],
                    evictee_balance.into(),
                    mint.mint.decimals,
                )?,
                &[
                    token_program.as_ref().clone(),
                    evictee_token.as_ref().clone(),
                    mint.as_ref().clone(),
                    global_vault.as_ref().clone(),
                ],
                global_vault_seeds_with_bump!(mint.info.key, bump),
            )?;
        } else {
            invoke_signed(
                &spl_token::instruction::transfer(
                    token_program.key,
                    global_vault.key,
                    evictee_token.key,
                    global_vault.key,
                    &[],
                    evictee_balance.into(),
                )?,
                &[
                    token_program.as_ref().clone(),
                    global_vault.as_ref().clone(),
                    evictee_token.as_ref().clone(),
                ],
                global_vault_seeds_with_bump!(mint.info.key, bump),
            )?;
        }

        emit_stack(GlobalWithdrawLog {
            global: *global.key,
            trader: *payer.key,
            global_atoms: GlobalAtoms::new(amount_atoms),
        })?;
    }

    // Evict
    {
        global_dynamic_account
            .evict_and_take_seat(&evictee_token.get_owner(), &trader_token.get_owner())?;

        emit_stack(GlobalEvictLog {
            evictee: evictee_token.get_owner(),
            evictor: trader_token.get_owner(),
            evictor_atoms: GlobalAtoms::new(amount_atoms),
            evictee_atoms: evictee_balance,
        })?;
    }

    // Deposit
    {
        // Needs to check the amount before because of transfer fees.
        let before_vault_balance_atoms: u64 = global_vault.get_balance_atoms();

        // Do the token transfer
        if *global_vault.owner == spl_token_2022::id() {
            invoke(
                &spl_token_2022::instruction::transfer_checked(
                    token_program.key,
                    trader_token.key,
                    mint.info.key,
                    global_vault.key,
                    payer.key,
                    &[],
                    amount_atoms,
                    mint.mint.decimals,
                )?,
                &[
                    token_program.as_ref().clone(),
                    trader_token.as_ref().clone(),
                    mint.as_ref().clone(),
                    global_vault.as_ref().clone(),
                    payer.as_ref().clone(),
                ],
            )?;
        } else {
            invoke(
                &spl_token::instruction::transfer(
                    token_program.key,
                    trader_token.key,
                    global_vault.key,
                    payer.key,
                    &[],
                    amount_atoms,
                )?,
                &[
                    token_program.as_ref().clone(),
                    trader_token.as_ref().clone(),
                    global_vault.as_ref().clone(),
                    payer.as_ref().clone(),
                ],
            )?;
        }

        let after_vault_balance_atoms: u64 = global_vault.get_balance_atoms();
        let deposited_amount_atoms: u64 = after_vault_balance_atoms
            .checked_sub(before_vault_balance_atoms)
            .unwrap();
        global_dynamic_account.deposit_global(payer.key, GlobalAtoms::new(deposited_amount_atoms))?;

        emit_stack(GlobalDepositLog {
            global: *global.key,
            trader: *payer.key,
            global_atoms: GlobalAtoms::new(amount_atoms),
        })?;
    }

    Ok(())
}
