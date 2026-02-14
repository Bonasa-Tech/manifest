use std::u64;

use hypertree::HyperTreeValueIteratorTrait;
use manifest::{
    quantities::{QuoteAtomsPerBaseAtom, WrapperU64},
    state::{
        constants::{MARKET_BLOCK_SIZE, MARKET_FIXED_SIZE, NO_EXPIRATION_LAST_VALID_SLOT},
        OrderType, RestingOrder,
    },
    validation::get_vault_address,
};
use solana_program_test::tokio;
use solana_sdk::signature::{Keypair, Signer};

use crate::{Side, TestFixture, Token, SOL_UNIT_SIZE, USDC_UNIT_SIZE};

#[tokio::test]
async fn place_order_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, SOL_UNIT_SIZE).await?;

    test_fixture
        .place_order(
            Side::Ask,
            1,
            1,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )
        .await?;

    Ok(())
}

#[tokio::test]
async fn place_order_fail_no_seat_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    assert!(test_fixture
        .place_order(
            Side::Ask,
            1,
            1,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit
        )
        .await
        .is_err());

    Ok(())
}

#[tokio::test]
async fn place_order_fail_no_deposit_yet_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    assert!(test_fixture
        .place_order(
            Side::Ask,
            1,
            1,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit
        )
        .await
        .is_err());

    Ok(())
}

#[tokio::test]
async fn place_order_fail_insufficient_funds_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 1 * SOL_UNIT_SIZE).await?;
    assert!(test_fixture
        .place_order(
            Side::Ask,
            2 * SOL_UNIT_SIZE,
            1,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit
        )
        .await
        .is_err());

    Ok(())
}

#[tokio::test]
async fn place_order_not_expand_if_not_needed_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 1 * SOL_UNIT_SIZE).await?;

    test_fixture
        .place_order(
            Side::Ask,
            1,
            1,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )
        .await?;
    test_fixture.cancel_order(0).await?;
    test_fixture
        .place_order(
            Side::Ask,
            1,
            1,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )
        .await?;

    let loaded_account = test_fixture
        .try_load(&test_fixture.market_fixture.key)
        .await?
        .unwrap();
    // Always 1 more than needed.
    assert_eq!(
        loaded_account.data.len(),
        MARKET_FIXED_SIZE + (3 * MARKET_BLOCK_SIZE)
    );

    Ok(())
}

#[tokio::test]
async fn match_limit_orders_basic_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 2 * SOL_UNIT_SIZE).await?;
    test_fixture
        .place_order(
            Side::Ask,
            2 * SOL_UNIT_SIZE,
            1,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )
        .await?;

    // Should succeed. It was funded with infinite lamports.
    let second_keypair: Keypair = test_fixture.second_keypair.insecure_clone();
    test_fixture.claim_seat_for_keypair(&second_keypair).await?;
    test_fixture
        .deposit_for_keypair(Token::USDC, 2_000 * USDC_UNIT_SIZE, &second_keypair)
        .await?;
    test_fixture
        .place_order_for_keypair(
            Side::Bid,
            1 * SOL_UNIT_SIZE,
            1,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;

    // Trade happens so we can withdraw.
    test_fixture
        .withdraw(Token::USDC, 1_000 * USDC_UNIT_SIZE)
        .await?;

    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_balance_atoms(&test_fixture.payer())
            .await,
        0
    );
    assert!(test_fixture
        .withdraw(Token::USDC, USDC_UNIT_SIZE)
        .await
        .is_err());

    test_fixture
        .place_order_for_keypair(
            Side::Bid,
            1 * SOL_UNIT_SIZE,
            1,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;
    test_fixture
        .withdraw(Token::USDC, 1_000 * USDC_UNIT_SIZE)
        .await?;
    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_balance_atoms(&test_fixture.payer())
            .await,
        0
    );

    // verify volume stats
    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_volume(&test_fixture.payer())
            .await,
        2_000 * USDC_UNIT_SIZE
    );
    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_volume(&second_keypair.pubkey())
            .await,
        2_000 * USDC_UNIT_SIZE
    );
    assert_eq!(
        test_fixture.market_fixture.market.fixed.get_quote_volume(),
        2_000 * USDC_UNIT_SIZE
    );

    Ok(())
}

