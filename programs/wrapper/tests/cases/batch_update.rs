use std::{mem::size_of, rc::Rc};

use hypertree::{
    get_helper, DataIndex, HyperTreeReadOperations, HyperTreeValueIteratorTrait, RBNode, NIL,
};
use manifest::{
    program::{
        batch_update::PlaceOrderParams as ManifestPlaceOrderParams,
        instruction_builders::batch_update_instruction as manifest_batch_update_instruction,
    },
    state::{constants::NO_EXPIRATION_LAST_VALID_SLOT, OrderType, RestingOrder},
};
use solana_account::Account;
use solana_keypair::Keypair;
use solana_program::{instruction::Instruction, pubkey::Pubkey};
use solana_program_test::tokio;
use solana_signer::Signer;
use wrapper::{
    instruction_builders::{batch_update_instruction, create_wrapper_instructions},
    market_info::MarketInfo,
    processors::{
        batch_upate::{WrapperCancelOrderParams, WrapperPlaceOrderParams},
        shared::MarketInfosTree,
    },
    wrapper_state::ManifestWrapperStateFixed,
};

use crate::{send_tx_with_retry, TestFixture, Token, SOL_UNIT_SIZE, USDC_UNIT_SIZE};

#[tokio::test]
async fn wrapper_batch_update_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, SOL_UNIT_SIZE).await?;

    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();

    // There is no order 0 for the cancel to get, but it will fail silently and continue on.
    let batch_update_ix: Instruction = batch_update_instruction(
        &test_fixture.market.key,
        &payer,
        &test_fixture.wrapper.key,
        vec![WrapperCancelOrderParams::new(0)],
        false,
        vec![WrapperPlaceOrderParams::new(
            0,
            1 * SOL_UNIT_SIZE,
            1,
            0,
            false,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )],
    );
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[batch_update_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // Cancel and place, so we have enough funds for the second one.
    let batch_update_ix: Instruction = batch_update_instruction(
        &test_fixture.market.key,
        &payer,
        &test_fixture.wrapper.key,
        vec![WrapperCancelOrderParams::new(0)],
        false,
        vec![WrapperPlaceOrderParams::new(
            0,
            1 * SOL_UNIT_SIZE,
            1,
            0,
            false,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )],
    );
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[batch_update_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    Ok(())
}

#[tokio::test]
async fn wrapper_batch_update_reuse_client_order_id_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 10 * SOL_UNIT_SIZE).await?;

    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();

    // All the orders have the same client order id.
    let batch_update_ix: Instruction = batch_update_instruction(
        &test_fixture.market.key,
        &payer,
        &test_fixture.wrapper.key,
        vec![],
        false,
        vec![
            WrapperPlaceOrderParams::new(
                0,
                1 * SOL_UNIT_SIZE,
                1,
                0,
                true,
                NO_EXPIRATION_LAST_VALID_SLOT,
                OrderType::Limit,
            ),
            WrapperPlaceOrderParams::new(
                0,
                1 * SOL_UNIT_SIZE,
                2,
                0,
                true,
                NO_EXPIRATION_LAST_VALID_SLOT,
                OrderType::Limit,
            ),
            WrapperPlaceOrderParams::new(
                0,
                1 * SOL_UNIT_SIZE,
                3,
                0,
                false,
                NO_EXPIRATION_LAST_VALID_SLOT,
                OrderType::Limit,
            ),
            WrapperPlaceOrderParams::new(
                0,
                1 * SOL_UNIT_SIZE,
                4,
                0,
                false,
                NO_EXPIRATION_LAST_VALID_SLOT,
                OrderType::Limit,
            ),
        ],
    );
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[batch_update_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // Cancel order 0 which is all of them
    let batch_update_ix: Instruction = batch_update_instruction(
        &test_fixture.market.key,
        &payer,
        &test_fixture.wrapper.key,
        vec![WrapperCancelOrderParams::new(0)],
        false,
        vec![],
    );
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[batch_update_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // Assert that there are no more orders on the book.
    let mut wrapper_account: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(test_fixture.wrapper.key)
        .await
        .expect("Fetch wrapper")
        .expect("Wrapper is not none");
    let (fixed_data, wrapper_dynamic_data) =
        wrapper_account.data[..].split_at_mut(size_of::<ManifestWrapperStateFixed>());

    let wrapper_fixed: &ManifestWrapperStateFixed = get_helper(fixed_data, 0);
    let market_infos_tree: MarketInfosTree = MarketInfosTree::new(
        wrapper_dynamic_data,
        wrapper_fixed.market_infos_root_index,
        NIL,
    );

    let market_info_index: DataIndex =
        market_infos_tree.lookup_index(&MarketInfo::new_empty(test_fixture.market.key, NIL));
    let market_info: &MarketInfo =
        get_helper::<RBNode<MarketInfo>>(wrapper_dynamic_data, market_info_index).get_value();
    let orders_root_index: DataIndex = market_info.orders_root_index;
    assert_eq!(
        orders_root_index, NIL,
        "Deleted all orders since they all had same client order id"
    );

    Ok(())
}

#[tokio::test]
async fn sync_remove_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 10 * SOL_UNIT_SIZE).await?;

    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
    let second_payer: Pubkey = test_fixture.second_keypair.pubkey();
    let second_payer_keypair: Keypair = test_fixture.second_keypair.insecure_clone();
    let second_wrapper_keypair: Keypair = Keypair::new();

    let create_wrapper_ixs: Vec<Instruction> =
        create_wrapper_instructions(&second_payer, &second_wrapper_keypair.pubkey()).unwrap();

    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &create_wrapper_ixs[..],
        Some(&second_payer),
        &[&second_payer_keypair, &second_wrapper_keypair],
    )
    .await?;

    test_fixture
        .claim_seat_for_keypair_with_wrapper(
            &test_fixture.second_keypair.insecure_clone(),
            &second_wrapper_keypair.pubkey(),
        )
        .await?;
    test_fixture
        .deposit_for_keypair_with_wrapper(
            Token::USDC,
            1_000 * USDC_UNIT_SIZE,
            &test_fixture.second_keypair.insecure_clone(),
            &second_wrapper_keypair.pubkey(),
        )
        .await?;

    let batch_update_ix: Instruction = batch_update_instruction(
        &test_fixture.market.key,
        &payer,
        &test_fixture.wrapper.key,
        vec![],
        false,
        vec![WrapperPlaceOrderParams::new(
            0,
            1 * SOL_UNIT_SIZE,
            1,
            0,
            false,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )],
    );
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[batch_update_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    let batch_update_ix: Instruction = batch_update_instruction(
        &test_fixture.market.key,
        &second_payer,
        &second_wrapper_keypair.pubkey(),
        vec![],
        false,
        vec![WrapperPlaceOrderParams::new(
            0,
            1 * SOL_UNIT_SIZE,
            1,
            0,
            true,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )],
    );
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[batch_update_ix],
        Some(&second_payer),
        &[&second_payer_keypair],
    )
    .await?;

    let batch_update_ix: Instruction = batch_update_instruction(
        &test_fixture.market.key,
        &payer,
        &test_fixture.wrapper.key,
        vec![],
        false,
        vec![],
    );
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[batch_update_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // Assert that there are no more orders on the book.
    let mut wrapper_account: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(test_fixture.wrapper.key)
        .await
        .expect("Fetch wrapper")
        .expect("Wrapper is not none");
    let (fixed_data, wrapper_dynamic_data) =
        wrapper_account.data[..].split_at_mut(size_of::<ManifestWrapperStateFixed>());

    let wrapper_fixed: &ManifestWrapperStateFixed = get_helper(fixed_data, 0);
    let market_infos_tree: MarketInfosTree = MarketInfosTree::new(
        wrapper_dynamic_data,
        wrapper_fixed.market_infos_root_index,
        NIL,
    );

    // Just need to lookup by market key so the rest doesnt matter.
    let market_info_index: DataIndex =
        market_infos_tree.lookup_index(&MarketInfo::new_empty(test_fixture.market.key, NIL));

    let market_info: &MarketInfo =
        get_helper::<RBNode<MarketInfo>>(wrapper_dynamic_data, market_info_index).get_value();
    let orders_root_index: DataIndex = market_info.orders_root_index;
    assert_eq!(orders_root_index, NIL, "Order matched");

    Ok(())
}

