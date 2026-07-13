use crate::*;
use cvt::{cvt_assert, cvt_assume};
use cvt_macros::rule;
use nondet::*;

use certora::hooks::last_called_remove_order_from_tree_and_free;
use solana_program::account_info::AccountInfo;

use certora::spec::place_order_checks::place_single_order_nondet_inputs;
use state::get_helper_order;

use crate::{
    certora::spec::no_funds_loss_util::*,
    program::get_mut_dynamic_account,
    quantities::{BaseAtoms, QuoteAtoms, QuoteAtomsPerBaseAtom, WrapperU64},
    state::{
        market::market_helpers::{AddOrderStatus, AddOrderToMarketInnerResult, AddSingleOrderCtx},
        DynamicAccount, MarketRefMut, RestingOrder,
    },
};
use hypertree::DataIndex;

// Rules to check if a matching order exists in the book,
// then matching will happen
pub fn matching_if_maker_order_exists<const IS_BID: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader = &acc_infos[0];
    let market_info = &acc_infos[1];

    let maker_trader = &acc_infos[7];
    let vault_base_token = &acc_infos[8];
    let vault_quote_token = &acc_infos[9];

    // -- market preconditions
    let maker_order_index: DataIndex = cvt_assume_market_preconditions::<IS_BID>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );

    let (args, remaining_base_atoms, now_slot) =
        place_single_order_nondet_inputs::<IS_BID>(market_info);

    // -- assumptions that maker_order is not expired and it matches on price
    let dynamic: &mut [u8; 8] = &mut [0; 8];
    let maker_order: &RestingOrder = get_helper_order(dynamic, maker_order_index).get_value();
    let maker_order_price = maker_order.get_price();
    // -- maker_order is not expired
    cvt_assume!(!maker_order.is_expired(now_slot));
    // -- maker_order matches on price
    if IS_BID {
        cvt_assume!(maker_order_price <= args.price);
    } else {
        cvt_assume!(maker_order_price >= args.price);
    }

    // -- call to place_single_order
    let (res, _total_base_atoms_traded, _total_quote_atoms_traded) = place_single_order!(
        market_info,
        args,
        remaining_base_atoms,
        now_slot,
        maker_order_index
    );

    // -- assert to make sure that the order matched partially or fully
    cvt_assert!(res.status == AddOrderStatus::PartialFill || res.status == AddOrderStatus::Filled);

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_matching_if_maker_order_exists_bid() {
    matching_if_maker_order_exists::<true /* IS_BID */>();
}

#[rule]
pub fn rule_matching_if_maker_order_exists_ask() {
    matching_if_maker_order_exists::<false /* IS_BID */>();
}

// Rules to check if matching happened, then prices must have crossed
pub fn crossed_prices_if_matched<const IS_BID: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader = &acc_infos[0];
    let market_info = &acc_infos[1];

    let maker_trader = &acc_infos[7];
    let vault_base_token = &acc_infos[8];
    let vault_quote_token = &acc_infos[9];

    // -- market preconditions
    let maker_order_index: DataIndex = cvt_assume_market_preconditions::<IS_BID>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );

    let (args, remaining_base_atoms, now_slot) =
        place_single_order_nondet_inputs::<IS_BID>(market_info);

    // -- get maker_order price
    let dynamic: &mut [u8; 8] = &mut [0; 8];
    let maker_order: &RestingOrder = get_helper_order(dynamic, maker_order_index).get_value();
    let maker_order_price = maker_order.get_price();
    let args_price = args.price;

    // -- call to place_single_order
    let (res, _total_base_atoms_traded, _total_quote_atoms_traded) = place_single_order!(
        market_info,
        args,
        remaining_base_atoms,
        now_slot,
        maker_order_index
    );
    // -- assume that the order matched partially or fully
    cvt_assume!(res.status == AddOrderStatus::PartialFill || res.status == AddOrderStatus::Filled);

    // -- assert to check that the orders must have matched on price
    if IS_BID {
        cvt_assert!(maker_order_price <= args_price);
    } else {
        cvt_assert!(maker_order_price >= args_price);
    }

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_crossed_prices_if_matched_bid() {
    crossed_prices_if_matched::<true /* IS_BID */>();
}

