use std::rc::Rc;

use hypertree::{DataIndex, HyperTreeValueIteratorTrait};
use manifest::{
    program::{
        batch_update::{CancelOrderParams, PlaceOrderParams},
        batch_update_instruction, global_add_trader_instruction, global_clean_instruction,
        global_deposit_instruction, global_evict_instruction, global_withdraw_instruction,
        swap_instruction,
    },
    quantities::{GlobalAtoms, QuoteAtomsPerBaseAtom, WrapperU64},
    state::{
        DynamicAccount, GlobalFixed, OrderType, RestingOrder, MARKET_BLOCK_SIZE, MAX_GLOBAL_SEATS,
        NO_EXPIRATION_LAST_VALID_SLOT,
    },
};
use solana_program_test::tokio;
use solana_sdk::{
    instruction::Instruction, pubkey::Pubkey, signature::Keypair, signer::Signer,
    system_instruction::transfer,
};

use crate::{
    send_tx_with_retry, GlobalFixture, MarketFixture, MintFixture, TestFixture, Token,
    TokenAccountFixture,
};

#[tokio::test]
async fn create_global() -> anyhow::Result<()> {
    let _test_fixture: TestFixture = TestFixture::new().await;

    Ok(())
}

#[tokio::test]
async fn global_add_trader() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    let payer: Pubkey = test_fixture.payer();
    test_fixture.global_add_trader().await?;

    test_fixture.global_fixture.reload().await;
    let global_dynamic_account: DynamicAccount<GlobalFixed, Vec<u8>> =
        test_fixture.global_fixture.global;

    // Verifying that the account exists and that there are zero there.
    let balance_atoms: GlobalAtoms = global_dynamic_account.get_balance_atoms(&payer);
    assert_eq!(balance_atoms, GlobalAtoms::ZERO);
    Ok(())
}

#[tokio::test]
async fn global_add_trader_repeat_fail() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.global_add_trader().await?;

    assert!(test_fixture.global_add_trader().await.is_err());
    Ok(())
}

#[tokio::test]
async fn global_deposit() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    let payer: Pubkey = test_fixture.payer();
    test_fixture.global_add_trader().await?;
    test_fixture.global_deposit(1_000_000).await?;

    test_fixture.global_fixture.reload().await;
    let global_dynamic_account: DynamicAccount<GlobalFixed, Vec<u8>> =
        test_fixture.global_fixture.global;

    // Verifying that the account exists and that there are tokens there.
    let balance_atoms: GlobalAtoms = global_dynamic_account.get_balance_atoms(&payer);
    assert_eq!(balance_atoms, GlobalAtoms::new(1_000_000));
    Ok(())
}

#[tokio::test]
async fn global_withdraw() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    let payer: Pubkey = test_fixture.payer();
    test_fixture.global_add_trader().await?;
    test_fixture.global_deposit(2_000_000).await?;
    test_fixture.global_withdraw(1_000_000).await?;

    test_fixture.global_fixture.reload().await;
    let global_dynamic_account: DynamicAccount<GlobalFixed, Vec<u8>> =
        test_fixture.global_fixture.global;

    // Verifying that the account exists and that there are tokens there.
    let balance_atoms: GlobalAtoms = global_dynamic_account.get_balance_atoms(&payer);
    assert_eq!(balance_atoms, GlobalAtoms::new(1_000_000));
    Ok(())
}

#[tokio::test]
async fn global_place_order() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;

    test_fixture.global_add_trader().await?;
    test_fixture.global_deposit(1_000_000).await?;

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

    test_fixture.market_fixture.reload().await;
    let orders: Vec<RestingOrder> = test_fixture.market_fixture.get_resting_orders().await;
    assert_eq!(orders.len(), 1, "Could not find resting order");

    Ok(())
}

#[tokio::test]
async fn global_place_order_only_global_quote() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;

    test_fixture.global_add_trader().await?;
    test_fixture.global_deposit(1_000_000).await?;

    let batch_update_ix: Instruction = batch_update_instruction(
        &test_fixture.market_fixture.key,
        &test_fixture.payer(),
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
        Some(*test_fixture.market_fixture.market.get_quote_mint()),
        None,
    );

    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[batch_update_ix],
        Some(&test_fixture.payer()),
        &[&test_fixture.payer_keypair().insecure_clone()],
    )
    .await?;

    test_fixture.market_fixture.reload().await;
    let orders: Vec<RestingOrder> = test_fixture.market_fixture.get_resting_orders().await;
    assert_eq!(orders.len(), 1, "Could not find resting order");
    assert_eq!(
        orders.get(0).unwrap().get_num_base_atoms(),
        10,
        "Order size was wrong"
    );
    assert_eq!(
        orders.get(0).unwrap().get_price(),
        QuoteAtomsPerBaseAtom::try_from(1.0).unwrap(),
        "Order price was wrong"
    );
    assert_eq!(
        orders.get(0).unwrap().get_order_type(),
        OrderType::Global,
        "Order type was wrong"
    );

    Ok(())
}

