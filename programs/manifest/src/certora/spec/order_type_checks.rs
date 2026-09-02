//! Order types other than plain limit orders.
//!
//! The rules elsewhere fix the taker to a limit order and let the maker be any
//! non-global type. These cover what is special about each of the other types:
//!
//! - PostOnly and Global takers must never take, they fail instead of crossing.
//! - ImmediateOrCancel takes like a limit order but never rests.
//! - Reverse and ReverseTight makers come back onto the other side of the book
//!   when they are filled, which moves funds around, so they get their own no
//!   loss of funds rules, including the coalesce case where the reverse order
//!   is folded into an existing order at the same price.
use crate::*;
use cvt::{cvt_assert, cvt_assume};
use cvt_macros::rule;
use nondet::*;

use solana_program::account_info::AccountInfo;

use crate::{
    certora::spec::{
        no_funds_loss_util::*, place_order_checks::place_single_order_nondet_inputs_with_type,
    },
    program::get_mut_dynamic_account,
    quantities::WrapperU64,
    state::{
        get_helper_order,
        market::market_helpers::{AddOrderStatus, AddOrderToMarketInnerResult, AddSingleOrderCtx},
        DynamicAccount, MarketRefMut, OrderType, RestingOrder,
    },
};
use hypertree::DataIndex;