#[rule]
pub fn rule_crossed_prices_if_matched_ask() {
    crossed_prices_if_matched::<false /* IS_BID */>();
}

// Rules to verify that trader balances are modified as expected in the fully_matched case
pub fn place_single_order_full_match_balances<const IS_BID: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader = &acc_infos[0];
    let market_info = &acc_infos[1];

    let maker_trader = &acc_infos[7];
    let vault_base_token = &acc_infos[8];
    let vault_quote_token = &acc_infos[9];

    // -- market assumptions

    let maker_order_index: DataIndex = cvt_assume_market_preconditions::<IS_BID>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );

    // -- the exact deltas below only hold for a maker that does not reverse
    cvt_assume_maker_not_reversible(maker_order_index);

    // -- record trader balances before place_single_order
    let (trader_base_old, trader_quote_old) = get_trader_balance!(market_info, trader.key);
    let (maker_trader_base_old, maker_trader_quote_old) =
        get_trader_balance!(market_info, maker_trader.key);

    // -- compute base_atoms_traded and quote_atoms_traded
    let dynamic = [0u8; 8];
    let maker_order = get_helper_order(&dynamic, maker_order_index).get_value();
    let base_atoms_traded: BaseAtoms = maker_order.get_num_base_atoms();
    let matched_price: QuoteAtomsPerBaseAtom = maker_order.get_price();
    let quote_atoms_traded: QuoteAtoms = matched_price
        .checked_quote_for_base(base_atoms_traded, IS_BID != true)
        .unwrap();

    let (args, remaining_base_atoms, now_slot) =
        place_single_order_nondet_inputs::<IS_BID>(market_info);

    // -- call to place_single_order
    let (res, _total_base_atoms_traded, _total_quote_atoms_traded) = place_single_order!(
        market_info,
        args,
        remaining_base_atoms,
        now_slot,
        maker_order_index
    );
    cvt_assume!(res.status == AddOrderStatus::Filled);

    // -- record trader balances after place_single_order
    let (trader_base_new, trader_quote_new) = get_trader_balance!(market_info, trader.key);
    let (maker_trader_base_new, maker_trader_quote_new) =
        get_trader_balance!(market_info, maker_trader.key);

    // -- asserts to establish that trader balances changed as expected
    if IS_BID {
        cvt_assert!(trader_base_new == trader_base_old + base_atoms_traded.as_u64());
        cvt_assert!(trader_quote_new == trader_quote_old - quote_atoms_traded.as_u64());
        cvt_assert!(maker_trader_base_new == maker_trader_base_old);
        cvt_assert!(maker_trader_quote_new == maker_trader_quote_old + quote_atoms_traded.as_u64());
    } else {
        cvt_assert!(trader_base_new == trader_base_old - base_atoms_traded.as_u64());
        cvt_assert!(trader_quote_new == trader_quote_old + quote_atoms_traded.as_u64());
        cvt_assert!(maker_trader_base_new == maker_trader_base_old + base_atoms_traded.as_u64());
        cvt_assert!(maker_trader_quote_new == maker_trader_quote_old);
    }

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_place_single_order_full_match_balances_bid() {
    place_single_order_full_match_balances::<true /* IS_BID */>();
}

#[rule]
pub fn rule_place_single_order_full_match_balances_ask() {
    place_single_order_full_match_balances::<false /* IS_BID */>();
}