#[tokio::test]
async fn global_cancel_order() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;

    test_fixture.global_add_trader().await?;
    test_fixture.global_deposit(1_000_000).await?;

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

    test_fixture
        .batch_update_with_global_for_keypair(
            None,
            vec![CancelOrderParams::new(0)],
            vec![],
            &test_fixture.payer_keypair().insecure_clone(),
        )
        .await?;

    test_fixture.market_fixture.reload().await;
    let orders: Vec<RestingOrder> = test_fixture.market_fixture.get_resting_orders().await;
    assert_eq!(orders.len(), 0, "Did not cancel");
    test_fixture.global_fixture.reload().await;

    Ok(())
}

#[tokio::test]
async fn global_match_order() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;

    test_fixture
        .claim_seat_for_keypair(&test_fixture.second_keypair.insecure_clone())
        .await?;
    test_fixture
        .global_add_trader_for_keypair(&test_fixture.second_keypair.insecure_clone())
        .await?;
    test_fixture
        .global_deposit_for_keypair(&test_fixture.second_keypair.insecure_clone(), 1_000_000)
        .await?;

    test_fixture
        .batch_update_with_global_for_keypair(
            None,
            vec![],
            vec![PlaceOrderParams::new(
                100,
                11,
                -1,
                true,
                OrderType::Global,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &test_fixture.second_keypair.insecure_clone(),
        )
        .await?;

    test_fixture.deposit(Token::SOL, 1_000_000).await?;

    test_fixture
        .batch_update_with_global_for_keypair(
            None,
            vec![],
            vec![PlaceOrderParams::new(
                100,
                9,
                -1,
                false,
                OrderType::Limit,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &test_fixture.payer_keypair().insecure_clone(),
        )
        .await?;

    test_fixture.market_fixture.reload().await;
    let orders: Vec<RestingOrder> = test_fixture.market_fixture.get_resting_orders().await;
    assert_eq!(orders.len(), 0, "Order still on orderbook");

    // Global buys 100 base for 110 quote
    // Local sells 100 base for 90 quote

    // Match will leave global: 0 quote, 100 base
    // match will leave local: 110 quote, 1_000_000 - 100 base

    assert_eq!(
        test_fixture
            .market_fixture
            .get_base_balance_atoms(&test_fixture.second_keypair.pubkey())
            .await,
        100
    );
    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_balance_atoms(&test_fixture.second_keypair.pubkey())
            .await,
        0
    );
    assert_eq!(
        test_fixture
            .market_fixture
            .get_base_balance_atoms(&test_fixture.payer())
            .await,
        1_000_000 - 100
    );
    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_balance_atoms(&test_fixture.payer())
            .await,
        110
    );
    test_fixture.global_fixture.reload().await;
    assert_eq!(
        test_fixture
            .global_fixture
            .global
            .get_balance_atoms(&test_fixture.second_keypair.insecure_clone().pubkey())
            .as_u64(),
        1_000_000 - 110
    );

    Ok(())
}

#[tokio::test]
async fn global_match_order_quote_no_bonus() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;

    test_fixture
        .claim_seat_for_keypair(&test_fixture.second_keypair.insecure_clone())
        .await?;
    test_fixture
        .global_add_trader_for_keypair(&test_fixture.second_keypair.insecure_clone())
        .await?;
    test_fixture
        .global_deposit_for_keypair(&test_fixture.second_keypair.insecure_clone(), 1_000_000)
        .await?;

    test_fixture
        .batch_update_with_global_for_keypair(
            None,
            vec![],
            vec![PlaceOrderParams::new(
                100,
                11,
                -1,
                true,
                OrderType::Global,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &test_fixture.second_keypair.insecure_clone(),
        )
        .await?;

    test_fixture.deposit(Token::SOL, 1_000_000).await?;

    test_fixture
        .batch_update_with_global_for_keypair(
            None,
            vec![],
            vec![PlaceOrderParams::new(
                1,
                9,
                -1,
                false,
                OrderType::Limit,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &test_fixture.payer_keypair().insecure_clone(),
        )
        .await?;

    test_fixture.market_fixture.reload().await;
    let orders: Vec<RestingOrder> = test_fixture.market_fixture.get_resting_orders().await;
    assert_eq!(orders.len(), 1, "Order still on orderbook");

    // Global buys 1 base for 1.1 quote --> rounded to 1 quote
    // Local sells 1 base for 1.1 quote --> rounded to 1 quote

    // Match will leave global: 1 quote, 1 base
    // match will leave local: 1 quote, 1_000_000 - 1 base

    assert_eq!(
        test_fixture
            .market_fixture
            .get_base_balance_atoms(&test_fixture.second_keypair.pubkey())
            .await,
        1
    );
    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_balance_atoms(&test_fixture.second_keypair.pubkey())
            .await,
        0
    );
    assert_eq!(
        test_fixture
            .market_fixture
            .get_base_balance_atoms(&test_fixture.payer())
            .await,
        1_000_000 - 1
    );
    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_balance_atoms(&test_fixture.payer())
            .await,
        1
    );

    test_fixture.global_fixture.reload().await;
    assert_eq!(
        test_fixture
            .global_fixture
            .global
            .get_balance_atoms(&test_fixture.second_keypair.insecure_clone().pubkey())
            .as_u64(),
        1_000_000 - 1
    );

    Ok(())
}

#[tokio::test]
async fn global_deposit_withdraw_22() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();

    let mut usdc_mint_fixture: MintFixture =
        MintFixture::new_with_version(Rc::clone(&test_fixture.context), Some(9), true).await;
    let mut global_fixture: GlobalFixture = GlobalFixture::new_with_token_program(
        Rc::clone(&test_fixture.context),
        &usdc_mint_fixture.key,
        &spl_token_2022::id(),
    )
    .await;

    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[global_add_trader_instruction(&global_fixture.key, &payer)],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // Make a throw away token account
    let token_account_keypair: Keypair = Keypair::new();
    let token_account_fixture: TokenAccountFixture = TokenAccountFixture::new_with_keypair_2022(
        Rc::clone(&test_fixture.context),
        &global_fixture.mint_key,
        &payer,
        &token_account_keypair,
    )
    .await;
    usdc_mint_fixture
        .mint_to_2022(&token_account_fixture.key, 1_000_000)
        .await;
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[global_deposit_instruction(
            &global_fixture.mint_key,
            &payer,
            &token_account_fixture.key,
            &spl_token_2022::id(),
            1_000_000,
        )],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    global_fixture.reload().await;
    let global_dynamic_account: &DynamicAccount<GlobalFixed, Vec<u8>> = &global_fixture.global;

    // Verifying that the account exists and that there are tokens there.
    let balance_atoms: GlobalAtoms = global_dynamic_account.get_balance_atoms(&payer);
    assert_eq!(balance_atoms, GlobalAtoms::new(1_000_000));

    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[global_withdraw_instruction(
            &global_fixture.mint_key,
            &payer,
            &token_account_fixture.key,
            &spl_token_2022::id(),
            1_000_000,
        )],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;
    global_fixture.reload().await;
    let global_dynamic_account: &DynamicAccount<GlobalFixed, Vec<u8>> = &global_fixture.global;
    let balance_atoms: GlobalAtoms = global_dynamic_account.get_balance_atoms(&payer);
    assert_eq!(balance_atoms, GlobalAtoms::new(0));

    Ok(())
}

#[tokio::test]
async fn global_match_22() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();

    let mut usdc_mint_fixture: MintFixture =
        MintFixture::new_with_version(Rc::clone(&test_fixture.context), Some(9), true).await;
    let mut global_fixture: GlobalFixture = GlobalFixture::new_with_token_program(
        Rc::clone(&test_fixture.context),
        &usdc_mint_fixture.key,
        &spl_token_2022::id(),
    )
    .await;

    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[global_add_trader_instruction(&global_fixture.key, &payer)],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // Make a throw away token account
    let token_account_keypair: Keypair = Keypair::new();
    let token_account_fixture: TokenAccountFixture = TokenAccountFixture::new_with_keypair_2022(
        Rc::clone(&test_fixture.context),
        &global_fixture.mint_key,
        &payer,
        &token_account_keypair,
    )
    .await;
    usdc_mint_fixture
        .mint_to_2022(&token_account_fixture.key, 1_000_000)
        .await;

    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[global_deposit_instruction(
            &global_fixture.mint_key,
            &payer,
            &token_account_fixture.key,
            &spl_token_2022::id(),
            1_000_000,
        )],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    let mut market_fixture: MarketFixture = MarketFixture::new(
        Rc::clone(&test_fixture.context),
        1, // different index to avoid PDA collision with default market
        9,
        &usdc_mint_fixture.key,
    )
    .await;
    market_fixture.reload().await;

    let claim_seat_ix: Instruction =
        manifest::program::claim_seat_instruction(&market_fixture.key, &payer);
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[claim_seat_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    let batch_update_ix: Instruction = batch_update_instruction(
        &market_fixture.key,
        &payer,
        None,
        vec![],
        vec![PlaceOrderParams::new(
            1_000_000,
            1,
            0,
            true,
            OrderType::Global,
            NO_EXPIRATION_LAST_VALID_SLOT,
        )],
        None, // no physical base mint in perps
        None,
        Some(*market_fixture.market.get_quote_mint()),
        Some(spl_token_2022::id()),
    );

    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[batch_update_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // Setup the second keypair to take.
    test_fixture
        .sol_mint_fixture
        .mint_to(&test_fixture.payer_sol_fixture.key, 1_000_000)
        .await;

    let swap_ix: Instruction = swap_instruction(
        &market_fixture.key,
        &payer,
        &test_fixture.sol_mint_fixture.key,
        &usdc_mint_fixture.key,
        &test_fixture.payer_sol_fixture.key,
        &token_account_fixture.key,
        1_000,
        0,
        true,
        true,
        spl_token::id(),
        spl_token_2022::id(),
        true,
    );

    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[swap_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    global_fixture.reload().await;
    let global_dynamic_account: DynamicAccount<GlobalFixed, Vec<u8>> = global_fixture.global;

    // Verify that the global account traded all of its tokens.
    let balance_atoms: GlobalAtoms = global_dynamic_account.get_balance_atoms(&payer);
    assert_eq!(balance_atoms, GlobalAtoms::new(999_000));
    market_fixture.reload().await;
    // Zero because swaps reset the amounts, even if it is a self trade.
    assert_eq!(market_fixture.get_base_balance_atoms(&payer).await, 0);
    assert_eq!(market_fixture.get_quote_balance_atoms(&payer).await, 0);
    Ok(())
}

#[tokio::test]
async fn global_insufficient() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;

    test_fixture.global_add_trader().await?;
    test_fixture.global_deposit(1_000_000).await?;

    test_fixture
        .batch_update_with_global_for_keypair(
            None,
            vec![],
            vec![PlaceOrderParams::new(
                100,
                1,
                0,
                true,
                OrderType::Global,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &test_fixture.payer_keypair().insecure_clone(),
        )
        .await?;
    test_fixture.global_withdraw(1_000_000).await?;

    test_fixture.deposit(Token::SOL, 1_000_000).await?;

    test_fixture
        .batch_update_with_global_for_keypair(
            None,
            vec![],
            vec![PlaceOrderParams::new(
                100,
                9,
                -1,
                false,
                OrderType::ImmediateOrCancel,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &test_fixture.payer_keypair().insecure_clone(),
        )
        .await?;

    test_fixture.market_fixture.reload().await;
    let orders: Vec<RestingOrder> = test_fixture.market_fixture.get_resting_orders().await;
    // Remove unbacked global order.
    assert_eq!(orders.len(), 0, "Order still on orderbook");

    // No trade happened.
    assert_eq!(
        test_fixture
            .market_fixture
            .get_base_balance_atoms(&test_fixture.payer())
            .await,
        1_000_000
    );
    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_balance_atoms(&test_fixture.payer())
            .await,
        0
    );
    test_fixture.global_fixture.reload().await;
    assert_eq!(
        test_fixture
            .global_fixture
            .global
            .get_balance_atoms(&test_fixture.payer()),
        0
    );

    Ok(())
}

#[tokio::test]
async fn global_get_balance_not_in_global() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    let payer: Pubkey = test_fixture.payer();

    test_fixture.global_fixture.reload().await;
    let global_dynamic_account: DynamicAccount<GlobalFixed, Vec<u8>> =
        test_fixture.global_fixture.global;

    let balance_atoms: GlobalAtoms = global_dynamic_account.get_balance_atoms(&payer);
    assert_eq!(balance_atoms, GlobalAtoms::ZERO);
    Ok(())
}

#[tokio::test]
async fn global_run_out_of_seats() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();

    for _ in 0..MAX_GLOBAL_SEATS {
        let keypair: Keypair = Keypair::new();
        // Fund gas and fee for the account.
        let _ = send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[
                transfer(
                    &payer,
                    &keypair.pubkey(),
                    10_000_000_u64 + 10_u64 * 2_039_280_u64 as u64,
                ),
                global_add_trader_instruction(&test_fixture.global_fixture.key, &keypair.pubkey()),
            ],
            Some(&payer),
            &[&payer_keypair, &keypair],
        )
        .await;
    }

    // Last one doesnt work.
    assert!(test_fixture.global_add_trader().await.is_err());

    Ok(())
}

#[tokio::test]
async fn global_evict() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;

    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();

    for _ in 0..MAX_GLOBAL_SEATS - 1 {
        let new_keypair: Keypair = Keypair::new();
        let token_account_keypair: Keypair = Keypair::new();
        let token_account_fixture: TokenAccountFixture = TokenAccountFixture::new_with_keypair(
            Rc::clone(&test_fixture.context),
            &test_fixture.global_fixture.mint_key,
            &new_keypair.pubkey(),
            &token_account_keypair,
        )
        .await;
        test_fixture
            .usdc_mint_fixture
            .mint_to(&token_account_fixture.key, 1_000_000)
            .await;

        // There are bugs with the first tx looking like it fails but actually
        // succeeds. Just continue on.
        let _ = send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[
                transfer(&payer, &new_keypair.pubkey(), 10_000_000),
                global_add_trader_instruction(
                    &test_fixture.global_fixture.key,
                    &new_keypair.pubkey(),
                ),
                global_deposit_instruction(
                    &test_fixture.global_fixture.mint_key,
                    &new_keypair.pubkey(),
                    &token_account_fixture.key,
                    &spl_token::id(),
                    1_000_000,
                ),
            ],
            Some(&payer_keypair.pubkey()),
            &[&payer_keypair, &new_keypair],
        )
        .await;
    }

    // Adds global for `payer`
    test_fixture.global_add_trader().await?;

    // Add an order for the user that will be evicted.
    test_fixture.claim_seat().await?;
    test_fixture.global_deposit(1_000).await?;

    test_fixture
        .batch_update_with_global_for_keypair(
            None,
            vec![],
            vec![PlaceOrderParams::new(
                100,
                1,
                0,
                true,
                OrderType::Global,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &test_fixture.payer_keypair().insecure_clone(),
        )
        .await?;

    let evictee_account_keypair: Keypair = Keypair::new();
    let evictee_account_fixture: TokenAccountFixture = TokenAccountFixture::new_with_keypair(
        Rc::clone(&test_fixture.context),
        &test_fixture.global_fixture.mint_key,
        &payer,
        &evictee_account_keypair,
    )
    .await;

    let evictor_account_keypair: Keypair = Keypair::new();
    let evictor_account_fixture: TokenAccountFixture = TokenAccountFixture::new_with_keypair(
        Rc::clone(&test_fixture.context),
        &test_fixture.global_fixture.mint_key,
        &test_fixture.second_keypair.pubkey(),
        &evictor_account_keypair,
    )
    .await;
    test_fixture
        .usdc_mint_fixture
        .mint_to(&evictor_account_fixture.key, 1_000_000)
        .await;

    // Second keypair is evicting first.
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[global_evict_instruction(
            &test_fixture.global_fixture.mint_key,
            &test_fixture.second_keypair.pubkey(),
            &evictor_account_fixture.key,
            &evictee_account_fixture.key,
            &spl_token::id(),
            1_000_000,
        )],
        Some(&test_fixture.second_keypair.pubkey()),
        &[&test_fixture.second_keypair.insecure_clone()],
    )
    .await?;

    test_fixture.global_fixture.reload().await;
    let global_dynamic_account: &DynamicAccount<GlobalFixed, Vec<u8>> =
        &test_fixture.global_fixture.global;

    // First got emptied.
    let balance_atoms: GlobalAtoms = global_dynamic_account.get_balance_atoms(&payer);
    assert_eq!(balance_atoms, GlobalAtoms::ZERO);
    let balance_atoms: GlobalAtoms =
        global_dynamic_account.get_balance_atoms(&test_fixture.second_keypair.pubkey());
    assert_eq!(balance_atoms, GlobalAtoms::new(1_000_000));

    // This verifies that the old order is not matchable because it would match
    // here.
    test_fixture
        .claim_seat_for_keypair(&test_fixture.second_keypair.insecure_clone())
        .await?;
    test_fixture
        .deposit_for_keypair(
            Token::SOL,
            1_000_000,
            &test_fixture.second_keypair.insecure_clone(),
        )
        .await?;
    test_fixture
        .batch_update_with_global_for_keypair(
            None,
            vec![],
            vec![PlaceOrderParams::new(
                100,
                1,
                0,
                false,
                OrderType::ImmediateOrCancel,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &test_fixture.payer_keypair().insecure_clone(),
        )
        .await?;
    test_fixture.global_fixture.reload().await;
    let global_dynamic_account: &DynamicAccount<GlobalFixed, Vec<u8>> =
        &test_fixture.global_fixture.global;
    let balance_atoms: GlobalAtoms =
        global_dynamic_account.get_balance_atoms(&test_fixture.second_keypair.pubkey());
    assert_eq!(balance_atoms, GlobalAtoms::new(1_000_000));

    // No match on IOC
    test_fixture.market_fixture.reload().await;
    assert_eq!(
        test_fixture
            .market_fixture
            .get_base_balance_atoms(&test_fixture.second_keypair.pubkey())
            .await,
        1_000_000
    );

    Ok(())
}

#[tokio::test]
async fn global_evict_22() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;

    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();

    let mut usdc_mint_fixture: MintFixture =
        MintFixture::new_with_version(Rc::clone(&test_fixture.context), Some(9), true).await;
    let mut global_fixture: GlobalFixture = GlobalFixture::new_with_token_program(
        Rc::clone(&test_fixture.context),
        &usdc_mint_fixture.key,
        &spl_token_2022::id(),
    )
    .await;

    for _ in 0..MAX_GLOBAL_SEATS - 1 {
        let new_keypair: Keypair = Keypair::new();
        let token_account_keypair: Keypair = Keypair::new();
        let token_account_fixture: TokenAccountFixture =
            TokenAccountFixture::new_with_keypair_2022(
                Rc::clone(&test_fixture.context),
                &usdc_mint_fixture.key,
                &new_keypair.pubkey(),
                &token_account_keypair,
            )
            .await;
        usdc_mint_fixture
            .mint_to_2022(&token_account_fixture.key, 1_000_000)
            .await;

        // There are bugs with the first tx looking like it fails but actually
        // succeeds. Just continue on.
        let _ = send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[
                transfer(&payer, &new_keypair.pubkey(), 10_000_000),
                global_add_trader_instruction(&global_fixture.key, &new_keypair.pubkey()),
                global_deposit_instruction(
                    &global_fixture.mint_key,
                    &new_keypair.pubkey(),
                    &token_account_fixture.key,
                    &spl_token_2022::id(),
                    1_000_000,
                ),
            ],
            Some(&payer_keypair.pubkey()),
            &[&payer_keypair, &new_keypair],
        )
        .await;
    }

    // Adds global for `payer` which is the evictee
    let evictee_account_keypair: Keypair = Keypair::new();
    let evictee_account_fixture: TokenAccountFixture = TokenAccountFixture::new_with_keypair_2022(
        Rc::clone(&test_fixture.context),
        &usdc_mint_fixture.key,
        &payer,
        &evictee_account_keypair,
    )
    .await;
    usdc_mint_fixture
        .mint_to_2022(&evictee_account_fixture.key, 1_000)
        .await;
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[
            global_add_trader_instruction(&global_fixture.key, &payer),
            global_deposit_instruction(
                &global_fixture.mint_key,
                &payer,
                &evictee_account_fixture.key,
                &spl_token_2022::id(),
                1_000,
            ),
        ],
        Some(&payer_keypair.pubkey()),
        &[&payer_keypair],
    )
    .await?;

    let evictor_account_keypair: Keypair = Keypair::new();
    let evictor_account_fixture: TokenAccountFixture = TokenAccountFixture::new_with_keypair_2022(
        Rc::clone(&test_fixture.context),
        &usdc_mint_fixture.key,
        &test_fixture.second_keypair.pubkey(),
        &evictor_account_keypair,
    )
    .await;
    usdc_mint_fixture
        .mint_to_2022(&evictor_account_fixture.key, 1_000_000)
        .await;

    // Second keypair is evicting first.
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[global_evict_instruction(
            &global_fixture.mint_key,
            &test_fixture.second_keypair.pubkey(),
            &evictor_account_fixture.key,
            &evictee_account_fixture.key,
            &spl_token_2022::id(),
            1_000_000,
        )],
        Some(&test_fixture.second_keypair.pubkey()),
        &[&test_fixture.second_keypair.insecure_clone()],
    )
    .await?;

    global_fixture.reload().await;
    let global_dynamic_account: &DynamicAccount<GlobalFixed, Vec<u8>> = &global_fixture.global;

    // First got emptied.
    let balance_atoms: GlobalAtoms = global_dynamic_account.get_balance_atoms(&payer);
    assert_eq!(balance_atoms, GlobalAtoms::ZERO);
    let balance_atoms: GlobalAtoms =
        global_dynamic_account.get_balance_atoms(&test_fixture.second_keypair.pubkey());
    assert_eq!(balance_atoms, GlobalAtoms::new(1_000_000));

    Ok(())
}

