//! Compute unit measurements for the wrapper, and for the same work done
//! straight on the core, so the wrapper's own overhead is a number.
//!
//! Every measurement simulates a transaction from a state the caller set up
//! and prints a line of the form `CU <name>: <units>`. Simulation does not
//! commit, so two shapes measured from the same state are comparable.
//!
//! The numbers are only meaningful with the compiled programs loaded
//! (`cargo test-sbf --arch=v2 --features "test,test-sbf"`); the native
//! processor plain `cargo test` uses does not meter compute.
//!
//! No test asserts a number. They exist so builds can be compared line by
//! line.

use std::{cell::RefMut, rc::Rc};

use manifest::{
    program::{
        batch_update::{CancelOrderParams, PlaceOrderParams},
        instruction_builders::batch_update_instruction as core_batch_update_instruction,
    },
    state::{constants::NO_EXPIRATION_LAST_VALID_SLOT, OrderType},
};
use solana_keypair::Keypair;
use solana_program::{instruction::Instruction, pubkey::Pubkey};
use solana_program_test::{tokio, ProgramTestContext};
use solana_transaction::Transaction;
use wrapper::{
    instruction_builders::batch_update_instruction,
    processors::batch_upate::{WrapperCancelOrderParams, WrapperPlaceOrderParams},
};

use crate::{send_tx_with_retry, TestFixture, Token, SOL_UNIT_SIZE};

/// Simulates `instructions` and returns the units consumed with the result.
async fn simulate(
    test_fixture: &TestFixture,
    instructions: &[Instruction],
    payer: &Pubkey,
    signers: &[&Keypair],
) -> (Result<(), String>, u64) {
    let mut context: RefMut<ProgramTestContext> = test_fixture.context.borrow_mut();
    let blockhash: solana_program::hash::Hash = context.get_new_latest_blockhash().await.unwrap();
    let transaction: Transaction =
        Transaction::new_signed_with_payer(instructions, Some(payer), signers, blockhash);
    let simulation = context
        .banks_client
        .simulate_transaction(transaction)
        .await
        .unwrap();
    let units_consumed: u64 = simulation
        .simulation_details
        .expect("simulation details present")
        .units_consumed;
    let result: Result<(), String> = match simulation.result {
        Some(Err(error)) => Err(format!("{error:?}")),
        _ => Ok(()),
    };
    (result, units_consumed)
}

async fn measure(test_fixture: &TestFixture, name: &str, instructions: &[Instruction]) -> u64 {
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
    let (result, units_consumed) =
        simulate(test_fixture, instructions, &payer, &[&payer_keypair]).await;
    if let Err(error) = result {
        panic!("{name} simulation failed: {error}");
    }
    println!("CU {name}: {units_consumed}");
    units_consumed
}

async fn measure_and_send(
    test_fixture: &TestFixture,
    name: &str,
    instructions: &[Instruction],
) -> anyhow::Result<u64> {
    let units: u64 = measure(test_fixture, name, instructions).await;
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        instructions,
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;
    Ok(units)
}

fn wrapper_ask(client_order_id: u64, price_mantissa: u32) -> WrapperPlaceOrderParams {
    WrapperPlaceOrderParams::new(
        client_order_id,
        SOL_UNIT_SIZE / 100,
        price_mantissa,
        -3,
        false,
        NO_EXPIRATION_LAST_VALID_SLOT,
        OrderType::Limit,
    )
}

fn core_ask(price_mantissa: u32) -> PlaceOrderParams {
    PlaceOrderParams::new(
        SOL_UNIT_SIZE / 100,
        price_mantissa,
        -3,
        false,
        OrderType::Limit,
        NO_EXPIRATION_LAST_VALID_SLOT,
    )
}

fn wrapper_batch(
    test_fixture: &TestFixture,
    cancels: Vec<WrapperCancelOrderParams>,
    orders: Vec<WrapperPlaceOrderParams>,
) -> Instruction {
    batch_update_instruction(
        &test_fixture.market.key,
        &test_fixture.payer(),
        &test_fixture.wrapper.key,
        cancels,
        false,
        orders,
    )
}

