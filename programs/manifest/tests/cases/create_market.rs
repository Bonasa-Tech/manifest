use std::cell::RefMut;

use manifest::program::create_market_instructions;
use solana_program_test::{tokio, ProgramTestContext};
use solana_sdk::{
    instruction::Instruction,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};

use crate::TestFixture;

#[tokio::test]
async fn create_market() -> anyhow::Result<()> {
    let _test_fixture: TestFixture = TestFixture::new().await;

    Ok(())
}

#[tokio::test]
async fn create_market_fail_same_base_and_quote() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;

    assert!(test_fixture
        .create_new_market(
            &test_fixture.sol_mint_fixture.key,
            &test_fixture.sol_mint_fixture.key
        )
        .await
        .is_err());
    Ok(())
}

#[tokio::test]
async fn create_market_fail_already_initialized() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;

    let context_cell: RefMut<ProgramTestContext> = test_fixture.context.borrow_mut();
    let market_keypair: Keypair = Keypair::new();
    let payer: &Pubkey = &context_cell.payer.pubkey();
    let create_market_ixs: Vec<Instruction> = create_market_instructions(
        &market_keypair.pubkey(),
        &test_fixture.sol_mint_fixture.key,
        &test_fixture.usdc_mint_fixture.key,
        payer,
    )
    .unwrap();

    let create_market_tx: Transaction = {
        Transaction::new_signed_with_payer(
            &create_market_ixs[..],
            Some(payer),
            &[&context_cell.payer, &market_keypair],
            context_cell.last_blockhash,
        )
    };
    context_cell
        .banks_client
        .process_transaction(create_market_tx)
        .await?;

    // Should fail the second time because already initialized. This is testing
    // failing in the program, so dont init market outside the program again.
    let create_market_tx: Transaction = {
        Transaction::new_signed_with_payer(
            &create_market_ixs[1..],
            Some(payer),
            &[&context_cell.payer],
            context_cell.last_blockhash,
        )
    };
    assert!(context_cell
        .banks_client
        .process_transaction(create_market_tx)
        .await
        .is_err());

    Ok(())
}
