use std::{
    fs::{read_to_string, File, OpenOptions},
    str::FromStr,
};

use borsh::ser::BorshSerialize;
use csv::Writer;
use manifest::{
    program::batch_update::PlaceOrderParams,
    state::{OrderType, NO_EXPIRATION_LAST_VALID_SLOT},
};
use rand::Rng;
use serde_json::Value;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_program_test::{BanksClientError, ProgramTest, ProgramTestContext};
use solana_sdk::{
    instruction::{Instruction, InstructionError}, message::AccountMeta, pubkey::Pubkey, signature::Keypair, signer::Signer, transaction::{Transaction, TransactionError}
};
use wrapper::{
    instruction::ManifestWrapperInstruction, processors::batch_upate::{WrapperBatchUpdateParams, WrapperCancelOrderParams, WrapperPlaceOrderParams}
};

use super::{
    cancel_all_wrapper_orders, match_order_in_wrapper_orders_tree,
    match_order_in_wrapper_orders_tree_for_cancel_up_to, obj_get_u64,
    parse_cancel_order_order_data, parse_cancel_up_to_order_order_data,
    parse_ioc_decoded_data, parse_limit_decoded_data, parse_post_only_decoded_data,
    parse_transaction_object, value_get_obj, value_get_str, value_get_u64, Obj, ParsedTransaction,
    PhoenixIOCOrderPacket, PhoenixLimitOrderPacket, PhoenixPostOnlyOrderPacket, Side, TestFixture,
};

