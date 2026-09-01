//! The wrapper's view of its open orders: it is refreshed against the core
//! only when something could have changed it, cancels are matched during that
//! same walk, the wrapper grows several slots at a time, and the orders are
//! kept in a linked list, with wrappers from before the list converted in
//! place on first use.

use std::{mem::size_of, rc::Rc};

use hypertree::{
    get_helper, get_mut_helper, validate_linked_list, DataIndex, HyperTreeReadOperations,
    HyperTreeValueIteratorTrait, HyperTreeWriteOperations, RBNode, RedBlackTree, NIL,
};
use manifest::{
    program::{
        batch_update::CancelOrderParams,
        instruction_builders::{
            batch_update_instruction as manifest_batch_update_instruction,
            withdraw_instruction as manifest_withdraw_instruction,
        },
    },
    quantities::{BaseAtoms, QuoteAtomsPerBaseAtom, WrapperU64},
    state::{constants::NO_EXPIRATION_LAST_VALID_SLOT, MarketFixed, OrderType},
};
use solana_account::{Account, AccountSharedData};
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_keypair::Keypair;
use solana_program::{instruction::Instruction, pubkey::Pubkey, rent::Rent};
use solana_program_test::{tokio, ProgramTestContext};
use solana_transaction::Transaction;
use wrapper::{
    instruction_builders::batch_update_instruction,
    market_info::MarketInfo,
    open_order::WrapperOpenOrder,
    processors::{
        batch_upate::{WrapperCancelOrderParams, WrapperPlaceOrderParams},
        shared::{
            MarketInfosTree, OpenOrdersListReadOnly, ORDERS_LAYOUT_LIST, ORDERS_LAYOUT_TREE,
            WRAPPER_BLOCK_SIZE,
        },
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

fn market_info_index(wrapper_dynamic_data: &mut [u8], market: &Pubkey) -> DataIndex {
    let wrapper_fixed: &ManifestWrapperStateFixed = get_helper(
        &wrapper_dynamic_data[..size_of::<ManifestWrapperStateFixed>()],
        0,
    );
    let market_infos_root_index: DataIndex = wrapper_fixed.market_infos_root_index;
    let (_, dynamic_data) =
        wrapper_dynamic_data.split_at_mut(size_of::<ManifestWrapperStateFixed>());
    let market_infos_tree: MarketInfosTree =
        MarketInfosTree::new(dynamic_data, market_infos_root_index, NIL);
    market_infos_tree.lookup_index(&MarketInfo::new_empty(*market, NIL))
}

/// The wrapper's market info for the fixture market and its open orders as
/// (client order id, order sequence number), sorted by client order id.
/// Checks that the orders are a well formed list.
async fn wrapper_view(test_fixture: &TestFixture) -> (MarketInfo, Vec<(u64, u64)>) {
    let mut account: Account = wrapper_account(test_fixture).await;
    let market_info_index: DataIndex =
        market_info_index(&mut account.data, &test_fixture.market.key);
    let (_fixed_data, wrapper_dynamic_data) =
        account.data[..].split_at(size_of::<ManifestWrapperStateFixed>());
    let market_info_node: &RBNode<MarketInfo> =
        get_helper::<RBNode<MarketInfo>>(wrapper_dynamic_data, market_info_index);
    assert_eq!(
        market_info_node.get_payload_type(),
        ORDERS_LAYOUT_LIST,
        "open orders are kept as a list"
    );
    let market_info: MarketInfo = *market_info_node.get_value();
    validate_linked_list::<WrapperOpenOrder>(wrapper_dynamic_data, market_info.orders_root_index)
        .expect("well formed open orders list");
    let mut orders: Vec<(u64, u64)> = Vec::new();
    if market_info.orders_root_index != NIL {
        let list: OpenOrdersListReadOnly =
            OpenOrdersListReadOnly::new(wrapper_dynamic_data, market_info.orders_root_index);
        for (_, order) in list.iter::<WrapperOpenOrder>() {
            orders.push((
                order.get_client_order_id(),
                order.get_order_sequence_number(),
            ));
        }
    }
    orders.sort();
    (market_info, orders)
}

/// Rewrites the wrapper account so the fixture market's open orders are laid
/// out as a red-black tree under the old layout flag, the way wrappers from
/// before the list look. The nodes stay in their blocks, only the headers and
/// the flag change.
async fn rewrite_orders_as_tree(test_fixture: &TestFixture) {
    let mut account: Account = wrapper_account(test_fixture).await;
    let market_info_index: DataIndex =
        market_info_index(&mut account.data, &test_fixture.market.key);
    {
        let (_fixed_data, wrapper_dynamic_data) =
            account.data[..].split_at_mut(size_of::<ManifestWrapperStateFixed>());
        let head_index: DataIndex =
            get_helper::<RBNode<MarketInfo>>(wrapper_dynamic_data, market_info_index)
                .get_value()
                .orders_root_index;
        let orders: Vec<(DataIndex, WrapperOpenOrder)> =
            OpenOrdersListReadOnly::new(wrapper_dynamic_data, head_index)
                .iter::<WrapperOpenOrder>()
                .map(|(index, order)| (index, *order))
                .collect();
        let mut tree: RedBlackTree<WrapperOpenOrder> =
            RedBlackTree::new(wrapper_dynamic_data, NIL, NIL);
        for (index, order) in orders {
            tree.insert(index, order);
        }
        let root_index: DataIndex = tree.get_root_index();
        let market_info_node: &mut RBNode<MarketInfo> =
            get_mut_helper::<RBNode<MarketInfo>>(wrapper_dynamic_data, market_info_index);
        market_info_node.get_mut_value().orders_root_index = root_index;
        market_info_node.set_payload_type(ORDERS_LAYOUT_TREE);
    }
    test_fixture
        .context
        .borrow_mut()
        .set_account(&test_fixture.wrapper.key, &AccountSharedData::from(account));
}

async fn orders_layout(test_fixture: &TestFixture) -> u8 {
    let mut account: Account = wrapper_account(test_fixture).await;
    let market_info_index: DataIndex =
        market_info_index(&mut account.data, &test_fixture.market.key);
    let (_fixed_data, wrapper_dynamic_data) =
        account.data[..].split_at(size_of::<ManifestWrapperStateFixed>());
    get_helper::<RBNode<MarketInfo>>(wrapper_dynamic_data, market_info_index).get_payload_type()
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

/// Runs `instructions` and returns the compute units consumed, requiring that
/// the transaction succeeded.
async fn units_consumed(
    test_fixture: &TestFixture,
    instructions: &[Instruction],
    payer: &Pubkey,
    signers: &[&Keypair],
) -> u64 {
    let mut context: std::cell::RefMut<ProgramTestContext> = test_fixture.context.borrow_mut();
    let blockhash: solana_program::hash::Hash = context.get_new_latest_blockhash().await.unwrap();
    let transaction: Transaction =
        Transaction::new_signed_with_payer(instructions, Some(payer), signers, blockhash);
    let metadata = context
        .banks_client
        .process_transaction_with_metadata(transaction)
        .await
        .unwrap();
    if let Err(error) = metadata.result {
        panic!("transaction failed: {error:?}");
    }
    metadata
        .metadata
        .expect("transaction metadata present")
        .compute_units_consumed
}

/// Writes `num_orders` open orders for the fixture market into the wrapper
/// account directly, laid out as a red-black tree under the old flag. Placing
/// them through the program would cost a transaction per handful and cap out
/// long before the sizes worth measuring, and what is being measured here is
/// the conversion, which only reads the wrapper.
///
/// The orders all point at core order index 0, which never gets read: the
/// market is left quiet, so `sync_fast` converts and then skips the walk
/// against the core.
async fn fabricate_legacy_tree(test_fixture: &TestFixture, num_orders: usize) {
    let mut account: Account = wrapper_account(test_fixture).await;
    let market_info_index: DataIndex =
        market_info_index(&mut account.data, &test_fixture.market.key);
    let fixed_size: usize = size_of::<ManifestWrapperStateFixed>();
    let bytes_allocated: usize = {
        let wrapper_fixed: &ManifestWrapperStateFixed = get_helper(&account.data[..fixed_size], 0);
        wrapper_fixed.num_bytes_allocated as usize
    };
    let added_bytes: usize = num_orders * WRAPPER_BLOCK_SIZE;
    account
        .data
        .resize(fixed_size + bytes_allocated + added_bytes, 0);
    account.lamports = account
        .lamports
        .max(Rent::default().minimum_balance(account.data.len()));

    {
        let (fixed_data, wrapper_dynamic_data) = account.data.split_at_mut(fixed_size);
        let wrapper_fixed: &mut ManifestWrapperStateFixed = get_mut_helper(fixed_data, 0);
        wrapper_fixed.num_bytes_allocated += added_bytes as u32;

        let price: QuoteAtomsPerBaseAtom = QuoteAtomsPerBaseAtom::try_from(1_f64).unwrap();
        let mut tree: RedBlackTree<WrapperOpenOrder> =
            RedBlackTree::new(wrapper_dynamic_data, NIL, NIL);
        for order_number in 0..num_orders {
            let index: DataIndex =
                (bytes_allocated + order_number * WRAPPER_BLOCK_SIZE) as DataIndex;
            tree.insert(
                index,
                WrapperOpenOrder::new(
                    order_number as u64 + 1,
                    order_number as u64 + 1,
                    price,
                    BaseAtoms::new(1),
                    NO_EXPIRATION_LAST_VALID_SLOT,
                    0,
                    false,
                    OrderType::Limit,
                ),
            );
        }
        let root_index: DataIndex = tree.get_root_index();
        let market_info_node: &mut RBNode<MarketInfo> =
            get_mut_helper::<RBNode<MarketInfo>>(wrapper_dynamic_data, market_info_index);
        market_info_node.get_mut_value().orders_root_index = root_index;
        market_info_node.set_payload_type(ORDERS_LAYOUT_TREE);
    }
    test_fixture
        .context
        .borrow_mut()
        .set_account(&test_fixture.wrapper.key, &AccountSharedData::from(account));
}

/// Converts a market with `num_orders` legacy open orders in one instruction
/// and returns what it cost, having checked that every order came through.
async fn convert_legacy_tree(num_orders: usize) -> anyhow::Result<u64> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, SOL_UNIT_SIZE).await?;
    fabricate_legacy_tree(&test_fixture, num_orders).await;
    assert_eq!(orders_layout(&test_fixture).await, ORDERS_LAYOUT_TREE);

    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
    // A batch update that places and cancels nothing still syncs, and the
    // market is quiet, so this is the conversion and almost nothing else.
    let instruction: Instruction = batch_update_instruction(
        &test_fixture.market.key,
        &payer,
        &test_fixture.wrapper.key,
        vec![],
        false,
        vec![],
    );
    // A converting client raises the limit off the 200,000 CU default, which
    // is what the instruction has to fit in otherwise.
    let units: u64 = units_consumed(
        &test_fixture,
        &[
            ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
            instruction,
        ],
        &payer,
        &[&payer_keypair],
    )
    .await;

    let (_, orders) = wrapper_view(&test_fixture).await;
    assert_eq!(
        orders.len(),
        num_orders,
        "every order survives the conversion",
    );
    Ok(units)
}

/// What converting a large legacy wrapper costs, and what that says about the
/// sizes it can be done at.
///
/// The conversion is two passes over one market's open orders inside a single
/// instruction, and it has to fit: every later instruction retries it, so a
/// wrapper whose conversion cannot fit would never make progress. Nothing
/// caps the number of open orders on a market, so the ceiling is whatever the
/// cost implies, which is worth a number rather than an assumption.
///
/// Measured here on SBPF v2 at 93 CU per order (100,298 CU at 1,000 orders,
/// 381,950 at 4,000). That is about 15,000 orders in a transaction raised to
/// the 1.4M limit, and about 2,100 in the 200,000 CU an instruction gets by
/// default, so a client converting a very large wrapper has to raise it.
///
/// Which is far past what anyone reaches. An open order costs a 96 byte
/// wrapper block and an 80 byte block on the core, so 15,000 of them is about
/// 10 SOL of rent on the wrapper and another 8 on the market, and cancelling
/// them does not give it back: freed blocks are reused through a free list,
/// neither account shrinks, and `collect` leaves what the current size needs.
/// That rent is spent for the life of the accounts. And it stops working
/// earlier than that for
/// ordinary reasons: placing an order leaves the market non-quiet, so the next
/// instruction walks every open order on it at a comparable cost per order,
/// and reaching n orders takes n such instructions, the last walking n-1. The
/// size where the conversion stops fitting is the size where placing and
/// cancelling already stopped fitting. Past that the funds are still reachable
/// either way: balances and resting orders live on the core, which the trader
/// can cancel and withdraw against directly without going through the wrapper
/// at all.
///
/// The numbers only mean something when the compiled program is loaded
/// (`cargo test-sbf --features "test,test-sbf"`); the native processor plain
/// `cargo test` uses does not meter compute, but the conversion still runs and
/// is still checked there.
#[tokio::test]
async fn migrate_a_large_legacy_tree_test() -> anyhow::Result<()> {
    let small: u64 = convert_legacy_tree(1_000).await?;
    let large: u64 = convert_legacy_tree(4_000).await?;
    println!("CU converting 1000 orders: {small}");
    println!("CU converting 4000 orders: {large}");

    #[cfg(feature = "test-sbf")]
    {
        // Per order, from the slope rather than the totals, so the fixed cost
        // of the instruction around it does not flatter the estimate.
        let per_order: u64 = (large - small) / 3_000;
        println!("CU per order converted: {per_order}");
        assert!(
            per_order < 200,
            "conversion cost {per_order} CU per order, more than budgeted",
        );
        // The instruction that converts has 1.4M CU to do it in. Nothing here
        // enforces a maximum number of open orders, so the ceiling is what
        // this cost implies, and it wants to stay far above the size a wrapper
        // can be grown to: placing an order on a market already walks all of
        // that market's open orders in the same instruction, at a comparable
        // cost per order, so a wrapper cannot be built past roughly the same
        // size it can be converted at.
        let ceiling: u64 = 1_400_000 / per_order;
        println!("orders convertible in one instruction: about {ceiling}");
        assert!(
            ceiling > 10_000,
            "only {ceiling} orders could be converted in one instruction",
        );
        assert!(large < 1_400_000, "converting 4000 orders took {large} CU");
    }
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

/// A wrapper from before the list keeps its orders in a tree. The first batch
/// update converts them in place, then cancels, placements and the free list
/// work on the list as usual.
#[tokio::test]
async fn migrate_tree_orders_on_batch_update_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 20 * SOL_UNIT_SIZE).await?;
    // A fresh wrapper starts on the list.
    assert_eq!(orders_layout(&test_fixture).await, ORDERS_LAYOUT_LIST);

    let orders: Vec<WrapperPlaceOrderParams> = (1..=7u64).map(|i| ask(i, i as u32)).collect();
    wrapper_batch(&test_fixture, vec![], orders).await?;
    rewrite_orders_as_tree(&test_fixture).await;
    assert_eq!(orders_layout(&test_fixture).await, ORDERS_LAYOUT_TREE);
    let size_before: usize = wrapper_account(&test_fixture).await.data.len();

    // Cancel three migrated orders and place one in the same batch.
    wrapper_batch(
        &test_fixture,
        vec![
            WrapperCancelOrderParams::new(2),
            WrapperCancelOrderParams::new(4),
            WrapperCancelOrderParams::new(6),
        ],
        vec![ask(8, 8)],
    )
    .await?;
    let (market_info, orders) = wrapper_view(&test_fixture).await;
    assert_eq!(orders, vec![(1, 0), (3, 2), (5, 4), (7, 6), (8, 7)]);
    assert_eq!(
        market_info.last_synced_order_sequence_number,
        market_sequence_number(&test_fixture).await
    );

    // The blocks freed by the cancels are reused: the two still free fit two
    // more orders without growing the wrapper.
    wrapper_batch(&test_fixture, vec![], vec![ask(9, 9), ask(10, 10)]).await?;
    let (_, orders) = wrapper_view(&test_fixture).await;
    assert_eq!(orders.len(), 7);
    assert_eq!(
        wrapper_account(&test_fixture).await.data.len(),
        size_before,
        "freed blocks are reused"
    );

    // And every migrated order can still be cancelled.
    let cancels: Vec<WrapperCancelOrderParams> = [1u64, 3, 5, 7, 8, 9, 10]
        .iter()
        .map(|id| WrapperCancelOrderParams::new(*id))
        .collect();
    wrapper_batch(&test_fixture, cancels, vec![]).await?;
    let (market_info, orders) = wrapper_view(&test_fixture).await;
    assert_eq!(orders, vec![]);
    assert_eq!(market_info.orders_root_index, NIL);
    Ok(())
}

