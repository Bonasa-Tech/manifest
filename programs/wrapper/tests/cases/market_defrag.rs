//! A CORE market defrag relocates this wrapper's resting orders, and the
//! wrapper keeps working.
//!
//! This is the case that broke a third-party integrator on mainnet. Defrag
//! moves surviving blocks down and truncates the freed tail, so a block offset
//! cached across transactions can point at a different order, or past the end
//! of a now-smaller account. A client that dereferences such an offset
//! without re-deriving it reads out of bounds and panics before it ever calls
//! the core - which is exactly what was observed: `SBF program panicked`,
//! ~2,100 compute units consumed, no CPI to manifest.
//!
//! The wrapper is expected to survive because `sync_fast` re-derives both the
//! trader index and every order's market index through `resolve_trader_index`
//! / `resolve_order_index` before anything is dereferenced. Those resolvers
//! bounds-check the hint against the CURRENT account length and fall back to a
//! lookup, so truncation cannot strand them. This test pins that behaviour.

use std::rc::Rc;

use hypertree::{get_helper, RBNode};
use manifest::{
    program::{batch_update::MarketDataTreeNodeType, defrag_instruction},
    state::{MarketFixed, RestingOrder, MARKET_BLOCK_SIZE},
};
use solana_account::{Account, AccountSharedData};
use solana_keypair::Keypair;
use solana_program::{instruction::Instruction, pubkey::Pubkey};
use solana_program_test::tokio;
use solana_signer::Signer;
use wrapper::processors::batch_upate::WrapperCancelOrderParams;

use crate::{send_tx_with_retry, TestFixture, Token, SOL_UNIT_SIZE};

use super::sync::{ask, wrapper_batch, wrapper_view};

/// The test-only collector that `Defrag` accepts under `--features test`.
fn collector(f: &TestFixture) -> Keypair {
    let key: Keypair = Keypair::new_from_array([42; 32]);
    f.context.borrow_mut().set_account(
        &key.pubkey(),
        &AccountSharedData::from(Account::new(
            1_000_000_000,
            0,
            &solana_sdk_ids::system_program::id(),
        )),
    );
    key
}

async fn market_account(f: &TestFixture) -> Account {
    f.context
        .borrow_mut()
        .banks_client
        .get_account(f.market.key)
        .await
        .unwrap()
        .unwrap()
}

/// Block offsets of every resting order in the market, in order.
fn resting_order_offsets(data: &[u8]) -> Vec<u32> {
    let fixed_size: usize = std::mem::size_of::<MarketFixed>();
    let dynamic: &[u8] = &data[fixed_size..];
    let mut offsets: Vec<u32> = Vec::new();
    let mut offset: usize = 0;
    while offset + MARKET_BLOCK_SIZE <= dynamic.len() {
        let index: u32 = offset as u32;
        let node = get_helper::<RBNode<RestingOrder>>(dynamic, index);
        if node.get_payload_type() == MarketDataTreeNodeType::RestingOrder as u8 {
            offsets.push(index);
        }
        offset += MARKET_BLOCK_SIZE;
    }
    offsets
}

