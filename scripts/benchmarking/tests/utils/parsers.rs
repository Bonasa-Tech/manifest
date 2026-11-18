use serde_json::Value;

use super::{
    obj_get_bool, obj_get_bool_if_exists, obj_get_opt_u64, obj_get_u128_from_hex, obj_get_u64,
    value_get_array, value_get_bool, value_get_f64, value_get_opt_u64, value_get_str,
    value_get_u64, Obj,
};

pub struct ParsedTransaction {
    pub signature: String,
    pub signer: String,
    pub slot: u64,
    pub block_time: u64,
    pub compute_unit_limit: u64,
    pub compute_unit_price: f64,
    pub is_jup_flow: bool,
    pub instructions_array: Vec<Value>,
}

pub fn parse_transaction_object(tx: &Value) -> ParsedTransaction {
    let signature: &str = value_get_str(tx, "signature");
    let signer: &str = value_get_str(tx, "signer");
    let slot: u64 = value_get_u64(tx, "slot");
    let block_time: u64 = value_get_u64(tx, "blockTime");

    let compute_unit_limit: u64 = value_get_opt_u64(tx, "computeUnitLimit").unwrap_or(200_000);
    let compute_unit_price: f64 = if tx.as_object().unwrap().contains_key("computeUnitPrice") {
        value_get_f64(tx, "computeUnitPrice")
    } else {
        0.0
    };
    let is_jup_flow: bool = value_get_bool(tx, "isJupFlow");
    let instructions_array: &Vec<Value> = value_get_array(tx, "instructions");

    return ParsedTransaction {
        signature: signature.to_owned(),
        signer: signer.to_owned(),
        slot,
        block_time,
        compute_unit_price,
        compute_unit_limit,
        is_jup_flow,
        instructions_array: instructions_array.to_vec(),
    };
}

#[derive(PartialEq)]
pub enum Side {
    Bid = 0,
    Ask = 1,
}
pub enum SelfTradeBehavior {
    Abort,
    CancelProvide,
    DecrementTake,
}
pub struct PhoenixIOCOrderPacket {
    pub side: Side,
    pub price_in_ticks: Option<u64>,
    pub num_base_lots: u64,
    pub num_quote_lots: u64,
    pub min_base_lots_to_fill: u64,
    pub min_quote_lots_to_fill: u64,
    pub self_trade_behavior: SelfTradeBehavior,
    pub match_limit: Option<u64>,
    pub client_order_id: u128,
    pub use_only_deposited_funds: bool,
    pub last_valid_slot: Option<u64>,
    pub last_valid_unix_timestamp_in_seconds: Option<u64>,
}

pub fn parse_ioc_decoded_data(order_data_object: &Obj) -> PhoenixIOCOrderPacket {
    let side: Side = match obj_get_u64(order_data_object, "side") {
        0 => Side::Bid,
        1 => Side::Ask,
        _ => Side::Bid,
    };

    let price_in_ticks: Option<u64> = obj_get_opt_u64(order_data_object, "priceInTicks");
    let num_base_lots: u64 = obj_get_u64(order_data_object, "numBaseLots");
    let num_quote_lots: u64 = obj_get_u64(order_data_object, "numQuoteLots");
    let min_base_lots_to_fill: u64 = obj_get_u64(order_data_object, "minBaseLotsToFill");
    let min_quote_lots_to_fill: u64 = obj_get_u64(order_data_object, "minQuoteLotsToFill");

    let self_trade_behavior: SelfTradeBehavior =
        match obj_get_u64(order_data_object, "selfTradeBehavior") {
            0 => SelfTradeBehavior::Abort,
            1 => SelfTradeBehavior::CancelProvide,
            2 => SelfTradeBehavior::DecrementTake,
            _ => SelfTradeBehavior::Abort,
        };
    let match_limit: Option<u64> = obj_get_opt_u64(order_data_object, "matchLimit");
    let client_order_id: u128 = obj_get_u128_from_hex(order_data_object, "clientOrderId");
    let use_only_deposited_funds: bool = obj_get_bool(order_data_object, "useOnlyDepositedFunds");
    let last_valid_slot: Option<u64> = obj_get_opt_u64(order_data_object, "lastValidSlot");
    let last_valid_unix_timestamp_in_seconds: Option<u64> =
        obj_get_opt_u64(order_data_object, "lastValidUnixTimestampInSeconds");

    PhoenixIOCOrderPacket {
        side,
        price_in_ticks,
        num_base_lots,
        num_quote_lots,
        min_base_lots_to_fill,
        min_quote_lots_to_fill,
        self_trade_behavior,
        match_limit,
        client_order_id,
        use_only_deposited_funds,
        last_valid_slot,
        last_valid_unix_timestamp_in_seconds,
    }
}

pub struct PhoenixLimitOrderPacket {
    pub side: Side,
    pub price_in_ticks: u64,
    pub num_base_lots: u64,
    pub self_trade_behavior: SelfTradeBehavior,
    pub match_limit: Option<u64>,
    pub client_order_id: u128,
    pub use_only_deposited_funds: bool,
    pub last_valid_slot: Option<u64>,
    pub last_valid_unix_timestamp_in_seconds: Option<u64>,
    pub fail_silently_on_insufficient_funds: bool,
}