// Rules to verify that trader balances are modified as expected in the partially_matched case
pub fn place_single_order_partial_match_balances<const IS_BID: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader = &acc_infos[0];
    let market_info = &acc_infos[1];

    let maker_trader = &acc_infos[7];
    let vault_base_token = &acc_infos[8];
    let vault_quote_token = &acc_infos[9];

    // -- market assumptions

    let maker_order_index: DataIndex = cvt_assume_market_preconditions::<IS_BID>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );

    // -- the exact deltas below only hold for a maker that does not reverse
    cvt_assume_maker_not_reversible(maker_order_index);

    // -- record trader balances before place_single_order
    let (trader_base_old, trader_quote_old) = get_trader_balance!(market_info, trader.key);
    let (maker_trader_base_old, maker_trader_quote_old) =
        get_trader_balance!(market_info, maker_trader.key);

    let (args, remaining_base_atoms, now_slot) =
        place_single_order_nondet_inputs::<IS_BID>(market_info);

    // -- compute base_atoms_traded and quote_atoms_traded
    let dynamic = [0u8; 8];
    let maker_order = get_helper_order(&dynamic, maker_order_index).get_value();
    let base_atoms_traded: BaseAtoms = remaining_base_atoms;
    let matched_price: QuoteAtomsPerBaseAtom = maker_order.get_price();
    let quote_atoms_traded: QuoteAtoms = matched_price
        .checked_quote_for_base(base_atoms_traded, IS_BID != false)
        .unwrap();

    // -- call to place_single_order
    let (res, _total_base_atoms_traded, _total_quote_atoms_traded) = place_single_order!(
        market_info,
        args,
        remaining_base_atoms,
        now_slot,
        maker_order_index
    );
    cvt_assume!(res.status == AddOrderStatus::PartialFill);

    // -- record trader balances after place_single_order
    let (trader_base_new, trader_quote_new) = get_trader_balance!(market_info, trader.key);
    let (maker_trader_base_new, maker_trader_quote_new) =
        get_trader_balance!(market_info, maker_trader.key);

    // -- asserts to establish that trader balances changed as expected
    if IS_BID {
        cvt_assert!(trader_base_new == trader_base_old + base_atoms_traded.as_u64());
        cvt_assert!(trader_quote_new == trader_quote_old - quote_atoms_traded.as_u64());
        cvt_assert!(maker_trader_base_new == maker_trader_base_old);
        cvt_assert!(maker_trader_quote_new == maker_trader_quote_old + quote_atoms_traded.as_u64());
    } else {
        cvt_assert!(trader_base_new == trader_base_old - base_atoms_traded.as_u64());
        cvt_assert!(trader_quote_new == trader_quote_old + quote_atoms_traded.as_u64());
        cvt_assert!(maker_trader_base_new == maker_trader_base_old + base_atoms_traded.as_u64());

        // maker may receive a bonus quote atom due to rounding
        cvt_assert!(maker_trader_quote_old <= maker_trader_quote_new);
        cvt_assert!(maker_trader_quote_new <= maker_trader_quote_old + 1);
    }

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_place_single_order_partial_match_balances_bid() {
    place_single_order_partial_match_balances::<true /* IS_BID */>();
}

#[rule]
pub fn rule_place_single_order_partial_match_balances_ask() {
    place_single_order_partial_match_balances::<false /* IS_BID */>();
}

// Rules to check if the maker_order is fully matched, then it is removed
pub fn matching_order_removed_if_fully_matched<const IS_BID: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader = &acc_infos[0];
    let market_info = &acc_infos[1];

    let maker_trader = &acc_infos[7];
    let vault_base_token = &acc_infos[8];
    let vault_quote_token = &acc_infos[9];

    // -- market assumptions

    let maker_order_index: DataIndex = cvt_assume_market_preconditions::<IS_BID>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );

    let (args, remaining_base_atoms, now_slot) =
        place_single_order_nondet_inputs::<IS_BID>(market_info);

    // -- call to place_single_order
    let (res, _total_base_atoms_traded, _total_quote_atoms_traded) = place_single_order!(
        market_info,
        args,
        remaining_base_atoms,
        now_slot,
        maker_order_index
    );
    cvt_assume!(res.status == AddOrderStatus::Filled);

    // -- assert that remove_order_from_tree_and_free was called
    cvt_assert!(last_called_remove_order_from_tree_and_free());

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_matching_order_removed_if_fully_matched_bid() {
    matching_order_removed_if_fully_matched::<true /* IS_BID */>();
}