pub async fn run_transactions(transactions_filepath: String, test_fixture: TestFixture) {
    let program_test: ProgramTest = test_fixture.program_test;
    let market_key: &Pubkey = &test_fixture.market;
    let mut program_test_context: ProgramTestContext = program_test.start_with_context().await;
    let _ = program_test_context.warp_to_slot(0);
    let tx_data_str: String = read_to_string(transactions_filepath).expect("Unable to read file");
    let tx_data_json: Value = serde_json::from_str(&tx_data_str).expect("Unable to parse");
    let base_lot_size: u64 = test_fixture.phx_base_lot_size;
    let quote_lot_size: u64 = test_fixture.phx_quote_lot_size;

    for tx in tx_data_json.as_array().unwrap() {
        // We merge more batch updates, so add up across all ix.
        let mut phx_cu: u64 = 0;

        let ParsedTransaction {
            instructions_array,
            signer,
            signature,
            ..
        } = parse_transaction_object(tx);

        let original_pubkey: Pubkey = Pubkey::from_str(&signer).unwrap();

        // TODO: Make this work for orders without a seat
        if !test_fixture.keypair_map.contains_key(&original_pubkey) {
            continue;
        }
        let keypair: &Keypair = test_fixture.keypair_map.get(&original_pubkey).unwrap();
        let payer: Pubkey = keypair.pubkey();

        let mut direct_place_orders: Vec<PlaceOrderParams> = Vec::new();
        let mut wrapper_place_orders: Vec<WrapperPlaceOrderParams> = Vec::new();
        let mut wrapper_cancel_orders: Vec<WrapperCancelOrderParams> = Vec::new();
        let mut cancel_all_orders: Vec<WrapperCancelOrderParams> = Vec::new();
        let mut cancel_all: bool = false;

        let wrapper_state: &Pubkey = test_fixture.trader_to_wrapper.get(&payer).unwrap();

        for instruction in instructions_array {
            let name: &str = value_get_str(&instruction, "instructionName");
            phx_cu += value_get_u64(&instruction, "cuConsumed");
            match name {
                "Swap" => {
                    let decoded_data_object: &Obj = value_get_obj(&instruction, "decodedData");
                    handle_swap(
                        &decoded_data_object,
                        &mut wrapper_place_orders,
                        &mut direct_place_orders,
                        base_lot_size,
                        quote_lot_size,
                    );
                }
                "CancelMultipleOrdersByIdWithFreeFunds" => {
                    let decoded_data_object: &Obj = value_get_obj(&instruction, "decodedData");
                    handle_cancel_multiple_by_id_with_free_funds(
                        &decoded_data_object,
                        &mut wrapper_cancel_orders,
                        &mut program_test_context,
                        market_key,
                        wrapper_state,
                        base_lot_size,
                        quote_lot_size,
                    )
                    .await;
                }
                "PlaceMultiplePostOnlyOrdersWithFreeFunds" => {
                    let decoded_data_object: &Obj = value_get_obj(&instruction, "decodedData");
                    handle_multiple_post_only_orders(
                        &decoded_data_object,
                        &mut wrapper_place_orders,
                        base_lot_size,
                        quote_lot_size,
                    );
                }
                "PlaceMultiplePostOnlyOrders" => {
                    let decoded_data_object: &Obj = value_get_obj(&instruction, "decodedData");
                    handle_multiple_post_only_orders(
                        &decoded_data_object,
                        &mut wrapper_place_orders,
                        base_lot_size,
                        quote_lot_size,
                    );
                }
                "PlaceLimitOrder" => {
                    let decoded_data_object: &Obj = value_get_obj(&instruction, "decodedData");
                    handle_place_limit_order(
                        &decoded_data_object,
                        &mut wrapper_place_orders,
                        base_lot_size,
                        quote_lot_size,
                    );
                }
                "PlaceLimitOrderWithFreeFunds" => {
                    let decoded_data_object: &Obj = value_get_obj(&instruction, "decodedData");
                    handle_place_limit_order(
                        &decoded_data_object,
                        &mut wrapper_place_orders,
                        base_lot_size,
                        quote_lot_size,
                    );
                }
                "CancelUpTo" => {
                    let decoded_data_object: &Obj = value_get_obj(&instruction, "decodedData");
                    handle_cancel_up_to(
                        &decoded_data_object,
                        &mut wrapper_cancel_orders,
                        &mut program_test_context,
                        market_key,
                        wrapper_state,
                        base_lot_size,
                        quote_lot_size,
                    )
                    .await;
                }
                "CancelUpToWithFreeFunds" => {
                    let decoded_data_object: &Obj = value_get_obj(&instruction, "decodedData");
                    handle_cancel_up_to(
                        &decoded_data_object,
                        &mut wrapper_cancel_orders,
                        &mut program_test_context,
                        market_key,
                        wrapper_state,
                        base_lot_size,
                        quote_lot_size,
                    )
                    .await;
                }
                "CancelAllOrdersWithFreeFunds" => {
                    cancel_all = true;
                    cancel_all_orders.extend(
                        cancel_all_wrapper_orders(
                            &mut program_test_context,
                            wrapper_state,
                            market_key,
                        )
                        .await,
                    );
                }
                "CancelAllOrders" => {
                    cancel_all = true;
                    cancel_all_orders.extend(
                        cancel_all_wrapper_orders(
                            &mut program_test_context,
                            wrapper_state,
                            market_key,
                        )
                        .await,
                    );
                }
                "CancelMultipleOrdersById" => {
                    let decoded_data_object: &Obj = value_get_obj(&instruction, "decodedData");
                    handle_cancel_multiple_by_id_with_free_funds(
                        &decoded_data_object,
                        &mut wrapper_cancel_orders,
                        &mut program_test_context,
                        market_key,
                        wrapper_state,
                        base_lot_size,
                        quote_lot_size,
                    )
                    .await;
                }
                _ => {
                    println!("Name {}", name);
                    assert!(false);
                }
            }
        }

        if !wrapper_cancel_orders.is_empty() || !wrapper_place_orders.is_empty() || cancel_all {
            wrapper_cancel_orders.extend(cancel_all_orders);
            // Match the published main baseline: expanded cancel-all entries
            // count toward CU/order, alongside explicit cancels and placements.
            let num_orders: usize = wrapper_cancel_orders.len() + wrapper_place_orders.len();
            println!(
                "Sending batch update for {} {} {} {}",
                original_pubkey,
                signature,
                wrapper_cancel_orders.len(),
                wrapper_place_orders.len()
            );
            // TODO: Lookup trader index hint
            let batch_update_ix: Instruction = batch_update_instruction(
                market_key,
                &payer,
                wrapper_state,
                wrapper_cancel_orders,
                false,
                wrapper_place_orders,
            );
            let budget_ix: Instruction =
                ComputeBudgetInstruction::set_compute_unit_limit(1_400_000);

            loop {
                let batch_update_tx: Transaction = {
                    Transaction::new_signed_with_payer(
                        &[budget_ix.clone(), batch_update_ix.clone()],
                        Some(&payer),
                        &[keypair],
                        program_test_context
                            .get_new_latest_blockhash()
                            .await
                            .unwrap(),
                    )
                };

                let sim = program_test_context
                    .banks_client
                    .simulate_transaction(batch_update_tx.clone())
                    .await
                    .unwrap();
                let result: Result<(), BanksClientError> = program_test_context
                    .banks_client
                    .process_transaction(batch_update_tx)
                    .await;
                if result.is_ok() {
                    let manifest_cu: u64 = sim.simulation_details.unwrap().units_consumed;
                    let file: File = OpenOptions::new()
                        .write(true)
                        .create(true)
                        .append(true)
                        .open("benchmark_results.csv")
                        .unwrap();

                    let mut writer: Writer<File> = Writer::from_writer(file);
                    writer
                        .write_record(&[
                            phx_cu.to_string(),
                            manifest_cu.to_string(),
                            num_orders.to_string(),
                            signature,
                        ])
                        .unwrap();
                    writer.flush().unwrap();

                    println!("Success");
                    break;
                }

                let error: BanksClientError = result.err().unwrap();
                match error {
                    BanksClientError::ClientError(err) => {
                        println!("client {:?}", err);
                        assert!(false);
                    }
                    BanksClientError::Io(err) => {
                        println!("io {:?}", err);
                        assert!(false);
                    }
                    BanksClientError::RpcError(rpc_err) => {
                        println!("rpc {:?}", rpc_err);
                        continue;
                    }
                    BanksClientError::TransactionError(err) => {
                        println!("Transaction error {:?}", err);
                        match err {
                            TransactionError::InstructionError(_index, instruction_error) => {
                                match instruction_error {
                                    InstructionError::InsufficientFunds => {
                                        // Fail silently to match behavior.
                                        break;
                                    }
                                    _ => {
                                        println!("Unexpected failure");
                                        assert!(false);
                                    }
                                }
                                continue;
                            }
                            _ => {
                                println!("Unexpected failure");
                                assert!(false);
                            }
                        }
                        assert!(false);
                    }
                    BanksClientError::SimulationError {
                        err,
                        logs,
                        units_consumed,
                        return_data,
                    } => {
                        println!("sim {:?}", err);
                        println!("logs {:?}", logs);
                        println!("cu {:?}", units_consumed);
                        println!("return {:?}", return_data);
                        assert!(false);
                    }
                }
            }
        }
    }
}

