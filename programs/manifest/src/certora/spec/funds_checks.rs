use crate::*;
use cvt::cvt_assume;
use cvt_macros::rule;
use nondet::*;

use solana_program::account_info::AccountInfo;

use crate::{
    program::{
        deposit::{process_deposit_core, DepositParams},
        get_mut_dynamic_account,
        withdraw::{process_withdraw_core, WithdrawParams},
    },
    quantities::{BaseAtoms, QuoteAtoms},
    state::{get_helper_order, AddOrderToMarketArgs, DynamicAccount, MarketRefMut, RestingOrder},
};
use hypertree::DataIndex;

use crate::certora::spec::no_funds_loss_util::*;
use state::{main_trader_index, second_trader_index};

fn rule_deposit_check<const IS_BASE: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let used_acc_infos: &[AccountInfo] = &acc_infos[..6];
    let trader: &AccountInfo = &used_acc_infos[0];
    let market_info: &AccountInfo = &used_acc_infos[1];
    let trader_token: &AccountInfo = &used_acc_infos[2];
    let vault_token: &AccountInfo = &used_acc_infos[3];

    let maker_trader: &AccountInfo = &acc_infos[7];
    let vault_base_token: &AccountInfo = &acc_infos[8];
    let vault_quote_token: &AccountInfo = &acc_infos[9];

    // -- market preconditions
    // the parameter true below implies there is a bid order,
    // but the rule_deposit_check does not consider any orders
    let maker_order_index: DataIndex = cvt_assume_market_preconditions::<true /* IS_BID */>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );
    // -- additional precondition for deposit
    let market_base_vault_pk: Pubkey = get_base_vault!(market_info);
    let market_quote_vault_pk: Pubkey = get_quote_vault!(market_info);
    // -- vault_token is either eqauls base vault or quote vault
    let vault_pk: Pubkey = if IS_BASE {
        market_base_vault_pk
    } else {
        market_quote_vault_pk
    };
    cvt_assume!(vault_token.key == &vault_pk);
    // -- trader and vault have different token accounts
    cvt_assume!(trader_token.key != vault_token.key);

    // if IS_BASE, then vault_base amount comes from vault_token
    // otherwise, vault_quote amount comes from vault_token
    let balances_old: AllBalances = if IS_BASE {
        record_all_balances(
            market_info,
            vault_token,
            vault_quote_token,
            trader,
            maker_trader,
            maker_order_index,
        )
    } else {
        record_all_balances(
            market_info,
            vault_base_token,
            vault_token,
            trader,
            maker_trader,
            maker_order_index,
        )
    };

    // -- assume no loss of funds invariant
    cvt_assume_funds_invariants(balances_old);

    // -- atempt to deposit an arbitrary amount
    let amount_arg: u64 = nondet();
    process_deposit_core(
        &crate::id(),
        &used_acc_infos,
        DepositParams::new(amount_arg, None),
    )
    .unwrap();

    let balances_new: AllBalances = if IS_BASE {
        record_all_balances(
            market_info,
            vault_token,
            vault_quote_token,
            trader,
            maker_trader,
            maker_order_index,
        )
    } else {
        record_all_balances(
            market_info,
            vault_base_token,
            vault_token,
            trader,
            maker_trader,
            maker_order_index,
        )
    };

    // -- assert no loss of funds invariant
    cvt_assert_funds_invariants(balances_new);

    // -- additional properties
    cvt_assert_deposit_extra::<IS_BASE>(balances_old, balances_new, amount_arg);

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_deposit_base() {
    rule_deposit_check::<true>();
}

#[rule]
pub fn rule_deposit_quote() {
    rule_deposit_check::<false>();
}