/// Relocates `relocations` blocks on the market, reclaiming no seats, so only
/// order movement is exercised.
async fn defrag_market(
    f: &TestFixture,
    key: &Keypair,
    relocations: Option<u32>,
) -> anyhow::Result<()> {
    let instruction: Instruction = defrag_instruction(
        &f.market.key,
        &key.pubkey(),
        &f.sol_mint.key,
        &f.usdc_mint.key,
        spl_token::id(),
        spl_token::id(),
        relocations,
    );
    let payer: Pubkey = f.payer();
    let payer_keypair: Keypair = f.payer_keypair().insecure_clone();
    send_tx_with_retry(
        Rc::clone(&f.context),
        &[instruction],
        Some(&payer),
        &[&payer_keypair, key],
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn market_defrag_relocates_orders_and_the_wrapper_still_works() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture
        .deposit(Token::SOL, 100 * SOL_UNIT_SIZE)
        .await?;

    // Several orders, then cancel from the middle so the market holds free
    // blocks below live ones. That is what gives defrag somewhere to relocate
    // into, and it is the normal shape of a market that has traded.
    wrapper_batch(
        &test_fixture,
        vec![],
        vec![ask(1, 10), ask(2, 11), ask(3, 12), ask(4, 13), ask(5, 14)],
    )
    .await?;
    wrapper_batch(
        &test_fixture,
        vec![
            WrapperCancelOrderParams::new(1),
            WrapperCancelOrderParams::new(2),
        ],
        vec![],
    )
    .await?;

    let before: Account = market_account(&test_fixture).await;
    let offsets_before: Vec<u32> = resting_order_offsets(&before.data);
    assert!(
        offsets_before.len() >= 3,
        "expected the surviving orders to still rest: {offsets_before:?}"
    );

    // Relocate, reclaiming no seats: orders move, seats do not.
    let key: Keypair = collector(&test_fixture);
    defrag_market(&test_fixture, &key, None).await?;

    let after: Account = market_account(&test_fixture).await;
    let offsets_after: Vec<u32> = resting_order_offsets(&after.data);
    assert_eq!(
        offsets_before.len(),
        offsets_after.len(),
        "defrag must not drop an order: {offsets_before:?} -> {offsets_after:?}"
    );
    // The point of the test: at least one live order actually moved, so every
    // offset the wrapper cached for it is now stale. Without this the test
    // would pass trivially.
    assert_ne!(
        offsets_before, offsets_after,
        "no order moved, so nothing stale is being exercised"
    );
    assert!(
        after.data.len() < before.data.len(),
        "defrag must shrink the account, stranding offsets past the new end: {} -> {}",
        before.data.len(),
        after.data.len()
    );

    // The wrapper's cached offsets are now wrong in both ways available: some
    // point at a different order, some point past the end of the account. Use
    // the wrapper anyway - cancel one order and place another. If the wrapper
    // dereferenced a cached offset the way the broken integrator did, this
    // aborts instead of succeeding.
    wrapper_batch(
        &test_fixture,
        vec![WrapperCancelOrderParams::new(3)],
        vec![ask(6, 15)],
    )
    .await?;

    let (_, orders) = wrapper_view(&test_fixture).await;
    let mut ids: Vec<u64> = orders.iter().map(|(id, _)| *id).collect();
    ids.sort();
    assert_eq!(
        ids,
        vec![4, 5, 6],
        "the cancel landed, the untouched orders survived, and the new order rests"
    );

    // And it keeps working: a second batch over the healed view still runs.
    wrapper_batch(
        &test_fixture,
        vec![
            WrapperCancelOrderParams::new(4),
            WrapperCancelOrderParams::new(5),
        ],
        vec![],
    )
    .await?;
    let (_, orders) = wrapper_view(&test_fixture).await;
    let mut ids: Vec<u64> = orders.iter().map(|(id, _)| *id).collect();
    ids.sort();
    assert_eq!(ids, vec![6], "cancels over relocated orders all succeeded");

    Ok(())
}

/// Cancel-only batches take the cheap path that skips re-reading the core, so
/// they are the most likely to trust a cached offset. A defrag immediately
/// before one must still be handled.
#[tokio::test]
async fn cancel_only_batch_after_market_defrag_succeeds() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture
        .deposit(Token::SOL, 100 * SOL_UNIT_SIZE)
        .await?;

    wrapper_batch(
        &test_fixture,
        vec![],
        vec![ask(1, 10), ask(2, 11), ask(3, 12)],
    )
    .await?;
    wrapper_batch(
        &test_fixture,
        vec![WrapperCancelOrderParams::new(1)],
        vec![],
    )
    .await?;

    let before: Account = market_account(&test_fixture).await;
    let key: Keypair = collector(&test_fixture);
    defrag_market(&test_fixture, &key, None).await?;
    let after: Account = market_account(&test_fixture).await;
    assert!(after.data.len() <= before.data.len());

    // Cancel only: no placement, so nothing forces a full re-read.
    wrapper_batch(
        &test_fixture,
        vec![WrapperCancelOrderParams::new(2)],
        vec![],
    )
    .await?;
    let (_, orders) = wrapper_view(&test_fixture).await;
    let ids: Vec<u64> = orders.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, vec![3], "the cancel-only path handled the relocation");
    Ok(())
}
