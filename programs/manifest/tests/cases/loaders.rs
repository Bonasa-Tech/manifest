use std::rc::Rc;

use borsh::BorshSerialize;
use manifest::{
    program::{
        batch_update::BatchUpdateParams, deposit::DepositParams,
        global_deposit::GlobalDepositParams, global_withdraw::GlobalWithdrawParams,
        ManifestInstruction, SwapParams,
    },
    state::{MarketFixed, OrderType, NO_EXPIRATION_LAST_VALID_SLOT},
    validation::{get_global_address, get_global_vault_address, get_vault_address},
};
use solana_keypair::Keypair;
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    system_instruction, system_program,
    sysvar::rent::Rent,
};
use solana_program_test::tokio;
use solana_signer::Signer;

use crate::{
    send_tx_with_retry, GlobalFixture, MintFixture, Side, TestFixture, Token, TokenAccountFixture,
    SOL_UNIT_SIZE, USDC_UNIT_SIZE,
};

#[tokio::test]
async fn bad_program_ids() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;

    let market_keypair: Keypair = Keypair::new();

    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
    let payer: &Pubkey = &payer_keypair.pubkey();

    let usdc_mint_f: MintFixture = test_fixture.usdc_mint_fixture;
    let spl_mint_f: MintFixture = test_fixture.sol_mint_fixture;
    let usdc_mint_key: Pubkey = usdc_mint_f.key;
    let spl_mint_key: Pubkey = spl_mint_f.key;

    let (base_vault, _) = get_vault_address(&market_keypair.pubkey(), &spl_mint_key);
    let (quote_vault, _) = get_vault_address(&market_keypair.pubkey(), &usdc_mint_key);
    let space: usize = std::mem::size_of::<MarketFixed>();

    {
        let create_market_ixs: Vec<Instruction> = vec![
            system_instruction::create_account(
                payer,
                &market_keypair.pubkey(),
                Rent::default().minimum_balance(space),
                space as u64,
                &manifest::id(),
            ),
            Instruction {
                program_id: manifest::id(),
                accounts: vec![
                    AccountMeta::new(*payer, true),
                    AccountMeta::new(*&market_keypair.pubkey(), false),
                    AccountMeta::new_readonly(system_program::id(), false),
                    AccountMeta::new_readonly(spl_mint_key, false),
                    AccountMeta::new_readonly(usdc_mint_key, false),
                    AccountMeta::new(base_vault, false),
                    AccountMeta::new(quote_vault, false),
                    AccountMeta::new_readonly(Pubkey::new_unique(), false),
                    AccountMeta::new_readonly(spl_token_2022::id(), false),
                ],
                data: [ManifestInstruction::CreateMarket.to_vec()].concat(),
            },
        ];
        assert!(send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &create_market_ixs[..],
            Some(&payer),
            &[&payer_keypair.insecure_clone(), &market_keypair],
        )
        .await
        .is_err());
    }

    {
        let create_market_ixs: Vec<Instruction> = vec![
            system_instruction::create_account(
                payer,
                &market_keypair.pubkey(),
                Rent::default().minimum_balance(space),
                space as u64,
                &manifest::id(),
            ),
            Instruction {
                program_id: manifest::id(),
                accounts: vec![
                    AccountMeta::new(*payer, true),
                    AccountMeta::new(*&market_keypair.pubkey(), false),
                    AccountMeta::new_readonly(Pubkey::new_unique(), false),
                    AccountMeta::new_readonly(spl_mint_key, false),
                    AccountMeta::new_readonly(usdc_mint_key, false),
                    AccountMeta::new(base_vault, false),
                    AccountMeta::new(quote_vault, false),
                    AccountMeta::new_readonly(spl_token::id(), false),
                    AccountMeta::new_readonly(spl_token_2022::id(), false),
                ],
                data: [ManifestInstruction::CreateMarket.to_vec()].concat(),
            },
        ];
        assert!(send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &create_market_ixs[..],
            Some(&payer),
            &[&payer_keypair.insecure_clone(), &market_keypair],
        )
        .await
        .is_err());
    }
    Ok(())
}