/// A taker that is not allowed to take never trades. In production a crossing
/// post only or global taker fails the instruction with PostOnlyCrosses.
/// Under the certora feature `require!` compiles to an assume, so that failure
/// shows up here as the crossing paths being infeasible: what is asserted is
/// that no reachable execution of such a taker records a trade. The reachable
/// executions are the non-crossing ones (Unmatched) and the expired-maker
/// cleanup (Canceled), which keeps the rule non-vacuous.
pub fn taker_cannot_take_check<const IS_BID: bool>(order_type: OrderType) {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader: &AccountInfo = &acc_infos[0];
    let market_info: &AccountInfo = &acc_infos[1];
    let maker_trader: &AccountInfo = &acc_infos[7];
    let vault_base_token: &AccountInfo = &acc_infos[8];
    let vault_quote_token: &AccountInfo = &acc_infos[9];

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
    cvt_assume_funds_invariants(balances_old);

    let (args, remaining_base_atoms, now_slot) = place_single_order_nondet_inputs_with_type::<IS_BID>(
        market_info,
        order_type,
        &[None, None],
    );

    let (res, total_base_atoms_traded, total_quote_atoms_traded) = place_single_order!(
        market_info,
        args,
        remaining_base_atoms,
        now_slot,
        maker_order_index
    );

    // -- no reachable execution matched
    cvt_assert!(res.status != AddOrderStatus::Filled);
    cvt_assert!(res.status != AddOrderStatus::PartialFill);
    cvt_assert!(total_base_atoms_traded == crate::quantities::BaseAtoms::ZERO);
    cvt_assert!(total_quote_atoms_traded == crate::quantities::QuoteAtoms::ZERO);

    // -- and the taker's balances are untouched (the maker may still be
    // -- refunded if their order was expired and got cleaned up)
    let balances_new: AllBalances = record_all_balances(
        market_info,
        vault_base_token,
        vault_quote_token,
        trader,
        maker_trader,
        maker_order_index,
    );
    cvt_assert_funds_invariants(balances_new);
    cvt_assert!(balances_old.trader_base == balances_new.trader_base);
    cvt_assert!(balances_old.trader_quote == balances_new.trader_quote);
    cvt_assert!(balances_old.vault_base == balances_new.vault_base);
    cvt_assert!(balances_old.vault_quote == balances_new.vault_quote);

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_post_only_cannot_take_bid() {
    taker_cannot_take_check::<true /* IS_BID */>(OrderType::PostOnly);
}

#[rule]
pub fn rule_post_only_cannot_take_ask() {
    taker_cannot_take_check::<false /* IS_BID */>(OrderType::PostOnly);
}

#[rule]
pub fn rule_global_taker_cannot_take_bid() {
    taker_cannot_take_check::<true /* IS_BID */>(OrderType::Global);
}

#[rule]
pub fn rule_global_taker_cannot_take_ask() {
    taker_cannot_take_check::<false /* IS_BID */>(OrderType::Global);
}

/// No loss of funds for takers of every order type that is allowed to take.
pub fn place_single_order_funds_check<const IS_BID: bool>(order_type: OrderType) {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader: &AccountInfo = &acc_infos[0];
    let market_info: &AccountInfo = &acc_infos[1];
    let maker_trader: &AccountInfo = &acc_infos[7];
    let vault_base_token: &AccountInfo = &acc_infos[8];
    let vault_quote_token: &AccountInfo = &acc_infos[9];

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

    cvt_assume_funds_invariants(balances_old);

    let (args, remaining_base_atoms, now_slot) = place_single_order_nondet_inputs_with_type::<IS_BID>(
        market_info,
        order_type,
        &[None, None],
    );

    let (_res, _total_base_atoms_traded, _total_quote_atoms_traded) = place_single_order!(
        market_info,
        args,
        remaining_base_atoms,
        now_slot,
        maker_order_index
    );

    let balances_new: AllBalances = record_all_balances(
        market_info,
        vault_base_token,
        vault_quote_token,
        trader,
        maker_trader,
        maker_order_index,
    );

    cvt_assert_funds_invariants(balances_new);

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_immediate_or_cancel_no_funds_loss_bid() {
    place_single_order_funds_check::<true /* IS_BID */>(OrderType::ImmediateOrCancel);
}

#[rule]
pub fn rule_immediate_or_cancel_no_funds_loss_ask() {
    place_single_order_funds_check::<false /* IS_BID */>(OrderType::ImmediateOrCancel);
}

#[rule]
pub fn rule_reverse_taker_no_funds_loss_bid() {
    place_single_order_funds_check::<true /* IS_BID */>(OrderType::Reverse);
}

#[rule]
pub fn rule_reverse_taker_no_funds_loss_ask() {
    place_single_order_funds_check::<false /* IS_BID */>(OrderType::Reverse);
}

/// A reverse maker is put back on the other side of the book when it is filled,
/// which debits the maker again and reserves the funds on the orderbook. No
/// funds may be lost or created on the way round.
pub fn reverse_maker_check<const IS_BID: bool, const IS_TIGHT: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader: &AccountInfo = &acc_infos[0];
    let market_info: &AccountInfo = &acc_infos[1];
    let maker_trader: &AccountInfo = &acc_infos[7];
    let vault_base_token: &AccountInfo = &acc_infos[8];
    let vault_quote_token: &AccountInfo = &acc_infos[9];

    let maker_order_index: DataIndex = cvt_assume_market_preconditions::<IS_BID>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );

    // -- the maker on the book is a reverse order
    let dynamic: &mut [u8; 8] = &mut [0; 8];
    let maker_order: &RestingOrder = get_helper_order(dynamic, maker_order_index).get_value();
    if IS_TIGHT {
        cvt_assume!(maker_order.get_order_type() == OrderType::ReverseTight);
    } else {
        cvt_assume!(maker_order.get_order_type() == OrderType::Reverse);
    }

    let balances_old: AllBalances = record_all_balances(
        market_info,
        vault_base_token,
        vault_quote_token,
        trader,
        maker_trader,
        maker_order_index,
    );

    cvt_assume_funds_invariants(balances_old);

    let (args, remaining_base_atoms, now_slot) = place_single_order_nondet_inputs_with_type::<IS_BID>(
        market_info,
        OrderType::Limit,
        &[None, None],
    );

    let (res, _total_base_atoms_traded, _total_quote_atoms_traded) = place_single_order!(
        market_info,
        args,
        remaining_base_atoms,
        now_slot,
        maker_order_index
    );
    cvt_assume!(res.status == AddOrderStatus::Filled || res.status == AddOrderStatus::PartialFill);

    let balances_new: AllBalances = record_all_balances(
        market_info,
        vault_base_token,
        vault_quote_token,
        trader,
        maker_trader,
        maker_order_index,
    );

    // -- no loss of funds, even though the maker order came back on the other side
    cvt_assert_funds_invariants(balances_new);

    // -- the vaults are untouched, matching only moves credit around inside the market
    cvt_assert!(balances_old.vault_base == balances_new.vault_base);
    cvt_assert!(balances_old.vault_quote == balances_new.vault_quote);
    cvt_assert!(
        balances_old
            .withdrawable_base
            .saturating_add(balances_old.orderbook_base)
            == balances_new
                .withdrawable_base
                .saturating_add(balances_new.orderbook_base)
    );
    cvt_assert!(
        balances_old
            .withdrawable_quote
            .saturating_add(balances_old.orderbook_quote)
            == balances_new
                .withdrawable_quote
                .saturating_add(balances_new.orderbook_quote)
    );

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_reverse_maker_no_funds_loss_bid() {
    reverse_maker_check::<true /* IS_BID */, false /* IS_TIGHT */>();
}

#[rule]
pub fn rule_reverse_maker_no_funds_loss_ask() {
    reverse_maker_check::<false /* IS_BID */, false /* IS_TIGHT */>();
}

#[rule]
pub fn rule_reverse_tight_maker_no_funds_loss_bid() {
    reverse_maker_check::<true /* IS_BID */, true /* IS_TIGHT */>();
}

#[rule]
pub fn rule_reverse_tight_maker_no_funds_loss_ask() {
    reverse_maker_check::<false /* IS_BID */, true /* IS_TIGHT */>();
}

/// A filled reverse maker whose come-back order coalesces into an existing
/// order at the same price. The coalesced order's backing grows, the maker is
/// debited exactly that growth, and no funds are lost or created.
///
/// This is the path where the maker's debit is NOT simply size times price:
/// the coalesced order's allocation is rounded as a whole, so the debit has to
/// be the difference of allocations or an atom strands in the vault (or the
/// order ends up under-backed when the coalesce target sits one price
/// increment away, which RestingOrder::eq tolerates).
pub fn reverse_coalesce_check<const IS_BID: bool, const IS_TIGHT: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader: &AccountInfo = &acc_infos[0];
    let market_info: &AccountInfo = &acc_infos[1];
    let maker_trader: &AccountInfo = &acc_infos[7];
    let vault_base_token: &AccountInfo = &acc_infos[8];
    let vault_quote_token: &AccountInfo = &acc_infos[9];

    let (maker_order_index, coalesce_order_index) =
        cvt_assume_reverse_coalesce_preconditions::<IS_BID, IS_TIGHT>(
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
    cvt_assume_funds_invariants(balances_old);

    // -- the coalesce target's reserved atoms are also part of the orderbook
    // -- aggregate, next to the maker order's
    let (coalesce_base_old, coalesce_quote_old) = get_order_atoms!(coalesce_order_index);
    cvt_assume!(balances_old
        .maker_order_base
        .checked_add(coalesce_base_old.as_u64())
        .is_some_and(|sum| sum <= balances_old.orderbook_base));
    cvt_assume!(balances_old
        .maker_order_quote
        .checked_add(coalesce_quote_old.as_u64())
        .is_some_and(|sum| sum <= balances_old.orderbook_quote));

    let (args, remaining_base_atoms, now_slot) = place_single_order_nondet_inputs_with_type::<IS_BID>(
        market_info,
        OrderType::Limit,
        &[None, None],
    );

    let (res, _total_base_atoms_traded, _total_quote_atoms_traded) = place_single_order!(
        market_info,
        args,
        remaining_base_atoms,
        now_slot,
        maker_order_index
    );
    cvt_assume!(res.status == AddOrderStatus::Filled || res.status == AddOrderStatus::PartialFill);

    let balances_new: AllBalances = record_all_balances(
        market_info,
        vault_base_token,
        vault_quote_token,
        trader,
        maker_trader,
        maker_order_index,
    );

    // -- no loss of funds through the coalesce
    cvt_assert_funds_invariants(balances_new);

    // -- the coalesced order only ever grows
    let (coalesce_base_new, _coalesce_quote_new) = get_order_atoms!(coalesce_order_index);
    cvt_assert!(coalesce_base_new.as_u64() >= coalesce_base_old.as_u64());

    // -- matching moves no tokens in or out of the vaults
    cvt_assert!(balances_old.vault_base == balances_new.vault_base);
    cvt_assert!(balances_old.vault_quote == balances_new.vault_quote);
    cvt_assert!(
        balances_old
            .withdrawable_base
            .saturating_add(balances_old.orderbook_base)
            == balances_new
                .withdrawable_base
                .saturating_add(balances_new.orderbook_base)
    );
    cvt_assert!(
        balances_old
            .withdrawable_quote
            .saturating_add(balances_old.orderbook_quote)
            == balances_new
                .withdrawable_quote
                .saturating_add(balances_new.orderbook_quote)
    );

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_reverse_coalesce_bid() {
    reverse_coalesce_check::<true /* IS_BID */, false /* IS_TIGHT */>();
}

#[rule]
pub fn rule_reverse_coalesce_ask() {
    reverse_coalesce_check::<false /* IS_BID */, false /* IS_TIGHT */>();
}

#[rule]
pub fn rule_reverse_tight_coalesce_bid() {
    reverse_coalesce_check::<true /* IS_BID */, true /* IS_TIGHT */>();
}

#[rule]
pub fn rule_reverse_tight_coalesce_ask() {
    reverse_coalesce_check::<false /* IS_BID */, true /* IS_TIGHT */>();
}
