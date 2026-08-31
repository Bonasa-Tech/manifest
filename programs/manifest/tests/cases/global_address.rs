//! Validation of global accounts: the canonical global addresses cached on
//! the market, how markets from before the cache existed pick it up, and that
//! accounts which are not the canonical global PDA are rejected on every path
//! even when they carry a perfect copy of a real global's data.

use std::rc::Rc;

use borsh::BorshSerialize;
use hypertree::get_helper;
use manifest::{
    program::{
        batch_update::PlaceOrderParams, batch_update_instruction, global_add_trader_instruction,
        global_deposit_instruction, ManifestInstruction,
    },
    state::{constants::NO_EXPIRATION_LAST_VALID_SLOT, MarketFixed, OrderType, RestingOrder},
    validation::{get_global_address, get_global_vault_address, get_vault_address},
};
use solana_account::{Account, AccountSharedData};
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_program::{pubkey::Pubkey, system_program};
use solana_program_test::tokio;

use crate::{send_tx_with_retry, TestFixture};

/// Byte offsets of the cached global addresses in `MarketFixed`: the last 64
/// bytes of the 256 byte header.
const BASE_GLOBAL_OFFSET: usize = 192;
const QUOTE_GLOBAL_OFFSET: usize = 224;

async fn get_account(test_fixture: &TestFixture, key: &Pubkey) -> Account {
    test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(*key)
        .await
        .unwrap()
        .expect("account exists")
}

async fn cached_globals(test_fixture: &TestFixture) -> (Pubkey, Pubkey) {
    let market: Account = get_account(test_fixture, &test_fixture.market_fixture.key).await;
    let market_fixed: &MarketFixed = get_helper::<MarketFixed>(&market.data, 0_u32);
    (
        *market_fixed.get_base_global(),
        *market_fixed.get_quote_global(),
    )
}

/// Turns the fixture's market into one created before the cache existed by
/// zeroing the cached addresses in place.
async fn clear_cached_globals(test_fixture: &TestFixture) {
    let mut market: Account = get_account(test_fixture, &test_fixture.market_fixture.key).await;
    market.data[BASE_GLOBAL_OFFSET..QUOTE_GLOBAL_OFFSET + 32].fill(0);
    test_fixture.context.borrow_mut().set_account(
        &test_fixture.market_fixture.key,
        &AccountSharedData::from(market),
    );
    assert_eq!(
        cached_globals(test_fixture).await,
        (Pubkey::default(), Pubkey::default())
    );
}

/// Batch update placing one global bid with the global accounts for
/// `global_mint` given explicitly, so tests can substitute the global account.
fn global_bid_instruction(
    test_fixture: &TestFixture,
    global_mint: &Pubkey,
    global: &Pubkey,
) -> Instruction {
    let market: Pubkey = test_fixture.market_fixture.key;
    let payer: Pubkey = test_fixture.payer();
    let mut instruction: Instruction = batch_update_instruction(
        &market,
        &payer,
        None,
        vec![],
        vec![PlaceOrderParams::new(
            10,
            1,
            0,
            true,
            OrderType::Global,
            NO_EXPIRATION_LAST_VALID_SLOT,
        )],
        None,
        None,
        Some(*global_mint),
        None,
    );
    // Accounts: payer, market, system program, mint, global, global vault,
    // market vault, token program.
    assert_eq!(
        instruction.accounts[4].pubkey,
        get_global_address(global_mint).0
    );
    instruction.accounts[4] = AccountMeta::new(*global, false);
    instruction
}

async fn send(test_fixture: &TestFixture, instruction: Instruction) -> bool {
    let payer_keypair: Keypair = test_fixture.payer_keypair();
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[instruction],
        Some(&test_fixture.payer()),
        &[&payer_keypair],
    )
    .await
    .is_ok()
}

#[tokio::test]
async fn global_addresses_cached_on_create_test() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;
    let (base_global, quote_global) = cached_globals(&test_fixture).await;
    assert_eq!(
        base_global,
        get_global_address(&test_fixture.sol_mint_fixture.key).0
    );
    assert_eq!(
        quote_global,
        get_global_address(&test_fixture.usdc_mint_fixture.key).0
    );
    Ok(())
}