#[tokio::test]
async fn create_market_wrong_vaults() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;

    let market_keypair: Keypair = Keypair::new();

    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
    let payer: &Pubkey = &payer_keypair.pubkey();

    let usdc_mint_f: MintFixture = test_fixture.usdc_mint_fixture;
    let spl_mint_f: MintFixture = test_fixture.sol_mint_fixture;
    let usdc_mint_key: Pubkey = usdc_mint_f.key;
    let spl_mint_key: Pubkey = spl_mint_f.key;

    let (base_vault, _) = get_vault_address(&market_keypair.pubkey(), &spl_mint_key);
    let (quote_vault, _) = get_vault_address(&market_keypair.pubkey(), &usdc_mint_key);
    let space: usize = std::mem::size_of::<MarketFixed>();

    {
        let fake_base_vault_keypair: Keypair = Keypair::new();
        let create_market_ixs: Vec<Instruction> = vec![
            system_instruction::create_account(
                payer,
                &market_keypair.pubkey(),
                Rent::default().minimum_balance(space),
                space as u64,
                &manifest::id(),
            ),
            Instruction {
                program_id: manifest::id(),
                accounts: vec![
                    AccountMeta::new(*payer, true),
                    AccountMeta::new(*&market_keypair.pubkey(), false),
                    AccountMeta::new_readonly(system_program::id(), false),
                    AccountMeta::new_readonly(spl_mint_key, false),
                    AccountMeta::new_readonly(usdc_mint_key, false),
                    AccountMeta::new(fake_base_vault_keypair.pubkey(), false),
                    AccountMeta::new(quote_vault, false),
                    AccountMeta::new_readonly(spl_token::id(), false),
                    AccountMeta::new_readonly(spl_token_2022::id(), false),
                ],
                data: [ManifestInstruction::CreateMarket.to_vec()].concat(),
            },
        ];
        assert!(send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &create_market_ixs[..],
            Some(&payer),
            &[&payer_keypair.insecure_clone(), &market_keypair],
        )
        .await
        .is_err());
    }

    {
        let fake_quote_vault_keypair: Keypair = Keypair::new();
        let create_market_ixs: Vec<Instruction> = vec![
            system_instruction::create_account(
                payer,
                &market_keypair.pubkey(),
                Rent::default().minimum_balance(space),
                space as u64,
                &manifest::id(),
            ),
            Instruction {
                program_id: manifest::id(),
                accounts: vec![
                    AccountMeta::new(*payer, true),
                    AccountMeta::new(*&market_keypair.pubkey(), false),
                    AccountMeta::new_readonly(system_program::id(), false),
                    AccountMeta::new_readonly(spl_mint_key, false),
                    AccountMeta::new_readonly(usdc_mint_key, false),
                    AccountMeta::new(base_vault, false),
                    AccountMeta::new(fake_quote_vault_keypair.pubkey(), false),
                    AccountMeta::new_readonly(spl_token::id(), false),
                    AccountMeta::new_readonly(spl_token_2022::id(), false),
                ],
                data: [ManifestInstruction::CreateMarket.to_vec()].concat(),
            },
        ];
        assert!(send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &create_market_ixs[..],
            Some(&payer),
            &[&payer_keypair.insecure_clone(), &market_keypair],
        )
        .await
        .is_err());
    }

    Ok(())
}

