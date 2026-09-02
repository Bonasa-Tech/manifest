use cvt::{cvt_assert, cvt_assume};
use cvt_macros::rule;
use nondet::*;

use crate::*;
use solana_program::account_info::AccountInfo;

use solana_cvt::token::spl_token_account_get_amount;
use state::{cvt_assume_main_trader_has_seat, is_second_seat_taken, second_trader_pk};

use crate::{
    program::{
        get_mut_dynamic_account,
        withdraw::{process_withdraw_core, WithdrawParams},
    },
    state::MarketRefMut,
};

// verifies when we use the fixed summary for the token2022 transfer,
// fails with a counterexample showing the transfer happening in the wrong direction otherwise,
// as long as we don't use the market initialization
#[rule]
pub fn rule_withdraw_withdraws() {
    crate::certora::spec::verification_utils::init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let used_acc_infos: &[AccountInfo] = &acc_infos[..6];
    let trader_token: &AccountInfo = &used_acc_infos[2];
    let vault_token: &AccountInfo = &used_acc_infos[3];
    let trader: &AccountInfo = &used_acc_infos[0];
    let market: &AccountInfo = &used_acc_infos[1];
    let unrelated_trader: &AccountInfo = &acc_infos[7];

    cvt_assume_main_trader_has_seat(trader.key);

    // -- trader and vault have different token accounts
    cvt_assume!(trader_token.key != vault_token.key);

    cvt_assume!(trader.key != unrelated_trader.key);
    cvt_assume!(unrelated_trader.key == second_trader_pk());
    cvt_assume!(is_second_seat_taken());

    let (trader_base_old, trader_quote_old) = get_trader_balance!(market, trader.key);
    let (unrelated_trader_base_old, unrelated_trader_quote_old) =
        get_trader_balance!(market, unrelated_trader.key);

    let trader_amount_old: u64 = spl_token_account_get_amount(trader_token);
    let vault_amount_old: u64 = spl_token_account_get_amount(vault_token);

    let amount: u64 = nondet();

    process_withdraw_core(
        &crate::id(),
        &used_acc_infos,
        WithdrawParams::new(amount, None),
    )
    .unwrap();

    let trader_amount: u64 = spl_token_account_get_amount(trader_token);
    let vault_amount: u64 = spl_token_account_get_amount(vault_token);

    cvlr::clog!(
        trader_amount_old,
        vault_amount_old,
        amount,
        trader_amount,
        vault_amount
    );

    cvt_assert!(trader_amount >= trader_amount_old);
    cvt_assert!(vault_amount_old >= vault_amount);
    let trader_diff: u64 = trader_amount - trader_amount_old;
    let vault_diff: u64 = vault_amount_old - vault_amount;

    cvt_assert!(trader_diff == amount);
    cvt_assert!(vault_diff == amount);

    let (trader_base, trader_quote) = get_trader_balance!(market, trader.key);

    let (unrelated_trader_base, unrelated_trader_quote) =
        get_trader_balance!(market, unrelated_trader.key);

    cvt_assert!(trader_base_old >= trader_base);
    cvt_assert!(trader_quote_old >= trader_quote);
    let trader_base_diff: u64 = trader_base_old - trader_base;
    let trader_quote_diff: u64 = trader_quote_old - trader_quote;

    // one of the diffs should be amount, the other zero
    cvt_assert!(trader_base_diff + trader_quote_diff == amount);
    cvt_assert!(trader_base_diff == 0 || trader_quote_diff == 0);

    cvt_assert!(
        unrelated_trader_base == unrelated_trader_base_old
            && unrelated_trader_quote == unrelated_trader_quote_old
    );

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_withdraw_does_not_revert() {
    crate::certora::spec::verification_utils::init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let used_acc_infos: &[AccountInfo] = &acc_infos[..6];
    let trader: &AccountInfo = &used_acc_infos[0];

    cvt_assume_main_trader_has_seat(trader.key);

    let amount: u64 = nondet();
    let result: ProgramResult = process_withdraw_core(
        &crate::id(),
        &used_acc_infos,
        WithdrawParams::new(amount, None),
    );
    cvt_assert!(result.is_ok());

    cvt_vacuity_check!();
}
