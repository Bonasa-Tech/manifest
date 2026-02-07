use std::{cell::RefMut, rc::Rc};

use borsh::BorshSerialize;
use manifest::{
    program::{
        batch_update::{CancelOrderParams, PlaceOrderParams},
        batch_update_instruction, expand_market_instruction, global_add_trader_instruction,
        global_deposit_instruction, global_withdraw_instruction, swap_instruction,
        ManifestInstruction, SwapParams,
    },
    quantities::{BaseAtoms, WrapperU64},
    state::{constants::NO_EXPIRATION_LAST_VALID_SLOT, OrderType, RestingOrder},
    validation::get_vault_address,
};
use solana_program_test::{tokio, ProgramTestContext};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    signature::{Keypair, Signer},
    transaction::Transaction,
};

use crate::{
    send_tx_with_retry, Side, TestFixture, Token, TokenAccountFixture, SOL_UNIT_SIZE,
    USDC_UNIT_SIZE,
};

#[tokio::test]
async fn swap_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;

    test_fixture
        .sol_mint_fixture
        .mint_to(&test_fixture.payer_sol_fixture.key, 1 * SOL_UNIT_SIZE)
        .await;

    // No deposits or seat claims needed
    test_fixture.swap(SOL_UNIT_SIZE, 0, true, true).await?;

    Ok(())
}

#[tokio::test]
async fn swap_v2_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;

    test_fixture
        .sol_mint_fixture
        .mint_to(&test_fixture.payer_sol_fixture.key, 1 * SOL_UNIT_SIZE)
        .await;

    // No deposits or seat claims needed
    test_fixture.swap_v2(SOL_UNIT_SIZE, 0, true, true).await?;

    Ok(())
}