#[rule]
pub fn rule_matching_order_removed_if_fully_matched_ask() {
    matching_order_removed_if_fully_matched::<false /* IS_BID */>();
}

// Rules to check if the maker_order was (i) not expired (ii) matched on price removed
// and (iii) removed from the tree; then it must have been fully_matched
pub fn matching_fully_matched_if_order_removed<const IS_BID: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader = &acc_infos[0];
    let market_info = &acc_infos[1];

    let maker_trader = &acc_infos[7];
    let vault_base_token = &acc_infos[8];
    let vault_quote_token = &acc_infos[9];

    // -- market assumptions

    let maker_order_index: DataIndex = cvt_assume_market_preconditions::<IS_BID>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );

    let (args, remaining_base_atoms, now_slot) =
        place_single_order_nondet_inputs::<IS_BID>(market_info);

    // -- assumptions that maker_order is not expired and it matches on price
    let dynamic: &mut [u8; 8] = &mut [0; 8];
    let maker_order: &RestingOrder = get_helper_order(dynamic, maker_order_index).get_value();
    // -- maker_order is not expired
    cvt_assume!(!maker_order.is_expired(now_slot));

    // -- call to place_single_order
    let (res, _total_base_atoms_traded, _total_quote_atoms_traded) = place_single_order!(
        market_info,
        args,
        remaining_base_atoms,
        now_slot,
        maker_order_index
    );

    // -- assume that remove_order_from_tree_and_free was called
    cvt_assume!(last_called_remove_order_from_tree_and_free());

    // -- assert that maker_order must have been fully matched
    cvt_assert!(res.status == AddOrderStatus::Filled);

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_matching_fully_matched_if_order_removed_bid() {
    matching_fully_matched_if_order_removed::<true /* IS_BID */>();
}

#[rule]
pub fn rule_matching_fully_matched_if_order_removed_ask() {
    matching_fully_matched_if_order_removed::<false /* IS_BID */>();
}

// Rules to check that the atoms contained in maker_order cannot increase after matching
pub fn matching_decrease_maker_order_atoms<const IS_BID: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader = &acc_infos[0];
    let market_info = &acc_infos[1];

    let maker_trader = &acc_infos[7];
    let vault_base_token = &acc_infos[8];
    let vault_quote_token = &acc_infos[9];

    // -- market assumptions

    let maker_order_index: DataIndex = cvt_assume_market_preconditions::<IS_BID>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );

    let (maker_order_base_old, maker_order_quote_old) = get_order_atoms!(maker_order_index);

    let (args, remaining_base_atoms, now_slot) =
        place_single_order_nondet_inputs::<IS_BID>(market_info);

    // -- call to place_single_order
    let (_res, _total_base_atoms_traded, _total_quote_atoms_traded) = place_single_order!(
        market_info,
        args,
        remaining_base_atoms,
        now_slot,
        maker_order_index
    );

    let (maker_order_base_new, maker_order_quote_new) = get_order_atoms!(maker_order_index);

    // -- assert that maker_order atoms cannot increase
    cvt_assert!(maker_order_base_new.as_u64() <= maker_order_base_old.as_u64());
    cvt_assert!(maker_order_quote_new.as_u64() <= maker_order_quote_old.as_u64());

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_matching_decrease_maker_order_atoms_bid() {
    matching_decrease_maker_order_atoms::<true /* IS_BID */>();
}

#[rule]
pub fn rule_matching_decrease_maker_order_atoms_ask() {
    matching_decrease_maker_order_atoms::<false /* IS_BID */>();
}
