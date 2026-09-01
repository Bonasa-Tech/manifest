//! The wrapper's view of its open orders: it is refreshed against the core
//! only when something could have changed it, cancels are matched during that
//! same walk, and the wrapper grows several slots at a time.

use std::{mem::size_of, rc::Rc};

use hypertree::{
    get_helper, DataIndex, HyperTreeReadOperations, HyperTreeValueIteratorTrait, RBNode, NIL,
};
use manifest::{
    program::{
        batch_update::CancelOrderParams,
        instruction_builders::{
            batch_update_instruction as manifest_batch_update_instruction,
            withdraw_instruction as manifest_withdraw_instruction,
        },
    },
    state::{constants::NO_EXPIRATION_LAST_VALID_SLOT, MarketFixed, OrderType},
};
use solana_account::Account;
use solana_keypair::Keypair;
use solana_program::{instruction::Instruction, pubkey::Pubkey};
use solana_program_test::tokio;
use wrapper::{
    instruction_builders::batch_update_instruction,
    market_info::MarketInfo,
    open_order::WrapperOpenOrder,
    processors::{
        batch_upate::{WrapperCancelOrderParams, WrapperPlaceOrderParams},
        shared::{MarketInfosTree, OpenOrdersTreeReadOnly, WRAPPER_BLOCK_SIZE},
    },
    wrapper_state::ManifestWrapperStateFixed,
};

use crate::{send_tx_with_retry, TestFixture, Token, SOL_UNIT_SIZE, USDC_UNIT_SIZE};

fn ask(client_order_id: u64, price_mantissa: u32) -> WrapperPlaceOrderParams {
    WrapperPlaceOrderParams::new(
        client_order_id,
        SOL_UNIT_SIZE,
        price_mantissa,
        -3,
        false,
        NO_EXPIRATION_LAST_VALID_SLOT,
        OrderType::Limit,
    )
}

async fn wrapper_account(test_fixture: &TestFixture) -> Account {
    test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(test_fixture.wrapper.key)
        .await
        .expect("Fetch wrapper")
        .expect("Wrapper is not none")
}

/// The wrapper's market info for the fixture market and its open orders as
/// (client order id, order sequence number), sorted by client order id.
async fn wrapper_view(test_fixture: &TestFixture) -> (MarketInfo, Vec<(u64, u64)>) {
    let mut account: Account = wrapper_account(test_fixture).await;
    let (fixed_data, wrapper_dynamic_data) =
        account.data[..].split_at_mut(size_of::<ManifestWrapperStateFixed>());
    let wrapper_fixed: &ManifestWrapperStateFixed = get_helper(fixed_data, 0);
    let market_infos_tree: MarketInfosTree = MarketInfosTree::new(
        wrapper_dynamic_data,
        wrapper_fixed.market_infos_root_index,
        NIL,
    );
    let market_info_index: DataIndex =
        market_infos_tree.lookup_index(&MarketInfo::new_empty(test_fixture.market.key, NIL));
    let market_info: MarketInfo =
        *get_helper::<RBNode<MarketInfo>>(wrapper_dynamic_data, market_info_index).get_value();
    let mut orders: Vec<(u64, u64)> = Vec::new();
    if market_info.orders_root_index != NIL {
        let tree: OpenOrdersTreeReadOnly =
            OpenOrdersTreeReadOnly::new(wrapper_dynamic_data, market_info.orders_root_index, NIL);
        for (_, order) in tree.iter::<WrapperOpenOrder>() {
            orders.push((
                order.get_client_order_id(),
                order.get_order_sequence_number(),
            ));
        }
    }
    orders.sort();
    (market_info, orders)
}

async fn market_sequence_number(test_fixture: &TestFixture) -> u64 {
    let account: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(test_fixture.market.key)
        .await
        .unwrap()
        .unwrap();
    get_helper::<MarketFixed>(&account.data, 0).get_order_sequence_number()
}

async fn wrapper_batch(
    test_fixture: &TestFixture,
    cancels: Vec<WrapperCancelOrderParams>,
    orders: Vec<WrapperPlaceOrderParams>,
) -> anyhow::Result<()> {
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
    let instruction: Instruction = batch_update_instruction(
        &test_fixture.market.key,
        &payer,
        &test_fixture.wrapper.key,
        cancels,
        false,
        orders,
    );
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[instruction],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;
    Ok(())
}

