//! What a batch update writes to the transaction log.
//!
//! It emits no event for the orders it places or cancels. Those events told
//! the sender what it had just asked for, so they were a syscall spent on
//! information the caller already had, and they displaced fills in a capped
//! log. Fills are the part a trader cannot predict, so they are still logged.
//!
//! That is a statement about placements and cancellations only, not about the
//! log as a whole: matching still emits whatever it needs to. A global maker
//! that cannot cover its order produces a `GlobalCleanupLog`, covered below,
//! and other instructions log as they always did.
#![cfg_attr(not(feature = "test-sbf"), allow(dead_code, unused_imports))]

use base64::Engine;
use hypertree::NIL;
use manifest::{
    logs::{CancelOrderLog, Discriminant, FillLog, GlobalCleanupLog, PlaceOrderLog},
    program::{
        batch_update::{CancelOrderParams, PlaceOrderParams},
        batch_update_instruction,
    },
    state::{constants::NO_EXPIRATION_LAST_VALID_SLOT, OrderType},
};
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_program::pubkey::Pubkey;
use solana_program_test::tokio;
use solana_signer::Signer;
use solana_transaction::Transaction;

use crate::{send_tx_with_retry, TestFixture, Token, SOL_UNIT_SIZE, USDC_UNIT_SIZE};

/// The program data payloads of a simulated transaction, discriminant
/// included, together with the raw log lines so a caller can see truncation
/// and the `Program return:` line.
async fn simulate_program_data(
    test_fixture: &TestFixture,
    instructions: Vec<Instruction>,
) -> (Vec<Vec<u8>>, Vec<String>) {
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair();
    let mut context = test_fixture.context.borrow_mut();
    let blockhash = context.get_new_latest_blockhash().await.unwrap();
    let transaction: Transaction = Transaction::new_signed_with_payer(
        &instructions,
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
    let logs: Vec<String> = simulation.simulation_details.unwrap().logs;
    let payloads: Vec<Vec<u8>> = logs
        .iter()
        .filter_map(|log: &String| log.strip_prefix("Program data: "))
        .filter_map(|data: &str| base64::engine::general_purpose::STANDARD.decode(data).ok())
        .collect();
    (payloads, logs)
}

/// The return data the runtime writes to the log as `Program return:` when the
/// instruction exits, decoded. This is a separate channel from the events: it
/// survives later invocations clearing the structured return data slot,
/// because it is already in the log by then.
fn program_return(logs: &[String]) -> Option<Vec<u8>> {
    logs.iter()
        .find_map(|line: &String| line.strip_prefix("Program return: "))
        .and_then(|rest: &str| rest.split_once(' '))
        .and_then(|(_program_id, data): (&str, &str)| {
            base64::engine::general_purpose::STANDARD.decode(data).ok()
        })
}

/// True for the two events a batch update no longer emits. They still exist
/// as types so that historical transactions can be decoded.
fn is_order_event(payload: &[u8]) -> bool {
    payload.starts_with(&PlaceOrderLog::discriminant())
        || payload.starts_with(&CancelOrderLog::discriminant())
}

/// Rests `count` asks from the fixture's second keypair, in batches so that
/// each transaction fits, and returns nothing: the orders are on the book.
async fn rest_asks(test_fixture: &mut TestFixture, count: usize) -> anyhow::Result<()> {
    let maker: Keypair = test_fixture.second_keypair.insecure_clone();
    let maker_key: Pubkey = maker.pubkey();
    test_fixture.claim_seat_for_keypair(&maker).await?;
    test_fixture
        .deposit_for_keypair(Token::SOL, (count as u64) * SOL_UNIT_SIZE, &maker)
        .await?;
    for chunk in (0..count).collect::<Vec<usize>>().chunks(8) {
        let orders: Vec<PlaceOrderParams> = chunk
            .iter()
            .map(|i: &usize| {
                PlaceOrderParams::new(
                    SOL_UNIT_SIZE,
                    1 + *i as u32,
                    -3,
                    false,
                    OrderType::Limit,
                    NO_EXPIRATION_LAST_VALID_SLOT,
                )
            })
            .collect();
        send_tx_with_retry(
            std::rc::Rc::clone(&test_fixture.context),
            &[batch_update_instruction(
                &test_fixture.market_fixture.key,
                &maker_key,
                None,
                vec![],
                orders,
                None,
                None,
                None,
                None,
            )],
            Some(&maker_key),
            &[&maker],
        )
        .await?;
    }
    Ok(())
}

/// One bid large enough to cross `count` resting asks.
fn cross_all(test_fixture: &TestFixture, count: usize) -> Instruction {
    batch_update_instruction(
        &test_fixture.market_fixture.key,
        &test_fixture.payer(),
        None,
        vec![],
        vec![PlaceOrderParams::new(
            (count as u64) * SOL_UNIT_SIZE,
            100,
            -3,
            true,
            OrderType::Limit,
            NO_EXPIRATION_LAST_VALID_SLOT,
        )],
        None,
        None,
        None,
        None,
    )
}

/// Program data is only visible in the logs when the compiled program runs
/// under `test-sbf`; the native processor test runtime drops it.
#[cfg(feature = "test-sbf")]
#[tokio::test]
async fn batch_update_logs_nothing_for_placing_and_cancelling_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 10 * SOL_UNIT_SIZE).await?;

    // An ask that rests without crossing anything.
    let place: Instruction = batch_update_instruction(
        &test_fixture.market_fixture.key,
        &test_fixture.payer(),
        None,
        vec![],
        vec![PlaceOrderParams::new(
            SOL_UNIT_SIZE,
            5,
            -3,
            false,
            OrderType::Limit,
            NO_EXPIRATION_LAST_VALID_SLOT,
        )],
        None,
        None,
        None,
        None,
    );
    let (payloads, _) = simulate_program_data(&test_fixture, vec![place.clone()]).await;
    assert!(
        payloads.is_empty(),
        "placing an order that does not fill logs nothing, got {} payloads",
        payloads.len(),
    );
    send_tx_with_retry(
        std::rc::Rc::clone(&test_fixture.context),
        &[place],
        Some(&test_fixture.payer()),
        &[&test_fixture.payer_keypair()],
    )
    .await?;

    // Cancelling it logs nothing either.
    let cancel: Instruction = batch_update_instruction(
        &test_fixture.market_fixture.key,
        &test_fixture.payer(),
        None,
        vec![CancelOrderParams::new(0)],
        vec![],
        None,
        None,
        None,
        None,
    );
    let (payloads, _) = simulate_program_data(&test_fixture, vec![cancel]).await;
    assert!(
        payloads.is_empty(),
        "cancelling an order logs nothing, got {} payloads",
        payloads.len(),
    );
    Ok(())
}

