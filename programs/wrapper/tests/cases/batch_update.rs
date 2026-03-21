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
use solana_program::instruction::Instruction;
use solana_program_test::tokio;
use solana_sdk::{account::Account, pubkey::Pubkey, signature::Keypair, signer::Signer};
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
    test_fixture.deposit(Token::SOL, 2 * SOL_UNIT_SIZE).await?;

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

    // Verify there are 2 asks on the market before cancel_all.
    test_fixture.market.reload().await;
    assert_eq!(
        test_fixture
            .market
            .market
            .get_asks()
            .iter::<RestingOrder>()
            .count(),
        2,
        "Two asks before cancel_all"
    );

    // cancel_all via wrapper should cancel both orders.
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