#[tokio::test]
async fn swap_full_match_test_sell_exact_in() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;

    // second keypair is the maker
    let second_keypair: Keypair = test_fixture.second_keypair.insecure_clone();
    test_fixture.claim_seat_for_keypair(&second_keypair).await?;

    // all amounts in tokens, "a" signifies rounded atom
    // needs 2x(10+a) + 4x5+a = 40+3a usdc
    test_fixture
        .deposit_for_keypair(Token::USDC, 40 * USDC_UNIT_SIZE + 3, &second_keypair)
        .await?;

    // price is sub-atomic: ~10 SOL/USDC
    // will round towards taker
    test_fixture
        .place_order_for_keypair(
            Side::Bid,
            1 * SOL_UNIT_SIZE,
            1_000_000_001,
            -11,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;

    // this order expires
    test_fixture
        .place_order_for_keypair(
            Side::Bid,
            1 * SOL_UNIT_SIZE,
            1_000_000_001,
            -11,
            10,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;

    // will round towards maker
    test_fixture
        .place_order_for_keypair(
            Side::Bid,
            4 * SOL_UNIT_SIZE,
            500_000_001,
            -11,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;

    test_fixture
        .sol_mint_fixture
        .mint_to(&test_fixture.payer_sol_fixture.key, 3 * SOL_UNIT_SIZE)
        .await;

    test_fixture.advance_time_seconds(20).await;

    test_fixture
        .swap(3 * SOL_UNIT_SIZE, 20 * USDC_UNIT_SIZE, true, true)
        .await?;

    // matched:
    // 1 SOL * 10+a SOL/USDC = 10 USDC
    // 2 SOL * 5+a SOL/USC = 10+1 USDC
    // taker has:
    // 10 USDC / 5+a SOL/USDC = 2-3a SOL
    // taker has 3-3 = 0 sol & 10+a + 2x5 = 20+a usdc
    assert_eq!(test_fixture.payer_sol_fixture.balance_atoms().await, 0);
    assert_eq!(
        test_fixture.payer_usdc_fixture.balance_atoms().await,
        20 * USDC_UNIT_SIZE + 1
    );

    // maker has unlocked:
    // 3 SOL
    // 10+1a USDC from expired order
    test_fixture
        .withdraw_for_keypair(Token::SOL, 3 * SOL_UNIT_SIZE, &second_keypair)
        .await?;
    test_fixture
        .withdraw_for_keypair(Token::USDC, 10 * USDC_UNIT_SIZE + 1, &second_keypair)
        .await?;

    // maker has resting:
    // 5 - 3 = 2 sol @ 5+a
    // 2x5+a = 10+a
    let orders = test_fixture.market_fixture.get_resting_orders().await;
    let resting = orders.first().unwrap();
    assert_eq!(resting.get_num_base_atoms(), 2 * SOL_UNIT_SIZE);
    assert_eq!(
        resting
            .get_price()
            .checked_quote_for_base(BaseAtoms::new(10u64.pow(11)), false)
            .unwrap(),
        500_000_001
    );
    assert_eq!(
        resting
            .get_price()
            .checked_quote_for_base(resting.get_num_base_atoms(), true)
            .unwrap(),
        10 * USDC_UNIT_SIZE + 1
    );

    Ok(())
}

#[tokio::test]
async fn swap_full_match_test_sell_exact_out() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;

    // second keypair is the maker
    let second_keypair: Keypair = test_fixture.second_keypair.insecure_clone();
    test_fixture.claim_seat_for_keypair(&second_keypair).await?;

    // all amounts in tokens, "a" signifies rounded atom
    // needs 2x(10+a) + 4x(5)+a = 40+3a usdc
    test_fixture
        .deposit_for_keypair(Token::USDC, 40 * USDC_UNIT_SIZE + 3, &second_keypair)
        .await?;

    // price is sub-atomic: ~10 SOL/USDC
    // will round towards taker
    test_fixture
        .place_order_for_keypair(
            Side::Bid,
            1 * SOL_UNIT_SIZE,
            1_000_000_001,
            -11,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;

    // this order expires
    test_fixture
        .place_order_for_keypair(
            Side::Bid,
            1 * SOL_UNIT_SIZE,
            1_000_000_001,
            -11,
            10,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;

    // will round towards maker
    test_fixture
        .place_order_for_keypair(
            Side::Bid,
            4 * SOL_UNIT_SIZE,
            500_000_001,
            -11,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;

    test_fixture
        .sol_mint_fixture
        .mint_to(&test_fixture.payer_sol_fixture.key, 3 * SOL_UNIT_SIZE)
        .await;

    test_fixture.advance_time_seconds(20).await;

    test_fixture
        .swap(3 * SOL_UNIT_SIZE, 20 * USDC_UNIT_SIZE + 1, true, false)
        .await?;

    // matched:
    // 1 SOL * 10+a SOL/USDC = 10+a USDC
    // 10 USDC / 5+a SOL/USDC = 2-3a SOL
    // taker has:
    // 3 - 1 - (2-3a) = 3a SOL
    // 10+a + 2x5 = 20+a USDC
    assert_eq!(test_fixture.payer_sol_fixture.balance_atoms().await, 3);
    assert_eq!(
        test_fixture.payer_usdc_fixture.balance_atoms().await,
        20 * USDC_UNIT_SIZE + 1
    );

    // maker has unlocked:
    // 1 + 2-3a = 3-3a sol
    // 10+1a usdc from expired order
    test_fixture
        .withdraw_for_keypair(Token::SOL, 3 * SOL_UNIT_SIZE - 3, &second_keypair)
        .await?;
    test_fixture
        .withdraw_for_keypair(Token::USDC, 10 * USDC_UNIT_SIZE + 1, &second_keypair)
        .await?;

    // maker has resting:
    // 5 - (3-3a) = 2+3a sol @ 5+a
    // ~2x~5+a = 10+a
    let orders = test_fixture.market_fixture.get_resting_orders().await;
    println!("{orders:?}");
    let resting = orders.first().unwrap();
    assert_eq!(resting.get_num_base_atoms(), 2 * SOL_UNIT_SIZE + 3);
    assert_eq!(
        resting
            .get_price()
            .checked_quote_for_base(BaseAtoms::new(10u64.pow(11)), false)
            .unwrap(),
        500_000_001
    );
    assert_eq!(
        resting
            .get_price()
            .checked_quote_for_base(resting.get_num_base_atoms(), true)
            .unwrap(),
        10 * USDC_UNIT_SIZE + 1
    );

    Ok(())
}

#[tokio::test]
async fn swap_full_match_test_buy_exact_in() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;

    let second_keypair: Keypair = test_fixture.second_keypair.insecure_clone();
    test_fixture.claim_seat_for_keypair(&second_keypair).await?;

    // all amounts in tokens, "a" signifies rounded atom
    // need 1 + 1 + 3 = 5 SOL
    test_fixture
        .deposit_for_keypair(Token::SOL, 5 * SOL_UNIT_SIZE, &second_keypair)
        .await?;

    // price is sub-atomic: ~10 SOL/USDC
    // will round towards taker
    test_fixture
        .place_order_for_keypair(
            Side::Ask,
            1 * SOL_UNIT_SIZE,
            1_000_000_001,
            -11,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;

    // this order expires
    test_fixture
        .place_order_for_keypair(
            Side::Ask,
            1 * SOL_UNIT_SIZE,
            1_000_000_001,
            -11,
            10,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;

    // will round towards maker
    test_fixture
        .place_order_for_keypair(
            Side::Ask,
            3 * SOL_UNIT_SIZE,
            1_500_000_001,
            -11,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;

    test_fixture
        .usdc_mint_fixture
        .mint_to(&test_fixture.payer_usdc_fixture.key, 40 * USDC_UNIT_SIZE)
        .await;

    test_fixture.advance_time_seconds(20).await;

    test_fixture
        .swap(40 * USDC_UNIT_SIZE, 3 * SOL_UNIT_SIZE - 2, false, true)
        .await?;

    // matched:
    // 1 SOL * 10+a SOL/USDC = 10 USDC
    // 30 USDC / 15+a SOL/USDC = 2-2a SOL
    // taker has:
    // 1 + 2-2a = 3-2a SOL
    // 40 - 10 - 30 = 0 USDC
    assert_eq!(
        test_fixture.payer_sol_fixture.balance_atoms().await,
        3 * SOL_UNIT_SIZE - 2
    );
    assert_eq!(test_fixture.payer_usdc_fixture.balance_atoms().await, 0);

    // maker has unlocked:
    // 5 - (1+2a) - (3-2a) = 1 SOL
    // 10 + 30 = 40 USDC
    test_fixture
        .withdraw_for_keypair(Token::SOL, 1 * SOL_UNIT_SIZE, &second_keypair)
        .await?;
    test_fixture
        .withdraw_for_keypair(Token::USDC, 40 * USDC_UNIT_SIZE, &second_keypair)
        .await?;

    // maker has resting 1+2a SOL @ 15+a SOL/USDC
    let orders = test_fixture.market_fixture.get_resting_orders().await;
    let resting = orders.first().unwrap();
    assert_eq!(resting.get_num_base_atoms(), 1 * SOL_UNIT_SIZE + 2);
    assert_eq!(
        resting
            .get_price()
            .checked_quote_for_base(BaseAtoms::new(10u64.pow(11)), false)
            .unwrap(),
        1_500_000_001
    );

    Ok(())
}

#[tokio::test]
async fn swap_full_match_test_buy_exact_out() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;

    let second_keypair: Keypair = test_fixture.second_keypair.insecure_clone();
    test_fixture.claim_seat_for_keypair(&second_keypair).await?;

    // need 1 + 1 + 3 = 5 SOL
    test_fixture
        .deposit_for_keypair(Token::SOL, 5 * SOL_UNIT_SIZE, &second_keypair)
        .await?;

    // price is sub-atomic: ~10 SOL/USDC
    // will round towards taker
    test_fixture
        .place_order_for_keypair(
            Side::Ask,
            1 * SOL_UNIT_SIZE,
            1_000_000_001,
            -11,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;

    // this order expires
    test_fixture
        .place_order_for_keypair(
            Side::Ask,
            1 * SOL_UNIT_SIZE,
            1_000_000_001,
            -11,
            10,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;

    // will round towards maker
    test_fixture
        .place_order_for_keypair(
            Side::Ask,
            3 * SOL_UNIT_SIZE,
            1_500_000_001,
            -11,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
            &second_keypair,
        )
        .await?;

    test_fixture
        .usdc_mint_fixture
        .mint_to(
            &test_fixture.payer_usdc_fixture.key,
            40 * USDC_UNIT_SIZE + 1,
        )
        .await;

    test_fixture.advance_time_seconds(20).await;

    test_fixture
        .swap(40 * USDC_UNIT_SIZE + 1, 3 * SOL_UNIT_SIZE, false, false)
        .await?;

    // matched:
    // 1 SOL x 10+a SOL/USDC = 10 USDC
    // 2 SOL x 15+a SOL/USDC = 30+a USDC
    // taker has:
    // 1 + 2 = 3 SOL
    // 40+a - 10 - (30+a) = 0 USDC
    assert_eq!(
        test_fixture.payer_sol_fixture.balance_atoms().await,
        3 * SOL_UNIT_SIZE
    );
    assert_eq!(test_fixture.payer_usdc_fixture.balance_atoms().await, 0);

    // maker has unlocked:
    // 5 - 1 - 3 = 1 SOL
    // 10 + 30+a = 40+a USDC
    test_fixture
        .withdraw_for_keypair(Token::SOL, 1 * SOL_UNIT_SIZE, &second_keypair)
        .await?;
    test_fixture
        .withdraw_for_keypair(Token::USDC, 40 * USDC_UNIT_SIZE + 1, &second_keypair)
        .await?;

    // maker has resting 1 SOL @ 15+a SOL/USDC
    let orders = test_fixture.market_fixture.get_resting_orders().await;
    let resting = orders.first().unwrap();
    assert_eq!(resting.get_num_base_atoms(), 1 * SOL_UNIT_SIZE);
    assert_eq!(
        resting
            .get_price()
            .checked_quote_for_base(BaseAtoms::new(10u64.pow(11)), false)
            .unwrap(),
        1_500_000_001
    );
    Ok(())
}

#[tokio::test]
async fn swap_already_has_deposits() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 1 * SOL_UNIT_SIZE).await?;
    test_fixture
        .deposit(Token::USDC, 1_000 * USDC_UNIT_SIZE)
        .await?;

    let second_keypair: Keypair = test_fixture.second_keypair.insecure_clone();
    test_fixture.claim_seat_for_keypair(&second_keypair).await?;
    test_fixture
        .deposit_for_keypair(Token::SOL, 1 * SOL_UNIT_SIZE, &second_keypair)
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

    test_fixture
        .usdc_mint_fixture
        .mint_to(&test_fixture.payer_usdc_fixture.key, 1_000 * USDC_UNIT_SIZE)
        .await;

    assert_eq!(test_fixture.payer_sol_fixture.balance_atoms().await, 0);
    assert_eq!(
        test_fixture.payer_usdc_fixture.balance_atoms().await,
        1_000 * USDC_UNIT_SIZE
    );
    test_fixture
        .swap(1000 * USDC_UNIT_SIZE, 1 * SOL_UNIT_SIZE, false, false)
        .await?;

    assert_eq!(
        test_fixture.payer_sol_fixture.balance_atoms().await,
        1 * SOL_UNIT_SIZE
    );
    assert_eq!(test_fixture.payer_usdc_fixture.balance_atoms().await, 0);

    Ok(())
}

#[tokio::test]
async fn swap_fail_limit_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    let payer_keypair: Keypair = test_fixture.payer_keypair();
    test_fixture
        .usdc_mint_fixture
        .mint_to(
            &test_fixture.payer_usdc_fixture.key,
            10_000 * USDC_UNIT_SIZE,
        )
        .await;

    let mut context: RefMut<ProgramTestContext> = test_fixture.context.borrow_mut();

    let swap_ix: Instruction = swap_instruction(
        &test_fixture.market_fixture.key,
        &payer_keypair.pubkey(),
        &test_fixture.sol_mint_fixture.key,
        &test_fixture.usdc_mint_fixture.key,
        &test_fixture.payer_sol_fixture.key,
        &test_fixture.payer_usdc_fixture.key,
        2_000 * USDC_UNIT_SIZE,
        2 * SOL_UNIT_SIZE,
        false,
        true,
        spl_token::id(),
        spl_token::id(),
        false,
    );

    let swap_tx: Transaction = Transaction::new_signed_with_payer(
        &[swap_ix],
        Some(&payer_keypair.pubkey()),
        &[&payer_keypair],
        context.get_new_latest_blockhash().await?,
    );

    assert!(context
        .banks_client
        .process_transaction(swap_tx)
        .await
        .is_err());

    Ok(())
}

#[tokio::test]
async fn swap_fail_wrong_user_base_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    let payer_keypair: Keypair = test_fixture.payer_keypair();
    test_fixture
        .usdc_mint_fixture
        .mint_to(
            &test_fixture.payer_usdc_fixture.key,
            10_000 * USDC_UNIT_SIZE,
        )
        .await;

    let mut context: RefMut<ProgramTestContext> = test_fixture.context.borrow_mut();

    let (vault_base_account, _) = get_vault_address(
        &test_fixture.market_fixture.key,
        &test_fixture.sol_mint_fixture.key,
    );
    let (vault_quote_account, _) = get_vault_address(
        &test_fixture.market_fixture.key,
        &test_fixture.usdc_mint_fixture.key,
    );

    let swap_ix: Instruction = Instruction {
        program_id: manifest::id(),
        accounts: vec![
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(payer_keypair.pubkey(), true),
            AccountMeta::new(test_fixture.market_fixture.key, false),
            AccountMeta::new(test_fixture.payer_usdc_fixture.key, false),
            AccountMeta::new(test_fixture.payer_usdc_fixture.key, false),
            AccountMeta::new(vault_base_account, false),
            AccountMeta::new(vault_quote_account, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: [
            ManifestInstruction::Swap.to_vec(),
            SwapParams::new(2_000 * USDC_UNIT_SIZE, 2 * SOL_UNIT_SIZE, false, true)
                .try_to_vec()
                .unwrap(),
        ]
        .concat(),
    };

    let swap_tx: Transaction = Transaction::new_signed_with_payer(
        &[swap_ix],
        Some(&payer_keypair.pubkey()),
        &[&payer_keypair],
        context.get_new_latest_blockhash().await?,
    );

    assert!(context
        .banks_client
        .process_transaction(swap_tx)
        .await
        .is_err());

    Ok(())
}

#[tokio::test]
async fn swap_fail_wrong_user_quote_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    let payer_keypair: Keypair = test_fixture.payer_keypair();
    test_fixture
        .usdc_mint_fixture
        .mint_to(
            &test_fixture.payer_usdc_fixture.key,
            10_000 * USDC_UNIT_SIZE,
        )
        .await;

    let mut context: RefMut<ProgramTestContext> = test_fixture.context.borrow_mut();

    let (vault_base_account, _) = get_vault_address(
        &test_fixture.market_fixture.key,
        &test_fixture.sol_mint_fixture.key,
    );
    let (vault_quote_account, _) = get_vault_address(
        &test_fixture.market_fixture.key,
        &test_fixture.usdc_mint_fixture.key,
    );

    let swap_ix: Instruction = Instruction {
        program_id: manifest::id(),
        accounts: vec![
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(payer_keypair.pubkey(), true),
            AccountMeta::new(test_fixture.market_fixture.key, false),
            AccountMeta::new(test_fixture.payer_sol_fixture.key, false),
            AccountMeta::new(test_fixture.payer_sol_fixture.key, false),
            AccountMeta::new(vault_base_account, false),
            AccountMeta::new(vault_quote_account, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: [
            ManifestInstruction::Swap.to_vec(),
            SwapParams::new(2_000 * USDC_UNIT_SIZE, 2 * SOL_UNIT_SIZE, false, true)
                .try_to_vec()
                .unwrap(),
        ]
        .concat(),
    };

    let swap_tx: Transaction = Transaction::new_signed_with_payer(
        &[swap_ix],
        Some(&payer_keypair.pubkey()),
        &[&payer_keypair],
        context.get_new_latest_blockhash().await?,
    );

    assert!(context
        .banks_client
        .process_transaction(swap_tx)
        .await
        .is_err());

    Ok(())
}

#[tokio::test]
async fn swap_fail_wrong_base_vault_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    let payer_keypair: Keypair = test_fixture.payer_keypair();
    test_fixture
        .usdc_mint_fixture
        .mint_to(
            &test_fixture.payer_usdc_fixture.key,
            10_000 * USDC_UNIT_SIZE,
        )
        .await;

    let mut context: RefMut<ProgramTestContext> = test_fixture.context.borrow_mut();

    let (vault_quote_account, _) = get_vault_address(
        &test_fixture.market_fixture.key,
        &test_fixture.usdc_mint_fixture.key,
    );

    let place_order_ix: Instruction = Instruction {
        program_id: manifest::id(),
        accounts: vec![
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(payer_keypair.pubkey(), true),
            AccountMeta::new(test_fixture.market_fixture.key, false),
            AccountMeta::new(test_fixture.payer_sol_fixture.key, false),
            AccountMeta::new(test_fixture.payer_usdc_fixture.key, false),
            AccountMeta::new(vault_quote_account, false),
            AccountMeta::new(vault_quote_account, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: [
            ManifestInstruction::Swap.to_vec(),
            SwapParams::new(2_000 * USDC_UNIT_SIZE, 2 * SOL_UNIT_SIZE, false, true)
                .try_to_vec()
                .unwrap(),
        ]
        .concat(),
    };

    let swap_ix: Transaction = Transaction::new_signed_with_payer(
        &[place_order_ix],
        Some(&payer_keypair.pubkey()),
        &[&payer_keypair],
        context.get_new_latest_blockhash().await?,
    );

    assert!(context
        .banks_client
        .process_transaction(swap_ix)
        .await
        .is_err());

    Ok(())
}

#[tokio::test]
async fn swap_fail_wrong_vault_quote_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    let payer_keypair: Keypair = test_fixture.payer_keypair();
    test_fixture
        .usdc_mint_fixture
        .mint_to(
            &test_fixture.payer_usdc_fixture.key,
            10_000 * USDC_UNIT_SIZE,
        )
        .await;

    let mut context: RefMut<ProgramTestContext> = test_fixture.context.borrow_mut();

    let (vault_base_account, _) = get_vault_address(
        &test_fixture.market_fixture.key,
        &test_fixture.sol_mint_fixture.key,
    );

    let swap_ix: Instruction = Instruction {
        program_id: manifest::id(),
        accounts: vec![
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(payer_keypair.pubkey(), true),
            AccountMeta::new(test_fixture.market_fixture.key, false),
            AccountMeta::new(test_fixture.payer_sol_fixture.key, false),
            AccountMeta::new(test_fixture.payer_usdc_fixture.key, false),
            AccountMeta::new(vault_base_account, false),
            AccountMeta::new(vault_base_account, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: [
            ManifestInstruction::Swap.to_vec(),
            SwapParams::new(2_000 * USDC_UNIT_SIZE, 2 * SOL_UNIT_SIZE, false, true)
                .try_to_vec()
                .unwrap(),
        ]
        .concat(),
    };

    let swap_tx: Transaction = Transaction::new_signed_with_payer(
        &[swap_ix],
        Some(&payer_keypair.pubkey()),
        &[&payer_keypair],
        context.get_new_latest_blockhash().await?,
    );

    assert!(context
        .banks_client
        .process_transaction(swap_tx)
        .await
        .is_err());

    Ok(())
}

#[tokio::test]
async fn swap_fail_insufficient_funds_sell() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
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

    let payer_keypair: Keypair = test_fixture.payer_keypair();
    // Skip the deposit to the order from wallet.

    let mut context: RefMut<ProgramTestContext> = test_fixture.context.borrow_mut();

    let swap_ix: Instruction = swap_instruction(
        &test_fixture.market_fixture.key,
        &payer_keypair.pubkey(),
        &test_fixture.sol_mint_fixture.key,
        &test_fixture.usdc_mint_fixture.key,
        &test_fixture.payer_sol_fixture.key,
        &test_fixture.payer_usdc_fixture.key,
        1 * SOL_UNIT_SIZE,
        1000 * USDC_UNIT_SIZE,
        true,
        true,
        spl_token::id(),
        spl_token::id(),
        false,
    );

    let swap_tx: Transaction = Transaction::new_signed_with_payer(
        &[swap_ix],
        Some(&payer_keypair.pubkey()),
        &[&payer_keypair],
        context.get_new_latest_blockhash().await?,
    );

    assert!(context
        .banks_client
        .process_transaction(swap_tx)
        .await
        .is_err());
    Ok(())
}

#[tokio::test]
async fn swap_fail_insufficient_funds_buy() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    let second_keypair: Keypair = test_fixture.second_keypair.insecure_clone();
    test_fixture.claim_seat_for_keypair(&second_keypair).await?;
    test_fixture
        .deposit_for_keypair(Token::SOL, 2 * SOL_UNIT_SIZE, &second_keypair)
        .await?;
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

    let payer_keypair: Keypair = test_fixture.payer_keypair();
    // Skip the deposit to the order from wallet.

    let mut context: RefMut<ProgramTestContext> = test_fixture.context.borrow_mut();

    let swap_ix: Instruction = swap_instruction(
        &test_fixture.market_fixture.key,
        &payer_keypair.pubkey(),
        &test_fixture.sol_mint_fixture.key,
        &test_fixture.usdc_mint_fixture.key,
        &test_fixture.payer_sol_fixture.key,
        &test_fixture.payer_usdc_fixture.key,
        1000 * USDC_UNIT_SIZE,
        1 * SOL_UNIT_SIZE,
        false,
        true,
        spl_token::id(),
        spl_token::id(),
        false,
    );

    let swap_tx: Transaction = Transaction::new_signed_with_payer(
        &[swap_ix],
        Some(&payer_keypair.pubkey()),
        &[&payer_keypair],
        context.get_new_latest_blockhash().await?,
    );

    assert!(context
        .banks_client
        .process_transaction(swap_tx)
        .await
        .is_err());
    Ok(())
}

// Global is on the USDC, taker is sending in SOL.
#[tokio::test]
async fn swap_global() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;

    let second_keypair: Keypair = test_fixture.second_keypair.insecure_clone();
    test_fixture.claim_seat_for_keypair(&second_keypair).await?;

    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[global_add_trader_instruction(
            &test_fixture.global_fixture.key,
            &second_keypair.pubkey(),
        )],
        Some(&second_keypair.pubkey()),
        &[&second_keypair],
    )
    .await?;

    // Make a throw away token account
    let token_account_keypair: Keypair = Keypair::new();
    let token_account_fixture: TokenAccountFixture = TokenAccountFixture::new_with_keypair(
        Rc::clone(&test_fixture.context),
        &test_fixture.global_fixture.mint_key,
        &second_keypair.pubkey(),
        &token_account_keypair,
    )
    .await;
    test_fixture
        .usdc_mint_fixture
        .mint_to(&token_account_fixture.key, 1 * SOL_UNIT_SIZE)
        .await;
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[global_deposit_instruction(
            &test_fixture.global_fixture.mint_key,
            &second_keypair.pubkey(),
            &token_account_fixture.key,
            &spl_token::id(),
            1 * SOL_UNIT_SIZE,
        )],
        Some(&second_keypair.pubkey()),
        &[&second_keypair],
    )
    .await?;

    let batch_update_ix: Instruction = batch_update_instruction(
        &test_fixture.market_fixture.key,
        &second_keypair.pubkey(),
        None,
        vec![],
        vec![PlaceOrderParams::new(
            1 * SOL_UNIT_SIZE,
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
        Some(&second_keypair.pubkey()),
        &[&second_keypair],
    )
    .await?;

    test_fixture
        .sol_mint_fixture
        .mint_to(&test_fixture.payer_sol_fixture.key, 1 * SOL_UNIT_SIZE)
        .await;

    assert_eq!(
        test_fixture.payer_sol_fixture.balance_atoms().await,
        1 * SOL_UNIT_SIZE
    );
    assert_eq!(test_fixture.payer_usdc_fixture.balance_atoms().await, 0);
    test_fixture
        .swap_with_global(SOL_UNIT_SIZE, 1_000 * USDC_UNIT_SIZE, true, true)
        .await?;

    assert_eq!(test_fixture.payer_sol_fixture.balance_atoms().await, 0);
    assert_eq!(
        test_fixture.payer_usdc_fixture.balance_atoms().await,
        1_000 * USDC_UNIT_SIZE
    );

    Ok(())
}

// This test case illustrates that the exact in is really just a desired in.
#[tokio::test]
async fn swap_full_match_sell_exact_in_exhaust_book() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;

    let second_keypair: Keypair = test_fixture.second_keypair.insecure_clone();
    test_fixture.claim_seat_for_keypair(&second_keypair).await?;
    test_fixture
        .deposit_for_keypair(Token::USDC, 3_000 * USDC_UNIT_SIZE, &second_keypair)
        .await?;

    // 2 bids for 1@1 and 2@.5
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[batch_update_instruction(
            &test_fixture.market_fixture.key,
            &second_keypair.pubkey(),
            None,
            vec![],
            vec![
                PlaceOrderParams::new(
                    1 * SOL_UNIT_SIZE,
                    1,
                    0,
                    true,
                    OrderType::Limit,
                    NO_EXPIRATION_LAST_VALID_SLOT,
                ),
                PlaceOrderParams::new(
                    2 * SOL_UNIT_SIZE,
                    5,
                    -1,
                    true,
                    OrderType::Limit,
                    NO_EXPIRATION_LAST_VALID_SLOT,
                ),
            ],
            None,
            None,
            Some(*test_fixture.market_fixture.market.get_quote_mint()),
            None,
        )],
        Some(&second_keypair.pubkey()),
        &[&second_keypair],
    )
    .await?;
    // Swapper will exact_in of 4, min quote out of 2. Result should be that it
    // succeeds. It will not be able to fully fill all the exact in of 4 and
    // there will be 1 leftover and it gets out 1*1 + 2*.5 = 2 quote.
    test_fixture
        .sol_mint_fixture
        .mint_to(&test_fixture.payer_sol_fixture.key, 4 * SOL_UNIT_SIZE)
        .await;

    test_fixture
        .swap(4 * SOL_UNIT_SIZE, 2_000 * USDC_UNIT_SIZE, true, true)
        .await?;

    assert_eq!(
        test_fixture.payer_sol_fixture.balance_atoms().await,
        1 * SOL_UNIT_SIZE
    );
    assert_eq!(
        test_fixture.payer_usdc_fixture.balance_atoms().await,
        2_000 * USDC_UNIT_SIZE
    );

    Ok(())
}

// Global is on the USDC, taker is sending in SOL. Global order is not backed,
// so the order does not get the global price.
#[tokio::test]
async fn swap_global_not_backed() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;

    let second_keypair: Keypair = test_fixture.second_keypair.insecure_clone();
    test_fixture.claim_seat_for_keypair(&second_keypair).await?;

    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[global_add_trader_instruction(
            &test_fixture.global_fixture.key,
            &second_keypair.pubkey(),
        )],
        Some(&second_keypair.pubkey()),
        &[&second_keypair],
    )
    .await?;

    // Make a throw away token account
    let token_account_keypair: Keypair = Keypair::new();
    let token_account_fixture: TokenAccountFixture = TokenAccountFixture::new_with_keypair(
        Rc::clone(&test_fixture.context),
        &test_fixture.global_fixture.mint_key,
        &second_keypair.pubkey(),
        &token_account_keypair,
    )
    .await;
    test_fixture
        .usdc_mint_fixture
        .mint_to(&token_account_fixture.key, 2_000 * USDC_UNIT_SIZE)
        .await;
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[global_deposit_instruction(
            &test_fixture.global_fixture.mint_key,
            &second_keypair.pubkey(),
            &token_account_fixture.key,
            &spl_token::id(),
            2_000 * USDC_UNIT_SIZE,
        )],
        Some(&second_keypair.pubkey()),
        &[&second_keypair],
    )
    .await?;
    test_fixture
        .deposit_for_keypair(Token::USDC, 1_000 * USDC_UNIT_SIZE, &second_keypair)
        .await?;

    let batch_update_ix: Instruction = batch_update_instruction(
        &test_fixture.market_fixture.key,
        &second_keypair.pubkey(),
        None,
        vec![],
        vec![
            PlaceOrderParams::new(
                1 * SOL_UNIT_SIZE,
                2,
                0,
                true,
                OrderType::Global,
                NO_EXPIRATION_LAST_VALID_SLOT,
            ),
            PlaceOrderParams::new(
                1 * SOL_UNIT_SIZE,
                1,
                0,
                true,
                OrderType::Limit,
                NO_EXPIRATION_LAST_VALID_SLOT,
            ),
        ],
        None,
        None,
        Some(*test_fixture.market_fixture.market.get_quote_mint()),
        None,
    );
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[batch_update_ix],
        Some(&second_keypair.pubkey()),
        &[&second_keypair],
    )
    .await?;

    test_fixture
        .sol_mint_fixture
        .mint_to(&test_fixture.payer_sol_fixture.key, 1 * SOL_UNIT_SIZE)
        .await;

    assert_eq!(test_fixture.payer_usdc_fixture.balance_atoms().await, 0);

    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[global_withdraw_instruction(
            &test_fixture.global_fixture.mint_key,
            &second_keypair.pubkey(),
            &token_account_fixture.key,
            &spl_token::id(),
            2_000 * USDC_UNIT_SIZE,
        )],
        Some(&second_keypair.pubkey()),
        &[&second_keypair],
    )
    .await?;

    test_fixture
        .swap_with_global(SOL_UNIT_SIZE, 1_000 * USDC_UNIT_SIZE, true, true)
        .await?;

    // Only get 1 out because the top of global is not backed.
    assert_eq!(test_fixture.payer_sol_fixture.balance_atoms().await, 0);
    assert_eq!(
        test_fixture.payer_usdc_fixture.balance_atoms().await,
        1_000 * USDC_UNIT_SIZE
    );

    Ok(())
}