/// A batch update that crosses the book emits one `FillLog` per fill, and no
/// event for the order that did the crossing.
#[cfg(feature = "test-sbf")]
#[tokio::test]
async fn batch_update_logs_every_fill_test() -> anyhow::Result<()> {
    const RESTING: usize = 24;
    let mut test_fixture: TestFixture = TestFixture::new().await;
    rest_asks(&mut test_fixture, RESTING).await?;
    test_fixture.claim_seat().await?;
    test_fixture
        .deposit(Token::USDC, 1_000 * USDC_UNIT_SIZE)
        .await?;

    let (payloads, logs) = simulate_program_data(
        &test_fixture,
        vec![
            ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
            cross_all(&test_fixture, RESTING),
        ],
    )
    .await;
    let fills: usize = payloads
        .iter()
        .filter(|payload: &&Vec<u8>| payload.starts_with(&FillLog::discriminant()))
        .count();
    let log_bytes: usize = logs.iter().map(|line: &String| line.len()).sum();
    println!("LOGS {fills} fills in {log_bytes} bytes of transaction log");

    assert_eq!(fills, RESTING, "every fill is logged");
    assert!(
        !payloads
            .iter()
            .any(|payload: &Vec<u8>| is_order_event(payload)),
        "and the order that did the filling is not logged",
    );
    // Nothing else happens to log in this scenario: the makers are ordinary
    // limit orders, so the fills are the whole of it. See the global case
    // below for matching that does log something else.
    assert_eq!(payloads.len(), RESTING);
    Ok(())
}

/// The orders that rested are still reported, on a different channel: the
/// runtime writes the instruction's return data into the log as a
/// `Program return:` line when the program exits. That happens before any
/// later invocation clears the structured return data slot, so it is readable
/// from transaction history even though `meta.returnData` ends up holding
/// whatever ran last, which through the wrapper is its fee transfer.
///
/// It is in the log, so it is subject to the same truncation as everything
/// else there.
#[cfg(feature = "test-sbf")]
#[tokio::test]
async fn the_orders_that_rested_are_in_the_return_log_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 10 * SOL_UNIT_SIZE).await?;

    let place: Instruction = batch_update_instruction(
        &test_fixture.market_fixture.key,
        &test_fixture.payer(),
        None,
        vec![],
        vec![PlaceOrderParams::new(
            SOL_UNIT_SIZE,
            5,
            -3,
            false,
            OrderType::Limit,
            NO_EXPIRATION_LAST_VALID_SLOT,
        )],
        None,
        None,
        None,
        None,
    );
    let (payloads, logs) = simulate_program_data(&test_fixture, vec![place]).await;

    assert!(
        !payloads
            .iter()
            .any(|payload: &Vec<u8>| is_order_event(payload)),
        "the placement is not logged as an event",
    );
    let returned: Vec<u8> = program_return(&logs).expect("the return data reaches the log");
    // Borsh: a u32 count, then (u64 sequence number, u32 order index) each.
    assert_eq!(
        u32::from_le_bytes(returned[..4].try_into().unwrap()),
        1,
        "one order rested",
    );
    assert_eq!(
        u64::from_le_bytes(returned[4..12].try_into().unwrap()),
        0,
        "with sequence number zero, the market's first order",
    );
    assert_ne!(
        u32::from_le_bytes(returned[12..16].try_into().unwrap()),
        NIL,
        "and an index on the book",
    );
    Ok(())
}