fn rule_withdraw_check<const IS_BASE: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let used_acc_infos: &[AccountInfo] = &acc_infos[..6];
    let trader: &AccountInfo = &used_acc_infos[0];
    let market_info: &AccountInfo = &used_acc_infos[1];
    let trader_token: &AccountInfo = &used_acc_infos[2];
    let vault_token: &AccountInfo = &used_acc_infos[3];

    let maker_trader: &AccountInfo = &acc_infos[7];
    let vault_base_token: &AccountInfo = &acc_infos[8];
    let vault_quote_token: &AccountInfo = &acc_infos[9];

    // -- market preconditions
    // the parameter true below implies there is a bid order,
    // but the rule_withdraw_check does not consider any orders
    let maker_order_index: DataIndex = cvt_assume_market_preconditions::<true /* IS_BID */>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );
    // -- additional precondition for deposit
    let market_base_vault_pk: Pubkey = get_base_vault!(market_info);
    let market_quote_vault_pk: Pubkey = get_quote_vault!(market_info);
    // -- vault_token is either eqauls base vault or quote vault
    let vault_pk: Pubkey = if IS_BASE {
        market_base_vault_pk
    } else {
        market_quote_vault_pk
    };
    cvt_assume!(vault_token.key == &vault_pk);
    // -- trader and vault have different token accounts
    cvt_assume!(trader_token.key != vault_token.key);

    // if IS_BASE, then vault_base amount comes from vault_token
    // otherwise, vault_quote amount comes from vault_token
    let balances_old: AllBalances = if IS_BASE {
        record_all_balances(
            market_info,
            vault_token,
            vault_quote_token,
            trader,
            maker_trader,
            maker_order_index,
        )
    } else {
        record_all_balances(
            market_info,
            vault_base_token,
            vault_token,
            trader,
            maker_trader,
            maker_order_index,
        )
    };

    // -- assume no loss of funds invariant
    cvt_assume_funds_invariants(balances_old);

    // -- atempt to withdraw an arbitrary amount
    let amount_arg: u64 = nondet();
    process_withdraw_core(
        &crate::id(),
        &used_acc_infos,
        WithdrawParams::new(amount_arg, None),
    )
    .unwrap();

    let balances_new: AllBalances = if IS_BASE {
        record_all_balances(
            market_info,
            vault_token,
            vault_quote_token,
            trader,
            maker_trader,
            maker_order_index,
        )
    } else {
        record_all_balances(
            market_info,
            vault_base_token,
            vault_token,
            trader,
            maker_trader,
            maker_order_index,
        )
    };

    // -- assert no loss of funds invariant
    cvt_assert_funds_invariants(balances_new);

    // -- additional properties
    cvt_assert_withdraw_extra::<IS_BASE>(balances_old, balances_new, amount_arg);

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_withdraw_base() {
    rule_withdraw_check::<true>();
}

#[rule]
pub fn rule_withdraw_quote() {
    rule_withdraw_check::<false>();
}

fn rest_remaining_check<const IS_BID: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader: &AccountInfo = &acc_infos[0];
    let market_info: &AccountInfo = &acc_infos[1];
    let maker_trader: &AccountInfo = &acc_infos[7];
    let vault_base_token: &AccountInfo = &acc_infos[8];
    let vault_quote_token: &AccountInfo = &acc_infos[9];

    // -- market preconditions
    let maker_order_index: DataIndex = cvt_assume_market_preconditions::<IS_BID>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );

    let balances_old: AllBalances = record_all_balances(
        market_info,
        vault_base_token,
        vault_quote_token,
        trader,
        maker_trader,
        maker_order_index,
    );

    // -- assume no loss of funds invariant
    cvt_assume_funds_invariants(balances_old);

    let args: AddOrderToMarketArgs = AddOrderToMarketArgs {
        market: *market_info.key,
        trader_index: main_trader_index(),
        num_base_atoms: nondet(),
        price: nondet(),
        is_bid: IS_BID,
        last_valid_slot: nondet(),
        order_type: state::OrderType::Limit,
        global_trade_accounts_opts: &[None, None],
        current_slot: Some(nondet()),
    };

    let remaining_base_atoms_arg: BaseAtoms = nondet();
    let order_sequence_number_arg: u64 = nondet();
    let total_base_atoms_traded_arg: BaseAtoms = nondet();
    let total_quote_atoms_traded_arg: QuoteAtoms = nondet();

    rest_remaining!(
        market_info,
        args,
        remaining_base_atoms_arg,
        order_sequence_number_arg,
        total_base_atoms_traded_arg,
        total_quote_atoms_traded_arg
    );

    let balances_new: AllBalances = record_all_balances(
        market_info,
        vault_base_token,
        vault_quote_token,
        trader,
        maker_trader,
        maker_order_index,
    );

    // -- assert no loss of funds invariant
    cvt_assert_funds_invariants(balances_new);

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_rest_remaining_ask() {
    rest_remaining_check::<false /* IS_BID */>();
}