#[tokio::test]
async fn match_limit_orders_basic_test_reverse() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture
        .deposit(Token::USDC, 2_000 * USDC_UNIT_SIZE)
        .await?;
    test_fixture
        .place_order(
            Side::Bid,
            2 * SOL_UNIT_SIZE,
            1,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )
        .await?;

    let second_keypair: Keypair = test_fixture.second_keypair.insecure_clone();
    test_fixture.claim_seat_for_keypair(&second_keypair).await?;
    test_fixture
        .deposit_for_keypair(Token::SOL, 2 * SOL_UNIT_SIZE, &second_keypair)
        .await?;
    test_fixture
        .place_order_for_keypair(
            Side::Ask,
            1 * SOL_UNIT_SIZE,
            1,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;

    test_fixture.withdraw(Token::SOL, SOL_UNIT_SIZE).await?;
    assert_eq!(
        test_fixture
            .market_fixture
            .get_base_balance_atoms(&test_fixture.payer())
            .await,
        0
    );

    assert!(test_fixture
        .withdraw(Token::SOL, SOL_UNIT_SIZE)
        .await
        .is_err());

    test_fixture
        .place_order_for_keypair(
            Side::Ask,
            1 * SOL_UNIT_SIZE,
            1,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;
    test_fixture.withdraw(Token::SOL, SOL_UNIT_SIZE).await?;
    assert_eq!(
        test_fixture
            .market_fixture
            .get_base_balance_atoms(&test_fixture.payer())
            .await,
        0
    );
    Ok(())
}

#[tokio::test]
async fn match_limit_orders_more_than_resting_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 2 * SOL_UNIT_SIZE).await?;
    test_fixture
        .place_order(
            Side::Ask,
            1 * SOL_UNIT_SIZE,
            1,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )
        .await?;

    let second_keypair: Keypair = test_fixture.second_keypair.insecure_clone();
    test_fixture.claim_seat_for_keypair(&second_keypair).await?;
    test_fixture
        .deposit_for_keypair(Token::USDC, 2_000 * USDC_UNIT_SIZE, &second_keypair)
        .await?;

    test_fixture
        .place_order_for_keypair(
            Side::Bid,
            2 * SOL_UNIT_SIZE,
            1,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;
    // Only one matches.
    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_balance_atoms(&test_fixture.payer())
            .await,
        1_000 * USDC_UNIT_SIZE
    );
    Ok(())
}

#[tokio::test]
async fn match_limit_orders_fail_expired_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 4 * SOL_UNIT_SIZE).await?;

    let second_keypair: Keypair = test_fixture.second_keypair.insecure_clone();
    test_fixture.claim_seat_for_keypair(&second_keypair).await?;
    test_fixture
        .deposit_for_keypair(Token::USDC, 16_000 * USDC_UNIT_SIZE, &second_keypair)
        .await?;

    test_fixture
        .place_order(
            Side::Ask,
            1 * SOL_UNIT_SIZE,
            1,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )
        .await?;
    // Slots may advance during tests, so expiration is set pretty far out.
    test_fixture
        .place_order(Side::Ask, 1 * SOL_UNIT_SIZE, 2, 0, 1_000, OrderType::Limit)
        .await?;
    test_fixture
        .place_order(
            Side::Ask,
            1 * SOL_UNIT_SIZE,
            3,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )
        .await?;
    test_fixture
        .place_order(
            Side::Ask,
            1 * SOL_UNIT_SIZE,
            4,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )
        .await?;

    // Expire the order @2
    test_fixture.advance_time_seconds(10_000).await;

    // Should match the first and third and last, skipping the 2 expired one.
    test_fixture
        .place_order_for_keypair(
            Side::Bid,
            4 * SOL_UNIT_SIZE,
            4,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;

    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_balance_atoms(&test_fixture.payer())
            .await,
        8_000 * USDC_UNIT_SIZE
    );
    // 1 + 3 + 4 = 8
    test_fixture
        .withdraw(Token::USDC, 8_000 * USDC_UNIT_SIZE)
        .await?;
    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_balance_atoms(&test_fixture.payer())
            .await,
        0
    );
    Ok(())
}