#[tokio::test]
async fn deposit_fail_missing_signer() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;

    let user_token_account: Pubkey = test_fixture.payer_sol_fixture.key;

    let payer: &Pubkey = &test_fixture.payer().clone();
    let payer_keypair: &Keypair = &test_fixture.payer_keypair().insecure_clone();
    let deposit_ix: Instruction = Instruction {
        program_id: manifest::id(),
        accounts: vec![
            // payer is not a signer, so it will fail
            AccountMeta::new(*payer, false),
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

#[tokio::test]
async fn swap_wrong_token_accounts() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture
        .deposit(Token::SOL, SOL_UNIT_SIZE * 1_000)
        .await?;
    test_fixture
        .deposit(Token::USDC, USDC_UNIT_SIZE * 1_000)
        .await?;
    test_fixture
        .sol_mint_fixture
        .mint_to(&test_fixture.payer_sol_fixture.key, 1 * SOL_UNIT_SIZE)
        .await;
    test_fixture
        .usdc_mint_fixture
        .mint_to(&test_fixture.payer_usdc_fixture.key, 1 * USDC_UNIT_SIZE)
        .await;

    test_fixture
        .place_order(
            Side::Ask,
            1,
            2,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )
        .await?;
    test_fixture
        .place_order(
            Side::Bid,
            1,
            1,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            OrderType::Limit,
        )
        .await?;

    let payer: &Pubkey = &test_fixture.payer().clone();
    let market: &Pubkey = &test_fixture.market_fixture.key;
    let trader_base_account: &Pubkey = &test_fixture.payer_sol_fixture.key;
    let trader_quote_account: &Pubkey = &test_fixture.payer_usdc_fixture.key;
    let (base_vault, _) = get_vault_address(market, &test_fixture.sol_mint_fixture.key);
    let (quote_vault, _) = get_vault_address(market, &test_fixture.usdc_mint_fixture.key);

    let payer_keypair: &Keypair = &test_fixture.payer_keypair().insecure_clone();
    // Wrong trader base
    {
        let swap_ix: Instruction = Instruction {
            program_id: manifest::id(),
            accounts: vec![
                AccountMeta::new(*payer, true),
                AccountMeta::new(*market, false),
                AccountMeta::new_readonly(system_program::id(), false),
                AccountMeta::new(base_vault, false),
                AccountMeta::new(*trader_quote_account, false),
                AccountMeta::new(base_vault, false),
                AccountMeta::new(quote_vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            data: [
                ManifestInstruction::Swap.to_vec(),
                SwapParams::new(1_000, 0, true, true).try_to_vec().unwrap(),
            ]
            .concat(),
        };
        assert!(send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[swap_ix],
            Some(payer),
            &[payer_keypair],
        )
        .await
        .is_err());
    }
    // Wrong base vault
    {
        let swap_ix: Instruction = Instruction {
            program_id: manifest::id(),
            accounts: vec![
                AccountMeta::new(*payer, true),
                AccountMeta::new(*market, false),
                AccountMeta::new_readonly(system_program::id(), false),
                AccountMeta::new(*trader_base_account, false),
                AccountMeta::new(*trader_quote_account, false),
                AccountMeta::new(*trader_base_account, false),
                AccountMeta::new(quote_vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            data: [
                ManifestInstruction::Swap.to_vec(),
                SwapParams::new(1_000, 0, true, true).try_to_vec().unwrap(),
            ]
            .concat(),
        };
        assert!(send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[swap_ix],
            Some(payer),
            &[payer_keypair],
        )
        .await
        .is_err());
    }
    // Wrong quote vault
    {
        let swap_ix: Instruction = Instruction {
            program_id: manifest::id(),
            accounts: vec![
                AccountMeta::new(*payer, true),
                AccountMeta::new(*market, false),
                AccountMeta::new_readonly(system_program::id(), false),
                AccountMeta::new(*trader_base_account, false),
                AccountMeta::new(*trader_quote_account, false),
                AccountMeta::new(base_vault, false),
                AccountMeta::new(*trader_quote_account, false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            data: [
                ManifestInstruction::Swap.to_vec(),
                SwapParams::new(1_000, 0, true, true).try_to_vec().unwrap(),
            ]
            .concat(),
        };
        assert!(send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[swap_ix],
            Some(payer),
            &[payer_keypair],
        )
        .await
        .is_err());
    }

    let global: &Pubkey = &test_fixture.sol_global_fixture.key;
    let (global_vault, _) = get_global_vault_address(&test_fixture.sol_mint_fixture.key);
    // Global is base
    {
        let swap_ix: Instruction = Instruction {
            program_id: manifest::id(),
            accounts: vec![
                AccountMeta::new(*payer, true),
                AccountMeta::new(*market, false),
                AccountMeta::new_readonly(system_program::id(), false),
                AccountMeta::new(*trader_base_account, false),
                AccountMeta::new(*trader_quote_account, false),
                AccountMeta::new(base_vault, false),
                AccountMeta::new(quote_vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new(*global, false),
                AccountMeta::new(global_vault, false),
            ],
            data: [
                ManifestInstruction::Swap.to_vec(),
                SwapParams::new(1_000, 0, true, true).try_to_vec().unwrap(),
            ]
            .concat(),
        };
        assert!(send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[swap_ix],
            Some(payer),
            &[payer_keypair],
        )
        .await
        .is_ok());
    }

    let global: &Pubkey = &test_fixture.global_fixture.key;
    // Wrong global vault
    {
        let swap_ix: Instruction = Instruction {
            program_id: manifest::id(),
            accounts: vec![
                AccountMeta::new(*payer, true),
                AccountMeta::new(*market, false),
                AccountMeta::new_readonly(system_program::id(), false),
                AccountMeta::new(*trader_base_account, false),
                AccountMeta::new(*trader_quote_account, false),
                AccountMeta::new(base_vault, false),
                AccountMeta::new(quote_vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new(*global, false),
                AccountMeta::new(quote_vault, false),
            ],
            data: [
                ManifestInstruction::Swap.to_vec(),
                SwapParams::new(1_000, 0, true, true).try_to_vec().unwrap(),
            ]
            .concat(),
        };
        assert!(send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[swap_ix],
            Some(payer),
            &[payer_keypair],
        )
        .await
        .is_err());
    }
    let mint_f: MintFixture = MintFixture::new(Rc::clone(&test_fixture.context), None).await;
    let global_f: GlobalFixture =
        GlobalFixture::new(Rc::clone(&test_fixture.context), &mint_f.key).await;
    let global: &Pubkey = &global_f.key;
    let (global_vault, _) = get_global_vault_address(&mint_f.key);
    // Global for the wrong token in general.
    {
        let swap_ix: Instruction = Instruction {
            program_id: manifest::id(),
            accounts: vec![
                AccountMeta::new(*payer, true),
                AccountMeta::new(*market, false),
                AccountMeta::new_readonly(system_program::id(), false),
                AccountMeta::new(*trader_base_account, false),
                AccountMeta::new(*trader_quote_account, false),
                AccountMeta::new(base_vault, false),
                AccountMeta::new(quote_vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new(*global, false),
                AccountMeta::new(global_vault, false),
            ],
            data: [
                ManifestInstruction::Swap.to_vec(),
                SwapParams::new(1_000, 0, true, true).try_to_vec().unwrap(),
            ]
            .concat(),
        };
        assert!(send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[swap_ix],
            Some(payer),
            &[payer_keypair],
        )
        .await
        .is_err());
    }

    // Global does not exist. This should succeed because it is possible that
    // the caller just included the PDAs to be safe, but didnt check that global
    // actually existed.
    {
        let empty_global_keypair: Keypair = Keypair::new();
        let swap_ix: Instruction = Instruction {
            program_id: manifest::id(),
            accounts: vec![
                AccountMeta::new(*payer, true),
                AccountMeta::new(*market, false),
                AccountMeta::new_readonly(system_program::id(), false),
                AccountMeta::new(*trader_base_account, false),
                AccountMeta::new(*trader_quote_account, false),
                AccountMeta::new(base_vault, false),
                AccountMeta::new(quote_vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new(empty_global_keypair.pubkey(), false),
                AccountMeta::new(empty_global_keypair.pubkey(), false),
            ],
            data: [
                ManifestInstruction::Swap.to_vec(),
                SwapParams::new(1_000, 0, true, true).try_to_vec().unwrap(),
            ]
            .concat(),
        };
        assert!(send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[swap_ix],
            Some(payer),
            &[payer_keypair],
        )
        .await
        .is_ok());
    }

    Ok(())
}

#[tokio::test]
async fn batch_update_wrong_global_accounts() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, SOL_UNIT_SIZE).await?;
    test_fixture.deposit(Token::USDC, USDC_UNIT_SIZE).await?;

    let payer: &Pubkey = &test_fixture.payer().clone();
    let payer_keypair: &Keypair = &test_fixture.payer_keypair().insecure_clone();
    let market: &Pubkey = &test_fixture.market_fixture.key;
    let base_mint: Pubkey = test_fixture.sol_mint_fixture.key;
    let quote_mint: Pubkey = test_fixture.usdc_mint_fixture.key;
    let (base_vault, _) = get_vault_address(market, &base_mint);
    let (quote_vault, _) = get_vault_address(market, &quote_mint);
    let base_global: Pubkey = test_fixture.sol_global_fixture.key;
    let quote_global: Pubkey = test_fixture.global_fixture.key;
    let (base_global_vault, _) = get_global_vault_address(&base_mint);
    let (quote_global_vault, _) = get_global_vault_address(&quote_mint);

    // Show that it is correct for the first iteration, then modify all the other accounts.
    {
        let batch_update_ix: Instruction = Instruction {
            program_id: manifest::id(),
            accounts: vec![
                AccountMeta::new(*payer, true),
                AccountMeta::new(*market, false),
                AccountMeta::new_readonly(system_program::id(), false),
                AccountMeta::new_readonly(base_mint, false),
                AccountMeta::new(base_global, false),
                AccountMeta::new(base_global_vault, false),
                AccountMeta::new(base_vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(quote_mint, false),
                AccountMeta::new(quote_global, false),
                AccountMeta::new(quote_global_vault, false),
                AccountMeta::new(quote_vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            data: [
                ManifestInstruction::BatchUpdate.to_vec(),
                BatchUpdateParams::new(None, vec![], vec![])
                    .try_to_vec()
                    .unwrap(),
            ]
            .concat(),
        };
        assert!(send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[batch_update_ix],
            Some(payer),
            &[payer_keypair],
        )
        .await
        .is_ok());
    }
    // Wrong mint
    {
        let mint_f: MintFixture = MintFixture::new(Rc::clone(&test_fixture.context), None).await;
        let batch_update_ix: Instruction = Instruction {
            program_id: manifest::id(),
            accounts: vec![
                AccountMeta::new(*payer, true),
                AccountMeta::new(*market, false),
                AccountMeta::new_readonly(system_program::id(), false),
                AccountMeta::new_readonly(mint_f.key, false),
                AccountMeta::new(base_global, false),
                AccountMeta::new(base_global_vault, false),
                AccountMeta::new(quote_vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(quote_mint, false),
                AccountMeta::new(quote_global, false),
                AccountMeta::new(quote_global_vault, false),
                AccountMeta::new(quote_vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            data: [
                ManifestInstruction::BatchUpdate.to_vec(),
                BatchUpdateParams::new(None, vec![], vec![])
                    .try_to_vec()
                    .unwrap(),
            ]
            .concat(),
        };
        assert!(send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[batch_update_ix],
            Some(payer),
            &[payer_keypair],
        )
        .await
        .is_err());
    }
    // Wrong global vault
    {
        let batch_update_ix: Instruction = Instruction {
            program_id: manifest::id(),
            accounts: vec![
                AccountMeta::new(*payer, true),
                AccountMeta::new(*market, false),
                AccountMeta::new_readonly(system_program::id(), false),
                AccountMeta::new_readonly(base_mint, false),
                AccountMeta::new(base_global, false),
                AccountMeta::new(quote_global_vault, false),
                AccountMeta::new(base_vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(quote_mint, false),
                AccountMeta::new(quote_global, false),
                AccountMeta::new(quote_global_vault, false),
                AccountMeta::new(quote_vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            data: [
                ManifestInstruction::BatchUpdate.to_vec(),
                BatchUpdateParams::new(None, vec![], vec![])
                    .try_to_vec()
                    .unwrap(),
            ]
            .concat(),
        };
        assert!(send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[batch_update_ix],
            Some(payer),
            &[payer_keypair],
        )
        .await
        .is_err());
    }
    // Wrong market vault
    {
        let batch_update_ix: Instruction = Instruction {
            program_id: manifest::id(),
            accounts: vec![
                AccountMeta::new(*payer, true),
                AccountMeta::new(*market, false),
                AccountMeta::new_readonly(system_program::id(), false),
                AccountMeta::new_readonly(base_mint, false),
                AccountMeta::new(base_global, false),
                AccountMeta::new(base_global_vault, false),
                AccountMeta::new(quote_vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(quote_mint, false),
                AccountMeta::new(quote_global, false),
                AccountMeta::new(quote_global_vault, false),
                AccountMeta::new(quote_vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            data: [
                ManifestInstruction::BatchUpdate.to_vec(),
                BatchUpdateParams::new(None, vec![], vec![])
                    .try_to_vec()
                    .unwrap(),
            ]
            .concat(),
        };
        assert!(send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[batch_update_ix],
            Some(payer),
            &[payer_keypair],
        )
        .await
        .is_err());
    }
    // Global for an unrelated mint
    {
        let mint_f: MintFixture = MintFixture::new(Rc::clone(&test_fixture.context), None).await;
        let global_f: GlobalFixture =
            GlobalFixture::new(Rc::clone(&test_fixture.context), &mint_f.key).await;
        let global: &Pubkey = &global_f.key;
        let (global_vault, _) = get_global_vault_address(&mint_f.key);
        let batch_update_ix: Instruction = Instruction {
            program_id: manifest::id(),
            accounts: vec![
                AccountMeta::new(*payer, true),
                AccountMeta::new(*market, false),
                AccountMeta::new_readonly(system_program::id(), false),
                AccountMeta::new_readonly(base_mint, false),
                AccountMeta::new(base_global, false),
                AccountMeta::new(base_global_vault, false),
                AccountMeta::new(base_vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(quote_mint, false),
                AccountMeta::new(*global, false),
                AccountMeta::new(global_vault, false),
                AccountMeta::new(quote_vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            data: [
                ManifestInstruction::BatchUpdate.to_vec(),
                BatchUpdateParams::new(None, vec![], vec![])
                    .try_to_vec()
                    .unwrap(),
            ]
            .concat(),
        };
        assert!(send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[batch_update_ix],
            Some(payer),
            &[payer_keypair],
        )
        .await
        .is_err());
    }
    // Global not initialized should succeed since it is not needed. This would
    // fail if the account is actually needed. This is useful in the case of a
    // bot accidentally including global PDAs without checking that they are
    // initialized.
    {
        let empty_global_keypair: Keypair = Keypair::new();
        let batch_update_ix: Instruction = Instruction {
            program_id: manifest::id(),
            accounts: vec![
                AccountMeta::new(*payer, true),
                AccountMeta::new(*market, false),
                AccountMeta::new_readonly(system_program::id(), false),
                AccountMeta::new_readonly(base_mint, false),
                AccountMeta::new(base_global, false),
                AccountMeta::new(base_global_vault, false),
                AccountMeta::new(base_vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(quote_mint, false),
                AccountMeta::new(empty_global_keypair.pubkey(), false),
                AccountMeta::new(empty_global_keypair.pubkey(), false),
                AccountMeta::new(quote_vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            data: [
                ManifestInstruction::BatchUpdate.to_vec(),
                BatchUpdateParams::new(None, vec![], vec![])
                    .try_to_vec()
                    .unwrap(),
            ]
            .concat(),
        };
        assert!(send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[batch_update_ix],
            Some(payer),
            &[payer_keypair],
        )
        .await
        .is_ok());
    }

    Ok(())
}

#[tokio::test]
async fn global_deposit_fail_incorrect_vault_test() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.global_add_trader().await?;

    let trader_token_account: Pubkey = test_fixture.payer_sol_fixture.key;

    let payer: &Pubkey = &test_fixture.payer().clone();
    let payer_keypair: &Keypair = &test_fixture.payer_keypair().insecure_clone();
    let mint: &Pubkey = &test_fixture.sol_mint_fixture.key;
    let (global, _global_bump) = get_global_address(mint);
    let (_global_vault, _global_vault_bump) = get_global_vault_address(mint);
    let deposit_ix: Instruction = Instruction {
        program_id: manifest::id(),
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(global, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new(trader_token_account, false),
            AccountMeta::new(trader_token_account, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: [
            ManifestInstruction::GlobalDeposit.to_vec(),
            // 0 atoms, so would succeed otherwise.
            GlobalDepositParams::new(0).try_to_vec().unwrap(),
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

#[tokio::test]
async fn global_withdraw_fail_incorrect_vault_test() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.global_add_trader().await?;

    let trader_token_account: Pubkey = test_fixture.payer_sol_fixture.key;

    let payer: &Pubkey = &test_fixture.payer().clone();
    let payer_keypair: &Keypair = &test_fixture.payer_keypair().insecure_clone();
    let mint: &Pubkey = &test_fixture.sol_mint_fixture.key;
    let (global, _global_bump) = get_global_address(mint);
    let (_global_vault, _global_vault_bump) = get_global_vault_address(mint);
    let deposit_ix: Instruction = Instruction {
        program_id: manifest::id(),
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(global, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new(trader_token_account, false),
            AccountMeta::new(trader_token_account, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: [
            ManifestInstruction::GlobalWithdraw.to_vec(),
            // 0 atoms, so would succeed otherwise.
            GlobalWithdrawParams::new(0).try_to_vec().unwrap(),
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

#[tokio::test]
async fn global_evict_loader() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;

    let payer: Pubkey = test_fixture.payer();

    // Adds global for `payer`
    test_fixture.global_add_trader().await?;

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

    let (global, _global_bump) = get_global_address(&test_fixture.global_fixture.mint_key);
    let wrong_global_vault: Pubkey = Pubkey::new_unique();

    assert!(send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[Instruction {
            program_id: manifest::id(),
            accounts: vec![
                AccountMeta::new(test_fixture.second_keypair.pubkey(), true),
                AccountMeta::new(global, false),
                AccountMeta::new_readonly(test_fixture.global_fixture.mint_key, false),
                AccountMeta::new(wrong_global_vault, false),
                AccountMeta::new(evictor_account_fixture.key, false),
                AccountMeta::new(evictee_account_fixture.key, false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            data: [
                ManifestInstruction::GlobalEvict.to_vec(),
                GlobalDepositParams::new(1_000_000).try_to_vec().unwrap(),
            ]
            .concat(),
        }],
        Some(&test_fixture.second_keypair.pubkey()),
        &[&test_fixture.second_keypair.insecure_clone()],
    )
    .await
    .is_err());

    Ok(())
}

#[tokio::test]
async fn loader_helpers() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.deposit(Token::SOL, SOL_UNIT_SIZE).await?;
    test_fixture.deposit(Token::USDC, USDC_UNIT_SIZE).await?;

    let payer: &Pubkey = &test_fixture.payer().clone();
    let payer_keypair: &Keypair = &test_fixture.payer_keypair().insecure_clone();
    let market: &Pubkey = &test_fixture.market_fixture.key;
    let base_mint: Pubkey = test_fixture.sol_mint_fixture.key;
    let quote_mint: Pubkey = test_fixture.usdc_mint_fixture.key;
    let (base_vault, _) = get_vault_address(market, &base_mint);
    let (quote_vault, _) = get_vault_address(market, &quote_mint);
    let base_global: Pubkey = test_fixture.sol_global_fixture.key;
    let quote_global: Pubkey = test_fixture.global_fixture.key;
    let (base_global_vault, _) = get_global_vault_address(&base_mint);
    let (quote_global_vault, _) = get_global_vault_address(&quote_mint);

    // Fail to verify that the market is owned by manfiest program.
    {
        let batch_update_ix: Instruction = Instruction {
            program_id: manifest::id(),
            accounts: vec![
                AccountMeta::new(*payer, true),
                AccountMeta::new(*payer, false),
                AccountMeta::new_readonly(system_program::id(), false),
                AccountMeta::new_readonly(base_mint, false),
                AccountMeta::new(base_global, false),
                AccountMeta::new(base_global_vault, false),
                AccountMeta::new(base_vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(quote_mint, false),
                AccountMeta::new(quote_global, false),
                AccountMeta::new(quote_global_vault, false),
                AccountMeta::new(quote_vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            data: [
                ManifestInstruction::BatchUpdate.to_vec(),
                BatchUpdateParams::new(None, vec![], vec![])
                    .try_to_vec()
                    .unwrap(),
            ]
            .concat(),
        };
        assert!(send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[batch_update_ix],
            Some(payer),
            &[payer_keypair],
        )
        .await
        .is_err());
    }
    Ok(())
}
