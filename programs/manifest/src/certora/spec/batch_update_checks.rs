use certora::hooks::*;
use cvt::{cvt_assert, cvt_assume};
use cvt_macros::rule;
use nondet::{acc_infos_with_mem_layout, nondet};
use state::{main_ask_order_index, main_bid_order_index, main_trader_index};
use std::cell::RefMut;
use vectors::{cvt_no_resizable_vec, no_resizable_vec::NoResizableVec};

use crate::{
    program::{batch_update::*, get_mut_dynamic_account},
    state::*,
    *,
};
use hypertree::DataIndex;

// helper to prepare a cancel order
fn prepare_cancel_order<const IS_BID: bool>(order_sequence_number: u64) -> CancelOrderParams {
    return CancelOrderParams::new(order_sequence_number);
}

// helper to prepare a cancel order with hint
fn prepare_cancel_order_with_hint<const IS_BID: bool>(
    order_sequence_number: u64,
) -> CancelOrderParams {
    // -- we assume an order is present in the main order slot
    if IS_BID {
        cvt_assume!(!is_bid_order_free());
    } else {
        cvt_assume!(!is_ask_order_free());
    }

    let order_index: DataIndex = if IS_BID {
        main_bid_order_index()
    } else {
        main_ask_order_index()
    };
    // -- needed as an argument to get_helper_order, but is not used
    let dynamic: &mut [u8; 8] = &mut [0; 8];
    let order: RestingOrder = get_helper_order(dynamic, order_index).value;
    // -- make sure the order is consistent with our IS_BID and IS_GLOBAL
    cvt_assume!(order.get_is_bid() == IS_BID);

    return CancelOrderParams::new_with_hint(order_sequence_number, Some(order_index));
}

// helper to prepare a place order
fn prepare_place_order<const IS_BID: bool>() -> PlaceOrderParams {
    // -- we assume an order is present in the main order slot
    if IS_BID {
        cvt_assume!(!is_bid_order_free());
    } else {
        cvt_assume!(!is_ask_order_free());
    }

    // The type of order doesn't really matter because we use a mock for place order
    return PlaceOrderParams::new(
        nondet(),
        nondet(),
        nondet(),
        IS_BID,
        OrderType::Limit,
        nondet(),
    );
}

// Parametric rule
pub fn rule_integrity_of_batch_update_cancel<const IS_BID: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let used_acc_infos: &[AccountInfo] = &acc_infos[..3];

    // one cancel order without hint
    let cancels: NoResizableVec<CancelOrderParams> =
        cvt_no_resizable_vec!([prepare_cancel_order::<IS_BID>(nondet())]; 10);
    // no place orders
    let orders: NoResizableVec<PlaceOrderParams> = cvt_no_resizable_vec!([]; 10);
    let trader_index: DataIndex = main_trader_index();
    let params: BatchUpdateParams = BatchUpdateParams::new(Some(trader_index), cancels, orders);

    let program_id: &Pubkey = &crate::id();
    // Important: by passing only three accounts, we won't have global trade accounts
    process_batch_update_core(&program_id, &used_acc_infos, params).unwrap();

    cvt_assert!(last_called_cancel_order());
    cvt_vacuity_check!();
}

macro_rules! get_order {
    ($market_acc_info:expr, $order_index:expr) => {{
        let market_data: &mut RefMut<&mut [u8]> =
            &mut $market_acc_info.try_borrow_mut_data().unwrap();
        let dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);
        let order: &RestingOrder = dynamic_account.get_order_by_index($order_index);
        *order
    }};
}

// Parametric rule
pub fn rule_integrity_of_batch_update_cancel_hint<const IS_BID: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let used_acc_infos: &[AccountInfo] = &acc_infos[..3];
    let market_info: &AccountInfo = &used_acc_infos[1];

    // One cancel order with hint
    let order_params: CancelOrderParams = prepare_cancel_order_with_hint::<IS_BID>(nondet());
    cvt_assert!(order_params.order_index_hint().is_some());
    let order_index: DataIndex = order_params.order_index_hint().unwrap();
    let order: RestingOrder = get_order!(market_info, order_index);

    let cancels: NoResizableVec<CancelOrderParams> = cvt_no_resizable_vec!([order_params]; 10);
    // No place orders
    let orders: NoResizableVec<PlaceOrderParams> = cvt_no_resizable_vec!([]; 10);
    let trader_index: DataIndex = main_trader_index();
    let params = BatchUpdateParams::new(Some(trader_index), cancels, orders);

    let program_id: &Pubkey = &crate::id();
    // Important: by passing only three accounts, we won't have global trade accounts
    process_batch_update_core(&program_id, &used_acc_infos, params).unwrap();

    cvt_assert!(last_called_cancel_order_by_index());
    // Our mocks produce always aligned order indexes
    cvt_assert!(order_index % (MARKET_BLOCK_SIZE as DataIndex) == 0);
    // Our mocks produce always aligned trader indexes
    cvt_assert!(trader_index % (MARKET_BLOCK_SIZE as DataIndex) == 0);
    cvt_assert!(order.get_is_bid() == IS_BID);
    cvt_assert!(order.get_trader_index() == trader_index);

    cvt_vacuity_check!();
}

// Parametric rule
pub fn rule_integrity_of_batch_update_place_order<const IS_BID: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let used_acc_infos: &[AccountInfo] = &acc_infos[..3];

    // no cancel orders
    let cancels: NoResizableVec<CancelOrderParams> = cvt_no_resizable_vec!([]; 10);
    // one place order
    let orders: NoResizableVec<PlaceOrderParams> =
        cvt_no_resizable_vec!([prepare_place_order::<IS_BID>()]; 10);
    let trader_index: DataIndex = main_trader_index();
    let params: BatchUpdateParams = BatchUpdateParams::new(Some(trader_index), cancels, orders);

    let program_id: &Pubkey = &crate::id();
    // Important: by passing only three accounts, we won't have global trade accounts
    process_batch_update_core(&program_id, &used_acc_infos, params).unwrap();

    cvt_assert!(last_called_place_order());
    cvt_vacuity_check!();
}

#[rule]
pub fn rule_integrity_of_batch_update_cancel_bid() {
    rule_integrity_of_batch_update_cancel::<true>()
}

#[rule]
#[inline(never)]
pub fn rule_integrity_of_batch_update_cancel_ask() {
    rule_integrity_of_batch_update_cancel::<false>()
}

#[rule]
fn rule_integrity_of_batch_update_cancel_hint_bid() {
    rule_integrity_of_batch_update_cancel_hint::<true>()
}

#[rule]
fn rule_integrity_of_batch_update_cancel_hint_ask() {
    rule_integrity_of_batch_update_cancel_hint::<false>()
}

#[rule]
fn rule_integrity_of_batch_update_place_order_bid() {
    rule_integrity_of_batch_update_place_order::<true>()
}

#[rule]
fn rule_integrity_of_batch_update_place_order_ask() {
    rule_integrity_of_batch_update_place_order::<false>()
}