/// Matching is free to log other things, and does: a global maker that cannot
/// cover its order is cleaned up during the match and says so. This pins the
/// narrow claim, that no placement or cancellation event is emitted, against a
/// batch update whose log is not just fills.
#[cfg(feature = "test-sbf")]
#[tokio::test]
async fn matching_an_unbacked_global_logs_the_cleanup_but_no_order_events_test(
) -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.global_add_trader().await?;
    test_fixture.global_deposit(1_000_000).await?;

    // A global bid, then the backing is taken away from under it.
    test_fixture
        .batch_update_with_global_for_keypair(
            None,
            vec![],
            vec![PlaceOrderParams::new(
                100,
                1,
                0,
                true,
                OrderType::Global,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &test_fixture.payer_keypair().insecure_clone(),
        )
        .await?;
    test_fixture.global_withdraw(1_000_000).await?;
    test_fixture.deposit(Token::SOL, 1_000_000).await?;

    // Crossing it finds the global unbacked and cleans it up.
    let cross: Instruction = batch_update_instruction(
        &test_fixture.market_fixture.key,
        &test_fixture.payer(),
        None,
        vec![],
        vec![PlaceOrderParams::new(
            100,
            9,
            -1,
            false,
            OrderType::ImmediateOrCancel,
            NO_EXPIRATION_LAST_VALID_SLOT,
        )],
        Some(*test_fixture.market_fixture.market.get_base_mint()),
        None,
        Some(*test_fixture.market_fixture.market.get_quote_mint()),
        None,
    );
    let (payloads, _) = simulate_program_data(&test_fixture, vec![cross]).await;

    assert!(
        payloads
            .iter()
            .any(|payload: &Vec<u8>| payload.starts_with(&GlobalCleanupLog::discriminant())),
        "the cleanup of the unbacked global is logged",
    );
    assert!(
        !payloads
            .iter()
            .any(|payload: &Vec<u8>| is_order_event(payload)),
        "but the order that triggered it still is not",
    );
    Ok(())
}

/// The limit worth knowing about: the transaction log is a fixed byte budget
/// for the whole transaction, and the runtime drops whatever does not fit, so
/// a trade that fills enough orders loses the tail of its own fills. Each
/// `FillLog` is about 310 bytes of base64 once the runtime has framed it, and
/// the budget is a node setting whose default is 10,000 bytes, so the ceiling
/// lands around thirty fills in one transaction.
///
/// Nothing on chain depends on this, and no fill is lost, only the record of
/// it: a client that must see every fill has to read the resulting balances or
/// split the trade across transactions rather than trust the log to be
/// complete. This is not new, but with the order events gone the fills are the
/// only thing left in the log, so it is the only place it can bite.
#[cfg(feature = "test-sbf")]
#[tokio::test]
async fn fills_past_the_transaction_log_budget_are_dropped_test() -> anyhow::Result<()> {
    const RESTING: usize = 40;
    let mut test_fixture: TestFixture = TestFixture::new().await;
    rest_asks(&mut test_fixture, RESTING).await?;
    test_fixture.claim_seat().await?;
    test_fixture
        .deposit(Token::USDC, 5_000 * USDC_UNIT_SIZE)
        .await?;

    let (payloads, logs) = simulate_program_data(
        &test_fixture,
        vec![
            ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
            cross_all(&test_fixture, RESTING),
        ],
    )
    .await;
    let fills: usize = payloads
        .iter()
        .filter(|payload: &&Vec<u8>| payload.starts_with(&FillLog::discriminant()))
        .count();
    let truncated: bool = logs
        .iter()
        .any(|line: &String| line.contains("Log truncated"));
    let log_bytes: usize = logs.iter().map(|line: &String| line.len()).sum();
    println!("LOGS {fills} of {RESTING} fills logged in {log_bytes} bytes, truncated {truncated}");

    assert!(
        fills < RESTING,
        "this documents that fills are lost once the log budget runs out; if all \
         {RESTING} were logged the budget grew and the comment above needs revisiting",
    );
    assert!(truncated, "the runtime reports the truncation");
    Ok(())
}