#[tokio::test]
async fn global_clean() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;

    test_fixture.global_add_trader().await?;
    test_fixture.global_deposit(1_000_000).await?;

    test_fixture
        .batch_update_with_global_for_keypair(
            None,
            vec![],
            vec![PlaceOrderParams::new(
                100,
                1,
                0,
                true,
                OrderType::Global,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &test_fixture.payer_keypair().insecure_clone(),
        )
        .await?;

    // Funds still there to back it.
    assert!(send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[global_clean_instruction(
            &test_fixture.global_fixture.key,
            &test_fixture.payer(),
            &test_fixture.market_fixture.key,
            MARKET_BLOCK_SIZE as DataIndex
        ),],
        Some(&test_fixture.payer()),
        &[&test_fixture.payer_keypair().insecure_clone()],
    )
    .await
    .is_err());

    // Clean should succeed.
    test_fixture.global_withdraw(1_000_000).await?;
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[global_clean_instruction(
            &test_fixture.global_fixture.key,
            &test_fixture.payer(),
            &test_fixture.market_fixture.key,
            MARKET_BLOCK_SIZE as DataIndex,
        )],
        Some(&test_fixture.payer()),
        &[&test_fixture.payer_keypair().insecure_clone()],
    )
    .await?;

    Ok(())
}