fn handle_swap(
    decoded_data_object: &Obj,
    wrapper_place_orders: &mut Vec<WrapperPlaceOrderParams>,
    direct_place_orders: &mut Vec<PlaceOrderParams>,
    base_lot_size: u64,
    quote_lot_size: u64,
) {
    let PhoenixIOCOrderPacket {
        side,
        price_in_ticks,
        num_base_lots,
        num_quote_lots,
        client_order_id,
        use_only_deposited_funds,
        ..
    } = parse_ioc_decoded_data(&decoded_data_object);

    let is_bid: bool = side == Side::Bid;
    let (price_mantissa, price_exponent) = if let Some(price) = price_in_ticks {
        price_ticks_to_decimal(price, base_lot_size, quote_lot_size)
    } else {
        if is_bid {
            (u32::MAX, 10)
        } else {
            (0, -20)
        }
    };
    let base_atoms: u64 = if is_bid {
        num_base_lots * base_lot_size
    } else {
        let price = price_mantissa as f64 * 10f64.powi(price_exponent.into());
        ((num_quote_lots as f64) / price) as u64
    };

    if use_only_deposited_funds {
        let wrapper_place_order: WrapperPlaceOrderParams = WrapperPlaceOrderParams::new(
            client_order_id as u64,
            base_atoms,
            price_mantissa,
            price_exponent,
            is_bid,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::ImmediateOrCancel,
        );
        wrapper_place_orders.push(wrapper_place_order);
    } else {
        let direct_place_order: PlaceOrderParams = PlaceOrderParams::new(
            base_atoms,
            price_mantissa,
            price_exponent,
            is_bid,
            OrderType::ImmediateOrCancel,
            NO_EXPIRATION_LAST_VALID_SLOT,
        );
        direct_place_orders.push(direct_place_order);
    }
}

async fn handle_cancel_multiple_by_id_with_free_funds(
    decoded_data_object: &Obj,
    wrapper_cancel_orders: &mut Vec<WrapperCancelOrderParams>,
    context: &mut ProgramTestContext,
    market: &Pubkey,
    wrapper: &Pubkey,
    base_lot_size: u64,
    quote_lot_size: u64,
) {
    let orders: &Value = decoded_data_object.get("orders").unwrap();
    for order in orders.as_array().unwrap() {
        match match_order_in_wrapper_orders_tree(
            context,
            wrapper,
            market,
            parse_cancel_order_order_data(&order.as_object().unwrap()),
            base_lot_size,
            quote_lot_size,
        )
        .await
        {
            Some(order) => {
                let wrapper_cancel_order: WrapperCancelOrderParams =
                    WrapperCancelOrderParams::new(order.get_client_order_id());
                wrapper_cancel_orders.push(wrapper_cancel_order);
            }
            None => {
                continue;
            }
        }
    }
}

async fn handle_cancel_up_to(
    decoded_data_object: &Obj,
    wrapper_cancel_orders: &mut Vec<WrapperCancelOrderParams>,
    context: &mut ProgramTestContext,
    market: &Pubkey,
    wrapper: &Pubkey,
    base_lot_size: u64,
    quote_lot_size: u64,
) {
    match_order_in_wrapper_orders_tree_for_cancel_up_to(
        context,
        wrapper,
        market,
        parse_cancel_up_to_order_order_data(decoded_data_object),
        base_lot_size,
        quote_lot_size,
        wrapper_cancel_orders,
    )
    .await;
}

