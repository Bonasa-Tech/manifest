use std::{mem::size_of, str::FromStr};

use hypertree::{
    get_helper, DataIndex, HyperTreeReadOperations, HyperTreeValueIteratorTrait,
    LinkedListReadOnly, RBNode, RedBlackTreeReadOnly, NIL,
};
use manifest::quantities::QuoteAtomsPerBaseAtom;
use serde_json::{Map, Value};
use solana_program_test::ProgramTestContext;
use solana_sdk::{account::Account, pubkey::Pubkey};
use wrapper::{
    market_info::MarketInfo,
    open_order::WrapperOpenOrder,
    processors::{
        batch_upate::WrapperCancelOrderParams,
        shared::{MarketInfosTree, MarketInfosTreeReadOnly},
    },
    wrapper_state::ManifestWrapperStateFixed,
};

use super::{PhoenixCancelOrderParams, PhoenixCancelUpToOrderParams, Side};

pub type Obj = Map<String, Value>;

/// `payload_type` a market info node carries once the program has converted
/// that market's open orders to a linked list. Zero, the value a market info
/// written before the change has, means they are still a red-black tree.
const ORDERS_LAYOUT_LIST: u8 = 1;

/// The open orders of one market, in whichever layout they are in right now.
/// The harness builds them as a tree, which is what a wrapper on chain looks
/// like before the program first touches it, and the program converts them on
/// that first instruction, so one replay sees both. Searching them is a linear
/// scan either way and does not depend on the order they come back in.
fn collect_open_orders(
    wrapper_dynamic_data: &[u8],
    market_info_index: DataIndex,
) -> Vec<WrapperOpenOrder> {
    let market_info_node: &RBNode<MarketInfo> =
        get_helper::<RBNode<MarketInfo>>(wrapper_dynamic_data, market_info_index);
    let orders_root_index: DataIndex = market_info_node.get_value().orders_root_index;
    if market_info_node.get_payload_type() == ORDERS_LAYOUT_LIST {
        let orders: LinkedListReadOnly<WrapperOpenOrder> =
            unsafe { LinkedListReadOnly::new(wrapper_dynamic_data, orders_root_index) };
        orders.iter().map(|(_index, order)| *order).collect()
    } else {
        let orders: RedBlackTreeReadOnly<WrapperOpenOrder> =
            unsafe { RedBlackTreeReadOnly::new(wrapper_dynamic_data, orders_root_index, NIL) };
        orders.iter().map(|(_index, order)| *order).collect()
    }
}

/// Turn a Phoenix cancel-all into explicit cancels for the orders represented
/// in this benchmark's wrapper account. The fixture places its initial and
/// replayed resting orders through the wrapper; its direct swaps are IOC.
pub async fn cancel_all_wrapper_orders(
    context: &mut ProgramTestContext,
    wrapper: &Pubkey,
    market: &Pubkey,
) -> Vec<WrapperCancelOrderParams> {
    let wrapper_account: Account = context
        .banks_client
        .get_account(*wrapper)
        .await
        .expect("Fetch wrapper")
        .expect("Wrapper is not none");
    let (fixed_data, wrapper_dynamic_data) =
        wrapper_account.data.split_at(size_of::<ManifestWrapperStateFixed>());
    let wrapper_fixed: &ManifestWrapperStateFixed = get_helper(fixed_data, 0);
    let market_infos_tree: MarketInfosTreeReadOnly = unsafe {
        MarketInfosTreeReadOnly::new(
            wrapper_dynamic_data,
            wrapper_fixed.market_infos_root_index,
            NIL,
        )
    };
    let market_info_index: DataIndex = market_infos_tree
        .lookup_index(&MarketInfo::new_empty(market.to_bytes().into(), NIL));
    collect_open_orders(wrapper_dynamic_data, market_info_index)
        .into_iter()
        .map(|order| WrapperCancelOrderParams::new(order.get_client_order_id()))
        .collect()
}

