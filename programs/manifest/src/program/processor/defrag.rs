use crate::{
    market_vault_seeds_with_bump,
    program::{batch_expand_market, get_dynamic_account, get_mut_dynamic_account, invoke_signed},
    require,
    state::MarketFixed,
    validation::{
        next_account_info, to_program_error, AccountViewExt, ManifestAccountInfo, Program, Signer,
        TokenAccountInfo, TokenProgram,
    },
};
use pinocchio::{
    account::AccountView,
    error::ProgramError,
    sysvars::{rent::Rent, Sysvar},
    ProgramResult,
};
use solana_program::{pubkey, pubkey::Pubkey};
use spl_token_2022::{
    extension::{transfer_fee::TransferFeeAmount, BaseStateWithExtensions, StateWithExtensions},
    state::Account,
};

#[cfg(not(feature = "test"))]
pub const DEFRAG_COLLECTOR: Pubkey = pubkey!("B6dmr2UAn2wgjdm3T4N1Vjd8oPYRRTguByW7AEngkeL6");
#[cfg(feature = "test")]
pub const DEFRAG_COLLECTOR: Pubkey = pubkey!("2iXtA8oeZqUU5pofxK971TCEvFGfems2AcDRaZHKD2pQ");

pub(crate) fn process_defrag(
    _program_id: &Pubkey,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    // Optional little endian u32 budget: how many nodes this run relocates,
    // zero or absent meaning unbounded. Reclaiming empty seats is never capped
    // because the seat tree is rebuilt from the survivors either way. A market
    // too large to compact in one transaction is compacted by repeating a
    // bounded run; each leaves a valid market no larger than it started.
    let limit: u32 = match data.first_chunk::<4>() {
        Some(bytes) => u32::from_le_bytes(*bytes),
        None => {
            require!(
                data.is_empty(),
                ProgramError::InvalidInstructionData,
                "Defrag takes an optional u32 budget"
            )?;
            0
        }
    };
    let iter = &mut accounts.iter();
    let collector = Signer::new_payer(next_account_info(iter)?)?;
    require!(
        *collector.pubkey() == DEFRAG_COLLECTOR,
        ProgramError::InvalidArgument,
        "Invalid collector"
    )?;
    let market = ManifestAccountInfo::<MarketFixed>::new(next_account_info(iter)?)?;
    let base_vault = next_account_info(iter)?;
    let quote_vault = next_account_info(iter)?;
    let base_program = TokenProgram::new(next_account_info(iter)?)?;
    let quote_program = TokenProgram::new(next_account_info(iter)?)?;
    let _system = Program::new(
        next_account_info(iter)?,
        &solana_sdk_ids::system_program::id(),
    )?;
    require!(
        market.info.is_writable() && base_vault.is_writable() && quote_vault.is_writable(),
        ProgramError::InvalidAccountData,
        "Defrag accounts must be writable"
    )?;
    let fixed = *market.get_fixed()?;
    for (vault, program, mint, key) in [
        (
            base_vault,
            &base_program,
            fixed.get_base_mint(),
            fixed.get_base_vault(),
        ),
        (
            quote_vault,
            &quote_program,
            fixed.get_quote_mint(),
            fixed.get_quote_vault(),
        ),
    ] {
        TokenAccountInfo::new_with_owner_and_key(vault, mint, key, key)?;
        require!(
            vault.owner_pubkey() == *program.pubkey(),
            ProgramError::IncorrectProgramId,
            "Wrong vault token program"
        )?;
    }
    // CPI transfers happen before direct market-lamport changes, so each CPI
    // observes balanced account lamports. WSOL principal and its reserve stay.
    for (vault, program, mint, bump) in [
        (
            base_vault,
            &base_program,
            fixed.get_base_mint(),
            fixed.get_base_vault_bump(),
        ),
        (
            quote_vault,
            &quote_program,
            fixed.get_quote_mint(),
            fixed.get_quote_vault_bump(),
        ),
    ] {
        let native = StateWithExtensions::<Account>::unpack(&vault.try_borrow()?)
            .map_err(to_program_error)?
            .base
            .is_native
            .is_some();
        let minimum = Rent::get()?.try_minimum_balance(vault.data_len())?;
        if !native && vault.lamports() > minimum {
            // Both deployed token programs use opcode 38 and the same account
            // layout. This SDK version's builder only accepts Token-2022, so
            // construct there and select the already validated vault program.
            let mut ix = spl_token_2022_interface::instruction::withdraw_excess_lamports(
                &spl_token_2022::id(),
                vault.pubkey(),
                collector.pubkey(),
                vault.pubkey(),
                &[],
            )
            .map_err(to_program_error)?;
            ix.program_id = *program.pubkey();
            invoke_signed(
                &ix,
                &[vault, collector.info, program.info],
                market_vault_seeds_with_bump!(market.pubkey(), mint, bump),
            )?;
        }
    }
    let missing = {
        let data = market.try_borrow()?;
        let state = get_dynamic_account::<MarketFixed>(&data);
        state.free_blocks_short_of_n(2).unwrap_or(0)
    };
    if missing != 0 {
        batch_expand_market(&collector, &market, missing)?;
    }
    let size = {
        let mut data = market.try_borrow_mut()?;
        get_mut_dynamic_account::<MarketFixed>(&mut data).defragment(limit)?
    };
    // Empty vaults may be closed only when no token/extension claims remain.
    let empty = size == crate::state::MARKET_FIXED_SIZE + 2 * crate::state::MARKET_BLOCK_SIZE;
    let mut closable = empty;
    for vault in [base_vault, quote_vault] {
        let data = vault.try_borrow()?;
        let account = StateWithExtensions::<Account>::unpack(&data).map_err(to_program_error)?;
        closable &= account.base.amount == 0;
        if let Ok(fees) = account.get_extension::<TransferFeeAmount>() {
            closable &= u64::from(fees.withheld_amount) == 0;
        }
        // Unknown or confidential extension state needs its own retirement
        // policy. Supported market vault extensions carry no other balances.
        for extension in account.get_extension_types().map_err(to_program_error)? {
            use spl_token_2022::extension::ExtensionType;
            closable &= matches!(
                extension,
                ExtensionType::TransferFeeAmount
                    | ExtensionType::TransferHookAccount
                    | ExtensionType::PausableAccount
                    | ExtensionType::ImmutableOwner
            );
        }
    }
    if closable {
        for (vault, program, mint, bump) in [
            (
                base_vault,
                &base_program,
                fixed.get_base_mint(),
                fixed.get_base_vault_bump(),
            ),
            (
                quote_vault,
                &quote_program,
                fixed.get_quote_mint(),
                fixed.get_quote_vault_bump(),
            ),
        ] {
            let ix = spl_token_2022_interface::instruction::close_account(
                program.pubkey(),
                vault.pubkey(),
                collector.pubkey(),
                vault.pubkey(),
                &[],
            )
            .map_err(to_program_error)?;
            invoke_signed(
                &ix,
                &[vault, collector.info, program.info],
                market_vault_seeds_with_bump!(market.pubkey(), mint, bump),
            )?;
        }
        let balance = collector
            .lamports()
            .checked_add(market.lamports())
            .ok_or(ProgramError::ArithmeticOverflow)?;
        collector.info.set_lamports(balance);
        market.info.close()?;
        return Ok(());
    }
    market.info.resize(size)?;
    let minimum = Rent::get()?.try_minimum_balance(size)?;
    let excess = market.lamports().saturating_sub(minimum);
    let destination = collector
        .lamports()
        .checked_add(excess)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    market.info.set_lamports(market.lamports() - excess);
    collector.info.set_lamports(destination);
    Ok(())
}