/// A cancel made directly on the core, bypassing the wrapper, must be picked
/// up by the next batch update: the stale entry is dropped and cancelling it
/// through the wrapper is a no-op rather than a failing index hint.
#[tokio::test]
async fn sync_after_direct_core_cancel_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 10 * SOL_UNIT_SIZE).await?;
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();

    // Two asks through the wrapper: sequence numbers 0 and 1.
    wrapper_batch(&test_fixture, vec![], vec![ask(1, 5), ask(2, 6)]).await?;
    let (market_info, orders) = wrapper_view(&test_fixture).await;
    assert_eq!(orders, vec![(1, 0), (2, 1)]);
    assert_eq!(
        market_info.last_synced_order_sequence_number,
        market_sequence_number(&test_fixture).await
    );

    // Cancel sequence number 0 on the core directly.
    let direct_cancel: Instruction = manifest_batch_update_instruction(
        &test_fixture.market.key,
        &payer,
        None,
        vec![CancelOrderParams::new(0)],
        vec![],
        None,
        None,
        None,
        None,
    );
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[direct_cancel],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // Cancel the stale order and the live one through the wrapper, and place
    // a new one. The freed balance makes the wrapper re-check its orders.
    wrapper_batch(
        &test_fixture,
        vec![
            WrapperCancelOrderParams::new(1),
            WrapperCancelOrderParams::new(2),
        ],
        vec![ask(3, 7)],
    )
    .await?;
    let (market_info, orders) = wrapper_view(&test_fixture).await;
    assert_eq!(orders, vec![(3, 2)]);
    assert_eq!(
        market_info.last_synced_order_sequence_number,
        market_sequence_number(&test_fixture).await
    );
    assert_eq!(market_info.num_open_global_orders, 0);
    Ok(())
}

/// Consecutive batch updates on a market nobody else touches keep the view
/// exact through the quiet-market shortcut.
#[tokio::test]
async fn sync_quiet_market_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 10 * SOL_UNIT_SIZE).await?;

    wrapper_batch(&test_fixture, vec![], vec![ask(1, 5), ask(2, 6), ask(3, 7)]).await?;
    // Cancel only: no placement, so the market sequence number is unchanged.
    wrapper_batch(
        &test_fixture,
        vec![WrapperCancelOrderParams::new(2)],
        vec![],
    )
    .await?;
    let (market_info, orders) = wrapper_view(&test_fixture).await;
    assert_eq!(orders, vec![(1, 0), (3, 2)]);
    assert_eq!(market_info.last_synced_order_sequence_number, 3);
    // Cancel and place, then cancel again, each starting from an exact view.
    wrapper_batch(
        &test_fixture,
        vec![WrapperCancelOrderParams::new(1)],
        vec![ask(4, 8), ask(5, 9)],
    )
    .await?;
    wrapper_batch(
        &test_fixture,
        vec![WrapperCancelOrderParams::new(4)],
        vec![],
    )
    .await?;
    let (market_info, orders) = wrapper_view(&test_fixture).await;
    assert_eq!(orders, vec![(3, 2), (5, 4)]);
    assert_eq!(market_info.last_synced_order_sequence_number, 5);
    assert_eq!(market_sequence_number(&test_fixture).await, 5);
    Ok(())
}

/// A batch that needs more slots than the wrapper has grows it once, by
/// whole groups of blocks.
#[tokio::test]
async fn expand_in_batches_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 20 * SOL_UNIT_SIZE).await?;
    let before: usize = wrapper_account(&test_fixture).await.data.len();

    let orders: Vec<WrapperPlaceOrderParams> = (1..=9u64).map(|i| ask(i, i as u32)).collect();
    wrapper_batch(&test_fixture, vec![], orders).await?;
    let (_, open_orders) = wrapper_view(&test_fixture).await;
    assert_eq!(open_orders.len(), 9);
    let after: usize = wrapper_account(&test_fixture).await.data.len();
    assert!(after > before);
    assert_eq!(
        (after - before) % (4 * WRAPPER_BLOCK_SIZE),
        0,
        "grows by groups of blocks"
    );
    Ok(())
}

/// The funds pre-check for bids still drops the bid that does not fit and
/// keeps the ones that do.
#[tokio::test]
async fn bid_funds_precheck_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture
        .deposit(Token::USDC, 10 * USDC_UNIT_SIZE)
        .await?;
    // Bids of 1 SOL at 4 USDC, 4 USDC and 4 USDC: the third exceeds the
    // 10 USDC balance and is dropped, the first two rest.
    let bid = |id: u64| {
        WrapperPlaceOrderParams::new(
            id,
            SOL_UNIT_SIZE,
            4,
            -3,
            true,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )
    };
    wrapper_batch(&test_fixture, vec![], vec![bid(1), bid(2), bid(3)]).await?;
    let (_, orders) = wrapper_view(&test_fixture).await;
    assert_eq!(orders, vec![(1, 0), (2, 1)]);
    Ok(())
}