#[tokio::test]
async fn match_limit_orders_partial_match_price_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 10 * SOL_UNIT_SIZE).await?;
    test_fixture
        .place_order(
            Side::Ask,
            1 * SOL_UNIT_SIZE,
            1,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )
        .await?;
    test_fixture
        .place_order(
            Side::Ask,
            1 * SOL_UNIT_SIZE,
            2,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )
        .await?;
    test_fixture
        .place_order(
            Side::Ask,
            1 * SOL_UNIT_SIZE,
            3,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )
        .await?;

    let second_keypair: Keypair = test_fixture.second_keypair.insecure_clone();
    test_fixture.claim_seat_for_keypair(&second_keypair).await?;
    test_fixture
        .deposit_for_keypair(Token::USDC, 10_000 * USDC_UNIT_SIZE, &second_keypair)
        .await?;
    test_fixture
        .place_order_for_keypair(
            Side::Bid,
            3 * SOL_UNIT_SIZE,
            2,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;

    // Trade happens so we can withdraw. But the other has not matched yet.
    test_fixture
        .withdraw(Token::USDC, 3_000 * USDC_UNIT_SIZE)
        .await?;
    test_fixture
        .withdraw_for_keypair(Token::SOL, 2 * SOL_UNIT_SIZE, &second_keypair)
        .await?;

    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_balance_atoms(&test_fixture.payer())
            .await,
        0
    );
    // 10 - 1 - 2 = 7
    assert_eq!(
        test_fixture
            .market_fixture
            .get_base_balance_atoms(&test_fixture.payer())
            .await,
        7 * SOL_UNIT_SIZE
    );

    Ok(())
}

#[tokio::test]
async fn match_limit_orders_with_large_deposits_test() -> anyhow::Result<()> {
    const DEPOSIT_BALANCE: u64 = u64::MAX / 512;
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::USDC, DEPOSIT_BALANCE).await?;
    test_fixture.deposit(Token::SOL, DEPOSIT_BALANCE).await?;
    test_fixture
        .place_order(
            Side::Bid,
            1_000_000_000,
            1,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )
        .await?;

    let second_keypair: Keypair = test_fixture.second_keypair.insecure_clone();
    test_fixture.claim_seat_for_keypair(&second_keypair).await?;
    test_fixture
        .deposit_for_keypair(Token::USDC, DEPOSIT_BALANCE, &second_keypair)
        .await?;
    test_fixture
        .deposit_for_keypair(Token::SOL, DEPOSIT_BALANCE, &second_keypair)
        .await?;
    test_fixture
        .place_order_for_keypair(
            Side::Ask,
            500_000_000,
            1,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;

    let mut user_balance_base = test_fixture
        .market_fixture
        .get_base_balance_atoms(&test_fixture.payer())
        .await;
    user_balance_base += test_fixture
        .market_fixture
        .get_base_balance_atoms(&second_keypair.pubkey())
        .await;
    let mut user_balance_quote = test_fixture
        .market_fixture
        .get_quote_balance_atoms(&test_fixture.payer())
        .await;
    user_balance_quote += test_fixture
        .market_fixture
        .get_quote_balance_atoms(&second_keypair.pubkey())
        .await;

    for (_, bid) in test_fixture
        .market_fixture
        .market
        .get_bids()
        .iter::<RestingOrder>()
    {
        let bid_balance_quote = (bid.get_num_base_atoms().checked_mul(bid.get_price(), true))
            .unwrap()
            .as_u64();
        println!("bid {bid_balance_quote}");
        user_balance_quote += bid_balance_quote;
    }

    for (_, ask) in test_fixture
        .market_fixture
        .market
        .get_asks()
        .iter::<RestingOrder>()
    {
        let ask_balance_base = ask.get_num_base_atoms().as_u64();
        println!("ask {ask_balance_base}");
        user_balance_base += ask_balance_base;
    }

    // In perps, base is virtual — no physical base vault
    let (vault_address_quote, _) = get_vault_address(
        &test_fixture.market_fixture.key,
        test_fixture.market_fixture.market.get_quote_mint(),
    );
    let vault_balance_quote: u64 = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_packed_account_data::<spl_token::state::Account>(vault_address_quote)
        .await
        .expect("quote vault")
        .amount;

    assert_eq!(user_balance_quote, vault_balance_quote);

    Ok(())
}