#[tokio::test]
async fn maintenance_clean() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.global_add_trader().await?;
    test_fixture.deposit(Token::USDC, 100).await?;

    test_fixture
        .batch_update_for_keypair(
            None,
            vec![],
            vec![PlaceOrderParams::new(
                100,
                1,
                0,
                true,
                OrderType::Limit,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &test_fixture.payer_keypair().insecure_clone(),
        )
        .await?;

    test_fixture.advance_time_seconds(14 * 24 * 60 * 60).await;

    // Clean should succeed.
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[global_clean_instruction(
            &test_fixture.global_fixture.key,
            &test_fixture.payer(),
            &test_fixture.market_fixture.key,
            MARKET_BLOCK_SIZE as DataIndex,
        )],
        Some(&test_fixture.payer()),
        &[&test_fixture.payer_keypair().insecure_clone()],
    )
    .await?;

    test_fixture.market_fixture.reload().await;

    let bids = test_fixture.market_fixture.market.get_bids();
    let next = bids.iter::<RestingOrder>().next();
    assert_eq!(next, None);

    Ok(())
}

#[tokio::test]
async fn global_clean_expired_without_global() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;

    test_fixture.global_add_trader().await?;
    test_fixture.global_deposit(1_000_000).await?;

    test_fixture
        .batch_update_with_global_for_keypair(
            None,
            vec![],
            vec![PlaceOrderParams::new(
                100,
                1,
                0,
                true,
                OrderType::Global,
                100,
            )],
            &test_fixture.payer_keypair().insecure_clone(),
        )
        .await?;
    test_fixture.advance_time_seconds(1_000).await;

    test_fixture.deposit(Token::SOL, 1_000_000).await?;

    // Succeeds but does not result in a match because expired. Did not include global.
    test_fixture
        .batch_update_for_keypair(
            None,
            vec![],
            vec![PlaceOrderParams::new(
                100,
                9,
                -1,
                false,
                OrderType::ImmediateOrCancel,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &test_fixture.payer_keypair().insecure_clone(),
        )
        .await?;

    test_fixture.market_fixture.reload().await;
    let orders: Vec<RestingOrder> = test_fixture.market_fixture.get_resting_orders().await;
    // Remove unbacked global order.
    assert_eq!(orders.len(), 0, "Order still on orderbook");

    // No trade happened.
    assert_eq!(
        test_fixture
            .market_fixture
            .get_base_balance_atoms(&test_fixture.payer())
            .await,
        1_000_000
    );
    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_balance_atoms(&test_fixture.payer())
            .await,
        0
    );
    test_fixture.global_fixture.reload().await;
    assert_eq!(
        test_fixture
            .global_fixture
            .global
            .get_balance_atoms(&test_fixture.payer()),
        1_000_000
    );

    Ok(())
}

