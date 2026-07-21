use std::cell::RefMut;

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{account_info::AccountInfo, entrypoint::ProgramResult, pubkey::Pubkey};
#[cfg(not(feature = "certora"))]
use solana_program::{program::invoke_signed, program_pack::Pack, rent::Rent, sysvar::Sysvar};
#[cfg(not(feature = "certora"))]
use spl_token::state::Account;

use crate::state::GlobalFixed;
#[cfg(not(feature = "certora"))]
use crate::{
    global_vault_seeds_with_bump, state::GAS_DEPOSIT_LAMPORTS, validation::get_global_vault_address,
};
use crate::{
    logs::{emit_stack, GlobalDepositLog, GlobalEvictLog, GlobalWithdrawLog},
    program::get_mut_dynamic_account,
    quantities::{GlobalAtoms, WrapperU64},
    require,
    state::GlobalRefMut,
    validation::{
        loaders::GlobalEvictContext, ManifestAccountInfo, MintAccountInfo, Signer,
        TokenAccountInfo, TokenProgram,
    },
};

#[cfg(not(feature = "certora"))]
use super::invoke;

#[cfg(feature = "certora")]
use {
    crate::certora::summaries::token::spl_token_2022_transfer_with_fee, early_panic::early_panic,
    solana_cvt::token::spl_token_transfer,
};

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
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let params: GlobalEvictParams = GlobalEvictParams::try_from_slice(data)?;
    process_global_evict_core(program_id, accounts, params)
}

#[cfg_attr(all(feature = "certora", not(feature = "certora-test")), early_panic)]
pub(crate) fn process_global_evict_core(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: GlobalEvictParams,
) -> ProgramResult {
    let global_evict_context: GlobalEvictContext = GlobalEvictContext::load(accounts)?;
    let GlobalEvictParams { amount_atoms } = params;

    let GlobalEvictContext {
        payer,
        global,
        mint,
        global_vault,
        trader_token,
        evictee_token,
        token_program,
        ..
    } = global_evict_context;

    charge_eviction_fee(&payer, &global)?;

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
        global_dynamic_account.verify_min_balance(&evictee_token.get_owner())?;
    }

    // Withdraw
    {
        let evictee_balance: GlobalAtoms =
            global_dynamic_account.get_balance_atoms(&evictee_token.get_owner());
        global_dynamic_account.withdraw_global(&evictee_token.get_owner(), evictee_balance)?;

        spl_token_transfer_from_global_vault_to_evictee(
            &token_program,
            &mint,
            &global_vault,
            &evictee_token,
            evictee_balance.into(),
        )?;

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

        spl_token_transfer_from_evictor_to_global_vault(
            &token_program,
            &mint,
            &trader_token,
            &global_vault,
            &payer,
            amount_atoms,
        )?;

        let after_vault_balance_atoms: u64 = global_vault.get_balance_atoms();
        let deposited_amount_atoms: u64 = after_vault_balance_atoms
            .checked_sub(before_vault_balance_atoms)
            .unwrap();

        // Verify that the actual deposited amount is greater than the evictee balance.
        // This check is done after the deposit to account for token22 transfer fees.
        require!(
            evictee_balance < GlobalAtoms::new(deposited_amount_atoms),
            crate::program::ManifestError::InvalidEvict,
            "Evictee balance {} is more than evictor deposited {}",
            evictee_balance.as_u64(),
            deposited_amount_atoms,
        )?;

        global_dynamic_account
            .deposit_global(payer.key, GlobalAtoms::new(deposited_amount_atoms))?;

        emit_stack(GlobalDepositLog {
            global: *global.key,
            trader: *payer.key,
            global_atoms: GlobalAtoms::new(amount_atoms),
        })?;
    }

    Ok(())
}

/// Charge a seat fee + eviction fee.
/// This is necessary to prevent an attack where an attacker would claim a
/// global seat and then delete their token account. In order for someone
/// else to get that seat, they would need to init a token account for the
/// attacker, giving them rent.
/// In addition backed orders might become unbacked by eviction. To prevent
/// someone evicting another seat for the sole purpose of claiming the
/// unbacked order penalty it's advised to not place more than 10000 global
/// orders using the same trader identity.
#[cfg(not(feature = "certora"))]
fn charge_eviction_fee<'a, 'info>(
    payer: &Signer<'a, 'info>,
    global: &ManifestAccountInfo<'a, 'info, GlobalFixed>,
) -> ProgramResult {
    let rent: Rent = Rent::get()?;
    invoke(
        &solana_program::system_instruction::transfer(
            &payer.key,
            &global.key,
            rent.minimum_balance(Account::LEN as usize) * 2 + 10000 * GAS_DEPOSIT_LAMPORTS,
        ),
        &[payer.info.clone(), global.info.clone()],
    )?;
    Ok(())
}