pub fn get_market_info_index_for_market(wrapper_data: &mut [u8], market_key: &Pubkey) -> DataIndex {
    let (fixed_data, wrapper_dynamic_data) =
        wrapper_data.split_at_mut(size_of::<ManifestWrapperStateFixed>());

    let wrapper_fixed: &ManifestWrapperStateFixed = get_helper(fixed_data, 0);
    let market_infos_tree: MarketInfosTree = unsafe {
        MarketInfosTree::new(
            wrapper_dynamic_data,
            wrapper_fixed.market_infos_root_index,
            NIL,
        )
    };

    // Just need to lookup by market key so the rest doesnt matter.
    let market_info_index: DataIndex =
        market_infos_tree.lookup_index(&MarketInfo::new_empty(market_key.to_bytes().into(), NIL));
    market_info_index
}

fn from_hex(str: &str) -> u64 {
    u64::from_str_radix(str, 16).unwrap()
}

pub fn from_hex_u128(str: &str) -> u128 {
    u128::from_str_radix(str, 16).unwrap()
}

pub async fn match_order_in_wrapper_orders_tree(
    context: &mut ProgramTestContext,
    wrapper: &Pubkey,
    market: &Pubkey,
    order: PhoenixCancelOrderParams,
    base_lot_size: u64,
    quote_lot_size: u64,
) -> Option<WrapperOpenOrder> {
    let wrapper_account: Account = context
        .banks_client
        .get_account(*wrapper)
        .await
        .expect("Fetch wrapper")
        .expect("Wrapper is not none");
    let (fixed_data, wrapper_dynamic_data) =
        wrapper_account.data[..].split_at(size_of::<ManifestWrapperStateFixed>());

    let wrapper_fixed: &ManifestWrapperStateFixed = get_helper(fixed_data, 0);
    let market_infos_tree: MarketInfosTreeReadOnly = unsafe {
        MarketInfosTreeReadOnly::new(
            wrapper_dynamic_data,
            wrapper_fixed.market_infos_root_index,
            NIL,
        )
    };

    // Just need to lookup by market key so the rest doesnt matter.
    let market_info_index: DataIndex =
        market_infos_tree.lookup_index(&MarketInfo::new_empty(market.to_bytes().into(), NIL));

    let open_orders: Vec<WrapperOpenOrder> =
        collect_open_orders(wrapper_dynamic_data, market_info_index);

    for open_order in open_orders.iter() {
        // Match on client order id or order sequence number. Initial orders do
        // not have a client order id parsed, so those were set to order
        // sequence number.
        if open_order.get_client_order_id() == order.order_sequence_number {
            return Some(*open_order);
        }
        if open_order.get_order_sequence_number() == order.order_sequence_number {
            return Some(*open_order);
        }
        if open_order.get_price()
            == QuoteAtomsPerBaseAtom::try_from(
                order.price_in_ticks as f64 * base_lot_size as f64 / quote_lot_size as f64,
            )
            .unwrap()
        {
            return Some(*open_order);
        }
    }
    None
}

pub async fn match_order_in_wrapper_orders_tree_for_cancel_up_to(
    context: &mut ProgramTestContext,
    wrapper: &Pubkey,
    market: &Pubkey,
    order: PhoenixCancelUpToOrderParams,
    base_lot_size: u64,
    quote_lot_size: u64,
    wrapper_cancel_orders: &mut Vec<WrapperCancelOrderParams>,
) {
    let wrapper_account: Account = context
        .banks_client
        .get_account(*wrapper)
        .await
        .expect("Fetch wrapper")
        .expect("Wrapper is not none");
    let (fixed_data, wrapper_dynamic_data) =
        wrapper_account.data[..].split_at(size_of::<ManifestWrapperStateFixed>());

    let wrapper_fixed: &ManifestWrapperStateFixed = get_helper(fixed_data, 0);
    let market_infos_tree: MarketInfosTreeReadOnly = unsafe {
        MarketInfosTreeReadOnly::new(
            wrapper_dynamic_data,
            wrapper_fixed.market_infos_root_index,
            NIL,
        )
    };

    let market_info_index: DataIndex =
        market_infos_tree.lookup_index(&MarketInfo::new_empty(market.to_bytes().into(), NIL));

    let open_orders: Vec<WrapperOpenOrder> =
        collect_open_orders(wrapper_dynamic_data, market_info_index);

    let PhoenixCancelUpToOrderParams {
        side,
        tick_limit,
        num_orders_to_cancel: _num_orders_to_cancel,
        num_orders_to_search: _num_orders_to_search,
    } = order;
    for open_order in open_orders.iter() {
        if open_order.get_is_bid() {
            if side != Side::Bid {
                continue;
            }
            if tick_limit.is_some()
                && QuoteAtomsPerBaseAtom::try_from(
                    tick_limit.unwrap() as f64 * base_lot_size as f64 / (quote_lot_size as f64),
                )
                .unwrap()
                    < open_order.get_price()
            {
                let wrapper_cancel_order: WrapperCancelOrderParams =
                    WrapperCancelOrderParams::new(open_order.get_client_order_id());
                wrapper_cancel_orders.push(wrapper_cancel_order);
            }
        } else {
            if side != Side::Ask {
                continue;
            }
            if tick_limit.is_some()
                && QuoteAtomsPerBaseAtom::try_from(
                    tick_limit.unwrap() as f64 * base_lot_size as f64 / (quote_lot_size as f64),
                )
                .unwrap()
                    > open_order.get_price()
            {
                let wrapper_cancel_order: WrapperCancelOrderParams =
                    WrapperCancelOrderParams::new(open_order.get_client_order_id());
                wrapper_cancel_orders.push(wrapper_cancel_order);
            }
        }
    }
}