pub fn parse_limit_decoded_data(order_data_object: &Obj) -> PhoenixLimitOrderPacket {
    let side: Side = match obj_get_u64(order_data_object, "side") {
        0 => Side::Bid,
        1 => Side::Ask,
        _ => Side::Bid,
    };

    let price_in_ticks: u64 = obj_get_u64(order_data_object, "priceInTicks");
    let num_base_lots: u64 = obj_get_u64(order_data_object, "numBaseLots");

    // Should always be defined, but found an order that doesnt fit the schema
    // kSHK9wU2c1JYJyCnAiEH47XhFBFL7e14idJzrpiNDdwRywxFx9efLWcs1Dcw1ALn366GeyixiojWa8dLj6DnEdg
    let self_trade_behavior: SelfTradeBehavior =
        if let Some(self_trade_behavior) = order_data_object.get("selfTradeBehavior") {
            match self_trade_behavior.as_u64().unwrap() {
                0 => SelfTradeBehavior::Abort,
                1 => SelfTradeBehavior::CancelProvide,
                2 => SelfTradeBehavior::DecrementTake,
                _ => SelfTradeBehavior::Abort,
            }
        } else {
            SelfTradeBehavior::Abort
        };
    let match_limit: Option<u64> = obj_get_opt_u64(order_data_object, "matchLimit");
    let client_order_id: u128 = obj_get_u128_from_hex(order_data_object, "clientOrderId");
    let use_only_deposited_funds: bool = obj_get_bool(order_data_object, "useOnlyDepositedFunds");
    let fail_silently_on_insufficient_funds: bool =
        obj_get_bool(order_data_object, "failSilentlyOnInsufficientFunds");
    let last_valid_slot: Option<u64> = obj_get_opt_u64(order_data_object, "lastValidSlot");
    let last_valid_unix_timestamp_in_seconds: Option<u64> =
        obj_get_opt_u64(order_data_object, "lastValidUnixTimestampInSeconds");

    PhoenixLimitOrderPacket {
        side,
        price_in_ticks,
        num_base_lots,
        self_trade_behavior,
        match_limit,
        client_order_id,
        use_only_deposited_funds,
        last_valid_slot,
        last_valid_unix_timestamp_in_seconds,
        fail_silently_on_insufficient_funds,
    }
}

pub struct PhoenixPostOnlyOrderPacket {
    pub side: Side,
    pub price_in_ticks: u64,
    pub num_base_lots: u64,
    pub client_order_id: u128,
    pub reject_post_only: bool,
    pub use_only_deposited_funds: bool,
    pub last_valid_slot: Option<u64>,
    pub last_valid_unix_timestamp_in_seconds: Option<u64>,
    pub fail_silently_on_insufficient_funds: bool,
}

pub fn parse_post_only_decoded_data(
    order_data_object: &Obj,
    is_bid: bool,
) -> PhoenixPostOnlyOrderPacket {
    let side: Side = if is_bid { Side::Bid } else { Side::Ask };

    let price_in_ticks: u64 = obj_get_u64(order_data_object, "priceInTicks");
    let num_base_lots: u64 = obj_get_u64(order_data_object, "sizeInBaseLots");
    let client_order_id: u128 = if order_data_object.get("clientOrderId").is_some() {
        obj_get_u128_from_hex(order_data_object, "clientOrderId")
    } else {
        0_u128
    };

    let reject_post_only: bool = obj_get_bool_if_exists(order_data_object, "rejectPostOnly");
    let use_only_deposited_funds: bool =
        obj_get_bool_if_exists(order_data_object, "useOnlyDepositedFunds");
    let fail_silently_on_insufficient_funds: bool =
        obj_get_bool_if_exists(order_data_object, "failSilentlyOnInsufficientFunds");
    let last_valid_slot: Option<u64> = obj_get_opt_u64(order_data_object, "lastValidSlot");
    let last_valid_unix_timestamp_in_seconds: Option<u64> =
        obj_get_opt_u64(order_data_object, "lastValidUnixTimestampInSeconds");

    PhoenixPostOnlyOrderPacket {
        side,
        price_in_ticks,
        num_base_lots,
        client_order_id,
        reject_post_only,
        use_only_deposited_funds,
        last_valid_slot,
        last_valid_unix_timestamp_in_seconds,
        fail_silently_on_insufficient_funds,
    }
}

pub struct PhoenixCancelOrderParams {
    pub side: Side,
    pub price_in_ticks: u64,
    pub order_sequence_number: u64,
}
pub fn parse_cancel_order_order_data(order_data_object: &Obj) -> PhoenixCancelOrderParams {
    let side: Side = match obj_get_u64(order_data_object, "side") {
        0 => Side::Bid,
        1 => Side::Ask,
        _ => Side::Bid,
    };
    let price_in_ticks: u64 = obj_get_u64(order_data_object, "priceInTicks");
    let order_sequence_number: u64 = obj_get_u64(order_data_object, "orderSequenceNumber");

    PhoenixCancelOrderParams {
        side,
        price_in_ticks,
        order_sequence_number,
    }
}

pub struct PhoenixCancelUpToOrderParams {
    pub side: Side,
    pub tick_limit: Option<u64>,
    pub num_orders_to_search: Option<u32>,
    pub num_orders_to_cancel: Option<u32>,
}
pub fn parse_cancel_up_to_order_order_data(
    order_data_object: &Obj,
) -> PhoenixCancelUpToOrderParams {
    let side: Side = match obj_get_u64(order_data_object, "side") {
        0 => Side::Bid,
        1 => Side::Ask,
        _ => Side::Bid,
    };

    let tick_limit: Option<u64> = obj_get_opt_u64(order_data_object, "tickLimit");
    let num_orders_to_search: Option<u32> =
        obj_get_opt_u64(order_data_object, "numOrdersToSearch").map(|x| x as u32);
    let num_orders_to_cancel: Option<u32> =
        obj_get_opt_u64(order_data_object, "numOrdersToCancel").map(|x| x as u32);

    PhoenixCancelUpToOrderParams {
        side,
        tick_limit,
        num_orders_to_search,
        num_orders_to_cancel,
    }
}