#[tokio::test]
async fn post_only_basic_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;

    // Ask for 2@10
    test_fixture.deposit(Token::SOL, 20 * SOL_UNIT_SIZE).await?;
    test_fixture
        .place_order(
            Side::Ask,
            2 * SOL_UNIT_SIZE,
            10,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )
        .await?;

    let second_keypair: Keypair = test_fixture.second_keypair.insecure_clone();
    test_fixture.claim_seat_for_keypair(&second_keypair).await?;
    test_fixture
        .deposit_for_keypair(Token::USDC, 1_000 * USDC_UNIT_SIZE, &second_keypair)
        .await?;

    // PostOnly should succeed because it doesnt match
    test_fixture
        .place_order_for_keypair(
            Side::Bid,
            1 * SOL_UNIT_SIZE,
            1,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::PostOnly,
            &second_keypair,
        )
        .await?;
    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_balance_atoms(&test_fixture.payer())
            .await,
        0
    );
    Ok(())
}

#[tokio::test]
async fn post_only_fail_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;

    // Ask for 2@10
    test_fixture.deposit(Token::SOL, 20 * SOL_UNIT_SIZE).await?;
    test_fixture
        .place_order(
            Side::Ask,
            2 * SOL_UNIT_SIZE,
            10,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )
        .await?;

    let second_keypair: Keypair = test_fixture.second_keypair.insecure_clone();
    test_fixture.claim_seat_for_keypair(&second_keypair).await?;
    test_fixture
        .deposit_for_keypair(Token::USDC, 20_000 * USDC_UNIT_SIZE, &second_keypair)
        .await?;

    // Post only should fail because it wants to match at 10.
    assert!(test_fixture
        .place_order_for_keypair(
            Side::Bid,
            1 * SOL_UNIT_SIZE,
            11,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::PostOnly,
            &second_keypair
        )
        .await
        .is_err());
    // All balance is on the orderbook.
    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_balance_atoms(&test_fixture.payer())
            .await,
        0
    );
    Ok(())
}

#[tokio::test]
async fn place_order_already_expired_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, SOL_UNIT_SIZE).await?;

    test_fixture.advance_time_seconds(10).await;
    assert!(test_fixture
        .place_order(Side::Ask, 1, 1, 0, 1, OrderType::Limit,)
        .await
        .is_err());

    Ok(())
}

// Transaction should succeed but no order rests on the book.
#[tokio::test]
async fn place_order_zero_price() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, SOL_UNIT_SIZE).await?;

    test_fixture
        .place_order(
            Side::Ask,
            1 * SOL_UNIT_SIZE,
            0,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )
        .await?;
    assert_eq!(
        test_fixture.market_fixture.get_resting_orders().await.len(),
        0
    );

    Ok(())
}

// Transaction should succeed but no order rests on the book.
#[tokio::test]
async fn place_order_zero_size() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, SOL_UNIT_SIZE).await?;

    test_fixture
        .place_order(
            Side::Ask,
            0,
            1,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )
        .await?;
    assert_eq!(
        test_fixture.market_fixture.get_resting_orders().await.len(),
        0
    );

    Ok(())
}