/// Cancel and replace when every atom is already working. The replacement is
/// only affordable out of what the cancel frees, so the wrapper's record of
/// the order being cancelled has to carry its real size: a placeholder size
/// would credit nothing, and the replacement would be dropped for want of
/// funds while the transaction still succeeded.
#[tokio::test]
async fn cancel_and_replace_at_exact_balance_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    // Exactly one order's worth, so the replacement cannot be funded twice.
    test_fixture.deposit(Token::SOL, SOL_UNIT_SIZE).await?;

    wrapper_batch(&test_fixture, vec![], vec![ask(1, 5)]).await?;
    let (_, orders) = wrapper_view(&test_fixture).await;
    assert_eq!(orders, vec![(1, 0)], "the first ask rests");

    wrapper_batch(
        &test_fixture,
        vec![WrapperCancelOrderParams::new(1)],
        vec![ask(2, 6)],
    )
    .await?;
    let (_, orders) = wrapper_view(&test_fixture).await;
    assert_eq!(
        orders,
        vec![(2, 1)],
        "the replacement must rest on the funds the cancel freed",
    );
    Ok(())
}

/// Leaves the wrapper listing an order the core no longer has, without
/// tripping either half of the quiet check: the order is cancelled directly
/// on the core and exactly the refund is withdrawn, so neither the market's
/// order sequence number nor the seat's balances have moved.
async fn strand_a_stale_entry(test_fixture: &TestFixture) -> anyhow::Result<()> {
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[
            manifest_batch_update_instruction(
                &test_fixture.market.key,
                &payer,
                None,
                vec![CancelOrderParams::new(0)],
                vec![],
                None,
                None,
                None,
                None,
            ),
            manifest_withdraw_instruction(
                &test_fixture.market.key,
                &payer,
                &test_fixture.sol_mint.key,
                SOL_UNIT_SIZE,
                &test_fixture.payer_sol.key,
                spl_token::id(),
                None,
            ),
        ],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;
    Ok(())
}

/// Cancelling a stale entry must not fail the batch. The cancel carries the
/// entry's index as a hint, and the core validates every cancel before it
/// processes any placement, so a hint pointing at an order that is no longer
/// there would abort the whole instruction: the batch that was supposed to
/// clear the entry could never run.
#[tokio::test]
async fn cancelling_a_stale_entry_succeeds_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 10 * SOL_UNIT_SIZE).await?;
    wrapper_batch(&test_fixture, vec![], vec![ask(1, 5), ask(2, 6)]).await?;
    strand_a_stale_entry(&test_fixture).await?;

    // Cancel the entry the core no longer has, alongside a live one.
    wrapper_batch(
        &test_fixture,
        vec![
            WrapperCancelOrderParams::new(1),
            WrapperCancelOrderParams::new(2),
        ],
        vec![ask(3, 7)],
    )
    .await?;
    let (_, orders) = wrapper_view(&test_fixture).await;
    assert_eq!(
        orders,
        vec![(3, 2)],
        "the stale entry is gone and the new order rests"
    );
    Ok(())
}

/// The same, through cancel_all, which sweeps every entry it holds.
#[tokio::test]
async fn cancel_all_over_a_stale_entry_succeeds_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 10 * SOL_UNIT_SIZE).await?;
    wrapper_batch(&test_fixture, vec![], vec![ask(1, 5), ask(2, 6)]).await?;
    strand_a_stale_entry(&test_fixture).await?;

    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[batch_update_instruction(
            &test_fixture.market.key,
            &payer,
            &test_fixture.wrapper.key,
            vec![],
            true,
            vec![],
        )],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;
    let (_, orders) = wrapper_view(&test_fixture).await;
    assert_eq!(
        orders,
        vec![],
        "cancel_all clears both the live and the stale entry"
    );
    Ok(())
}

/// The stale entry that a cancel made directly on the core can leave behind,
/// when a withdrawal of exactly the freed amount hides it from the balance
/// check, is cleared by the next batch update that places an order.
#[tokio::test]
async fn stale_entry_cleared_by_the_next_placement_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 10 * SOL_UNIT_SIZE).await?;
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();

    wrapper_batch(&test_fixture, vec![], vec![ask(1, 5), ask(2, 6)]).await?;

    // Cancel one on the core, then withdraw exactly what it freed, so neither
    // the market's order sequence number nor the seat's balances have moved
    // and the opening sync sees a quiet market.
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[
            manifest_batch_update_instruction(
                &test_fixture.market.key,
                &payer,
                None,
                vec![CancelOrderParams::new(0)],
                vec![],
                None,
                None,
                None,
                None,
            ),
            manifest_withdraw_instruction(
                &test_fixture.market.key,
                &payer,
                &test_fixture.sol_mint.key,
                SOL_UNIT_SIZE,
                &test_fixture.payer_sol.key,
                spl_token::id(),
                None,
            ),
        ],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;
    let (_, orders) = wrapper_view(&test_fixture).await;
    assert_eq!(
        orders,
        vec![(1, 0), (2, 1)],
        "the wrapper still lists the order that was cancelled on the core",
    );

    // Placing runs the core's matching loop, so the sync after it re-reads.
    wrapper_batch(&test_fixture, vec![], vec![ask(3, 7)]).await?;
    let (_, orders) = wrapper_view(&test_fixture).await;
    assert_eq!(
        orders,
        vec![(2, 1), (3, 2)],
        "the stale entry is gone once an order has been placed",
    );
    Ok(())
}
