use std::rc::Rc;

use hypertree::DataIndex;
use manifest::{
    program::{batch_update::CancelOrderParams, batch_update_instruction},
    state::{OrderType, MARKET_BLOCK_SIZE},
};
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_program::pubkey::Pubkey;
use solana_program_test::tokio;
use solana_signer::Signer;

use crate::{send_tx_with_retry, Side, TestFixture, Token, SOL_UNIT_SIZE, USDC_UNIT_SIZE};

#[tokio::test]
async fn cancel_order_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, SOL_UNIT_SIZE).await?;

    test_fixture
        .place_order(Side::Ask, 1, 1, 0, u32::MAX, OrderType::Limit)
        .await?;

    // First order always has order sequence number of zero.
    test_fixture.cancel_order(0).await?;

    Ok(())
}

#[tokio::test]
async fn cancel_order_bid_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::USDC, USDC_UNIT_SIZE).await?;

    test_fixture
        .place_order(Side::Bid, 1, 1, 0, u32::MAX, OrderType::Limit)
        .await?;

    // First order always has order sequence number of zero.
    test_fixture.cancel_order(0).await?;

    Ok(())
}

// This test failed when the rounding on cancel was incorrect. Now it passes. It
// shows that all funds are retrievable.
#[tokio::test]
async fn cancel_order_rounding_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture
        .deposit(Token::USDC, 2 * USDC_UNIT_SIZE)
        .await?;

    test_fixture
        .place_order(Side::Bid, 1, 11, -1, u32::MAX, OrderType::Limit)
        .await?;

    test_fixture.cancel_order(0).await?;
    test_fixture
        .withdraw(Token::USDC, 2 * USDC_UNIT_SIZE)
        .await?;

    Ok(())
}

#[tokio::test]
async fn cancel_order_fail_other_trader_order_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;

    // Should succeed. It was funded with infinite lamports.
    let second_keypair: Keypair = test_fixture.second_keypair.insecure_clone();
    test_fixture.claim_seat_for_keypair(&second_keypair).await?;
    test_fixture
        .deposit_for_keypair(Token::SOL, 2 * SOL_UNIT_SIZE, &second_keypair)
        .await?;
    test_fixture
        .place_order_for_keypair(
            Side::Ask,
            1,
            1,
            0,
            u32::MAX,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;

    assert!(test_fixture.cancel_order(0).await.is_err());

    assert!(test_fixture
        .batch_update_for_keypair(
            None,
            vec![CancelOrderParams::new_with_hint(
                0,
                Some((MARKET_BLOCK_SIZE * 2) as DataIndex)
            )],
            vec![],
            &test_fixture.payer_keypair()
        )
        .await
        .is_err());

    Ok(())
}

#[tokio::test]
async fn cancel_order_sequence_number_not_exist_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;

    test_fixture.deposit(Token::SOL, SOL_UNIT_SIZE).await?;
    test_fixture
        .place_order(Side::Ask, 1, 1, 0, u32::MAX, OrderType::Limit)
        .await?;

    // Sequence number does not exist. It fails closed.
    assert!(test_fixture.cancel_order(1234).await.is_err());

    Ok(())
}

#[tokio::test]
async fn cancel_order_multiple_bid_on_orderbook_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture
        .deposit(Token::USDC, 10 * USDC_UNIT_SIZE)
        .await?;

    test_fixture
        .place_order(Side::Bid, 1, 1, 0, u32::MAX, OrderType::Limit)
        .await?;
    test_fixture
        .place_order(Side::Bid, 2, 2, 0, u32::MAX, OrderType::Limit)
        .await?;

    // First order always has order sequence number of zero.
    test_fixture.cancel_order(0).await?;

    Ok(())
}

#[tokio::test]
async fn cancel_order_with_hint_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    let payer: Pubkey = test_fixture.context.borrow().payer.pubkey();
    let payer_keypair: Keypair = test_fixture.context.borrow().payer.insecure_clone();

    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, SOL_UNIT_SIZE).await?;

    test_fixture
        .place_order(Side::Ask, 1, 1, 0, u32::MAX, OrderType::Limit)
        .await?;

    let cancel_order_ix: Instruction = batch_update_instruction(
        &test_fixture.market_fixture.key,
        &payer,
        None,
        vec![
            // 0 is ClaimedSeat, next is the order
            CancelOrderParams::new_with_hint(0, Some((1 * MARKET_BLOCK_SIZE).try_into().unwrap())),
        ],
        vec![],
        None,
        None,
        None,
        None,
    );

    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[cancel_order_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    Ok(())
}