#[tokio::test]
async fn reverse_order_type_test() -> anyhow::Result<()> {
    // Default payer places reverse orders on both booksides.
    // Buy 3@1.0, Sell 3@3.0
    // Spread is 50%, so the new order is top of book after fill.
    // (1.0 -> 2.0, 3.0 -> 1.5)

    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 10 * SOL_UNIT_SIZE).await?;
    test_fixture
        .deposit(Token::USDC, 10_000 * USDC_UNIT_SIZE)
        .await?;
    test_fixture
        .place_order(
            Side::Bid,
            3 * SOL_UNIT_SIZE,
            1,
            0,
            50_000,
            OrderType::Reverse,
        )
        .await?;
    test_fixture
        .place_order(
            Side::Ask,
            3 * SOL_UNIT_SIZE,
            3,
            0,
            50_000,
            OrderType::Reverse,
        )
        .await?;

    // Setup the second keypair.
    let second_keypair: Keypair = test_fixture.second_keypair.insecure_clone();
    test_fixture.claim_seat_for_keypair(&second_keypair).await?;
    test_fixture
        .deposit_for_keypair(Token::USDC, 10_000 * USDC_UNIT_SIZE, &second_keypair)
        .await?;
    test_fixture
        .deposit_for_keypair(Token::SOL, 10_000 * SOL_UNIT_SIZE, &second_keypair)
        .await?;

    // Partial fill on each, then full fill.
    test_fixture
        .place_order_for_keypair(
            Side::Bid,
            1 * SOL_UNIT_SIZE,
            4,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;

    // 7 = 10 - 3. All proceeds were put back on the book
    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_balance_atoms(&test_fixture.payer())
            .await,
        7_000 * USDC_UNIT_SIZE
    );
    // 10 - 3 = 7
    assert_eq!(
        test_fixture
            .market_fixture
            .get_base_balance_atoms(&test_fixture.payer())
            .await,
        7 * SOL_UNIT_SIZE
    );
    let resting_orders: Vec<RestingOrder> = test_fixture.market_fixture.get_resting_orders().await;
    // bids first, asks second.
    assert_eq!(resting_orders.len(), 3);
    let first_bid: &RestingOrder = resting_orders.get(0).unwrap();
    let second_bid: &RestingOrder = resting_orders.get(1).unwrap();
    // First bid is the new flipped version of the first ask.
    // Book is bid: 2@1.5, 3@1
    assert_eq!(first_bid.get_is_bid(), true);
    assert_eq!(second_bid.get_is_bid(), true);
    assert_eq!(first_bid.get_num_base_atoms().as_u64(), 2 * SOL_UNIT_SIZE);
    assert_eq!(second_bid.get_num_base_atoms().as_u64(), 3 * SOL_UNIT_SIZE);
    assert_eq!(
        first_bid.get_price(),
        QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(15, -1).unwrap()
    );
    assert_eq!(
        second_bid.get_price(),
        QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(1, 0).unwrap()
    );

    let first_ask: &RestingOrder = resting_orders.get(2).unwrap();
    assert_eq!(first_ask.get_is_bid(), false);
    assert_eq!(first_ask.get_num_base_atoms().as_u64(), 2 * SOL_UNIT_SIZE);
    assert_eq!(
        first_ask.get_price(),
        QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(3, 0).unwrap()
    );

    // Full fill
    test_fixture
        .place_order_for_keypair(
            Side::Bid,
            2 * SOL_UNIT_SIZE,
            4,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;
    let resting_orders: Vec<RestingOrder> = test_fixture.market_fixture.get_resting_orders().await;
    // bids first, asks second.
    assert_eq!(resting_orders.len(), 2);
    let first_bid: &RestingOrder = resting_orders.get(0).unwrap();
    let second_bid: &RestingOrder = resting_orders.get(1).unwrap();
    // First bid is the flipped version of the first ask fully coalesced.
    // Book is bid: 6@1.5, 3@1
    assert_eq!(first_bid.get_is_bid(), true);
    assert_eq!(second_bid.get_is_bid(), true);
    assert_eq!(first_bid.get_num_base_atoms().as_u64(), 6 * SOL_UNIT_SIZE);
    assert_eq!(second_bid.get_num_base_atoms().as_u64(), 3 * SOL_UNIT_SIZE);
    assert_eq!(
        first_bid.get_price(),
        QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(15, -1).unwrap()
    );
    assert_eq!(
        second_bid.get_price(),
        QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(1, 0).unwrap()
    );
    // Balance is unchanged.
    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_balance_atoms(&test_fixture.payer())
            .await,
        7_000 * USDC_UNIT_SIZE
    );
    assert_eq!(
        test_fixture
            .market_fixture
            .get_base_balance_atoms(&test_fixture.payer())
            .await,
        7 * SOL_UNIT_SIZE
    );

    // Fill in the other direction puts back the ask.
    test_fixture
        .place_order_for_keypair(
            Side::Ask,
            2 * SOL_UNIT_SIZE,
            1,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;
    let resting_orders: Vec<RestingOrder> = test_fixture.market_fixture.get_resting_orders().await;
    assert_eq!(resting_orders.len(), 3);
    let first_bid: &RestingOrder = resting_orders.get(0).unwrap();
    let second_bid: &RestingOrder = resting_orders.get(1).unwrap();
    // First bid is the flipped version of the first ask fully coalesced.
    // Book is bid: 4@1.5, 3@1
    assert_eq!(first_bid.get_is_bid(), true);
    assert_eq!(second_bid.get_is_bid(), true);
    assert_eq!(first_bid.get_num_base_atoms().as_u64(), 4 * SOL_UNIT_SIZE);
    assert_eq!(second_bid.get_num_base_atoms().as_u64(), 3 * SOL_UNIT_SIZE);
    assert_eq!(
        first_bid.get_price(),
        QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(15, -1).unwrap()
    );
    assert_eq!(
        second_bid.get_price(),
        QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(1, 0).unwrap()
    );
    let first_ask: &RestingOrder = resting_orders.get(2).unwrap();
    assert_eq!(first_ask.get_is_bid(), false);
    assert_eq!(first_ask.get_num_base_atoms().as_u64(), 2 * SOL_UNIT_SIZE);
    assert_eq!(
        first_ask.get_price(),
        QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(3, 0).unwrap()
    );

    // Maker's balances are unchanged.
    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_balance_atoms(&test_fixture.payer())
            .await,
        7_000 * USDC_UNIT_SIZE
    );
    assert_eq!(
        test_fixture
            .market_fixture
            .get_base_balance_atoms(&test_fixture.payer())
            .await,
        7 * SOL_UNIT_SIZE
    );

    Ok(())
}