#[tokio::test]
async fn global_stop_without_global() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;

    test_fixture.global_add_trader().await?;
    test_fixture.global_deposit(1_000_000).await?;

    test_fixture
        .batch_update_with_global_for_keypair(
            None,
            vec![],
            vec![PlaceOrderParams::new(
                100,
                1,
                0,
                true,
                OrderType::Global,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &test_fixture.payer_keypair().insecure_clone(),
        )
        .await?;
    test_fixture.deposit(Token::SOL, 1_000_000).await?;

    // Succeeds but does not result in a match because expired. Did not include global.
    test_fixture
        .batch_update_for_keypair(
            None,
            vec![],
            vec![PlaceOrderParams::new(
                100,
                9,
                -1,
                false,
                OrderType::ImmediateOrCancel,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &test_fixture.payer_keypair().insecure_clone(),
        )
        .await?;

    test_fixture.market_fixture.reload().await;
    let orders: Vec<RestingOrder> = test_fixture.market_fixture.get_resting_orders().await;
    // Did not remove the global order.
    assert_eq!(orders.len(), 1, "Order removed orderbook");

    // No trade happened.
    assert_eq!(
        test_fixture
            .market_fixture
            .get_base_balance_atoms(&test_fixture.payer())
            .await,
        1_000_000
    );
    assert_eq!(
        test_fixture
            .market_fixture
            .get_quote_balance_atoms(&test_fixture.payer())
            .await,
        0
    );
    test_fixture.global_fixture.reload().await;
    assert_eq!(
        test_fixture
            .global_fixture
            .global
            .get_balance_atoms(&test_fixture.payer()),
        1_000_000
    );

    Ok(())
}

#[tokio::test]
async fn global_deposit_with_transfer_fee() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();

    // Create a Token-2022 mint with 10% transfer fee (1000 basis points)
    let transfer_fee_bps: u16 = 1000; // 10%
    let mut mint_fixture: MintFixture = MintFixture::new_with_transfer_fee(
        Rc::clone(&test_fixture.context),
        9, // decimals
        transfer_fee_bps,
    )
    .await;

    let mut global_fixture: GlobalFixture = GlobalFixture::new_with_token_program(
        Rc::clone(&test_fixture.context),
        &mint_fixture.key,
        &spl_token_2022::id(),
    )
    .await;

    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[global_add_trader_instruction(&global_fixture.key, &payer)],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // Create a Token-2022 token account for the mint with transfer fee
    // These accounts need the TransferFeeAmount extension
    let token_account_keypair: Keypair = Keypair::new();
    let token_account_fixture: TokenAccountFixture =
        TokenAccountFixture::new_with_keypair_2022_transfer_fee(
            Rc::clone(&test_fixture.context),
            &mint_fixture.key,
            &payer,
            &token_account_keypair,
        )
        .await;

    // Mint 1_000_000 tokens
    let deposit_amount: u64 = 1_000_000;
    mint_fixture
        .mint_to_2022(&token_account_fixture.key, deposit_amount)
        .await;

    // Deposit to global
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[global_deposit_instruction(
            &mint_fixture.key,
            &payer,
            &token_account_fixture.key,
            &spl_token_2022::id(),
            deposit_amount,
        )],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    global_fixture.reload().await;
    let global_dynamic_account: &DynamicAccount<GlobalFixed, Vec<u8>> = &global_fixture.global;

    // With 10% transfer fee, the vault should receive 90% of the deposited amount
    // Transfer fee calculation: fee = amount * fee_bps / 10000
    // fee = 1_000_000 * 1000 / 10000 = 100_000
    // amount_after_fee = 1_000_000 - 100_000 = 900_000
    let expected_balance: u64 = deposit_amount - (deposit_amount * transfer_fee_bps as u64 / 10000);
    let balance_atoms: GlobalAtoms = global_dynamic_account.get_balance_atoms(&payer);
    assert_eq!(
        balance_atoms,
        GlobalAtoms::new(expected_balance),
        "Balance should be {} after 10% transfer fee, but got {}",
        expected_balance,
        balance_atoms.as_u64()
    );

    Ok(())
}

#[tokio::test]
async fn global_crash_without_global() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;

    test_fixture.global_add_trader().await?;
    test_fixture.global_deposit(1_000_000).await?;

    test_fixture
        .batch_update_with_global_for_keypair(
            None,
            vec![],
            vec![PlaceOrderParams::new(
                100,
                1,
                0,
                true,
                OrderType::Global,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            &test_fixture.payer_keypair().insecure_clone(),
        )
        .await?;
    test_fixture.deposit(Token::SOL, 1_000_000).await?;

    assert!(
        test_fixture
            .batch_update_for_keypair(
                None,
                vec![],
                vec![PlaceOrderParams::new(
                    100,
                    9,
                    -1,
                    false,
                    OrderType::Limit,
                    NO_EXPIRATION_LAST_VALID_SLOT,
                )],
                &test_fixture.payer_keypair().insecure_clone(),
            )
            .await
            .is_err(),
        "Walked past a global without global account and left crossed book"
    );

    Ok(())
}