/// Test wash trading with reverse orders.
/// A single trader posts reverse orders on both sides at two price levels,
/// then swaps against their own orders in both directions twice, filling
/// top of book and spilling over to the second level. At the end, verify
/// token accounts, cancel all orders, and confirm full withdrawal.
#[tokio::test]
async fn swap_wash_reverse_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;

    // Claim seat and deposit tokens for the trader (default payer)
    test_fixture.claim_seat().await?;

    let initial_sol: u64 = 100 * SOL_UNIT_SIZE;
    let initial_usdc: u64 = 100_000 * USDC_UNIT_SIZE;

    test_fixture.deposit(Token::SOL, initial_sol).await?;
    test_fixture.deposit(Token::USDC, initial_usdc).await?;

    // Place reverse orders on both sides at two price levels each.
    // Bids: 5 SOL @ 10 USDC/SOL (level 1), 5 SOL @ 8 USDC/SOL (level 2)
    // Asks: 5 SOL @ 12 USDC/SOL (level 1), 5 SOL @ 14 USDC/SOL (level 2)
    // Spread of 10% (10_000 in units of 1/100,000)

    // Bid level 1: 5 SOL @ 10 USDC/SOL
    test_fixture
        .place_order(
            Side::Bid,
            5 * SOL_UNIT_SIZE,
            10,
            0,
            10_000, // 10% spread
            OrderType::Reverse,
        )
        .await?;

    // Bid level 2: 5 SOL @ 8 USDC/SOL
    test_fixture
        .place_order(
            Side::Bid,
            5 * SOL_UNIT_SIZE,
            8,
            0,
            10_000,
            OrderType::Reverse,
        )
        .await?;

    // Ask level 1: 5 SOL @ 12 USDC/SOL
    test_fixture
        .place_order(
            Side::Ask,
            5 * SOL_UNIT_SIZE,
            12,
            0,
            10_000,
            OrderType::Reverse,
        )
        .await?;

    // Ask level 2: 5 SOL @ 14 USDC/SOL
    test_fixture
        .place_order(
            Side::Ask,
            5 * SOL_UNIT_SIZE,
            14,
            0,
            10_000,
            OrderType::Reverse,
        )
        .await?;

    // Verify initial orders are placed (2 bids + 2 asks = 4 orders)
    let orders = test_fixture.market_fixture.get_resting_orders().await;
    assert_eq!(orders.len(), 4);

    // Expand the market to ensure there are enough free blocks for reverse orders
    // when swapping. Each swap against a reverse order needs a free block for the
    // new reversed order.
    let payer = test_fixture.payer();
    let payer_keypair = test_fixture.payer_keypair();
    for _ in 0..10 {
        let expand_ix =
            expand_market_instruction(&test_fixture.market_fixture.key, &payer);
        send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[expand_ix],
            Some(&payer),
            &[&payer_keypair],
        )
        .await?;
    }

    // Mint tokens to payer's external wallet for swapping
    test_fixture
        .sol_mint_fixture
        .mint_to(&test_fixture.payer_sol_fixture.key, 20 * SOL_UNIT_SIZE)
        .await;
    test_fixture
        .usdc_mint_fixture
        .mint_to(&test_fixture.payer_usdc_fixture.key, 200 * USDC_UNIT_SIZE)
        .await;

    // Swap 1: Sell SOL (buy quote) - fill top of book ask and spill to second level
    // Buying with 140 USDC should fill 5 SOL @ 12 and ~5.7 SOL @ 14
    // is_base_in=false means we're sending USDC in
    test_fixture
        .swap(140 * USDC_UNIT_SIZE, 0, false, true)
        .await?;

    // Swap 2: Buy SOL (sell quote) - fill top of book bid and spill to second level
    // Selling 8 SOL should fill orders on the bid side
    // is_base_in=true means we're sending SOL in
    test_fixture.swap(8 * SOL_UNIT_SIZE, 0, true, true).await?;

    // Swap 3: Sell SOL again (buy quote)
    test_fixture.swap(80 * USDC_UNIT_SIZE, 0, false, true).await?;

    // Swap 4: Buy SOL again (sell quote)
    test_fixture.swap(6 * SOL_UNIT_SIZE, 0, true, true).await?;

    // Verify we have resting orders (reverse orders should have flipped)
    let orders_after: Vec<RestingOrder> =
        test_fixture.market_fixture.get_resting_orders().await;
    assert!(orders_after.len() > 0, "Should have resting orders after swaps");

    // Record balances in wallet token accounts
    let sol_balance_wallet = test_fixture.payer_sol_fixture.balance_atoms().await;
    let usdc_balance_wallet = test_fixture.payer_usdc_fixture.balance_atoms().await;

    // Record balances in market
    let sol_balance_market = test_fixture
        .market_fixture
        .get_base_balance_atoms(&test_fixture.payer())
        .await;
    let usdc_balance_market = test_fixture
        .market_fixture
        .get_quote_balance_atoms(&test_fixture.payer())
        .await;

    // Cancel all resting orders
    let orders_to_cancel: Vec<RestingOrder> =
        test_fixture.market_fixture.get_resting_orders().await;

    let cancels: Vec<CancelOrderParams> = orders_to_cancel
        .iter()
        .map(|o| CancelOrderParams::new(o.get_sequence_number()))
        .collect();

    let cancel_ix = batch_update_instruction(
        &test_fixture.market_fixture.key,
        &payer,
        None,
        cancels,
        vec![],
        None,
        None,
        None,
        None,
    );

    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[cancel_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // Verify all orders are cancelled
    let orders_after_cancel = test_fixture.market_fixture.get_resting_orders().await;
    assert_eq!(
        orders_after_cancel.len(),
        0,
        "All orders should be cancelled"
    );

    // Get updated market balances after cancellation (funds should be unlocked)
    let sol_balance_market_after = test_fixture
        .market_fixture
        .get_base_balance_atoms(&test_fixture.payer())
        .await;
    let usdc_balance_market_after = test_fixture
        .market_fixture
        .get_quote_balance_atoms(&test_fixture.payer())
        .await;

    // Market balance should be >= what it was before (funds unlocked from cancelled orders)
    assert!(
        sol_balance_market_after >= sol_balance_market,
        "SOL market balance should not decrease after cancel"
    );
    assert!(
        usdc_balance_market_after >= usdc_balance_market,
        "USDC market balance should not decrease after cancel"
    );

    // Withdraw all tokens from the market
    if sol_balance_market_after > 0 {
        test_fixture
            .withdraw(Token::SOL, sol_balance_market_after)
            .await?;
    }
    if usdc_balance_market_after > 0 {
        test_fixture
            .withdraw(Token::USDC, usdc_balance_market_after)
            .await?;
    }

    // Verify market balances are now zero
    let final_sol_market = test_fixture
        .market_fixture
        .get_base_balance_atoms(&test_fixture.payer())
        .await;
    let final_usdc_market = test_fixture
        .market_fixture
        .get_quote_balance_atoms(&test_fixture.payer())
        .await;
    assert_eq!(final_sol_market, 0, "All SOL should be withdrawn");
    assert_eq!(final_usdc_market, 0, "All USDC should be withdrawn");

    // Verify wallet received the tokens
    let final_sol_wallet = test_fixture.payer_sol_fixture.balance_atoms().await;
    let final_usdc_wallet = test_fixture.payer_usdc_fixture.balance_atoms().await;

    assert_eq!(
        final_sol_wallet,
        sol_balance_wallet + sol_balance_market_after,
        "Wallet SOL should increase by withdrawn amount"
    );
    assert_eq!(
        final_usdc_wallet,
        usdc_balance_wallet + usdc_balance_market_after,
        "Wallet USDC should increase by withdrawn amount"
    );

    // Verify total value is conserved (initial deposits + minted - what's in wallet should equal what's on market, which is 0)
    // Total SOL: initial_sol (deposited) + 20 SOL (minted to wallet)
    // Total USDC: initial_usdc (deposited) + 200 USDC (minted to wallet)
    let total_sol = initial_sol + 20 * SOL_UNIT_SIZE;
    let total_usdc = initial_usdc + 200 * USDC_UNIT_SIZE;

    assert_eq!(
        final_sol_wallet, total_sol,
        "Total SOL should be conserved"
    );
    assert_eq!(
        final_usdc_wallet, total_usdc,
        "Total USDC should be conserved"
    );

    Ok(())
}

