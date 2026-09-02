use super::verification_utils::init_static;
use crate::{
    certora::spec::no_funds_loss_util::{
        cvt_assert_funds_invariants, cvt_assume_basic_market_preconditions,
        cvt_assume_funds_invariants, record_all_balances_without_order, AllBalances,
    },
    create_empty_market, cvt_static_initializer, cvt_vacuity_check,
};
use cvt::{cvt_assert, cvt_assume};
use cvt_macros::rule;
use nondet::*;
use solana_cvt::token::spl_token_account_get_amount;

use crate::{
    program::{process_swap_core, SwapParams},
    state::MarketFixed,
};
use hypertree::get_mut_helper;
use solana_program::account_info::AccountInfo;

#[rule]
/// This rule can be further refined if additional specifications are given on the arguments to swap
pub fn rule_integrity_swap() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let used_acc_infos: &[AccountInfo] = &acc_infos[..8];
    let market_info: &AccountInfo = &used_acc_infos[1];
    let trader_base_info: &AccountInfo = &used_acc_infos[2];
    let trader_quote_info: &AccountInfo = &used_acc_infos[3];
    let _base_vault_info: &AccountInfo = &used_acc_infos[4];
    let _quote_vault_info: &AccountInfo = &used_acc_infos[5];

    // Create an empty market
    create_empty_market!(market_info);

    let params = SwapParams::new(nondet(), nondet(), true, true);
    let in_atoms: u64 = params.in_atoms;
    let _out_atoms: u64 = params.out_atoms;
    let trader_base_amount_old: u64 = spl_token_account_get_amount(trader_base_info);
    let trader_quote_amount_old: u64 = spl_token_account_get_amount(trader_quote_info);

    process_swap_core(&crate::id(), &used_acc_infos, params).unwrap();

    let trader_base_amount: u64 = spl_token_account_get_amount(trader_base_info);
    let trader_quote_amount: u64 = spl_token_account_get_amount(trader_quote_info);

    // the trader pays with base
    cvt_assert!(trader_base_amount <= trader_base_amount_old);
    let trader_out: u64 = trader_base_amount_old - trader_base_amount;

    // the trader gets quote
    cvt_assert!(trader_quote_amount >= trader_quote_amount_old);
    let _trader_in: u64 = trader_quote_amount - trader_quote_amount_old;

    cvt_assert!(trader_out <= in_atoms);

    cvt_vacuity_check!()
}

/// Parametric rule: no loss of funds for swap
fn rule_swap_check<const IS_BASE: bool, const IS_EXACT: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let used_acc_infos: &[AccountInfo] = &acc_infos[..8];
    let trader: &AccountInfo = &used_acc_infos[0];
    let market: &AccountInfo = &used_acc_infos[1];
    let trader_base_token: &AccountInfo = &used_acc_infos[2];
    let trader_quote_token: &AccountInfo = &used_acc_infos[3];
    let vault_base_token: &AccountInfo = &used_acc_infos[4];
    let vault_quote_token: &AccountInfo = &used_acc_infos[5];
    // we only care about having a pubkey for the maker
    let maker_trader: &AccountInfo = &acc_infos[9];

    cvt_assume!(trader.key != vault_base_token.key);
    cvt_assume!(trader.key != vault_quote_token.key);
    cvt_assume!(trader_base_token.key != vault_base_token.key);
    cvt_assume!(trader_quote_token.key != vault_quote_token.key);

    // -- basic market assumptions
    cvt_assume_basic_market_preconditions(
        market,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );

    // -- record balances before swap
    let old_balances: AllBalances = record_all_balances_without_order(
        market,
        vault_base_token,
        vault_quote_token,
        trader,
        maker_trader,
    );

    // -- assume no loss of funds invariant
    cvt_assume_funds_invariants(old_balances);

    let in_atoms: u64 = nondet();
    let out_atoms: u64 = nondet();
    // -- in_atoms does not overflow ghost aggregate
    if IS_BASE {
        cvt_assume!(in_atoms
            .checked_add(old_balances.withdrawable_base)
            .is_some());
    } else {
        cvt_assume!(in_atoms
            .checked_add(old_balances.withdrawable_quote)
            .is_some());
    }

    let params: SwapParams = SwapParams::new(in_atoms, out_atoms, IS_BASE, IS_EXACT);
    process_swap_core(&crate::id(), &used_acc_infos, params).unwrap();

    let new_balances = record_all_balances_without_order(
        market,
        vault_base_token,
        vault_quote_token,
        trader,
        maker_trader,
    );

    // -- check no loss of funds invariant
    cvt_assert_funds_invariants(new_balances);

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_swap_base_exact() {
    rule_swap_check::<true, true>();
}

#[rule]
pub fn rule_swap_base_not_exact() {
    rule_swap_check::<true, false>();
}

#[rule]
pub fn rule_swap_quote_exact() {
    rule_swap_check::<false, true>();
}

#[rule]
pub fn rule_swap_quote_not_exact() {
    rule_swap_check::<false, false>();
}
