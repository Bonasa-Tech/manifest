use pinocchio::account_info::RefMut;
use pinocchio::account_info::AccountInfo;
use crate::validation::AccountInfoExt;
use pinocchio::ProgramResult;

use hypertree::trace;
use solana_program::{
    program_pack::Pack, pubkey::Pubkey,
    rent::Rent, sysvar::Sysvar,
};
use spl_token::state::Account;

use crate::{
    logs::{emit_stack, GlobalAddTraderLog},
    program::{expand_global, get_mut_dynamic_account, invoke},
    state::GlobalRefMut,
    validation::loaders::GlobalAddTraderContext,
};

pub(crate) fn process_global_add_trader(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    _data: &[u8],
) -> ProgramResult {
    trace!("process_global_add_trader accs={accounts:?}");
    let global_add_trader_context: GlobalAddTraderContext = GlobalAddTraderContext::load(accounts)?;

    let GlobalAddTraderContext { payer, global, .. } = global_add_trader_context;

    {
        // Charge a seat fee.
        // This is necessary to prevent an attack where an attacker would claim a
        // global seat and then delete their token account. In order for someone
        // else to get that seat, they would need to init a token account for the
        // attacker, giving them rent.
        let rent: Rent = Rent::get()?;
        invoke(
            &solana_program::system_instruction::transfer(
                &payer.pubkey(),
                &global.pubkey(),
                rent.minimum_balance(Account::LEN as usize) * 2,
            ),
            &[payer.info.clone(), global.info.clone()],
        )?;
    }

    // Needs a spot for this trader on the global account.
    expand_global(&payer, &global)?;

    let global_data: &mut RefMut<[u8]> = &mut global.try_borrow_mut_data()?;
    let mut global_dynamic_account: GlobalRefMut = get_mut_dynamic_account(global_data);

    global_dynamic_account.add_trader(payer.pubkey())?;

    emit_stack(GlobalAddTraderLog {
        global: *global.pubkey(),
        trader: *payer.pubkey(),
    })?;

    Ok(())
}