fn handle_multiple_post_only_orders(
    decoded_data_object: &Obj,
    place_orders: &mut Vec<WrapperPlaceOrderParams>,
    base_lot_size: u64,
    quote_lot_size: u64,
) {
    let bids: &Value = decoded_data_object.get("bids").unwrap();
    let asks: &Value = decoded_data_object.get("asks").unwrap();

    let fail_behavior: u64 = obj_get_u64(decoded_data_object, "failedMultipleLimitOrderBehavior");
    let slide: bool = fail_behavior == 0 || fail_behavior == 2;
    let order_type: OrderType = if slide {
        OrderType::Limit
    } else {
        OrderType::PostOnly
    };

    for (orders, is_bid) in [(bids, true), (asks, false)] {
        for order in orders.as_array().unwrap() {
            let PhoenixPostOnlyOrderPacket {
                price_in_ticks,
                num_base_lots,
                client_order_id,
                last_valid_slot,
                ..
            } =
            // TODO: If not use_only_deposited_funds, then do a deposit
            parse_post_only_decoded_data(order.as_object().unwrap(), is_bid);

            let (price_mantissa, price_exponent) =
                price_ticks_to_decimal(price_in_ticks, base_lot_size, quote_lot_size);
            let base_atoms: u64 = num_base_lots * base_lot_size;
            let last_valid_slot: u32 = if let Some(slot) = last_valid_slot {
                slot as u32
            } else {
                NO_EXPIRATION_LAST_VALID_SLOT
            };
            // Program does not handle multiple with same client order id, so make new ones.
            let client_order_id: u64 = if client_order_id != 0 {
                client_order_id as u64
            } else {
                rand::thread_rng().gen_range(0..u64::MAX)
            };

            let wrapper_place_order: WrapperPlaceOrderParams = WrapperPlaceOrderParams::new(
                client_order_id,
                base_atoms,
                price_mantissa,
                price_exponent,
                is_bid,
                last_valid_slot,
                order_type,
            );
            place_orders.push(wrapper_place_order);
        }
    }
}

fn handle_place_limit_order(
    decoded_data_object: &Obj,
    place_orders: &mut Vec<WrapperPlaceOrderParams>,
    base_lot_size: u64,
    quote_lot_size: u64,
) {
    let PhoenixLimitOrderPacket {
        side,
        price_in_ticks,
        num_base_lots,
        client_order_id,
        ..
    } = parse_limit_decoded_data(&decoded_data_object);

    let is_bid: bool = side == Side::Bid;

    let (price_mantissa, price_exponent) =
        price_ticks_to_decimal(price_in_ticks, base_lot_size, quote_lot_size);
    let base_atoms: u64 = num_base_lots * base_lot_size;
    // TODO: use_only_deposited_funds should decide if from wallet or not

    // Note: Does not use last valid slot because difficult to manage clocks.
    let wrapper_place_order: WrapperPlaceOrderParams = WrapperPlaceOrderParams::new(
        client_order_id as u64,
        base_atoms,
        price_mantissa,
        price_exponent,
        is_bid,
        NO_EXPIRATION_LAST_VALID_SLOT,
        OrderType::Limit,
    );
    place_orders.push(wrapper_place_order);
}

fn price_ticks_to_decimal(
    price_in_ticks: u64,
    base_lot_size: u64,
    quote_lot_size: u64,
) -> (u32, i8) {
    let mut mantissa = price_in_ticks;
    let mut exponent = -(base_lot_size.ilog10() as i8) - (quote_lot_size.ilog10() as i8);
    while mantissa > u32::MAX as u64 {
        mantissa /= 10;
        exponent += 1;
    }
    (mantissa as u32, exponent)
}


fn batch_update_instruction(
    market: &Pubkey,
    owner: &Pubkey,
    wrapper_state: &Pubkey,
    cancels: Vec<WrapperCancelOrderParams>,
    cancel_all: bool,
    orders: Vec<WrapperPlaceOrderParams>,
) -> Instruction {
    Instruction {
        program_id: wrapper::ID.to_bytes().into(),
        accounts: vec![
            AccountMeta::new(*wrapper_state, false),
            AccountMeta::new_readonly(manifest::ID.to_bytes().into(), false),
            AccountMeta::new(*owner, true),
            AccountMeta::new(*market, false),
            AccountMeta::new_readonly(Pubkey::default(), false),
        ],
        data: [
            ManifestWrapperInstruction::BatchUpdate.to_vec(),
            WrapperBatchUpdateParams::new(cancels, cancel_all, orders)
                .try_to_vec()
                .unwrap(),
        ]
        .concat(),
    }
}