#[rule]
pub fn rule_rest_remaining_bid() {
    rest_remaining_check::<true /* IS_BID */>();
}

pub fn cancel_order_by_index_check<const IS_BID: bool>() {
    // IS_BID = true sets up the rule such that the order to
    // be canceled is ask, and vice-versa

    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader: &AccountInfo = &acc_infos[0];
    let market_info: &AccountInfo = &acc_infos[1];
    let maker_trader: &AccountInfo = &acc_infos[2];
    let vault_base_token: &AccountInfo = &acc_infos[3];
    let vault_quote_token: &AccountInfo = &acc_infos[4];

    // -- market preconditions
    let maker_order_index: DataIndex = cvt_assume_market_preconditions::<IS_BID>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );

    let balances_old: AllBalances = record_all_balances(
        market_info,
        vault_base_token,
        vault_quote_token,
        trader,
        maker_trader,
        maker_order_index,
    );

    // -- assume no loss of funds invariant
    cvt_assume_funds_invariants(balances_old);

    // -- call to cancel_order_by_index
    let order_index: DataIndex = maker_order_index;
    cancel_order_by_index!(market_info, order_index);

    let balances_new: AllBalances = record_all_balances(
        market_info,
        vault_base_token,
        vault_quote_token,
        trader,
        maker_trader,
        maker_order_index,
    );

    // -- assert no loss of funds invariant
    cvt_assert_funds_invariants(balances_new);

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_cancel_order_by_index_ask() {
    // IS_BID = true cancels an ask order
    cancel_order_by_index_check::<true /* IS_BID */>();
}

#[rule]
pub fn rule_cancel_order_by_index_bid() {
    // IS_BID = false cancels a bid order
    cancel_order_by_index_check::<false /* IS_BID */>();
}

pub fn cancel_order_check<const IS_BID: bool>() {
    // IS_BID = true sets up the rule such that the order to
    // be canceled is ask, and vice-versa

    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader: &AccountInfo = &acc_infos[0];
    let market_info: &AccountInfo = &acc_infos[1];
    let maker_trader: &AccountInfo = &acc_infos[2];
    let vault_base_token: &AccountInfo = &acc_infos[3];
    let vault_quote_token: &AccountInfo = &acc_infos[4];

    // -- market preconditions
    let maker_order_index: DataIndex = cvt_assume_market_preconditions::<IS_BID>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );

    let balances_old: AllBalances = record_all_balances(
        market_info,
        vault_base_token,
        vault_quote_token,
        trader,
        maker_trader,
        maker_order_index,
    );

    // -- assume no loss of funds invariant
    cvt_assume_funds_invariants(balances_old);

    // -- call to cancel_order_by_index
    let trader_index: DataIndex = second_trader_index();
    let order_index: DataIndex = maker_order_index;
    // -- needed as an argument to get_helper_order, but is not used
    let dynamic: &mut [u8; 8] = &mut [0; 8];
    let resting_order: &RestingOrder = get_helper_order(dynamic, order_index).get_value();
    let order_sequence_number: u64 = resting_order.get_sequence_number();
    {
        let market_data: &mut std::cell::RefMut<&mut [u8]> =
            &mut market_info.try_borrow_mut_data().unwrap();
        let mut dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);
        dynamic_account
            .cancel_order(trader_index, order_sequence_number, &[None, None])
            .unwrap();
    };

    let balances_new: AllBalances = record_all_balances(
        market_info,
        vault_base_token,
        vault_quote_token,
        trader,
        maker_trader,
        maker_order_index,
    );

    // -- assert no loss of funds invariant
    cvt_assert_funds_invariants(balances_new);

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_cancel_order_ask() {
    // IS_BID = true cancels an ask order
    cancel_order_check::<true /* IS_BID */>();
}

#[rule]
pub fn rule_cancel_order_bid() {
    // IS_BID = false cancels a bid order
    cancel_order_check::<false /* IS_BID */>();
}
