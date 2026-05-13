use manifest::{
    program::batch_update::{CancelOrderParams, PlaceOrderParams},
    state::{OrderType, NO_EXPIRATION_LAST_VALID_SLOT},
};
use solana_signer::Signer;

use crate::{TestFixture, SOL_UNIT_SIZE, USDC_UNIT_SIZE};

async fn scenario(
    fixture: &mut TestFixture,
    maker_is_bid: bool,
    price_mantissa: u32,
    price_exponent: i8,
    place_atoms: u64,
    match_atoms: u64,
) -> anyhow::Result<()> {
    fixture
        .batch_update_for_keypair(
            None,
            vec![],
            vec![PlaceOrderParams::new(
                place_atoms,
                price_mantissa,
                price_exponent,
                maker_is_bid,
                OrderType::Limit,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &fixture.payer_keypair(),
        )
        .await?;

    fixture
        .batch_update_for_keypair(
            None,
            vec![],
            vec![PlaceOrderParams::new(
                match_atoms,
                price_mantissa,
                price_exponent,
                !maker_is_bid,
                OrderType::Limit,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &fixture.second_keypair.insecure_clone(),
        )
        .await?;

    // Seat is first, then the first order
    fixture
        .batch_update_for_keypair(
            None,
            vec![CancelOrderParams::new(0)],
            vec![],
            &fixture.payer_keypair(),
        )
        .await?;

    Ok(())
}

async fn verify_balances(
    test_fixture: &mut TestFixture,
    maker_base_atoms: u64,
    maker_quote_atoms: u64,
    taker_base_atoms: u64,
    taker_quote_atoms: u64,
) -> anyhow::Result<()> {
    assert_eq!(
        test_fixture
            .market_fixture
            .get_base_balance_atoms(&test_fixture.payer())
            .await,
        maker_base_atoms
    );
    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_balance_atoms(&test_fixture.payer())
            .await,
        maker_quote_atoms
    );

    assert_eq!(
        test_fixture
            .market_fixture
            .get_base_balance_atoms(&test_fixture.second_keypair.pubkey())
            .await,
        taker_base_atoms
    );

    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_balance_atoms(&test_fixture.second_keypair.pubkey())
            .await,
        taker_quote_atoms
    );

    Ok(())
}

/*
Scenarios:
F - Taker fully matches maker order
P - Taker partially matches maker order (remainder is cancelled)
E - Order is exact quote = price * base_atoms
R - Order is rounded quote = round(price * base)

Maker | Taker | Case / Comment
E     | EF    | test_match_full_no_rounding
E     | EP    | test_match_partial_no_rounding
E     | RF    | impossible - not tested
E     | RP    | test_match_partial_exact_place_round_match
R     | EF    | impossible - not tested
R     | EP    | test_match_partial_round_place_exact_match
R     | RF    | test_match_full_round_place_round_match
R     | RP    | test_match_partial_round_place_round_match
*/

#[tokio::test]
async fn test_match_full_no_rounding() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::try_new_for_matching_test().await?;

    let err = scenario(
        &mut test_fixture,
        false,
        1,
        -3,
        1_000 * SOL_UNIT_SIZE,
        1_000 * SOL_UNIT_SIZE,
    )
    .await;
    assert!(err.is_err(), "expect cancel to fail due to full match");

    verify_balances(
        &mut test_fixture,
        0,
        11_000 * USDC_UNIT_SIZE,
        2_000 * SOL_UNIT_SIZE,
        9_000 * USDC_UNIT_SIZE,
    )
    .await
}

#[tokio::test]
async fn test_match_partial_no_rounding() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::try_new_for_matching_test().await?;

    let _ = scenario(
        &mut test_fixture,
        false,
        1,
        -3,
        1_000 * SOL_UNIT_SIZE,
        500 * SOL_UNIT_SIZE,
    )
    .await;

    verify_balances(
        &mut test_fixture,
        500 * SOL_UNIT_SIZE,
        10_500 * USDC_UNIT_SIZE,
        1_500 * SOL_UNIT_SIZE,
        9_500 * USDC_UNIT_SIZE,
    )
    .await
}

#[tokio::test]
async fn test_match_partial_exact_place_round_match() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::try_new_for_matching_test().await?;

    let _ = scenario(&mut test_fixture, false, 1, -3, 1000, 1).await;

    verify_balances(
        &mut test_fixture,
        1000 * SOL_UNIT_SIZE - 1,
        10000 * USDC_UNIT_SIZE + 1,
        1000 * SOL_UNIT_SIZE + 1,
        10000 * USDC_UNIT_SIZE - 1,
    )
    .await
}

#[tokio::test]
async fn test_match_partial_round_place_exact_match() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::try_new_for_matching_test().await?;

    let _ = scenario(&mut test_fixture, false, 1, -3, 1111, 1000).await;

    verify_balances(
        &mut test_fixture,
        1000 * SOL_UNIT_SIZE - 1000,
        10000 * USDC_UNIT_SIZE + 1,
        1000 * SOL_UNIT_SIZE + 1000,
        10000 * USDC_UNIT_SIZE - 1,
    )
    .await
}

#[tokio::test]
async fn test_match_full_round_place_round_match() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::try_new_for_matching_test().await?;

    let err = scenario(&mut test_fixture, false, 1, -3, 1, 1).await;
    assert!(err.is_err(), "expect cancel to fail due to full match");

    verify_balances(
        &mut test_fixture,
        1000 * SOL_UNIT_SIZE - 1,
        10000 * USDC_UNIT_SIZE,
        1000 * SOL_UNIT_SIZE + 1,
        10000 * USDC_UNIT_SIZE,
    )
    .await
}

#[tokio::test]
async fn test_match_partial_round_place_round_match() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::try_new_for_matching_test().await?;

    let _ = scenario(&mut test_fixture, false, 1, -3, 2, 1).await;

    verify_balances(
        &mut test_fixture,
        1000 * SOL_UNIT_SIZE - 1,
        10000 * USDC_UNIT_SIZE + 1,
        1000 * SOL_UNIT_SIZE + 1,
        10000 * USDC_UNIT_SIZE - 1,
    )
    .await
}