/// A market created before the cache existed gets both addresses filled in by
/// its first global batch update, and keeps trading afterwards.
#[tokio::test]
async fn global_addresses_lazily_cached_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.global_add_trader().await?;
    test_fixture.global_deposit(1_000_000).await?;
    clear_cached_globals(&test_fixture).await;

    // Both mints are passed, so both sides get cached.
    test_fixture
        .batch_update_with_global_for_keypair(
            None,
            vec![],
            vec![PlaceOrderParams::new(
                10,
                1,
                0,
                true,
                OrderType::Global,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &test_fixture.payer_keypair().insecure_clone(),
        )
        .await?;
    assert_eq!(
        cached_globals(&test_fixture).await,
        (
            get_global_address(&test_fixture.sol_mint_fixture.key).0,
            get_global_address(&test_fixture.usdc_mint_fixture.key).0,
        )
    );

    // Second trade takes the cached path.
    test_fixture
        .batch_update_with_global_for_keypair(
            None,
            vec![],
            vec![PlaceOrderParams::new(
                10,
                2,
                0,
                true,
                OrderType::Global,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &test_fixture.payer_keypair().insecure_clone(),
        )
        .await?;
    test_fixture.market_fixture.reload().await;
    let orders: Vec<RestingOrder> = test_fixture.market_fixture.get_resting_orders().await;
    assert_eq!(orders.len(), 2);
    Ok(())
}

/// The global for one mint is not accepted in the slot of another, cached or
/// not.
#[tokio::test]
async fn global_address_other_mint_rejected_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.global_add_trader().await?;
    test_fixture.global_deposit(1_000_000).await?;
    let usdc_mint: Pubkey = test_fixture.usdc_mint_fixture.key;
    let sol_global: Pubkey = get_global_address(&test_fixture.sol_mint_fixture.key).0;

    // Cached path.
    assert!(
        !send(
            &test_fixture,
            global_bid_instruction(&test_fixture, &usdc_mint, &sol_global)
        )
        .await
    );
    // Derivation path of a market from before the cache.
    clear_cached_globals(&test_fixture).await;
    assert!(
        !send(
            &test_fixture,
            global_bid_instruction(&test_fixture, &usdc_mint, &sol_global)
        )
        .await
    );
    assert_eq!(
        cached_globals(&test_fixture).await,
        (Pubkey::default(), Pubkey::default())
    );
    // The real global still works.
    let usdc_global: Pubkey = get_global_address(&usdc_mint).0;
    assert!(
        send(
            &test_fixture,
            global_bid_instruction(&test_fixture, &usdc_mint, &usdc_global)
        )
        .await
    );
    Ok(())
}

/// The attack the address check exists for: an account owned by this program
/// that is a byte for byte copy of a real global, including its stored bump,
/// but does not sit at the program derived address. The runtime does not let
/// anyone assign a non-empty account to a program, so this can only be set up
/// in a test, and every path must still reject it.
#[tokio::test]
async fn global_forged_account_rejected_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.global_add_trader().await?;
    test_fixture.global_deposit(1_000_000).await?;
    let usdc_mint: Pubkey = test_fixture.usdc_mint_fixture.key;
    let usdc_global: Pubkey = get_global_address(&usdc_mint).0;
    let forged_global: Pubkey = Pubkey::new_unique();
    let real: Account = get_account(&test_fixture, &usdc_global).await;
    assert_eq!(real.owner, manifest::id());
    test_fixture
        .context
        .borrow_mut()
        .set_account(&forged_global, &AccountSharedData::from(real));

    // Batch update, cached and derivation paths.
    assert!(
        !send(
            &test_fixture,
            global_bid_instruction(&test_fixture, &usdc_mint, &forged_global)
        )
        .await
    );
    clear_cached_globals(&test_fixture).await;
    assert!(
        !send(
            &test_fixture,
            global_bid_instruction(&test_fixture, &usdc_mint, &forged_global)
        )
        .await
    );

    // Global instructions validate against the stored bump.
    let payer: Pubkey = test_fixture.payer();
    let add_trader: Instruction = global_add_trader_instruction(&forged_global, &payer);
    assert!(!send(&test_fixture, add_trader).await);

    let mut deposit: Instruction = global_deposit_instruction(
        &usdc_mint,
        &payer,
        &test_fixture.payer_usdc_fixture.key,
        &spl_token::id(),
        1,
    );
    assert_eq!(deposit.accounts[1].pubkey, usdc_global);
    deposit.accounts[1] = AccountMeta::new(forged_global, false);
    assert!(!send(&test_fixture, deposit).await);

    // And a swap that names the forged global.
    let (sol_vault, _) = get_vault_address(
        &test_fixture.market_fixture.key,
        &test_fixture.sol_mint_fixture.key,
    );
    let (usdc_vault, _) = get_vault_address(&test_fixture.market_fixture.key, &usdc_mint);
    let swap: Instruction = Instruction {
        program_id: manifest::id(),
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(test_fixture.market_fixture.key, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new(test_fixture.payer_sol_fixture.key, false),
            AccountMeta::new(test_fixture.payer_usdc_fixture.key, false),
            AccountMeta::new(sol_vault, false),
            AccountMeta::new(usdc_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new(forged_global, false),
            AccountMeta::new(get_global_vault_address(&usdc_mint).0, false),
        ],
        data: [
            ManifestInstruction::Swap.to_vec(),
            manifest::program::SwapParams::new(1, 0, false, true)
                .try_to_vec()
                .unwrap(),
        ]
        .concat(),
    };
    assert!(!send(&test_fixture, swap).await);

    // The genuine global is unaffected.
    assert!(
        send(
            &test_fixture,
            global_bid_instruction(&test_fixture, &usdc_mint, &usdc_global)
        )
        .await
    );
    Ok(())
}