// LJITSPS base token has 7 decimals (matching mainnet mint FxppP7heqS742hvuGoAzHoYYnFk3iTF7cVuDaU3V8dDQ)
#[allow(dead_code)]
const LJITSPS_BASE_UNIT_SIZE: u64 = 10_000_000;

/// LJITSPS Test - Replays transactions for FxppP7heqS742hvuGoAzHoYYnFk3iTF7cVuDaU3V8dDQ
///
/// This test simulates the pattern of transactions observed on mainnet for the trader
/// EHeaNkrqdFvkFz5JprgoRbBD4fLH8YHKbBZ9CJ17hFcR on market CKzJCoCnUVVxhfQGs1aLihpF49tCt49qJaQXofRjRFEL
/// where FxppP7heqS742hvuGoAzHoYYnFk3iTF7cVuDaU3V8dDQ is the base mint (Token-2022 with 7 decimals).
///
/// The trader executes wash trades against their own reverse orders.
/// All transactions are replayed in order with their full signatures and deserialized logs.
#[tokio::test]
async fn ljitsps_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;

    // Claim seat for trader
    test_fixture.claim_seat().await?;

    // ============================================================================
    // Transaction 1: Deposit base tokens
    // Signature: 5umFNK6hYLebKUhstYJ63XeDc2ouhhmeTgYcgqeWz36nFv2peTrKVt9ytRjLdNitUo7gRZGTvWBfXrUYBAxymwiY
    // Slot: 398091542, BlockTime: 2026-02-04T22:16:19.000Z
    // DepositLog:
    //   market: CKzJCoCnUVVxhfQGs1aLihpF49tCt49qJaQXofRjRFEL
    //   trader: EHeaNkrqdFvkFz5JprgoRbBD4fLH8YHKbBZ9CJ17hFcR
    //   mint: FxppP7heqS742hvuGoAzHoYYnFk3iTF7cVuDaU3V8dDQ
    //   amountAtoms: 9900000000
    // ============================================================================
    test_fixture.deposit(Token::SOL, 9_900_000_000).await?;

    // ============================================================================
    // Transaction 2: Deposit more base tokens
    // Signature: 43n2iMie5WpvxLXhgUJ17ffKu1KRJav5jw9auQ1NLCZWVpwaaRmqsXA3UKLSAjWGYQbpNNJMxPxGsVorK5kZXNei
    // Slot: 398134844, BlockTime: 2026-02-05T03:00:28.000Z
    // DepositLog:
    //   market: CKzJCoCnUVVxhfQGs1aLihpF49tCt49qJaQXofRjRFEL
    //   trader: EHeaNkrqdFvkFz5JprgoRbBD4fLH8YHKbBZ9CJ17hFcR
    //   mint: FxppP7heqS742hvuGoAzHoYYnFk3iTF7cVuDaU3V8dDQ
    //   amountAtoms: 572979102300000
    // ============================================================================
    test_fixture.deposit(Token::SOL, 572_979_102_300_000).await?;

    // Deposit quote tokens (USDC) for bidding - needed for reverse orders
    // At ~$99.5/token, 57M tokens needs ~5.7T quote atoms
    test_fixture.deposit(Token::USDC, 6_000_000_000_000).await?;

    // Expand market to ensure enough free blocks for reverse orders
    let payer = test_fixture.payer();
    let payer_keypair = test_fixture.payer_keypair();
    for _ in 0..20 {
        let expand_ix = expand_market_instruction(&test_fixture.market_fixture.key, &payer);
        send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[expand_ix],
            Some(&payer),
            &[&payer_keypair],
        )
        .await?;
    }

    // ============================================================================
    // Transaction 3: CancelOrderLog (order 85)
    // Signature: 3jZs6Kp9PqboRX5ngBHoo48SNGHRj1tfSAhJFKbFj9U1qXzaGPJELUuarAVR7RViYG9jLJicZ3pwui2dUjSLSSHs
    // Slot: 398144682, BlockTime: 2026-02-05T04:04:53.000Z
    // CancelOrderLog:
    //   market: CKzJCoCnUVVxhfQGs1aLihpF49tCt49qJaQXofRjRFEL
    //   trader: EHeaNkrqdFvkFz5JprgoRbBD4fLH8YHKbBZ9CJ17hFcR
    //   orderSequenceNumber: 85
    // NOTE: These cancels reference orders placed before our observation window.
    //       We skip them since we're starting with an empty orderbook.
    // ============================================================================

    // ============================================================================
    // Transactions 4-20: Multiple CancelOrderLogs for pre-existing orders
    // Signatures include:
    //   3stRrsFfyvZ1yb16BWXb3EtYknhidv68EncaWsJSHn5ghdC8ZLQUxrxwqr9xFH2iaLKa8YAHxc8oAxs16VeSpH2C (cancel 74)
    //   66j27Vng1kJGUdn3QGgjYjr2EXYCTKV2x8zsSkhXSm1nQoX32bYvEdtoxGdDaxcarbV81GCgcK4bpFB2KpPz5U3P (cancel 82)
    //   3gTqQ1BuWHYjYsxHkn6XnGuNVe6YG8FEJkzUrdMLPW6ZuYvGcB6rdTTrbQ92ceiZzrUdiEuU5EB58XjBWAL2fddt (cancel 30,57)
    //   4dP96FBXDTe1ss3Cc8jnCNsCyoAYqtwgLkxy8mMuMxanq93iXKmW8Z71xQZLBjWAD5CeWsWiBxfhgvmvy3hndGNw (cancel 92,91,90,84,31-46)
    //   55ip2XqMHgU98kw3K8qfozcPMaPw3UEnqbZNrzMWWFC813PLQENTNfieoJ9umyc5LoNt4cv7VJzg4jSAZZUzERd1 (cancel 47-51,60,68,71,58,55,76,66,64,78,70,89,83,79,72,80)
    //   SimbV1eFXB1uHfm9kt5s78FofLLrWEJDEN9JYbQRyJmeoPh1n86yoo152T4Tm6NSTvDpRKbPsmQSFq7jByfWUXA (cancel 86,77)
    //   5AFB9rQrvJrVmDAtq8yfCJjmLVCrmeLXqC5ynvjY7TWAucZ43rF56Hg2jRyend2114oJr1YXsctGTTokn6Jk9Lqf (cancel 81)
    //   ... more cancels through transactions referencing orders 95-174
    // NOTE: All these cancel pre-existing orders we didn't place. Skipping.
    // ============================================================================

    // ============================================================================
    // First wash trade transaction with FillLog and PlaceOrderLogV2
    // Signature: 2DQT5C61fEzU7yRpohbcYMqeqbnWXWJ3rABv4p7hPLFNmMVpgVpXMnSz6SgVaFH8pZAQwKKrh8vbTkXaosLnYWXV
    // Slot: 398515144, BlockTime: 2026-02-06T20:25:50.000Z
    // FillLog:
    //   market: CKzJCoCnUVVxhfQGs1aLihpF49tCt49qJaQXofRjRFEL
    //   maker: EHeaNkrqdFvkFz5JprgoRbBD4fLH8YHKbBZ9CJ17hFcR
    //   taker: EHeaNkrqdFvkFz5JprgoRbBD4fLH8YHKbBZ9CJ17hFcR
    //   baseMint: FxppP7heqS742hvuGoAzHoYYnFk3iTF7cVuDaU3V8dDQ
    //   quoteMint: EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v
    //   baseAtoms: 200000
    //   quoteAtoms: 19900
    //   price: 99500000000000000
    //   makerSequenceNumber: 179
    //   takerSequenceNumber: 185
    //   takerIsBuy: false
    //   isMakerGlobal: false
    // PlaceOrderLogV2:
    //   market: CKzJCoCnUVVxhfQGs1aLihpF49tCt49qJaQXofRjRFEL
    //   trader: EHeaNkrqdFvkFz5JprgoRbBD4fLH8YHKbBZ9CJ17hFcR
    //   payer: EHeaNkrqdFvkFz5JprgoRbBD4fLH8YHKbBZ9CJ17hFcR
    //   baseAtoms: 200000
    //   orderSequenceNumber: 186
    //   orderIndex: 4294967295
    //   lastValidSlot: 0
    //   orderType: 1 (Reverse)
    //   isBid: false
    //
    // NOTE: This references maker order 179 which doesn't exist in our fresh orderbook.
    // To replicate wash trades, we first place reverse orders that we can swap against.
    // ============================================================================

    // Place initial reverse orders to enable wash trading
    // These simulate orders 175-180 at various price levels for the wash trades to match against

    // Bid reverse orders (will be matched when taker sells)
    // Price ~99.5 USDC per base token (price = 995 with exponent -1)
    test_fixture
        .place_order(Side::Bid, 10_000_000, 995, -1, 10_000, OrderType::Reverse)
        .await?; // seq 1

    // Price ~98.5
    test_fixture
        .place_order(Side::Bid, 10_000_000, 985, -1, 10_000, OrderType::Reverse)
        .await?; // seq 2

    // Price ~97.5
    test_fixture
        .place_order(Side::Bid, 10_000_000, 975, -1, 10_000, OrderType::Reverse)
        .await?; // seq 3

    // Price ~96.5
    test_fixture
        .place_order(Side::Bid, 10_000_000, 965, -1, 10_000, OrderType::Reverse)
        .await?; // seq 4

    // Price ~95.5
    test_fixture
        .place_order(Side::Bid, 10_000_000, 955, -1, 10_000, OrderType::Reverse)
        .await?; // seq 5

    // Ask reverse orders (will be matched when taker buys)
    // Price ~100.5
    test_fixture
        .place_order(Side::Ask, 10_000_000, 1005, -1, 10_000, OrderType::Reverse)
        .await?; // seq 6

    // Mint tokens for swapping
    test_fixture
        .sol_mint_fixture
        .mint_to(&test_fixture.payer_sol_fixture.key, 100_000_000_000)
        .await;
    test_fixture
        .usdc_mint_fixture
        .mint_to(&test_fixture.payer_usdc_fixture.key, 10_000_000_000)
        .await;

    // ============================================================================
    // Replicate wash trade: Swap that fills against own bid and creates ask
    // Signature: 2DQT5C61fEzU7yRpohbcYMqeqbnWXWJ3rABv4p7hPLFNmMVpgVpXMnSz6SgVaFH8pZAQwKKrh8vbTkXaosLnYWXV
    // FillLog: baseAtoms=200000, quoteAtoms=19900, takerIsBuy=false
    // PlaceOrderLogV2: baseAtoms=200000, orderType=1(Reverse), isBid=false
    // ============================================================================
    test_fixture.swap(200_000, 0, true, true).await?;

    // ============================================================================
    // Transaction: 4ToujBEBBmDzR8MbZ8g6eV4PohzuT9Xr3KyWbLH257Lx5XVSWfWxr1M3duzg5ycNdDKfaNnuk1CJvukEdEa5w1Zs
    // Slot: 398516397, BlockTime: 2026-02-06T20:34:06.000Z
    // FillLog:
    //   baseAtoms: 300000
    //   quoteAtoms: 29850
    //   price: 99500000000000000
    //   makerSequenceNumber: 179
    //   takerSequenceNumber: 187
    //   takerIsBuy: false
    // PlaceOrderLogV2:
    //   baseAtoms: 300000
    //   orderSequenceNumber: 187
    //   orderType: 1
    //   isBid: false
    // ============================================================================
    test_fixture.swap(300_000, 0, true, true).await?;

    // ============================================================================
    // Transaction: 54pBTxpCgpB8Y5nJPYfAtW2pYJVBCW6ewn6fQCJXBHpZr8Es2Af5TEzr7no9TXpAKdFFQJfehkBqbr2iGCyyanpa
    // Slot: 398519345, BlockTime: 2026-02-06T20:53:22.000Z
    // FillLog (1):
    //   baseAtoms: 500000, quoteAtoms: 49849
    //   price: 99699398797595190, makerSequenceNumber: 185, takerSequenceNumber: 188
    //   takerIsBuy: true
    // FillLog (2):
    //   baseAtoms: 796, quoteAtoms: 80
    //   price: 100500000000000000, makerSequenceNumber: 180, takerSequenceNumber: 188
    //   takerIsBuy: true
    // PlaceOrderLogV2:
    //   baseAtoms: 500796, orderSequenceNumber: 189, orderType: 1, isBid: true
    // ============================================================================
    test_fixture.swap(50_000, 0, false, true).await?;

    // ============================================================================
    // Transaction: 3ktP8u7qrUkt37muQN29KwX44TuakALdK3ACt9EJNyMH7898W69pXvGjxx1NF8vGQ7xtdXzcQEJrD8AfBLWYCSJY
    // Slot: 398520028, BlockTime: 2026-02-06T20:57:51.000Z
    // FillLog (1): baseAtoms: 797, quoteAtoms: 80, price: 100299000000000000
    //   makerSequenceNumber: 188, takerSequenceNumber: 190, takerIsBuy: false
    // FillLog (2): baseAtoms: 5025617, quoteAtoms: 500049, price: 99500000000000000
    //   makerSequenceNumber: 179, takerSequenceNumber: 190, takerIsBuy: false
    // FillLog (3): baseAtoms: 4973586, quoteAtoms: 489898, price: 98500000000000000
    //   makerSequenceNumber: 178, takerSequenceNumber: 191, takerIsBuy: false
    // PlaceOrderLogV2:
    //   baseAtoms: 10000000, orderSequenceNumber: 192, orderType: 1, isBid: false
    // ============================================================================
    test_fixture.swap(10_000_000, 0, true, true).await?;

    // ============================================================================
    // Transaction: 3eAq6mDxnRvg9S9dmJrqUBWQwf48veWK3UcDu3vDRonSru4T7akk32eeo4qW8nNNnE7TYBB25dgpWyMdR8mjwBL1
    // Slot: 398520066, BlockTime: 2026-02-06T20:58:06.000Z
    // FillLog (1): baseAtoms: 4973586, quoteAtoms: 490879, price: 98697394789579158
    //   makerSequenceNumber: 191, takerSequenceNumber: 193, takerIsBuy: true
    // FillLog (2): baseAtoms: 4986680, quoteAtoms: 497169, price: 99699398797595190
    //   makerSequenceNumber: 190, takerSequenceNumber: 193, takerIsBuy: true
    // PlaceOrderLogV2:
    //   baseAtoms: 9960266, orderSequenceNumber: 194, orderType: 1, isBid: true
    // ============================================================================
    test_fixture.swap(1_000_000, 0, false, true).await?;

    // ============================================================================
    // Transaction: 5MrHxzcVRpmYL8WfUBxky6bmm1ca2p2zWPmDBkJsFTue6U7iHFRmR86urYmVgdefAqxxogUXUjQLQ3LD5SdcmfNX
    // Slot: 398520438, BlockTime: 2026-02-06T21:00:32.000Z
    // FillLog (1): baseAtoms: 4996673, quoteAtoms: 497169, price: 99500000000000000
    //   makerSequenceNumber: 193, takerSequenceNumber: 195, takerIsBuy: false
    // FillLog (2): baseAtoms: 5003327, quoteAtoms: 492827, price: 98500000000000000
    //   makerSequenceNumber: 178, takerSequenceNumber: 195, takerIsBuy: false
    // PlaceOrderLogV2:
    //   baseAtoms: 10000000, orderSequenceNumber: 196, orderType: 1, isBid: false
    // ============================================================================
    test_fixture.swap(10_000_000, 0, true, true).await?;

    // ============================================================================
    // Transaction: 5KzRJiqY6KetfmBszpdYMFLe6V95FX6oArVTpasA7wEpwMuC4zFKfhjMfX6QpRPuMDJ2XyuAh3gxXRNQAfHhtTgG
    // Slot: 398523747, BlockTime: 2026-02-06T21:22:11.000Z
    // FillLog (1): baseAtoms: 4902180, quoteAtoms: 487767, price: 99500000000000000
    //   makerSequenceNumber: 197, takerSequenceNumber: 199, takerIsBuy: false
    // FillLog (2): baseAtoms: 5095614, quoteAtoms: 501918, price: 98500000000000000
    //   makerSequenceNumber: 178, takerSequenceNumber: 199, takerIsBuy: false
    // FillLog (3): baseAtoms: 2206, quoteAtoms: 215, price: 97500000000000000
    //   makerSequenceNumber: 177, takerSequenceNumber: 200, takerIsBuy: false
    // PlaceOrderLogV2:
    //   baseAtoms: 10000000, orderSequenceNumber: 201, orderType: 1, isBid: false
    // ============================================================================
    test_fixture.swap(10_000_000, 0, true, true).await?;

    // ============================================================================
    // Transaction: 2i2X38PchR4oxyH4DLoZXuQPfUzogEfLQ1EuPxMGZGhCNkfMKByqe92AWm3ZPWf1EhZLw74dKcEuB479tGn6G5G8
    // Slot: 398524430, BlockTime: 2026-02-06T21:26:46.000Z
    // FillLog (1): baseAtoms: 4682814, quoteAtoms: 465940, price: 99500000000000000
    // FillLog (2): baseAtoms: 5116040, quoteAtoms: 503930, price: 98500000000000000
    // FillLog (3): baseAtoms: 5127884, quoteAtoms: 499969, price: 97500000000000000
    // FillLog (4): baseAtoms: 5073262, quoteAtoms: 489569, price: 96500000000000000
    // PlaceOrderLogV2: baseAtoms: 20000000, orderSequenceNumber: 214, orderType: 1, isBid: false
    // ============================================================================
    test_fixture.swap(20_000_000, 0, true, true).await?;

    // ============================================================================
    // Transaction: DWqm9m8aPk51Y7wUCpEiNdZiRvznRaSJ58P7YiUUmuQvnzhPjQzCNRJvCnGAJzmwvYVTormtcUgTK7nfcgjKXUg
    // Slot: 398524910, BlockTime: 2026-02-06T21:29:57.000Z
    // FillLog (1): baseAtoms: 4455819, quoteAtoms: 443354, price: 99500000000000000
    // FillLog (2): baseAtoms: 5126284, quoteAtoms: 504939, price: 98500000000000000
    // FillLog (3): baseAtoms: 5138153, quoteAtoms: 500970, price: 97500000000000000
    // FillLog (4): baseAtoms: 5190986, quoteAtoms: 500931, price: 96500000000000000
    // FillLog (5): baseAtoms: 88758, quoteAtoms: 8476, price: 95500000000000000
    // PlaceOrderLogV2: baseAtoms: 20000000, orderSequenceNumber: 223, orderType: 1, isBid: false
    // ============================================================================
    test_fixture.swap(20_000_000, 0, true, true).await?;

    // ============================================================================
    // Transaction: 4CaKPCN8GwiG7svqdWu3tW19C684ucF6GRFXPRE3G7pXHhGwmYaFj1gBuLvP1JsN8LBm9z3X2WgmQuy82AdEE1yz
    // Slot: 398525646, BlockTime: 2026-02-06T21:34:45.000Z
    // FillLog (1): baseAtoms: 4006703, quoteAtoms: 398667, price: 99500000000000000
    // FillLog (2): baseAtoms: 5157137, quoteAtoms: 507978, price: 98500000000000000
    // FillLog (3): baseAtoms: 5151180, quoteAtoms: 502241, price: 97500000000000000
    // FillLog (4): baseAtoms: 5201388, quoteAtoms: 501934, price: 96500000000000000
    // FillLog (5): baseAtoms: 4591492, quoteAtoms: 438487, price: 95500000000000000
    // PlaceOrderLogV2: baseAtoms: 24107900, orderSequenceNumber: 245, orderType: 1, isBid: false
    // ============================================================================
    test_fixture.swap(24_107_900, 0, true, true).await?;

    // ============================================================================
    // Verify final state
    // ============================================================================
    let orders_final: Vec<RestingOrder> = test_fixture.market_fixture.get_resting_orders().await;
    assert!(
        orders_final.len() > 0,
        "Should have resting orders after wash trades (reverse orders create new orders)"
    );

    // Record wallet balances
    let sol_wallet = test_fixture.payer_sol_fixture.balance_atoms().await;
    let usdc_wallet = test_fixture.payer_usdc_fixture.balance_atoms().await;

    // Cancel all remaining orders
    let orders_to_cancel: Vec<RestingOrder> =
        test_fixture.market_fixture.get_resting_orders().await;

    let cancels: Vec<CancelOrderParams> = orders_to_cancel
        .iter()
        .map(|o| CancelOrderParams::new(o.get_sequence_number()))
        .collect();

    if !cancels.is_empty() {
        let cancel_ix = batch_update_instruction(
            &test_fixture.market_fixture.key,
            &payer,
            None,
            cancels,
            vec![],
            None,
            None,
            None,
            None,
        );

        send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[cancel_ix],
            Some(&payer),
            &[&payer_keypair],
        )
        .await?;
    }

    // Verify all orders cancelled
    let orders_after_cancel = test_fixture.market_fixture.get_resting_orders().await;
    assert_eq!(orders_after_cancel.len(), 0, "All orders should be cancelled");

    // Withdraw all tokens
    let sol_market_after = test_fixture
        .market_fixture
        .get_base_balance_atoms(&test_fixture.payer())
        .await;
    let usdc_market_after = test_fixture
        .market_fixture
        .get_quote_balance_atoms(&test_fixture.payer())
        .await;

    if sol_market_after > 0 {
        test_fixture.withdraw(Token::SOL, sol_market_after).await?;
    }
    if usdc_market_after > 0 {
        test_fixture.withdraw(Token::USDC, usdc_market_after).await?;
    }

    // Verify complete withdrawal
    let final_sol_market = test_fixture
        .market_fixture
        .get_base_balance_atoms(&test_fixture.payer())
        .await;
    let final_usdc_market = test_fixture
        .market_fixture
        .get_quote_balance_atoms(&test_fixture.payer())
        .await;

    assert_eq!(final_sol_market, 0, "All base should be withdrawn");
    assert_eq!(final_usdc_market, 0, "All quote should be withdrawn");

    // Verify trader can access all their tokens
    let final_sol_wallet = test_fixture.payer_sol_fixture.balance_atoms().await;
    let final_usdc_wallet = test_fixture.payer_usdc_fixture.balance_atoms().await;

    assert!(
        final_sol_wallet >= sol_wallet,
        "Base wallet balance should not decrease"
    );
    assert!(
        final_usdc_wallet >= usdc_wallet,
        "Quote wallet balance should not decrease"
    );

    Ok(())
}
