use std::{cell::Ref, mem::size_of};
use pinocchio::sysvars::{rent::Rent, Sysvar};
use pinocchio::account_info::AccountInfo;
use crate::validation::AccountInfoExt;
use pinocchio::ProgramResult;

use crate::{
    global_seeds_with_bump, global_vault_seeds_with_bump,
    logs::{emit_stack, GlobalCreateLog},
    program::invoke,
    state::GlobalFixed,
    utils::create_account,
    validation::{get_global_vault_address, loaders::GlobalCreateContext},
};
use hypertree::{get_mut_helper, trace};
use solana_program::{
    program_pack::Pack, pubkey::Pubkey,
    system_instruction, };
use spl_token_2022::{
    extension::{BaseStateWithExtensions, ExtensionType, PodStateWithExtensions},
    pod::PodMint,
    state::Account,
};

pub(crate) fn process_global_create(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    _data: &[u8],
) -> ProgramResult {
    {
        trace!("process_global_create accs={accounts:?}");
        let global_create_context: GlobalCreateContext = GlobalCreateContext::load(accounts)?;

        let GlobalCreateContext {
            payer,
            global,
            system_program,
            global_mint,
            global_vault,
            token_program,
            global_bump,
        } = global_create_context;
        let (expected_global_vault_key, global_vault_bump) =
            get_global_vault_address(global_mint.info.pubkey());

        // Make the global account.
        {
            let global_seeds: Vec<Vec<u8>> = vec![
                b"global".to_vec(),
                global_mint.info.pubkey().as_ref().to_vec(),
                vec![global_bump],
            ];

            if global.info.lamports() > 0 {
                crate::program::invoke_signed(
                    &system_instruction::transfer(
                        global.info.pubkey(),
                        payer.info.pubkey(),
                        global.info.lamports(),
                    ),
                    &[payer.info,
                        global.info,
                        system_program.info,
                    ],
                    global_seeds_with_bump!(global_mint.info.pubkey(), global_bump),
                )?;
            }
            create_account(
                payer.as_ref(),
                global.as_ref(),
                system_program.as_ref(),
                &crate::id(),
                &Rent::get()?,
                size_of::<GlobalFixed>() as u64,
                global_seeds,
            )?;

            // Setup the empty market
            let empty_global_fixed: GlobalFixed = GlobalFixed::new_empty_with_bumps(
                global_mint.as_ref().pubkey(),
                expected_global_vault_key,
                global_vault_bump,
                global_bump,
            );
            assert_eq!(global.info.data_len(), size_of::<GlobalFixed>());

            let global_bytes: &mut [u8] = &mut global.info.try_borrow_mut_data()?[..];
            *get_mut_helper::<GlobalFixed>(global_bytes, 0_u32) = empty_global_fixed;

            // Global does not require a permanent free block for swapping.
        }

        // Make the global vault.
        {
            // We dont have to deserialize the mint, just check the owner.
            let is_mint_22: bool = *global_mint.info.owner_pubkey() == spl_token_2022::id();
            let token_program_for_mint: Pubkey = if is_mint_22 {
                spl_token_2022::id()
            } else {
                spl_token::id()
            };

            let global_vault_seeds: Vec<Vec<u8>> = vec![
                b"global-vault".to_vec(),
                global_mint.info.pubkey().as_ref().to_vec(),
                vec![global_vault_bump],
            ];
            assert_eq!(expected_global_vault_key, *global_vault.info.pubkey());
            let rent: Rent = Rent::get()?;

            if global_vault.info.lamports() > 0 {
                crate::program::invoke_signed(
                    &system_instruction::transfer(
                        global_vault.info.pubkey(),
                        payer.info.pubkey(),
                        global_vault.info.lamports(),
                    ),
                    &[payer.info,
                        global_vault.info,
                        system_program.info,
                    ],
                    global_vault_seeds_with_bump!(global_mint.info.pubkey(), global_vault_bump),
                )?;
            }

            if is_mint_22 {
                let mint_data: pinocchio::account_info::Ref<[u8]> = global_mint.info.try_borrow_data()?;
                let mint_with_extension: PodStateWithExtensions<'_, PodMint> =
                    PodStateWithExtensions::<PodMint>::unpack(&mint_data).unwrap();
                let mint_extensions: Vec<ExtensionType> =
                    mint_with_extension.get_extension_types().map_err(to_program_error)?;
                let required_extensions: Vec<ExtensionType> =
                    ExtensionType::get_required_init_account_extensions(&mint_extensions);
                let space: usize =
                    ExtensionType::try_calculate_account_len::<Account>(&required_extensions).map_err(to_program_error)?;
                create_account(
                    payer.as_ref(),
                    global_vault.info,
                    system_program.as_ref(),
                    &token_program_for_mint,
                    &rent,
                    space as u64,
                    global_vault_seeds,
                )?;
                invoke(
                    &spl_token_2022::instruction::initialize_account3(
                        &spl_token_2022::id(),
                        global_vault.as_ref().pubkey(),
                        global_mint.info.pubkey(),
                        global_vault.as_ref().pubkey(),
                    ).map_err(to_program_error)?,
                    &[
                        payer.as_ref(),
                        global_vault.as_ref(),
                        global_mint.as_ref(),
                        token_program.as_ref(),
                    ],
                )?;
            } else {
                let space: usize = spl_token::state::Account::LEN;
                create_account(
                    payer.as_ref(),
                    global_vault.info,
                    system_program.as_ref(),
                    &token_program_for_mint,
                    &rent,
                    space as u64,
                    global_vault_seeds,
                )?;
                invoke(
                    &spl_token::instruction::initialize_account3(
                        &spl_token::id(),
                        global_vault.as_ref().pubkey(),
                        global_mint.info.pubkey(),
                        global_vault.as_ref().pubkey(),
                    ).map_err(to_program_error)?,
                    &[
                        payer.as_ref(),
                        global_vault.as_ref(),
                        global_mint.as_ref(),
                        token_program.as_ref(),
                    ],
                )?;
            }
        }

        emit_stack(GlobalCreateLog {
            global: *global.info.pubkey(),
            creator: *payer.pubkey(),
        })?;
    }

    Ok(())
}
