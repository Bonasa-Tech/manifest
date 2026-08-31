//! The batched order event: one `BatchUpdateLog` payload per batch update
//! carrying every cancel and placement, instead of one event per order.
#![cfg_attr(not(feature = "test-sbf"), allow(dead_code, unused_imports))]

use base64::Engine;
use bytemuck::from_bytes;
use manifest::{
    logs::{BatchUpdateLog, CancelOrderLogEntry, Discriminant, PlaceOrderLogEntry},
    program::{
        batch_update::{CancelOrderParams, PlaceOrderParams},
        batch_update_instruction,
    },
    quantities::WrapperU64,
    state::{constants::NO_EXPIRATION_LAST_VALID_SLOT, OrderType},
};
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_program::pubkey::Pubkey;
use solana_program_test::tokio;
use solana_transaction::Transaction;

use crate::{TestFixture, Token, SOL_UNIT_SIZE};

/// Program data payloads of a simulated transaction, discriminant included.
async fn simulate_program_data(
    test_fixture: &TestFixture,
    instruction: Instruction,
) -> Vec<Vec<u8>> {
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair();
    let mut context = test_fixture.context.borrow_mut();
    let blockhash = context.get_new_latest_blockhash().await.unwrap();
    let transaction: Transaction = Transaction::new_signed_with_payer(
        &[instruction],
        Some(&payer),
        &[&payer_keypair],
        blockhash,
    );
    let simulation = context
        .banks_client
        .simulate_transaction(transaction)
        .await
        .unwrap();
    assert!(
        matches!(simulation.result, Some(Ok(()))),
        "{:?}",
        simulation.result
    );
    simulation
        .simulation_details
        .unwrap()
        .logs
        .iter()
        .filter_map(|log: &String| log.strip_prefix("Program data: "))
        .map(|data: &str| {
            base64::engine::general_purpose::STANDARD
                .decode(data)
                .expect("valid base64")
        })
        .collect()
}

/// Program data is only visible in the logs when the compiled program runs
/// under `test-sbf`; the native processor test runtime drops it.
#[cfg(feature = "test-sbf")]
#[tokio::test]
async fn batch_update_log_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 10 * SOL_UNIT_SIZE).await?;
    // Two resting asks with sequence numbers 0 and 1.
    for price_mantissa in [5u32, 6u32] {
        test_fixture
            .place_order(
                crate::Side::Ask,
                SOL_UNIT_SIZE,
                price_mantissa,
                -3,
                NO_EXPIRATION_LAST_VALID_SLOT,
                OrderType::Limit,
            )
            .await?;
    }
    let payer: Pubkey = test_fixture.payer();
    let market: Pubkey = test_fixture.market_fixture.key;

    // Cancel order 0 and place two asks (sequence numbers 2 and 3).
    let instruction: Instruction = batch_update_instruction(
        &market,
        &payer,
        None,
        vec![CancelOrderParams::new(0)],
        vec![
            PlaceOrderParams::new(
                2 * SOL_UNIT_SIZE,
                7,
                -3,
                false,
                OrderType::Limit,
                NO_EXPIRATION_LAST_VALID_SLOT,
            ),
            PlaceOrderParams::new(
                3 * SOL_UNIT_SIZE,
                8,
                -3,
                false,
                OrderType::PostOnly,
                NO_EXPIRATION_LAST_VALID_SLOT,
            ),
        ],
        None,
        None,
        None,
        None,
    );
    let payloads: Vec<Vec<u8>> = simulate_program_data(&test_fixture, instruction).await;
    let payload: &Vec<u8> = payloads
        .iter()
        .find(|payload: &&Vec<u8>| payload.starts_with(&BatchUpdateLog::discriminant()))
        .unwrap_or_else(|| {
            panic!(
                "batch update log not emitted, program data discriminants: {:?}",
                payloads
                    .iter()
                    .map(|payload: &Vec<u8>| payload[..8.min(payload.len())].to_vec())
                    .collect::<Vec<Vec<u8>>>()
            )
        });
    // Exactly one order event for the whole batch, nothing per order.
    assert_eq!(
        payloads
            .iter()
            .filter(|payload: &&Vec<u8>| payload.starts_with(&BatchUpdateLog::discriminant()))
            .count(),
        1
    );

    let header_size: usize = std::mem::size_of::<BatchUpdateLog>();
    let cancel_size: usize = std::mem::size_of::<CancelOrderLogEntry>();
    let order_size: usize = std::mem::size_of::<PlaceOrderLogEntry>();
    let header: &BatchUpdateLog = from_bytes(&payload[8..8 + header_size]);
    assert_eq!(header.market, market);
    assert_eq!(header.trader, payer);
    assert_eq!(header.num_cancels, 1);
    assert_eq!(header.num_orders, 2);
    assert_eq!(
        payload.len(),
        8 + header_size + cancel_size + 2 * order_size,
        "payload is header, cancels, then orders"
    );

    let mut offset: usize = 8 + header_size;
    let cancel: &CancelOrderLogEntry = from_bytes(&payload[offset..offset + cancel_size]);
    assert_eq!(cancel.order_sequence_number, 0);
    offset += cancel_size;

    let first: &PlaceOrderLogEntry = from_bytes(&payload[offset..offset + order_size]);
    assert_eq!(first.order_sequence_number, 2);
    assert_eq!(first.base_atoms.as_u64(), 2 * SOL_UNIT_SIZE);
    assert_eq!(first.order_type, OrderType::Limit);
    assert_eq!(bytemuck::bytes_of(&first.is_bid), &[0u8]);
    offset += order_size;
    let second: &PlaceOrderLogEntry = from_bytes(&payload[offset..offset + order_size]);
    assert_eq!(second.order_sequence_number, 3);
    assert_eq!(second.base_atoms.as_u64(), 3 * SOL_UNIT_SIZE);
    assert_eq!(second.order_type, OrderType::PostOnly);
    assert_ne!(first.order_index, second.order_index);
    Ok(())
}