/// The sync done by deposits and withdrawals converts too, and a tree that
/// the core has moved on from is reconciled in the same pass.
#[tokio::test]
async fn migrate_tree_orders_on_sync_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 10 * SOL_UNIT_SIZE).await?;
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();

    wrapper_batch(&test_fixture, vec![], vec![ask(1, 5), ask(2, 6), ask(3, 7)]).await?;
    rewrite_orders_as_tree(&test_fixture).await;

    // Cancel sequence number 1 on the core directly, so the tree is stale.
    let direct_cancel: Instruction = manifest_batch_update_instruction(
        &test_fixture.market.key,
        &payer,
        None,
        vec![CancelOrderParams::new(1)],
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

    test_fixture.deposit(Token::SOL, SOL_UNIT_SIZE).await?;
    let (_, orders) = wrapper_view(&test_fixture).await;
    assert_eq!(orders, vec![(1, 0), (3, 2)]);
    Ok(())
}

/// A market info under the old flag with no orders converts to an empty list.
#[tokio::test]
async fn migrate_empty_tree_orders_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 10 * SOL_UNIT_SIZE).await?;
    rewrite_orders_as_tree(&test_fixture).await;
    assert_eq!(orders_layout(&test_fixture).await, ORDERS_LAYOUT_TREE);

    wrapper_batch(&test_fixture, vec![], vec![ask(1, 5)]).await?;
    let (_, orders) = wrapper_view(&test_fixture).await;
    assert_eq!(orders, vec![(1, 0)]);
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