#[tokio::test]
async fn reverse_order_tight_type_test() -> anyhow::Result<()> {
    // Default payer places reverse orders on both booksides.
    // Sell 3@3
    // Spread is .005%, so the new order is top of book after fill.

    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 10 * SOL_UNIT_SIZE).await?;
    test_fixture
        .deposit(Token::USDC, 10_000 * USDC_UNIT_SIZE)
        .await?;
    test_fixture
        .place_order(
            Side::Ask,
            3 * SOL_UNIT_SIZE,
            3,
            0,
            50_000,
            OrderType::ReverseTight,
        )
        .await?;

    // Setup the second keypair.
    let second_keypair: Keypair = test_fixture.second_keypair.insecure_clone();
    test_fixture.claim_seat_for_keypair(&second_keypair).await?;
    test_fixture
        .deposit_for_keypair(Token::USDC, 10_000 * USDC_UNIT_SIZE, &second_keypair)
        .await?;
    test_fixture
        .deposit_for_keypair(Token::SOL, 10_000 * SOL_UNIT_SIZE, &second_keypair)
        .await?;

    // Partial fill
    test_fixture
        .place_order_for_keypair(
            Side::Bid,
            1 * SOL_UNIT_SIZE,
            4,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;

    // 10. Did not change except now due to rounding, there are extra atoms.
    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_balance_atoms(&test_fixture.payer())
            .await,
        10_000 * USDC_UNIT_SIZE
    );
    // 10 - 3 = 7
    assert_eq!(
        test_fixture
            .market_fixture
            .get_base_balance_atoms(&test_fixture.payer())
            .await,
        7 * SOL_UNIT_SIZE
    );
    let resting_orders: Vec<RestingOrder> = test_fixture.market_fixture.get_resting_orders().await;
    // bids first, asks second.
    assert_eq!(resting_orders.len(), 2);
    let first_bid: &RestingOrder = resting_orders.get(0).unwrap();
    // First bid is the new flipped version of the first ask.
    assert_eq!(first_bid.get_is_bid(), true);
    assert_eq!(
        first_bid.get_price(),
        QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(29_985, -4).unwrap()
    );
    assert_eq!(first_bid.get_num_base_atoms().as_u64(), 1_000_500_250);

    let first_ask: &RestingOrder = resting_orders.get(1).unwrap();
    assert_eq!(first_ask.get_is_bid(), false);
    assert_eq!(first_ask.get_num_base_atoms().as_u64(), 2 * SOL_UNIT_SIZE);
    assert_eq!(
        first_ask.get_price(),
        QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(3, 0).unwrap()
    );

    // Full fill
    test_fixture
        .place_order_for_keypair(
            Side::Bid,
            2 * SOL_UNIT_SIZE,
            4,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;
    let resting_orders: Vec<RestingOrder> = test_fixture.market_fixture.get_resting_orders().await;
    assert_eq!(resting_orders.len(), 1);
    let first_bid: &RestingOrder = resting_orders.get(0).unwrap();
    assert_eq!(first_bid.get_is_bid(), true);
    assert_eq!(first_bid.get_num_base_atoms().as_u64(), 3_001_500_750);
    assert_eq!(
        first_bid.get_price(),
        QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(29_985, -4).unwrap()
    );
    // Balance is same.
    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_balance_atoms(&test_fixture.payer())
            .await,
        10_000_000_000
    );
    assert_eq!(
        test_fixture
            .market_fixture
            .get_base_balance_atoms(&test_fixture.payer())
            .await,
        7 * SOL_UNIT_SIZE
    );

    // Fill in the other direction puts back the ask.
    test_fixture
        .place_order_for_keypair(
            Side::Ask,
            2 * SOL_UNIT_SIZE,
            1,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;
    let resting_orders: Vec<RestingOrder> = test_fixture.market_fixture.get_resting_orders().await;
    assert_eq!(resting_orders.len(), 2);
    let first_bid: &RestingOrder = resting_orders.get(0).unwrap();
    // First bid is the flipped version of the first ask fully coalesced.
    // Book is bid: 4@1.5, 3@1
    assert_eq!(first_bid.get_is_bid(), true);
    assert_eq!(first_bid.get_num_base_atoms().as_u64(), 1_001_500_750);
    assert_eq!(
        first_bid.get_price(),
        QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(29_985, -4).unwrap()
    );
    let first_ask: &RestingOrder = resting_orders.get(1).unwrap();
    assert_eq!(first_ask.get_is_bid(), false);
    assert_eq!(first_ask.get_num_base_atoms().as_u64(), 2 * SOL_UNIT_SIZE);
    assert_eq!(
        first_ask.get_price(),
        QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(3, 0).unwrap()
    );

    // Maker's balances are unchanged.
    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_balance_atoms(&test_fixture.payer())
            .await,
        10_000_000_000
    );
    assert_eq!(
        test_fixture
            .market_fixture
            .get_base_balance_atoms(&test_fixture.payer())
            .await,
        7 * SOL_UNIT_SIZE
    );

    Ok(())
}