pub fn value_get_str<'a>(value: &'a Value, key: &'a str) -> &'a str {
    value.get(key).unwrap().as_str().unwrap()
}

pub fn value_get_pubkey(value: &Value, key: &str) -> Pubkey {
    Pubkey::from_str(value.get(key).unwrap().as_str().unwrap()).unwrap()
}

pub fn value_get_u64(value: &Value, key: &str) -> u64 {
    if value.get(key).unwrap().is_string() {
        from_hex(value.get(key).unwrap().as_str().unwrap())
    } else {
        value.get(key).unwrap().as_u64().unwrap()
    }
}

pub fn value_get_f64(value: &Value, key: &str) -> f64 {
    value.get(key).unwrap().as_f64().unwrap()
}

pub fn value_get_bool(value: &Value, key: &str) -> bool {
    value.get(key).unwrap().as_bool().unwrap()
}

pub fn value_get_array<'a>(value: &'a Value, key: &'a str) -> &'a Vec<Value> {
    value.get(key).unwrap().as_array().unwrap()
}

pub fn value_get_obj<'a>(value: &'a Value, key: &'a str) -> &'a Obj {
    value.get(key).unwrap().as_object().unwrap()
}

pub fn value_get_opt_u64(value: &Value, key: &str) -> Option<u64> {
    let object: &Obj = value.as_object().unwrap();
    if object.contains_key(key) && !object.get(key).unwrap().is_null() {
        Some(object.get(key).unwrap().as_u64().unwrap())
    } else {
        None
    }
}

pub fn obj_get_str<'a>(object: &'a Obj, key: &'a str) -> &'a str {
    object.get(key).unwrap().as_str().unwrap()
}

pub fn obj_get_u64(object: &Obj, key: &str) -> u64 {
    if object.get(key).unwrap().is_string() {
        from_hex(object.get(key).unwrap().as_str().unwrap())
    } else {
        object.get(key).unwrap().as_u64().unwrap()
    }
}

pub fn obj_get_f64(object: &Obj, key: &str) -> f64 {
    object.get(key).unwrap().as_f64().unwrap()
}

pub fn obj_get_bool(object: &Obj, key: &str) -> bool {
    object.get(key).unwrap().as_bool().unwrap()
}

pub fn obj_get_bool_if_exists(object: &Obj, key: &str) -> bool {
    if object.contains_key(key) && !object.get(key).unwrap().is_null() {
        object.get(key).unwrap().as_bool().unwrap()
    } else {
        false
    }
}

pub fn obj_get_u128_from_hex(object: &Obj, key: &str) -> u128 {
    from_hex_u128(object.get(key).unwrap().as_str().unwrap())
}

pub fn obj_get_obj<'a>(object: &'a Obj, key: &'a str) -> &'a Obj {
    object.get(key).unwrap().as_object().unwrap()
}

pub fn obj_get_opt_u64(object: &Obj, key: &str) -> Option<u64> {
    if object.contains_key(key) && !object.get(key).unwrap().is_null() {
        if object.get(key).unwrap().is_string() {
            Some(from_hex(object.get(key).unwrap().as_str().unwrap()))
        } else {
            Some(object.get(key).unwrap().as_u64().unwrap())
        }
    } else {
        None
    }
}