/// (Summary) The seat fee + eviction fee system-program transfer, modeled as a
/// direct lamport move so the prover can track it. The fee amount depends on
/// rent, sysvar state the prover does not model, so it is a nondeterministic
/// amount the payer can cover.
#[cfg(feature = "certora")]
fn charge_eviction_fee<'a, 'info>(
    payer: &Signer<'a, 'info>,
    global: &ManifestAccountInfo<'a, 'info, GlobalFixed>,
) -> ProgramResult {
    let fee_lamports: u64 = ::nondet::nondet();
    cvt::cvt_assume!(**payer.info.lamports.borrow() >= fee_lamports);
    cvt::cvt_assume!(**global.info.lamports.borrow() <= u64::MAX - fee_lamports);
    **payer.info.lamports.borrow_mut() -= fee_lamports;
    **global.info.lamports.borrow_mut() += fee_lamports;
    Ok(())
}

/** Transfer the evictee's balance from the global vault to their token account **/
#[cfg(not(feature = "certora"))]
fn spl_token_transfer_from_global_vault_to_evictee<'a, 'info>(
    token_program: &TokenProgram<'a, 'info>,
    mint: &MintAccountInfo<'a, 'info>,
    global_vault: &TokenAccountInfo<'a, 'info>,
    evictee_token: &TokenAccountInfo<'a, 'info>,
    amount_atoms: u64,
) -> ProgramResult {
    let (_, bump) = get_global_vault_address(mint.info.key);

    if *global_vault.owner == spl_token_2022::id() {
        invoke_signed(
            &spl_token_2022::instruction::transfer_checked(
                token_program.key,
                global_vault.key,
                mint.info.key,
                evictee_token.key,
                global_vault.key,
                &[],
                amount_atoms,
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
                amount_atoms,
            )?,
            &[
                token_program.as_ref().clone(),
                global_vault.as_ref().clone(),
                evictee_token.as_ref().clone(),
            ],
            global_vault_seeds_with_bump!(mint.info.key, bump),
        )?;
    }
    Ok(())
}

/** (Summary) Transfer the evictee's balance from the global vault to their
token account. The mint may carry a transfer fee on the token-2022 path; the
fee is withheld from what the evictee receives, the vault is debited the full
amount. **/
#[cfg(feature = "certora")]
fn spl_token_transfer_from_global_vault_to_evictee<'a, 'info>(
    _token_program: &TokenProgram<'a, 'info>,
    _mint: &MintAccountInfo<'a, 'info>,
    global_vault: &TokenAccountInfo<'a, 'info>,
    evictee_token: &TokenAccountInfo<'a, 'info>,
    amount_atoms: u64,
) -> ProgramResult {
    if *global_vault.owner == spl_token_2022::id() {
        spl_token_2022_transfer_with_fee(
            global_vault.info,
            evictee_token.info,
            global_vault.info,
            amount_atoms,
        )
    } else {
        spl_token_transfer(
            global_vault.info,
            evictee_token.info,
            global_vault.info,
            amount_atoms,
        )
    }
}

/** Transfer the evictor's deposit from their token account to the global vault **/
#[cfg(not(feature = "certora"))]
fn spl_token_transfer_from_evictor_to_global_vault<'a, 'info>(
    token_program: &TokenProgram<'a, 'info>,
    mint: &MintAccountInfo<'a, 'info>,
    trader_token: &TokenAccountInfo<'a, 'info>,
    global_vault: &TokenAccountInfo<'a, 'info>,
    payer: &Signer<'a, 'info>,
    amount_atoms: u64,
) -> ProgramResult {
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
    Ok(())
}

/** (Summary) Transfer the evictor's deposit from their token account to the
global vault. The mint may carry a transfer fee on the token-2022 path, so the
vault can receive less than the requested amount; the processor credits the
vault balance delta. **/
#[cfg(feature = "certora")]
fn spl_token_transfer_from_evictor_to_global_vault<'a, 'info>(
    _token_program: &TokenProgram<'a, 'info>,
    _mint: &MintAccountInfo<'a, 'info>,
    trader_token: &TokenAccountInfo<'a, 'info>,
    global_vault: &TokenAccountInfo<'a, 'info>,
    payer: &Signer<'a, 'info>,
    amount_atoms: u64,
) -> ProgramResult {
    if *global_vault.owner == spl_token_2022::id() {
        spl_token_2022_transfer_with_fee(
            trader_token.info,
            global_vault.info,
            payer.info,
            amount_atoms,
        )
    } else {
        spl_token_transfer(
            trader_token.info,
            global_vault.info,
            payer.info,
            amount_atoms,
        )
    }
}