fn core_batch(
    test_fixture: &TestFixture,
    cancels: Vec<CancelOrderParams>,
    orders: Vec<PlaceOrderParams>,
) -> Instruction {
    core_batch_update_instruction(
        &test_fixture.market.key,
        &test_fixture.payer(),
        None,
        cancels,
        orders,
        None,
        None,
        None,
        None,
    )
}

/// Placing orders, through the wrapper and straight on the core, from the
/// same state. The difference is what the wrapper costs on top: its own
/// bookkeeping, plus the CPI into the core and the account size charge that
/// comes with it.
#[tokio::test]
async fn cu_place() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture
        .deposit(Token::SOL, 100 * SOL_UNIT_SIZE)
        .await?;

    for count in [1usize, 5, 10] {
        let orders: Vec<WrapperPlaceOrderParams> = (1..=count as u64)
            .map(|i| wrapper_ask(i, 5 + i as u32))
            .collect();
        measure(
            &test_fixture,
            &format!("wrapper_place_{count}"),
            &[wrapper_batch(&test_fixture, vec![], orders)],
        )
        .await;
        let core_orders: Vec<PlaceOrderParams> =
            (1..=count as u32).map(|i| core_ask(5 + i)).collect();
        measure(
            &test_fixture,
            &format!("core_place_{count}"),
            &[core_batch(&test_fixture, vec![], core_orders)],
        )
        .await;
    }
    Ok(())
}

/// Cancelling, and the mixed shape a maker actually sends. Measured on a book
/// where this trader already has twenty resting orders, which is what makes
/// the per open order costs visible.
#[tokio::test]
async fn cu_cancel_and_replace() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture
        .deposit(Token::SOL, 100 * SOL_UNIT_SIZE)
        .await?;

    // Twenty resting orders, placed in batches so no single transaction runs
    // out of compute.
    for batch in 0..4u64 {
        let orders: Vec<WrapperPlaceOrderParams> = (1..=5u64)
            .map(|i| {
                let id: u64 = batch * 5 + i;
                wrapper_ask(id, 100 + id as u32)
            })
            .collect();
        measure_and_send(
            &test_fixture,
            &format!("wrapper_place_5_at_{}_resting", batch * 5),
            &[wrapper_batch(&test_fixture, vec![], orders)],
        )
        .await?;
    }

    for count in [1u64, 5, 10] {
        let cancels: Vec<WrapperCancelOrderParams> =
            (1..=count).map(WrapperCancelOrderParams::new).collect();
        measure(
            &test_fixture,
            &format!("wrapper_cancel_{count}_of_20"),
            &[wrapper_batch(&test_fixture, cancels, vec![])],
        )
        .await;
    }

    let cancels: Vec<WrapperCancelOrderParams> =
        (1..=5u64).map(WrapperCancelOrderParams::new).collect();
    let orders: Vec<WrapperPlaceOrderParams> = (21..=25u64)
        .map(|i| wrapper_ask(i, 100 + i as u32))
        .collect();
    measure(
        &test_fixture,
        "wrapper_cancel_5_place_5_of_20",
        &[wrapper_batch(&test_fixture, cancels, orders)],
    )
    .await;

    // The same replace done straight on the core, without the wrapper's
    // bookkeeping, for the floor this could approach.
    let core_cancels: Vec<CancelOrderParams> = (0..5u64).map(CancelOrderParams::new).collect();
    let core_orders: Vec<PlaceOrderParams> = (21..=25u32).map(|i| core_ask(100 + i)).collect();
    measure(
        &test_fixture,
        "core_cancel_5_place_5_of_20",
        &[core_batch(&test_fixture, core_cancels, core_orders)],
    )
    .await;

    // What a batch that changes nothing costs: the sync, the CPI and the
    // fixed overhead with no orders at all.
    measure(
        &test_fixture,
        "wrapper_empty_batch_of_20",
        &[wrapper_batch(&test_fixture, vec![], vec![])],
    )
    .await;
    measure(
        &test_fixture,
        "core_empty_batch_of_20",
        &[core_batch(&test_fixture, vec![], vec![])],
    )
    .await;
    Ok(())
}

/// The instructions around trading.
#[tokio::test]
async fn cu_other_instructions() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 10 * SOL_UNIT_SIZE).await?;
    measure(
        &test_fixture,
        "wrapper_empty_batch_of_0",
        &[wrapper_batch(&test_fixture, vec![], vec![])],
    )
    .await;
    Ok(())
}