#[tokio::test]
async fn wrapper_batch_update_cancel_all_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 20 * SOL_UNIT_SIZE).await?;

    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();

    // Place an order via the wrapper.
    let batch_update_ix: Instruction = batch_update_instruction(
        &test_fixture.market.key,
        &payer,
        &test_fixture.wrapper.key,
        vec![],
        false,
        vec![WrapperPlaceOrderParams::new(
            0,
            1 * SOL_UNIT_SIZE,
            1,
            0,
            false,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )],
    );
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[batch_update_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // Add enough wrapper orders to exceed the per-call owned-cancellation cap.
    for client_order_id in 1..=16 {
        let batch_update_ix = batch_update_instruction(
            &test_fixture.market.key,
            &payer,
            &test_fixture.wrapper.key,
            vec![],
            false,
            vec![WrapperPlaceOrderParams::new(
                client_order_id,
                SOL_UNIT_SIZE,
                client_order_id as u32 + 1,
                0,
                false,
                NO_EXPIRATION_LAST_VALID_SLOT,
                OrderType::Limit,
            )],
        );
        send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[batch_update_ix],
            Some(&payer),
            &[&payer_keypair],
        )
        .await?;
    }

    // Place an order directly via the manifest program (bypassing the wrapper).
    let manifest_place_ix: Instruction = manifest_batch_update_instruction(
        &test_fixture.market.key,
        &payer,
        None,
        vec![],
        vec![ManifestPlaceOrderParams::new(
            1 * SOL_UNIT_SIZE,
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
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[manifest_place_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // Verify there are 18 asks on the market before cancel_all.
    test_fixture.market.reload().await;
    assert_eq!(
        test_fixture
            .market
            .market
            .get_asks()
            .iter::<RestingOrder>()
            .count(),
        18,
        "Wrapper and direct asks before cancel_all"
    );

    // cancel_all is bounded to 16 owned cancellations per transaction.
    let batch_update_ix: Instruction = batch_update_instruction(
        &test_fixture.market.key,
        &payer,
        &test_fixture.wrapper.key,
        vec![],
        true,
        vec![],
    );
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[batch_update_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    test_fixture.market.reload().await;
    assert_eq!(
        test_fixture
            .market
            .market
            .get_asks()
            .iter::<RestingOrder>()
            .count(),
        2,
        "First bounded cancel_all leaves work for a retry",
    );

    // The wrapper sync consumed all 16 core cancellation slots, so no physical
    // market block was scanned. Zero remains the start cursor and must not be
    // reported as completion.
    let mut wrapper_account_after_first_pass: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(test_fixture.wrapper.key)
        .await
        .expect("Fetch wrapper")
        .expect("Wrapper is not none");
    let (fixed_after_first_pass, dynamic_after_first_pass) = wrapper_account_after_first_pass
        .data
        .split_at_mut(size_of::<ManifestWrapperStateFixed>());
    let wrapper_fixed_after_first_pass: &ManifestWrapperStateFixed =
        get_helper(fixed_after_first_pass, 0);
    let market_infos_after_first_pass: MarketInfosTree = MarketInfosTree::new(
        dynamic_after_first_pass,
        wrapper_fixed_after_first_pass.market_infos_root_index,
        NIL,
    );
    let market_info_index_after_first_pass: DataIndex = market_infos_after_first_pass
        .lookup_index(&MarketInfo::new_empty(test_fixture.market.key, NIL));
    let market_info_after_first_pass: &MarketInfo = get_helper::<RBNode<MarketInfo>>(
        dynamic_after_first_pass,
        market_info_index_after_first_pass,
    )
    .get_value();
    assert_eq!(market_info_after_first_pass.last_updated_slot, 0);

    let batch_update_ix: Instruction = batch_update_instruction(
        &test_fixture.market.key,
        &payer,
        &test_fixture.wrapper.key,
        vec![],
        true,
        vec![],
    );
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[batch_update_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // Assert that there are no more orders on the wrapper.
    let mut wrapper_account: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(test_fixture.wrapper.key)
        .await
        .expect("Fetch wrapper")
        .expect("Wrapper is not none");
    let (fixed_data, wrapper_dynamic_data) =
        wrapper_account.data[..].split_at_mut(size_of::<ManifestWrapperStateFixed>());

    let wrapper_fixed: &ManifestWrapperStateFixed = get_helper(fixed_data, 0);
    let market_infos_tree: MarketInfosTree = MarketInfosTree::new(
        wrapper_dynamic_data,
        wrapper_fixed.market_infos_root_index,
        NIL,
    );

    let market_info_index: DataIndex =
        market_infos_tree.lookup_index(&MarketInfo::new_empty(test_fixture.market.key, NIL));

    let market_info: &MarketInfo =
        get_helper::<RBNode<MarketInfo>>(wrapper_dynamic_data, market_info_index).get_value();
    let orders_root_index: DataIndex = market_info.orders_root_index;
    assert_eq!(orders_root_index, NIL, "Deleted all orders in cancel all");
    assert_eq!(
        market_info.last_updated_slot, NIL,
        "NIL exclusively marks a completed physical scan",
    );

    // Assert that the market order book is empty (both wrapper and non-wrapper orders cancelled).
    test_fixture.market.reload().await;
    assert_eq!(
        test_fixture
            .market
            .market
            .get_asks()
            .iter::<RestingOrder>()
            .count(),
        0,
        "No asks remaining on market"
    );
    assert_eq!(
        test_fixture
            .market
            .market
            .get_bids()
            .iter::<RestingOrder>()
            .count(),
        0,
        "No bids remaining on market"
    );

    Ok(())
}

#[tokio::test]
async fn wrapper_cancel_all_scans_past_unrelated_trader_orders() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, SOL_UNIT_SIZE).await?;

    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
    let unrelated_trader: Keypair = test_fixture.second_keypair.insecure_clone();
    let unrelated_wrapper: Keypair = Keypair::new();

    let create_unrelated_wrapper_ixs: Vec<Instruction> =
        create_wrapper_instructions(&unrelated_trader.pubkey(), &unrelated_wrapper.pubkey())?;
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &create_unrelated_wrapper_ixs,
        Some(&unrelated_trader.pubkey()),
        &[&unrelated_trader, &unrelated_wrapper],
    )
    .await?;
    test_fixture
        .claim_seat_for_keypair_with_wrapper(&unrelated_trader, &unrelated_wrapper.pubkey())
        .await?;
    test_fixture
        .deposit_for_keypair_with_wrapper(
            Token::SOL,
            40 * SOL_UNIT_SIZE,
            &unrelated_trader,
            &unrelated_wrapper.pubkey(),
        )
        .await?;

    // Allocate enough unrelated resting orders to verify that cancellation is
    // not bounded by the 16-entry core cancel batch size.
    for batch_start in (0_u64..40).step_by(8) {
        let orders: Vec<WrapperPlaceOrderParams> = (batch_start..batch_start + 8)
            .map(|client_order_id: u64| {
                WrapperPlaceOrderParams::new(
                    client_order_id,
                    SOL_UNIT_SIZE,
                    client_order_id as u32 + 10,
                    0,
                    false,
                    NO_EXPIRATION_LAST_VALID_SLOT,
                    OrderType::Limit,
                )
            })
            .collect();
        let place_unrelated_ix: Instruction = batch_update_instruction(
            &test_fixture.market.key,
            &unrelated_trader.pubkey(),
            &unrelated_wrapper.pubkey(),
            vec![],
            false,
            orders,
        );
        send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[place_unrelated_ix],
            Some(&unrelated_trader.pubkey()),
            &[&unrelated_trader],
        )
        .await?;
    }

    // Put the victim's untracked direct-core order after those physical blocks.
    let victim_direct_order_ix: Instruction = manifest_batch_update_instruction(
        &test_fixture.market.key,
        &payer,
        None,
        vec![],
        vec![ManifestPlaceOrderParams::new(
            SOL_UNIT_SIZE,
            1,
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
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[victim_direct_order_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    let cancel_all_ix: Instruction = batch_update_instruction(
        &test_fixture.market.key,
        &payer,
        &test_fixture.wrapper.key,
        vec![],
        true,
        vec![],
    );
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[cancel_all_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;
    test_fixture.market.reload().await;
    assert_eq!(
        test_fixture
            .market
            .market
            .get_asks()
            .iter::<RestingOrder>()
            .count(),
        40,
        "The bounded physical scan reaches the victim past another trader's orders",
    );

    Ok(())
}

#[tokio::test]
async fn wrapper_ignore_post_only() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 2 * SOL_UNIT_SIZE).await?;
    test_fixture
        .deposit(Token::USDC, 2 * USDC_UNIT_SIZE)
        .await?;

    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();

    let batch_update_ix: Instruction = batch_update_instruction(
        &test_fixture.market.key,
        &payer,
        &test_fixture.wrapper.key,
        vec![],
        false,
        vec![WrapperPlaceOrderParams::new(
            0,
            1 * SOL_UNIT_SIZE,
            1,
            0,
            false,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )],
    );
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[batch_update_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // This post only will cross, so it should not get placed.
    let batch_update_ix: Instruction = batch_update_instruction(
        &test_fixture.market.key,
        &payer,
        &test_fixture.wrapper.key,
        vec![],
        false,
        vec![WrapperPlaceOrderParams::new(
            0,
            1 * SOL_UNIT_SIZE,
            2,
            0,
            true,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::PostOnly,
        )],
    );
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[batch_update_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    test_fixture.market.reload().await;
    // There is just one ask and did not match due to post only.
    assert_eq!(
        test_fixture
            .market
            .market
            .get_asks()
            .iter::<RestingOrder>()
            .count(),
        1
    );

    Ok(())
}

#[tokio::test]
async fn wrapper_forwards_post_only_after_bounded_expired_scan() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture
        .deposit(Token::USDC, 200_000 * USDC_UNIT_SIZE)
        .await?;

    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
    let expired_order_trader: Keypair = test_fixture.second_keypair.insecure_clone();
    let expired_order_wrapper: Keypair = Keypair::new();

    let create_expired_order_wrapper_ixs: Vec<Instruction> = create_wrapper_instructions(
        &expired_order_trader.pubkey(),
        &expired_order_wrapper.pubkey(),
    )?;
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &create_expired_order_wrapper_ixs,
        Some(&expired_order_trader.pubkey()),
        &[&expired_order_trader, &expired_order_wrapper],
    )
    .await?;
    test_fixture
        .claim_seat_for_keypair_with_wrapper(&expired_order_trader, &expired_order_wrapper.pubkey())
        .await?;
    test_fixture
        .deposit_for_keypair_with_wrapper(
            Token::SOL,
            40 * SOL_UNIT_SIZE,
            &expired_order_trader,
            &expired_order_wrapper.pubkey(),
        )
        .await?;

    // More expired asks than the wrapper's advisory price-discovery quota.
    for batch_start in (0_u64..40).step_by(8) {
        let orders: Vec<WrapperPlaceOrderParams> = (batch_start..batch_start + 8)
            .map(|client_order_id: u64| {
                WrapperPlaceOrderParams::new(
                    client_order_id,
                    SOL_UNIT_SIZE,
                    client_order_id as u32 + 10,
                    0,
                    false,
                    1_000,
                    OrderType::Limit,
                )
            })
            .collect();
        let place_expiring_asks_ix: Instruction = batch_update_instruction(
            &test_fixture.market.key,
            &expired_order_trader.pubkey(),
            &expired_order_wrapper.pubkey(),
            vec![],
            false,
            orders,
        );
        send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[place_expiring_asks_ix],
            Some(&expired_order_trader.pubkey()),
            &[&expired_order_trader],
        )
        .await?;
    }
    test_fixture.advance_time_seconds(10_000).await;

    // The wrapper cannot establish the opposite price within 32 steps. It
    // must forward this order so the core can prune the expired prefix and
    // make the authoritative PostOnly decision.
    let post_only_bid_ix: Instruction = batch_update_instruction(
        &test_fixture.market.key,
        &payer,
        &test_fixture.wrapper.key,
        vec![],
        false,
        vec![WrapperPlaceOrderParams::new(
            1,
            SOL_UNIT_SIZE,
            100,
            0,
            true,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::PostOnly,
        )],
    );
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[post_only_bid_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    test_fixture.market.reload().await;
    assert_eq!(
        test_fixture
            .market
            .market
            .get_bids()
            .iter::<RestingOrder>()
            .count(),
        1,
        "PostOnly order is forwarded instead of silently dropped",
    );
    assert_eq!(
        test_fixture
            .market
            .market
            .get_asks()
            .iter::<RestingOrder>()
            .count(),
        0,
        "The core prunes the expired prefix",
    );

    Ok(())
}

#[tokio::test]
async fn wrapper_preserves_cancels_when_post_only_price_is_unknown() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture
        .deposit(Token::USDC, 200_000 * USDC_UNIT_SIZE)
        .await?;

    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();

    // Give the mixed batch a real wrapper-tracked cancellation to preserve.
    let resting_bid_ix: Instruction = batch_update_instruction(
        &test_fixture.market.key,
        &payer,
        &test_fixture.wrapper.key,
        vec![],
        false,
        vec![WrapperPlaceOrderParams::new(
            7,
            SOL_UNIT_SIZE,
            1,
            0,
            true,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )],
    );
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[resting_bid_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    let maker: Keypair = test_fixture.second_keypair.insecure_clone();
    let maker_wrapper: Keypair = Keypair::new();
    let create_maker_wrapper_ixs: Vec<Instruction> =
        create_wrapper_instructions(&maker.pubkey(), &maker_wrapper.pubkey())?;
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &create_maker_wrapper_ixs,
        Some(&maker.pubkey()),
        &[&maker, &maker_wrapper],
    )
    .await?;
    test_fixture
        .claim_seat_for_keypair_with_wrapper(&maker, &maker_wrapper.pubkey())
        .await?;
    test_fixture
        .deposit_for_keypair_with_wrapper(
            Token::SOL,
            41 * SOL_UNIT_SIZE,
            &maker,
            &maker_wrapper.pubkey(),
        )
        .await?;

    // Put forty soon-expired asks ahead of a live crossing ask. The wrapper's
    // 32-step advisory scan cannot see the live maker, but the core would find
    // it after pruning and reject the PostOnly bid, rolling back the cancel.
    for batch_start in (0_u64..40).step_by(8) {
        let orders: Vec<WrapperPlaceOrderParams> = (batch_start..batch_start + 8)
            .map(|client_order_id: u64| {
                WrapperPlaceOrderParams::new(
                    client_order_id,
                    SOL_UNIT_SIZE,
                    client_order_id as u32 + 10,
                    0,
                    false,
                    1_000,
                    OrderType::Limit,
                )
            })
            .collect();
        let place_expiring_asks_ix: Instruction = batch_update_instruction(
            &test_fixture.market.key,
            &maker.pubkey(),
            &maker_wrapper.pubkey(),
            vec![],
            false,
            orders,
        );
        send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[place_expiring_asks_ix],
            Some(&maker.pubkey()),
            &[&maker],
        )
        .await?;
    }
    let place_live_ask_ix: Instruction = batch_update_instruction(
        &test_fixture.market.key,
        &maker.pubkey(),
        &maker_wrapper.pubkey(),
        vec![],
        false,
        vec![WrapperPlaceOrderParams::new(
            100,
            SOL_UNIT_SIZE,
            60,
            0,
            false,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )],
    );
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[place_live_ask_ix],
        Some(&maker.pubkey()),
        &[&maker],
    )
    .await?;
    test_fixture.advance_time_seconds(10_000).await;

    let mixed_batch_ix: Instruction = batch_update_instruction(
        &test_fixture.market.key,
        &payer,
        &test_fixture.wrapper.key,
        vec![WrapperCancelOrderParams::new(7)],
        false,
        vec![
            WrapperPlaceOrderParams::new(
                8,
                SOL_UNIT_SIZE,
                100,
                0,
                true,
                NO_EXPIRATION_LAST_VALID_SLOT,
                OrderType::PostOnly,
            ),
            WrapperPlaceOrderParams::new(
                9,
                SOL_UNIT_SIZE,
                2,
                0,
                true,
                NO_EXPIRATION_LAST_VALID_SLOT,
                OrderType::PostOnly,
            ),
        ],
    );
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[mixed_batch_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    test_fixture.market.reload().await;
    assert_eq!(
        test_fixture
            .market
            .market
            .get_bids()
            .iter::<RestingOrder>()
            .count(),
        1,
        "The cancel lands and the provably non-crossing replacement rests",
    );
    assert_eq!(
        test_fixture
            .market
            .market
            .get_asks()
            .iter::<RestingOrder>()
            .count(),
        1,
        "The unsafe PostOnly is suppressed while the safe one prunes expired asks",
    );

    Ok(())
}

#[tokio::test]
async fn wrapper_batch_update_slots_from_now() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, SOL_UNIT_SIZE).await?;

    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();

    // There is no order 0 for the cancel to get, but it will fail silently and continue on.
    let batch_update_ix: Instruction = batch_update_instruction(
        &test_fixture.market.key,
        &payer,
        &test_fixture.wrapper.key,
        vec![WrapperCancelOrderParams::new(0)],
        false,
        vec![WrapperPlaceOrderParams::new(
            0,
            1 * SOL_UNIT_SIZE,
            1,
            0,
            false,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )],
    );
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[batch_update_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // Cancel and place, so we have enough funds for the second one.
    let batch_update_ix: Instruction = batch_update_instruction(
        &test_fixture.market.key,
        &payer,
        &test_fixture.wrapper.key,
        vec![WrapperCancelOrderParams::new(0)],
        false,
        vec![WrapperPlaceOrderParams::new(
            0,
            1 * SOL_UNIT_SIZE,
            1,
            0,
            false,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )],
    );
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[batch_update_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    Ok(())
}
