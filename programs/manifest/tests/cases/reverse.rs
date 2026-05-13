use crate::{send_tx_with_retry, TestFixture};
use hypertree::HyperTreeValueIteratorTrait;
use manifest::{
    program::swap_instruction,
    quantities::WrapperU64,
    state::{BooksideReadOnly, RestingOrder},
};
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_program::{pubkey, pubkey::Pubkey};
use solana_signer::Signer;
use spl_associated_token_account::get_associated_token_address;
use std::{rc::Rc, str::FromStr};

#[tokio::test]
async fn reverse_coalesce() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.market_fixture.key = pubkey!("ENhU8LsaR7vDD2G1CsWcsuSGNrih9Cv5WZEk7q9kPapQ");

    // Swap. second_keypair was loaded with tokens for the correct mints.
    test_fixture.market_fixture.reload().await;

    let second_payer: Pubkey = test_fixture.second_keypair.pubkey();
    let second_payer_keypair: Keypair = test_fixture.second_keypair.insecure_clone();
    let usdc_mint: Pubkey =
        Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
    let sol_mint: Pubkey = Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap();
    let user_usdc_ata: Pubkey = get_associated_token_address(&second_payer, &usdc_mint);
    let user_sol_ata: Pubkey = get_associated_token_address(&second_payer, &sol_mint);

    test_fixture.market_fixture.reload().await;

    let swap_ix: Instruction = swap_instruction(
        &test_fixture.market_fixture.key,
        &second_payer,
        &sol_mint,
        &usdc_mint,
        &user_sol_ata,
        &user_usdc_ata,
        100_000_000,
        0,
        false,
        true,
        spl_token::id(),
        spl_token::id(),
        false,
    );
    let limit_instruction = ComputeBudgetInstruction::set_compute_unit_limit(1_400_000);

    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[limit_instruction, swap_ix],
        Some(&second_payer),
        &[&second_payer_keypair],
    )
    .await?;

    test_fixture.market_fixture.reload().await;

    // Show that the top of asks got the empty orders cleared.
    let asks: BooksideReadOnly = test_fixture.market_fixture.market.get_asks();
    for (ind, (_, ask)) in asks.iter::<RestingOrder>().enumerate() {
        if ind > 5 {
            break;
        }
        assert!(ask.get_num_base_atoms().as_u64() > 0);
    }
    // Show that the top of bids has no empty orders.
    let bids: BooksideReadOnly = test_fixture.market_fixture.market.get_bids();
    for (ind, (_, bid)) in bids.iter::<RestingOrder>().enumerate() {
        if ind > 5 {
            break;
        }
        assert!(bid.get_num_base_atoms().as_u64() > 0);
    }

    Ok(())
}

#[tokio::test]
async fn reverse_coalesce_usdt() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.market_fixture.key = pubkey!("8sjV1AqBFvFuADBCQHhotaRq5DFFYSjjg1jMyVWMqXvZ");

    // Swap. second_keypair was loaded with tokens for the correct mints.
    test_fixture.market_fixture.reload().await;

    let second_payer: Pubkey = test_fixture.second_keypair.pubkey();
    let second_payer_keypair: Keypair = test_fixture.second_keypair.insecure_clone();
    let usdt_mint: Pubkey =
        Pubkey::from_str("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB").unwrap();
    let usdc_mint: Pubkey =
        Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
    let user_usdt_ata: Pubkey = get_associated_token_address(&second_payer, &usdt_mint);
    let user_usdc_ata: Pubkey = get_associated_token_address(&second_payer, &usdc_mint);

    test_fixture.market_fixture.reload().await;

    test_fixture.market_fixture.reload().await;

    let swap_ix: Instruction = swap_instruction(
        &test_fixture.market_fixture.key,
        &second_payer,
        &usdt_mint,
        &usdc_mint,
        &user_usdt_ata,
        &user_usdc_ata,
        10_000_000_000,
        0,
        false,
        true,
        spl_token::id(),
        spl_token::id(),
        false,
    );
    let limit_instruction = ComputeBudgetInstruction::set_compute_unit_limit(1_400_000);

    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[limit_instruction, swap_ix],
        Some(&second_payer),
        &[&second_payer_keypair],
    )
    .await?;

    test_fixture.market_fixture.reload().await;

    // Show that the top of asks got the empty orders cleared.
    let asks: BooksideReadOnly = test_fixture.market_fixture.market.get_asks();
    for (ind, (_, ask)) in asks.iter::<RestingOrder>().enumerate() {
        if ind > 5 {
            break;
        }
        assert!(ask.get_num_base_atoms().as_u64() > 0);
    }
    // Show that the top of bids has no empty orders.
    let bids: BooksideReadOnly = test_fixture.market_fixture.market.get_bids();
    for (ind, (_, bid)) in bids.iter::<RestingOrder>().enumerate() {
        if ind > 5 {
            break;
        }
        assert!(bid.get_num_base_atoms().as_u64() > 0);
    }

    Ok(())
}
