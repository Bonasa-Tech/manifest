use std::rc::Rc;

use borsh::BorshSerialize;
use manifest::program::{withdraw::WithdrawParams, withdraw_instruction, ManifestInstruction};
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_program::{instruction::AccountMeta, pubkey::Pubkey};
use solana_program_test::{tokio, ProgramTestContext};
use solana_signer::Signer;
use solana_transaction::Transaction;

use crate::{MintFixture, TestFixture, Token, TokenAccountFixture, SOL_UNIT_SIZE};

#[tokio::test]
async fn withdraw_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;

    // Withdraw also does a mint to user token account.
    test_fixture.deposit(Token::SOL, 1 * SOL_UNIT_SIZE).await?;
    test_fixture.withdraw(Token::SOL, 1 * SOL_UNIT_SIZE).await?;

    Ok(())
}

#[tokio::test]
async fn withdraw_quote_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;

    // Withdraw also does a mint to user token account.
    test_fixture.deposit(Token::USDC, 1 * SOL_UNIT_SIZE).await?;
    test_fixture
        .withdraw(Token::USDC, 1 * SOL_UNIT_SIZE)
        .await?;

    Ok(())
}

#[tokio::test]
async fn withdraw_insufficient_funds_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;

    assert!(test_fixture
        .withdraw(Token::SOL, 1 * SOL_UNIT_SIZE)
        .await
        .is_err());

    Ok(())
}

#[tokio::test]
async fn withdraw_user_insufficient_funds_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;

    // Deposit is on a different keypair, so the out of funds is not from token
    // program.
    let second_keypair: solana_keypair::Keypair = test_fixture.second_keypair.insecure_clone();
    test_fixture.claim_seat_for_keypair(&second_keypair).await?;
    test_fixture
        .deposit_for_keypair(Token::SOL, 1 * SOL_UNIT_SIZE, &second_keypair)
        .await?;

    assert!(test_fixture
        .withdraw(Token::SOL, SOL_UNIT_SIZE)
        .await
        .is_err());

    Ok(())
}

#[tokio::test]
async fn withdraw_fail_incorrect_mint_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, 1 * SOL_UNIT_SIZE).await?;

    let payer_keypair: &Keypair = &test_fixture.payer_keypair().insecure_clone();

    let mut new_mint_f: MintFixture =
        MintFixture::new(Rc::clone(&test_fixture.context), Some(9)).await;
    let payer_new_mint_fixture: TokenAccountFixture = TokenAccountFixture::new(
        Rc::clone(&test_fixture.context),
        &new_mint_f.key,
        &payer_keypair.pubkey(),
    )
    .await;

    // Same code as deposit except there is no mint to.
    let (mint, user_token_account) = {
        let user_token_account: Pubkey = payer_new_mint_fixture.key;
        new_mint_f.mint_to(&user_token_account, SOL_UNIT_SIZE).await;
        (&new_mint_f.key, user_token_account)
    };

    let payer: &Pubkey = &test_fixture.payer().clone();
    let payer_keypair: &Keypair = &test_fixture.payer_keypair().insecure_clone();
    let withdraw_ix: Instruction = withdraw_instruction(
        &test_fixture.market_fixture.key,
        payer,
        mint,
        SOL_UNIT_SIZE,
        &user_token_account,
        spl_token::id(),
        None,
    );
    let mut context: std::cell::RefMut<ProgramTestContext> = test_fixture.context.borrow_mut();

    let withdraw_tx: Transaction = {
        Transaction::new_signed_with_payer(
            &[withdraw_ix],
            Some(payer),
            &[payer_keypair],
            context.get_new_latest_blockhash().await?,
        )
    };

    assert!(context
        .banks_client
        .process_transaction(withdraw_tx)
        .await
        .is_err());

    Ok(())
}

#[tokio::test]
async fn withdraw_fail_incorrect_vault_test() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;

    let user_token_account: Pubkey = test_fixture.payer_sol_fixture.key;

    let payer: &Pubkey = &test_fixture.payer().clone();
    let payer_keypair: &Keypair = &test_fixture.payer_keypair().insecure_clone();
    let withdraw_ix: Instruction = Instruction {
        program_id: manifest::id(),
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(test_fixture.market_fixture.key, false),
            AccountMeta::new(user_token_account, false),
            AccountMeta::new(user_token_account, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: [
            ManifestInstruction::Withdraw.to_vec(),
            WithdrawParams::new(SOL_UNIT_SIZE, None)
                .try_to_vec()
                .unwrap(),
        ]
        .concat(),
    };
    let mut context: std::cell::RefMut<ProgramTestContext> = test_fixture.context.borrow_mut();

    let withdraw_tx: Transaction = {
        Transaction::new_signed_with_payer(
            &[withdraw_ix],
            Some(payer),
            &[payer_keypair],
            context.get_new_latest_blockhash().await?,
        )
    };

    assert!(context
        .banks_client
        .process_transaction(withdraw_tx)
        .await
        .is_err());

    Ok(())
}
