use std::rc::Rc;

use borsh::ser::BorshSerialize;
use manifest::program::{deposit::DepositParams, deposit_instruction, ManifestInstruction};
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_program::{instruction::AccountMeta, pubkey::Pubkey};
use solana_program_test::tokio;
use solana_signer::Signer;

use crate::{
    send_tx_with_retry, MintFixture, TestFixture, Token, TokenAccountFixture, SOL_UNIT_SIZE,
};

#[tokio::test]
async fn basic_deposit_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;

    // Deposit also does a mint to user token account.
    test_fixture.deposit(Token::SOL, SOL_UNIT_SIZE).await?;

    Ok(())
}

#[tokio::test]
async fn deposit_fail_no_seat_yet_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;

    assert!(test_fixture
        .deposit(Token::SOL, SOL_UNIT_SIZE)
        .await
        .is_err());

    Ok(())
}

#[tokio::test]
async fn deposit_fail_insufficient_funds_test() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;

    // Same code as deposit except there is no mint to.
    let (mint, user_token_account) = {
        let user_token_account: Pubkey = test_fixture.payer_sol_fixture.key;
        (&test_fixture.sol_mint_fixture.key, user_token_account)
    };

    let payer: &Pubkey = &test_fixture.payer().clone();
    let payer_keypair: &Keypair = &test_fixture.payer_keypair().insecure_clone();

    assert!(send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[deposit_instruction(
            &test_fixture.market_fixture.key,
            payer,
            mint,
            1,
            &user_token_account,
            spl_token::id(),
            None,
        )],
        Some(payer),
        &[payer_keypair],
    )
    .await
    .is_err());

    Ok(())
}

#[tokio::test]
async fn deposit_fail_incorrect_mint_test() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;

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

    assert!(send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[deposit_instruction(
            &test_fixture.market_fixture.key,
            payer,
            mint,
            SOL_UNIT_SIZE,
            &user_token_account,
            spl_token::id(),
            None,
        )],
        Some(payer),
        &[payer_keypair],
    )
    .await
    .is_err());

    Ok(())
}

#[tokio::test]
async fn global_deposit_fail_incorrect_vault_test() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;

    let user_token_account: Pubkey = test_fixture.payer_sol_fixture.key;

    let payer: &Pubkey = &test_fixture.payer().clone();
    let payer_keypair: &Keypair = &test_fixture.payer_keypair().insecure_clone();
    let deposit_ix: Instruction = Instruction {
        program_id: manifest::id(),
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(test_fixture.market_fixture.key, false),
            AccountMeta::new(user_token_account, false),
            AccountMeta::new(user_token_account, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: [
            ManifestInstruction::Deposit.to_vec(),
            DepositParams::new(SOL_UNIT_SIZE, None)
                .try_to_vec()
                .unwrap(),
        ]
        .concat(),
    };
    assert!(send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[deposit_ix],
        Some(payer),
        &[payer_keypair],
    )
    .await
    .is_err());

    Ok(())
}
