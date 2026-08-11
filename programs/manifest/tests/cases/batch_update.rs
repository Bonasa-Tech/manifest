use hypertree::DataIndex;
use manifest::{
    program::{
        batch_update::{CancelOrderParams, PlaceOrderParams},
        batch_update_instruction,
    },
    state::{OrderType, MARKET_BLOCK_SIZE, NO_EXPIRATION_LAST_VALID_SLOT},
};
use solana_keypair::Keypair;
use solana_program::instruction::Instruction;
use solana_program_test::tokio;
use solana_signer::Signer;

use crate::{send_tx_with_retry, TestFixture, Token, SOL_UNIT_SIZE, USDC_UNIT_SIZE};

#[tokio::test]
async fn undefined_order_types_are_rejected_before_market_mutation() -> anyhow::Result<()> {
    for invalid_order_type in [6_u8, u8::MAX] {
        let mut test_fixture: TestFixture = TestFixture::new().await;
        test_fixture.claim_seat().await?;
        test_fixture.deposit(Token::SOL, SOL_UNIT_SIZE).await?;
        let market_before: Vec<u8> = test_fixture
            .try_load(&test_fixture.market_fixture.key)
            .await?
            .unwrap()
            .data;
        let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
        let mut instruction: Instruction = batch_update_instruction(
            &test_fixture.market_fixture.key,
            &payer_keypair.pubkey(),
            None,
            vec![],
            vec![PlaceOrderParams::new(
                SOL_UNIT_SIZE,
                2,
                0,
                false,
                OrderType::Limit,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            None,
            None,
            None,
            None,
        );
        let order_type_byte: &mut u8 = instruction.data.last_mut().unwrap();
        *order_type_byte = invalid_order_type;

        assert!(send_tx_with_retry(
            std::rc::Rc::clone(&test_fixture.context),
            &[instruction],
            Some(&payer_keypair.pubkey()),
            &[&payer_keypair],
        )
        .await
        .is_err());
        let market_after: Vec<u8> = test_fixture
            .try_load(&test_fixture.market_fixture.key)
            .await?
            .unwrap()
            .data;
        assert_eq!(market_after, market_before);
    }
    Ok(())
}

#[tokio::test]
async fn batch_update_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 1 * SOL_UNIT_SIZE).await?;

    // First uses the trader hint, second does not.
    test_fixture
        .batch_update_for_keypair(
            Some(0),
            vec![],
            vec![PlaceOrderParams::new(
                1 * SOL_UNIT_SIZE,
                1,
                0,
                false,
                OrderType::Limit,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &test_fixture.payer_keypair(),
        )
        .await?;

    test_fixture
        .batch_update_for_keypair(
            None,
            vec![CancelOrderParams::new(0)],
            vec![PlaceOrderParams::new(
                1 * SOL_UNIT_SIZE,
                1,
                0,
                false,
                OrderType::Limit,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &test_fixture.payer_keypair(),
        )
        .await?;

    // Hinted cancel wrong seq num
    assert!(test_fixture
        .batch_update_for_keypair(
            Some(0),
            vec![CancelOrderParams::new_with_hint(
                0,
                Some((MARKET_BLOCK_SIZE * 1).try_into().unwrap()),
            )],
            vec![],
            &test_fixture.payer_keypair(),
        )
        .await
        .is_err());

    // Hinted cancel
    test_fixture
        .batch_update_for_keypair(
            Some(0),
            vec![CancelOrderParams::new_with_hint(
                1,
                Some((MARKET_BLOCK_SIZE * 1).try_into().unwrap()),
            )],
            vec![],
            &test_fixture.payer_keypair(),
        )
        .await?;

    Ok(())
}

#[tokio::test]
async fn batch_update_fill_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture
        .deposit(Token::SOL, 1_000 * SOL_UNIT_SIZE)
        .await?;
    test_fixture
        .deposit(Token::USDC, 1_000 * USDC_UNIT_SIZE)
        .await?;

    test_fixture
        .batch_update_for_keypair(
            None,
            vec![],
            vec![PlaceOrderParams::new(
                1 * SOL_UNIT_SIZE,
                1,
                0,
                false,
                OrderType::Limit,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &test_fixture.payer_keypair(),
        )
        .await?;

    test_fixture
        .batch_update_for_keypair(
            None,
            vec![],
            vec![PlaceOrderParams::new(
                1 * SOL_UNIT_SIZE,
                1,
                0,
                true,
                OrderType::Limit,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &test_fixture.payer_keypair(),
        )
        .await?;

    Ok(())
}

#[tokio::test]
async fn batch_update_partial_fill_then_cancel_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture
        .deposit(Token::SOL, 1_000 * SOL_UNIT_SIZE)
        .await?;
    test_fixture
        .deposit(Token::USDC, 10_000 * USDC_UNIT_SIZE)
        .await?;

    test_fixture
        .batch_update_for_keypair(
            None,
            vec![],
            vec![PlaceOrderParams::new(
                2 * SOL_UNIT_SIZE,
                1,
                0,
                false,
                OrderType::Limit,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &test_fixture.payer_keypair(),
        )
        .await?;

    test_fixture
        .batch_update_for_keypair(
            None,
            vec![],
            vec![PlaceOrderParams::new(
                1 * SOL_UNIT_SIZE,
                1,
                0,
                true,
                OrderType::Limit,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &test_fixture.payer_keypair(),
        )
        .await?;

    // Seat is first, then the first order
    test_fixture
        .batch_update_for_keypair(
            None,
            vec![CancelOrderParams::new_with_hint(
                0,
                Some((1 * MARKET_BLOCK_SIZE) as DataIndex),
            )],
            vec![PlaceOrderParams::new(
                1 * SOL_UNIT_SIZE,
                1,
                0,
                true,
                OrderType::Limit,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &test_fixture.payer_keypair(),
        )
        .await?;

    // Verify that there was a cancel and that the partially filled order that
    // got reinserted onto the tree has a valid payload type when cancelling
    // with a hint.
    assert_eq!(
        test_fixture
            .market_fixture
            .get_base_balance_atoms(&test_fixture.payer())
            .await,
        1_000 * SOL_UNIT_SIZE
    );

    Ok(())
}
