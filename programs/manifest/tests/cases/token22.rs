use std::{cell::RefCell, rc::Rc, u64};

use manifest::{
    program::{
        batch_update::PlaceOrderParams, batch_update_instruction, claim_seat_instruction,
        create_market_instructions, deposit_instruction, swap_instruction, withdraw_instruction,
    },
    quantities::WrapperU64,
    state::{OrderType, NO_EXPIRATION_LAST_VALID_SLOT},
};
use solana_program_test::{processor, ProgramTest, ProgramTestContext};
use solana_sdk::{
    instruction::Instruction, program_pack::Pack, pubkey::Pubkey, rent::Rent, signature::Keypair,
    signer::Signer, system_instruction::create_account,
};

use crate::{send_tx_with_retry, MintFixture, RUST_LOG_DEFAULT};

#[tokio::test]
async fn token22_base() -> anyhow::Result<()> {
    // Create market with one token being 22
    // Deposit both sides, place order both sides, swap both ways, withdraw both sides

    let program_test: ProgramTest = ProgramTest::new(
        "manifest",
        manifest::ID,
        processor!(manifest::process_instruction),
    );

    solana_logger::setup_with_default(RUST_LOG_DEFAULT);

    let market_keypair: Keypair = Keypair::new();

    let context: Rc<RefCell<ProgramTestContext>> =
        Rc::new(RefCell::new(program_test.start_with_context().await));

    // Be careful. There are 2 payers. The one on the context that will shortly be created and this one. We dont just use the
    //let payer_keypair: Keypair = Keypair::new();
    //let payer: &Pubkey = &payer_keypair.pubkey();
    let payer_keypair: Keypair = context.borrow().payer.insecure_clone();
    let payer: &Pubkey = &payer_keypair.pubkey();

    // For this test, usdc is old token and spl is token22.
    let usdc_mint_f: MintFixture =
        MintFixture::new_with_version(Rc::clone(&context), Some(6), false).await;
    // Does not need to use extensions.
    let spl_mint_f: MintFixture =
        MintFixture::new_with_version(Rc::clone(&context), Some(9), true).await;
    let usdc_mint_key: Pubkey = usdc_mint_f.key;
    let spl_mint_key: Pubkey = spl_mint_f.key;

    // Create the market with SPL as base which is 2022, USDC as quote which is normal.
    let create_market_ixs: Vec<Instruction> = create_market_instructions(
        &market_keypair.pubkey(),
        &spl_mint_f.key,
        &usdc_mint_f.key,
        payer,
    )
    .unwrap();
    send_tx_with_retry(
        Rc::clone(&context),
        &create_market_ixs[..],
        Some(&payer),
        &[&payer_keypair.insecure_clone(), &market_keypair],
    )
    .await?;

    // Claim seats
    let claim_seat_ix: Instruction = claim_seat_instruction(&market_keypair.pubkey(), &payer);
    send_tx_with_retry(
        Rc::clone(&context),
        &[claim_seat_ix],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    // Create depositor token accounts
    let usdc_token_account_keypair: Keypair = Keypair::new();
    let spl_token_account_keypair: Keypair = Keypair::new();
    let rent: Rent = context.borrow_mut().banks_client.get_rent().await.unwrap();
    let create_spl_token_account_ix: Instruction = create_account(
        payer,
        &spl_token_account_keypair.pubkey(),
        rent.minimum_balance(spl_token_2022::state::Account::LEN),
        spl_token_2022::state::Account::LEN as u64,
        &spl_token_2022::id(),
    );
    let init_spl_token_account_ix: Instruction = spl_token_2022::instruction::initialize_account(
        &spl_token_2022::id(),
        &spl_token_account_keypair.pubkey(),
        &spl_mint_key,
        payer,
    )
    .unwrap();
    let create_usdc_token_account_ix: Instruction = create_account(
        payer,
        &usdc_token_account_keypair.pubkey(),
        rent.minimum_balance(spl_token::state::Account::LEN),
        spl_token::state::Account::LEN as u64,
        &spl_token::id(),
    );
    let init_usdc_token_account_ix: Instruction = spl_token::instruction::initialize_account(
        &spl_token::id(),
        &usdc_token_account_keypair.pubkey(),
        &usdc_mint_key,
        payer,
    )
    .unwrap();
    send_tx_with_retry(
        Rc::clone(&context),
        &[
            create_spl_token_account_ix,
            init_spl_token_account_ix,
            create_usdc_token_account_ix,
            init_usdc_token_account_ix,
        ],
        Some(&payer),
        &[
            &payer_keypair.insecure_clone(),
            &spl_token_account_keypair.insecure_clone(),
            &usdc_token_account_keypair.insecure_clone(),
        ],
    )
    .await?;

    // Add funds to those token accounts.
    let spl_mint_to_instruction: Instruction = spl_token_2022::instruction::mint_to(
        &spl_token_2022::ID,
        &spl_mint_key,
        &spl_token_account_keypair.pubkey(),
        &payer,
        &[&payer],
        1_000_000_000_000_000,
    )
    .unwrap();
    let usdc_mint_to_instruction: Instruction = spl_token::instruction::mint_to(
        &spl_token::ID,
        &usdc_mint_key,
        &usdc_token_account_keypair.pubkey(),
        &payer,
        &[&payer],
        1_000_000_000_000_000,
    )
    .unwrap();
    send_tx_with_retry(
        Rc::clone(&context),
        &[spl_mint_to_instruction, usdc_mint_to_instruction],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    // Call deposit for each token account for a partial amount so we can swap later.
    let deposit_spl_ix: Instruction = deposit_instruction(
        &market_keypair.pubkey(),
        &payer,
        &spl_mint_key,
        1_000_000_000,
        &spl_token_account_keypair.pubkey(),
        spl_token_2022::id(),
        None,
    );
    let deposit_usdc_ix: Instruction = deposit_instruction(
        &market_keypair.pubkey(),
        &payer,
        &usdc_mint_key,
        1_000_000_000,
        &usdc_token_account_keypair.pubkey(),
        spl_token::id(),
        None,
    );
    send_tx_with_retry(
        Rc::clone(&context),
        &[deposit_spl_ix, deposit_usdc_ix],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    // Call withdraw
    let withdraw_spl_ix: Instruction = withdraw_instruction(
        &market_keypair.pubkey(),
        &payer,
        &spl_mint_key,
        1_000,
        &spl_token_account_keypair.pubkey(),
        spl_token_2022::id(),
        None,
    );
    let withdraw_usdc_ix: Instruction = withdraw_instruction(
        &market_keypair.pubkey(),
        &payer,
        &usdc_mint_key,
        1_000,
        &usdc_token_account_keypair.pubkey(),
        spl_token::id(),
        None,
    );
    send_tx_with_retry(
        Rc::clone(&context),
        &[withdraw_spl_ix, withdraw_usdc_ix],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;
    {
        let market_account: solana_sdk::account::Account = context
            .borrow_mut()
            .banks_client
            .get_account(market_keypair.pubkey())
            .await
            .unwrap()
            .unwrap();

        let market: manifest::state::MarketValue =
            manifest::program::get_dynamic_value(market_account.data.as_slice());
        let balance = market.get_trader_balance(&payer);
        assert_eq!(balance.0.as_u64(), 999999000);
    }

    // Place orders on both sides to
    let place_order_ix: Instruction = batch_update_instruction(
        &market_keypair.pubkey(),
        &payer,
        None,
        vec![],
        vec![
            PlaceOrderParams::new(
                1_000,
                9,
                -1,
                true,
                OrderType::PostOnly,
                NO_EXPIRATION_LAST_VALID_SLOT,
            ),
            PlaceOrderParams::new(
                1_000,
                11,
                -1,
                false,
                OrderType::PostOnly,
                NO_EXPIRATION_LAST_VALID_SLOT,
            ),
        ],
        None,
        None,
        None,
        None,
    );
    send_tx_with_retry(
        Rc::clone(&context),
        &[place_order_ix],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    // Swap using both directions
    let swap_base_in_ix: Instruction = swap_instruction(
        &market_keypair.pubkey(),
        &payer,
        &spl_mint_key,
        &usdc_mint_key,
        &spl_token_account_keypair.pubkey(),
        &usdc_token_account_keypair.pubkey(),
        100,
        10,
        true,
        true,
        spl_token_2022::id(),
        spl_token::id(),
        false,
    );
    let swap_base_out_ix: Instruction = swap_instruction(
        &market_keypair.pubkey(),
        &payer,
        &spl_mint_key,
        &usdc_mint_key,
        &spl_token_account_keypair.pubkey(),
        &usdc_token_account_keypair.pubkey(),
        100,
        10,
        false,
        true,
        spl_token_2022::id(),
        spl_token::id(),
        false,
    );
    send_tx_with_retry(
        Rc::clone(&context),
        &[swap_base_in_ix, swap_base_out_ix],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    Ok(())
}

#[tokio::test]
async fn token22_quote() -> anyhow::Result<()> {
    // Create market with one token being 22
    // Deposit both sides, place order both sides, swap both ways, withdraw both sides

    let program_test: ProgramTest = ProgramTest::new(
        "manifest",
        manifest::ID,
        processor!(manifest::process_instruction),
    );

    solana_logger::setup_with_default(RUST_LOG_DEFAULT);

    let market_keypair: Keypair = Keypair::new();

    let context: Rc<RefCell<ProgramTestContext>> =
        Rc::new(RefCell::new(program_test.start_with_context().await));

    // Be careful. There are 2 payers. The one on the context that will shortly be created and this one. We dont just use the
    //let payer_keypair: Keypair = Keypair::new();
    //let payer: &Pubkey = &payer_keypair.pubkey();
    let payer_keypair: Keypair = context.borrow().payer.insecure_clone();
    let payer: &Pubkey = &payer_keypair.pubkey();

    // For this test, usdc is old token and spl is token22.
    let usdc_mint_f: MintFixture =
        MintFixture::new_with_version(Rc::clone(&context), Some(6), true).await;
    // Does not need to use extensions.
    let spl_mint_f: MintFixture =
        MintFixture::new_with_version(Rc::clone(&context), Some(9), false).await;
    let usdc_mint_key: Pubkey = usdc_mint_f.key;
    let spl_mint_key: Pubkey = spl_mint_f.key;

    // Create the market with SPL as base which is normal, USDC as quote which is 2022.
    let create_market_ixs: Vec<Instruction> = create_market_instructions(
        &market_keypair.pubkey(),
        &spl_mint_f.key,
        &usdc_mint_f.key,
        payer,
    )
    .unwrap();
    send_tx_with_retry(
        Rc::clone(&context),
        &create_market_ixs[..],
        Some(&payer),
        &[&payer_keypair.insecure_clone(), &market_keypair],
    )
    .await?;

    // Claim seats
    let claim_seat_ix: Instruction = claim_seat_instruction(&market_keypair.pubkey(), &payer);
    send_tx_with_retry(
        Rc::clone(&context),
        &[claim_seat_ix],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    // Create depositor token accounts
    let usdc_token_account_keypair: Keypair = Keypair::new();
    let spl_token_account_keypair: Keypair = Keypair::new();
    let rent: Rent = context.borrow_mut().banks_client.get_rent().await.unwrap();
    let create_spl_token_account_ix: Instruction = create_account(
        payer,
        &spl_token_account_keypair.pubkey(),
        rent.minimum_balance(spl_token::state::Account::LEN),
        spl_token::state::Account::LEN as u64,
        &spl_token::id(),
    );
    let init_spl_token_account_ix: Instruction = spl_token::instruction::initialize_account(
        &spl_token::id(),
        &spl_token_account_keypair.pubkey(),
        &spl_mint_key,
        payer,
    )
    .unwrap();
    let create_usdc_token_account_ix: Instruction = create_account(
        payer,
        &usdc_token_account_keypair.pubkey(),
        rent.minimum_balance(spl_token_2022::state::Account::LEN),
        spl_token_2022::state::Account::LEN as u64,
        &spl_token_2022::id(),
    );
    let init_usdc_token_account_ix: Instruction = spl_token_2022::instruction::initialize_account(
        &spl_token_2022::id(),
        &usdc_token_account_keypair.pubkey(),
        &usdc_mint_key,
        payer,
    )
    .unwrap();
    send_tx_with_retry(
        Rc::clone(&context),
        &[
            create_spl_token_account_ix,
            init_spl_token_account_ix,
            create_usdc_token_account_ix,
            init_usdc_token_account_ix,
        ],
        Some(&payer),
        &[
            &payer_keypair.insecure_clone(),
            &spl_token_account_keypair.insecure_clone(),
            &usdc_token_account_keypair.insecure_clone(),
        ],
    )
    .await?;

    // Add funds to those token accounts.
    let spl_mint_to_instruction: Instruction = spl_token::instruction::mint_to(
        &spl_token::ID,
        &spl_mint_key,
        &spl_token_account_keypair.pubkey(),
        &payer,
        &[&payer],
        1_000_000_000_000_000,
    )
    .unwrap();
    let usdc_mint_to_instruction: Instruction = spl_token_2022::instruction::mint_to(
        &spl_token_2022::ID,
        &usdc_mint_key,
        &usdc_token_account_keypair.pubkey(),
        &payer,
        &[&payer],
        1_000_000_000_000_000,
    )
    .unwrap();
    send_tx_with_retry(
        Rc::clone(&context),
        &[spl_mint_to_instruction, usdc_mint_to_instruction],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    // Call deposit for each token account for a partial amount so we can swap later.
    let deposit_spl_ix: Instruction = deposit_instruction(
        &market_keypair.pubkey(),
        &payer,
        &spl_mint_key,
        1_000_000_000,
        &spl_token_account_keypair.pubkey(),
        spl_token::id(),
        None,
    );
    let deposit_usdc_ix: Instruction = deposit_instruction(
        &market_keypair.pubkey(),
        &payer,
        &usdc_mint_key,
        1_000_000_000,
        &usdc_token_account_keypair.pubkey(),
        spl_token_2022::id(),
        None,
    );
    send_tx_with_retry(
        Rc::clone(&context),
        &[deposit_spl_ix, deposit_usdc_ix],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    // Call withdraw
    let withdraw_spl_ix: Instruction = withdraw_instruction(
        &market_keypair.pubkey(),
        &payer,
        &spl_mint_key,
        1_000,
        &spl_token_account_keypair.pubkey(),
        spl_token::id(),
        None,
    );
    let withdraw_usdc_ix: Instruction = withdraw_instruction(
        &market_keypair.pubkey(),
        &payer,
        &usdc_mint_key,
        1_000,
        &usdc_token_account_keypair.pubkey(),
        spl_token_2022::id(),
        None,
    );
    send_tx_with_retry(
        Rc::clone(&context),
        &[withdraw_spl_ix, withdraw_usdc_ix],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    // Place orders on both sides to
    let place_order_ix: Instruction = batch_update_instruction(
        &market_keypair.pubkey(),
        &payer,
        None,
        vec![],
        vec![
            PlaceOrderParams::new(
                1_000,
                9,
                -1,
                true,
                OrderType::PostOnly,
                NO_EXPIRATION_LAST_VALID_SLOT,
            ),
            PlaceOrderParams::new(
                1_000,
                11,
                -1,
                false,
                OrderType::PostOnly,
                NO_EXPIRATION_LAST_VALID_SLOT,
            ),
        ],
        None,
        None,
        None,
        None,
    );
    send_tx_with_retry(
        Rc::clone(&context),
        &[place_order_ix],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    // Swap using both directions
    let swap_base_in_ix: Instruction = swap_instruction(
        &market_keypair.pubkey(),
        &payer,
        &spl_mint_key,
        &usdc_mint_key,
        &spl_token_account_keypair.pubkey(),
        &usdc_token_account_keypair.pubkey(),
        100,
        10,
        true,
        true,
        spl_token::id(),
        spl_token_2022::id(),
        false,
    );
    let swap_base_out_ix: Instruction = swap_instruction(
        &market_keypair.pubkey(),
        &payer,
        &spl_mint_key,
        &usdc_mint_key,
        &spl_token_account_keypair.pubkey(),
        &usdc_token_account_keypair.pubkey(),
        100,
        10,
        false,
        true,
        spl_token::id(),
        spl_token_2022::id(),
        false,
    );
    send_tx_with_retry(
        Rc::clone(&context),
        &[swap_base_in_ix, swap_base_out_ix],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    Ok(())
}

#[tokio::test]
async fn token22_deposit_transfer_fee() -> anyhow::Result<()> {
    let program_test: ProgramTest = ProgramTest::new(
        "manifest",
        manifest::ID,
        processor!(manifest::process_instruction),
    );
    solana_logger::setup_with_default(RUST_LOG_DEFAULT);

    let market_keypair: Keypair = Keypair::new();

    let context: Rc<RefCell<ProgramTestContext>> =
        Rc::new(RefCell::new(program_test.start_with_context().await));

    let payer_keypair: Keypair = context.borrow().payer.insecure_clone();
    let payer: &Pubkey = &payer_keypair.pubkey();

    // For this test, usdc is old token and spl is token22.
    let usdc_mint_f: MintFixture =
        MintFixture::new_with_version(Rc::clone(&context), Some(6), false).await;

    let spl_mint_keypair: Keypair = Keypair::new();
    let extension_types: Vec<spl_token_2022::extension::ExtensionType> =
        vec![spl_token_2022::extension::ExtensionType::TransferFeeConfig];
    let space: usize = spl_token_2022::extension::ExtensionType::try_calculate_account_len::<
        spl_token_2022::state::Mint,
    >(&extension_types)
    .unwrap();
    // first create the mint account for the new NFT
    let mint_rent: u64 = solana_program::sysvar::rent::Rent::default().minimum_balance(space);

    let init_account_ix: Instruction = create_account(
        &payer,
        &spl_mint_keypair.pubkey(),
        mint_rent,
        space as u64,
        &spl_token_2022::id(),
    );
    let init_mint_ix: Instruction = spl_token_2022::instruction::initialize_mint2(
        &spl_token_2022::id(),
        &spl_mint_keypair.pubkey(),
        &payer,
        None,
        6,
    )
    .unwrap();

    // 1_000 bps =  10%
    let transfer_fee_ix: Instruction =
        spl_token_2022::extension::transfer_fee::instruction::initialize_transfer_fee_config(
            &spl_token_2022::id(),
            &spl_mint_keypair.pubkey(),
            None,
            None,
            1_000,
            u64::MAX,
        )
        .unwrap();

    send_tx_with_retry(
        Rc::clone(&context),
        &[init_account_ix, transfer_fee_ix, init_mint_ix],
        Some(&payer),
        &[&payer_keypair, &spl_mint_keypair],
    )
    .await
    .unwrap();

    let spl_mint_key: Pubkey = spl_mint_keypair.pubkey();

    // Create the market with SPL as base which is 2022, USDC as quote which is normal.
    let create_market_ixs: Vec<Instruction> = create_market_instructions(
        &market_keypair.pubkey(),
        &spl_mint_key,
        &usdc_mint_f.key,
        payer,
    )
    .unwrap();
    send_tx_with_retry(
        Rc::clone(&context),
        &create_market_ixs[..],
        Some(&payer),
        &[&payer_keypair.insecure_clone(), &market_keypair],
    )
    .await?;

    // Claim seat
    let claim_seat_ix: Instruction = claim_seat_instruction(&market_keypair.pubkey(), &payer);
    send_tx_with_retry(
        Rc::clone(&context),
        &[claim_seat_ix],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    // Create depositor token accounts
    let spl_token_account_keypair: Keypair = Keypair::new();
    let rent: Rent = context.borrow_mut().banks_client.get_rent().await.unwrap();
    let create_spl_token_account_ix: Instruction = create_account(
        payer,
        &spl_token_account_keypair.pubkey(),
        rent.minimum_balance(spl_token_2022::state::Account::LEN + 13),
        spl_token_2022::state::Account::LEN as u64 + 13,
        &spl_token_2022::id(),
    );
    let init_spl_token_account_ix: Instruction = spl_token_2022::instruction::initialize_account(
        &spl_token_2022::id(),
        &spl_token_account_keypair.pubkey(),
        &spl_mint_key,
        payer,
    )
    .unwrap();
    send_tx_with_retry(
        Rc::clone(&context),
        &[create_spl_token_account_ix, init_spl_token_account_ix],
        Some(&payer),
        &[
            &payer_keypair.insecure_clone(),
            &spl_token_account_keypair.insecure_clone(),
        ],
    )
    .await?;

    // Add funds to token account.
    let spl_mint_to_instruction: Instruction = spl_token_2022::instruction::mint_to(
        &spl_token_2022::ID,
        &spl_mint_key,
        &spl_token_account_keypair.pubkey(),
        &payer,
        &[&payer],
        1_000_000_000_000_000,
    )
    .unwrap();
    send_tx_with_retry(
        Rc::clone(&context),
        &[spl_mint_to_instruction],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    let deposit_spl_ix: Instruction = deposit_instruction(
        &market_keypair.pubkey(),
        &payer,
        &spl_mint_key,
        1_000_000_000,
        &spl_token_account_keypair.pubkey(),
        spl_token_2022::id(),
        None,
    );
    send_tx_with_retry(
        Rc::clone(&context),
        &[deposit_spl_ix],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    let market_account: solana_sdk::account::Account = context
        .borrow_mut()
        .banks_client
        .get_account(market_keypair.pubkey())
        .await
        .unwrap()
        .unwrap();

    let market: manifest::state::MarketValue =
        manifest::program::get_dynamic_value(market_account.data.as_slice());
    let balance = market.get_trader_balance(&payer);
    assert_eq!(balance.0.as_u64(), 900_000_000);

    Ok(())
}

/// Test that get_now_epoch correctly determines which transfer fee to apply
/// based on the current epoch. Tests the epoch-based fee switching behavior.
#[tokio::test]
async fn token22_transfer_fee_epoch_switching() -> anyhow::Result<()> {
    let program_test: ProgramTest = ProgramTest::new(
        "manifest",
        manifest::ID,
        processor!(manifest::process_instruction),
    );
    solana_logger::setup_with_default(RUST_LOG_DEFAULT);

    let market_keypair: Keypair = Keypair::new();

    let context: Rc<RefCell<ProgramTestContext>> =
        Rc::new(RefCell::new(program_test.start_with_context().await));

    let payer_keypair: Keypair = context.borrow().payer.insecure_clone();
    let payer: &Pubkey = &payer_keypair.pubkey();

    // Create a regular SPL token for quote
    let usdc_mint_f: MintFixture =
        MintFixture::new_with_version(Rc::clone(&context), Some(6), false).await;

    // Create Token22 mint with transfer fee config
    // Initial fee: 5% (500 bps)
    let spl_mint_keypair: Keypair = Keypair::new();
    let extension_types: Vec<spl_token_2022::extension::ExtensionType> =
        vec![spl_token_2022::extension::ExtensionType::TransferFeeConfig];
    let space: usize = spl_token_2022::extension::ExtensionType::try_calculate_account_len::<
        spl_token_2022::state::Mint,
    >(&extension_types)
    .unwrap();
    let mint_rent: u64 = solana_program::sysvar::rent::Rent::default().minimum_balance(space);

    let init_account_ix: Instruction = create_account(
        &payer,
        &spl_mint_keypair.pubkey(),
        mint_rent,
        space as u64,
        &spl_token_2022::id(),
    );

    // Initialize with 5% fee (500 bps)
    let transfer_fee_ix: Instruction =
        spl_token_2022::extension::transfer_fee::instruction::initialize_transfer_fee_config(
            &spl_token_2022::id(),
            &spl_mint_keypair.pubkey(),
            Some(&payer), // transfer_fee_config_authority
            Some(&payer), // withdraw_withheld_authority
            500,          // 5% fee
            u64::MAX,
        )
        .unwrap();

    let init_mint_ix: Instruction = spl_token_2022::instruction::initialize_mint2(
        &spl_token_2022::id(),
        &spl_mint_keypair.pubkey(),
        &payer,
        None,
        6,
    )
    .unwrap();

    send_tx_with_retry(
        Rc::clone(&context),
        &[init_account_ix, transfer_fee_ix, init_mint_ix],
        Some(&payer),
        &[&payer_keypair, &spl_mint_keypair],
    )
    .await
    .unwrap();

    let spl_mint_key: Pubkey = spl_mint_keypair.pubkey();

    // Create market
    let create_market_ixs: Vec<Instruction> = create_market_instructions(
        &market_keypair.pubkey(),
        &spl_mint_key,
        &usdc_mint_f.key,
        payer,
    )
    .unwrap();
    send_tx_with_retry(
        Rc::clone(&context),
        &create_market_ixs[..],
        Some(&payer),
        &[&payer_keypair.insecure_clone(), &market_keypair],
    )
    .await?;

    // Claim seat
    let claim_seat_ix: Instruction = claim_seat_instruction(&market_keypair.pubkey(), &payer);
    send_tx_with_retry(
        Rc::clone(&context),
        &[claim_seat_ix],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    // Create token accounts
    let spl_token_account_keypair: Keypair = Keypair::new();
    let usdc_token_account_keypair: Keypair = Keypair::new();
    let rent: Rent = context.borrow_mut().banks_client.get_rent().await.unwrap();

    // SPL token account needs extra space for transfer fee extension
    let create_spl_token_account_ix: Instruction = create_account(
        payer,
        &spl_token_account_keypair.pubkey(),
        rent.minimum_balance(spl_token_2022::state::Account::LEN + 13),
        spl_token_2022::state::Account::LEN as u64 + 13,
        &spl_token_2022::id(),
    );
    let init_spl_token_account_ix: Instruction = spl_token_2022::instruction::initialize_account(
        &spl_token_2022::id(),
        &spl_token_account_keypair.pubkey(),
        &spl_mint_key,
        payer,
    )
    .unwrap();

    let create_usdc_token_account_ix: Instruction = create_account(
        payer,
        &usdc_token_account_keypair.pubkey(),
        rent.minimum_balance(spl_token::state::Account::LEN),
        spl_token::state::Account::LEN as u64,
        &spl_token::id(),
    );
    let init_usdc_token_account_ix: Instruction = spl_token::instruction::initialize_account(
        &spl_token::id(),
        &usdc_token_account_keypair.pubkey(),
        &usdc_mint_f.key,
        payer,
    )
    .unwrap();

    send_tx_with_retry(
        Rc::clone(&context),
        &[
            create_spl_token_account_ix,
            init_spl_token_account_ix,
            create_usdc_token_account_ix,
            init_usdc_token_account_ix,
        ],
        Some(&payer),
        &[
            &payer_keypair.insecure_clone(),
            &spl_token_account_keypair.insecure_clone(),
            &usdc_token_account_keypair.insecure_clone(),
        ],
    )
    .await?;

    // Mint tokens
    let spl_mint_to_instruction: Instruction = spl_token_2022::instruction::mint_to(
        &spl_token_2022::ID,
        &spl_mint_key,
        &spl_token_account_keypair.pubkey(),
        &payer,
        &[&payer],
        1_000_000_000_000,
    )
    .unwrap();
    let usdc_mint_to_instruction: Instruction = spl_token::instruction::mint_to(
        &spl_token::ID,
        &usdc_mint_f.key,
        &usdc_token_account_keypair.pubkey(),
        &payer,
        &[&payer],
        1_000_000_000_000,
    )
    .unwrap();
    send_tx_with_retry(
        Rc::clone(&context),
        &[spl_mint_to_instruction, usdc_mint_to_instruction],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    // Deposit with 5% fee - depositing 1_000_000_000 should result in 950_000_000 credited
    let deposit_spl_ix: Instruction = deposit_instruction(
        &market_keypair.pubkey(),
        &payer,
        &spl_mint_key,
        1_000_000_000,
        &spl_token_account_keypair.pubkey(),
        spl_token_2022::id(),
        None,
    );
    send_tx_with_retry(
        Rc::clone(&context),
        &[deposit_spl_ix],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    // Verify 5% fee was applied (1_000_000_000 * 0.95 = 950_000_000)
    {
        let market_account: solana_sdk::account::Account = context
            .borrow_mut()
            .banks_client
            .get_account(market_keypair.pubkey())
            .await
            .unwrap()
            .unwrap();

        let market: manifest::state::MarketValue =
            manifest::program::get_dynamic_value(market_account.data.as_slice());
        let balance = market.get_trader_balance(&payer);
        assert_eq!(
            balance.0.as_u64(),
            950_000_000,
            "Expected 5% fee to be applied (950_000_000), got {}",
            balance.0.as_u64()
        );
    }

    // Now schedule a new fee (10% = 1000 bps) for a future epoch
    // Get current epoch and schedule for epoch + 1
    let current_clock: solana_sdk::clock::Clock = context
        .borrow_mut()
        .banks_client
        .get_sysvar()
        .await
        .unwrap();
    let current_epoch = current_clock.epoch;

    let set_fee_ix: Instruction =
        spl_token_2022::extension::transfer_fee::instruction::set_transfer_fee(
            &spl_token_2022::id(),
            &spl_mint_key,
            &payer,
            &[],
            1000, // 10% fee
            u64::MAX,
        )
        .unwrap();

    send_tx_with_retry(
        Rc::clone(&context),
        &[set_fee_ix],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    // Advance epoch to activate the new fee
    // The new fee becomes active 2 epochs after it's set
    let mut new_clock = current_clock;
    new_clock.epoch = current_epoch + 2;
    new_clock.slot += 1_000_000; // Advance slot significantly
    context.borrow_mut().set_sysvar(&new_clock);

    // Deposit again - now 10% fee should be applied
    let deposit_spl_ix_2: Instruction = deposit_instruction(
        &market_keypair.pubkey(),
        &payer,
        &spl_mint_key,
        1_000_000_000,
        &spl_token_account_keypair.pubkey(),
        spl_token_2022::id(),
        None,
    );
    send_tx_with_retry(
        Rc::clone(&context),
        &[deposit_spl_ix_2],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    // Verify 10% fee was applied (previous 950_000_000 + 900_000_000 = 1_850_000_000)
    {
        let market_account: solana_sdk::account::Account = context
            .borrow_mut()
            .banks_client
            .get_account(market_keypair.pubkey())
            .await
            .unwrap()
            .unwrap();

        let market: manifest::state::MarketValue =
            manifest::program::get_dynamic_value(market_account.data.as_slice());
        let balance = market.get_trader_balance(&payer);
        assert_eq!(
            balance.0.as_u64(),
            1_850_000_000,
            "Expected 10% fee on second deposit (950M + 900M = 1.85B), got {}",
            balance.0.as_u64()
        );
    }

    Ok(())
}

/// Test that transfer fee is correctly applied based on the older epoch fee
/// when the current epoch is before the newer fee epoch.
#[tokio::test]
async fn token22_transfer_fee_older_epoch() -> anyhow::Result<()> {
    let program_test: ProgramTest = ProgramTest::new(
        "manifest",
        manifest::ID,
        processor!(manifest::process_instruction),
    );
    solana_logger::setup_with_default(RUST_LOG_DEFAULT);

    let market_keypair: Keypair = Keypair::new();

    let context: Rc<RefCell<ProgramTestContext>> =
        Rc::new(RefCell::new(program_test.start_with_context().await));

    let payer_keypair: Keypair = context.borrow().payer.insecure_clone();
    let payer: &Pubkey = &payer_keypair.pubkey();

    // Create a regular SPL token for quote
    let usdc_mint_f: MintFixture =
        MintFixture::new_with_version(Rc::clone(&context), Some(6), false).await;

    // Create Token22 mint with transfer fee config starting at 0%
    let spl_mint_keypair: Keypair = Keypair::new();
    let extension_types: Vec<spl_token_2022::extension::ExtensionType> =
        vec![spl_token_2022::extension::ExtensionType::TransferFeeConfig];
    let space: usize = spl_token_2022::extension::ExtensionType::try_calculate_account_len::<
        spl_token_2022::state::Mint,
    >(&extension_types)
    .unwrap();
    let mint_rent: u64 = solana_program::sysvar::rent::Rent::default().minimum_balance(space);

    let init_account_ix: Instruction = create_account(
        &payer,
        &spl_mint_keypair.pubkey(),
        mint_rent,
        space as u64,
        &spl_token_2022::id(),
    );

    // Initialize with 0% fee
    let transfer_fee_ix: Instruction =
        spl_token_2022::extension::transfer_fee::instruction::initialize_transfer_fee_config(
            &spl_token_2022::id(),
            &spl_mint_keypair.pubkey(),
            Some(&payer),
            Some(&payer),
            0, // 0% fee initially
            u64::MAX,
        )
        .unwrap();

    let init_mint_ix: Instruction = spl_token_2022::instruction::initialize_mint2(
        &spl_token_2022::id(),
        &spl_mint_keypair.pubkey(),
        &payer,
        None,
        6,
    )
    .unwrap();

    send_tx_with_retry(
        Rc::clone(&context),
        &[init_account_ix, transfer_fee_ix, init_mint_ix],
        Some(&payer),
        &[&payer_keypair, &spl_mint_keypair],
    )
    .await
    .unwrap();

    let spl_mint_key: Pubkey = spl_mint_keypair.pubkey();

    // Schedule a 20% fee for a future epoch
    let set_fee_ix: Instruction =
        spl_token_2022::extension::transfer_fee::instruction::set_transfer_fee(
            &spl_token_2022::id(),
            &spl_mint_key,
            &payer,
            &[],
            2000, // 20% fee scheduled
            u64::MAX,
        )
        .unwrap();

    send_tx_with_retry(
        Rc::clone(&context),
        &[set_fee_ix],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    // Create market (still at epoch 0, so 0% fee should apply)
    let create_market_ixs: Vec<Instruction> = create_market_instructions(
        &market_keypair.pubkey(),
        &spl_mint_key,
        &usdc_mint_f.key,
        payer,
    )
    .unwrap();
    send_tx_with_retry(
        Rc::clone(&context),
        &create_market_ixs[..],
        Some(&payer),
        &[&payer_keypair.insecure_clone(), &market_keypair],
    )
    .await?;

    // Claim seat
    let claim_seat_ix: Instruction = claim_seat_instruction(&market_keypair.pubkey(), &payer);
    send_tx_with_retry(
        Rc::clone(&context),
        &[claim_seat_ix],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    // Create token account
    let spl_token_account_keypair: Keypair = Keypair::new();
    let rent: Rent = context.borrow_mut().banks_client.get_rent().await.unwrap();

    let create_spl_token_account_ix: Instruction = create_account(
        payer,
        &spl_token_account_keypair.pubkey(),
        rent.minimum_balance(spl_token_2022::state::Account::LEN + 13),
        spl_token_2022::state::Account::LEN as u64 + 13,
        &spl_token_2022::id(),
    );
    let init_spl_token_account_ix: Instruction = spl_token_2022::instruction::initialize_account(
        &spl_token_2022::id(),
        &spl_token_account_keypair.pubkey(),
        &spl_mint_key,
        payer,
    )
    .unwrap();

    send_tx_with_retry(
        Rc::clone(&context),
        &[create_spl_token_account_ix, init_spl_token_account_ix],
        Some(&payer),
        &[
            &payer_keypair.insecure_clone(),
            &spl_token_account_keypair.insecure_clone(),
        ],
    )
    .await?;

    // Mint tokens
    let spl_mint_to_instruction: Instruction = spl_token_2022::instruction::mint_to(
        &spl_token_2022::ID,
        &spl_mint_key,
        &spl_token_account_keypair.pubkey(),
        &payer,
        &[&payer],
        1_000_000_000_000,
    )
    .unwrap();
    send_tx_with_retry(
        Rc::clone(&context),
        &[spl_mint_to_instruction],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    // Deposit with 0% fee (older epoch fee) - full amount should be credited
    let deposit_spl_ix: Instruction = deposit_instruction(
        &market_keypair.pubkey(),
        &payer,
        &spl_mint_key,
        1_000_000_000,
        &spl_token_account_keypair.pubkey(),
        spl_token_2022::id(),
        None,
    );
    send_tx_with_retry(
        Rc::clone(&context),
        &[deposit_spl_ix],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    // Verify 0% fee was applied (full amount credited)
    {
        let market_account: solana_sdk::account::Account = context
            .borrow_mut()
            .banks_client
            .get_account(market_keypair.pubkey())
            .await
            .unwrap()
            .unwrap();

        let market: manifest::state::MarketValue =
            manifest::program::get_dynamic_value(market_account.data.as_slice());
        let balance = market.get_trader_balance(&payer);
        assert_eq!(
            balance.0.as_u64(),
            1_000_000_000,
            "Expected 0% fee (older epoch fee), full amount (1_000_000_000), got {}",
            balance.0.as_u64()
        );
    }

    Ok(())
}

/// Test that get_now_epoch correctly handles the transition from zero fee to non-zero fee
/// when the epoch advances.
#[tokio::test]
async fn token22_transfer_fee_zero_to_nonzero() -> anyhow::Result<()> {
    let program_test: ProgramTest = ProgramTest::new(
        "manifest",
        manifest::ID,
        processor!(manifest::process_instruction),
    );
    solana_logger::setup_with_default(RUST_LOG_DEFAULT);

    let market_keypair: Keypair = Keypair::new();

    let context: Rc<RefCell<ProgramTestContext>> =
        Rc::new(RefCell::new(program_test.start_with_context().await));

    let payer_keypair: Keypair = context.borrow().payer.insecure_clone();
    let payer: &Pubkey = &payer_keypair.pubkey();

    // Create a regular SPL token for quote
    let usdc_mint_f: MintFixture =
        MintFixture::new_with_version(Rc::clone(&context), Some(6), false).await;

    // Create Token22 mint with 0% initial transfer fee
    let spl_mint_keypair: Keypair = Keypair::new();
    let extension_types: Vec<spl_token_2022::extension::ExtensionType> =
        vec![spl_token_2022::extension::ExtensionType::TransferFeeConfig];
    let space: usize = spl_token_2022::extension::ExtensionType::try_calculate_account_len::<
        spl_token_2022::state::Mint,
    >(&extension_types)
    .unwrap();
    let mint_rent: u64 = solana_program::sysvar::rent::Rent::default().minimum_balance(space);

    let init_account_ix: Instruction = create_account(
        &payer,
        &spl_mint_keypair.pubkey(),
        mint_rent,
        space as u64,
        &spl_token_2022::id(),
    );

    // Start with 0% fee
    let transfer_fee_ix: Instruction =
        spl_token_2022::extension::transfer_fee::instruction::initialize_transfer_fee_config(
            &spl_token_2022::id(),
            &spl_mint_keypair.pubkey(),
            Some(&payer),
            Some(&payer),
            0,
            u64::MAX,
        )
        .unwrap();

    let init_mint_ix: Instruction = spl_token_2022::instruction::initialize_mint2(
        &spl_token_2022::id(),
        &spl_mint_keypair.pubkey(),
        &payer,
        None,
        6,
    )
    .unwrap();

    send_tx_with_retry(
        Rc::clone(&context),
        &[init_account_ix, transfer_fee_ix, init_mint_ix],
        Some(&payer),
        &[&payer_keypair, &spl_mint_keypair],
    )
    .await
    .unwrap();

    let spl_mint_key: Pubkey = spl_mint_keypair.pubkey();

    // Create market
    let create_market_ixs: Vec<Instruction> = create_market_instructions(
        &market_keypair.pubkey(),
        &spl_mint_key,
        &usdc_mint_f.key,
        payer,
    )
    .unwrap();
    send_tx_with_retry(
        Rc::clone(&context),
        &create_market_ixs[..],
        Some(&payer),
        &[&payer_keypair.insecure_clone(), &market_keypair],
    )
    .await?;

    // Claim seat
    let claim_seat_ix: Instruction = claim_seat_instruction(&market_keypair.pubkey(), &payer);
    send_tx_with_retry(
        Rc::clone(&context),
        &[claim_seat_ix],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    // Create token account
    let spl_token_account_keypair: Keypair = Keypair::new();
    let rent: Rent = context.borrow_mut().banks_client.get_rent().await.unwrap();

    let create_spl_token_account_ix: Instruction = create_account(
        payer,
        &spl_token_account_keypair.pubkey(),
        rent.minimum_balance(spl_token_2022::state::Account::LEN + 13),
        spl_token_2022::state::Account::LEN as u64 + 13,
        &spl_token_2022::id(),
    );
    let init_spl_token_account_ix: Instruction = spl_token_2022::instruction::initialize_account(
        &spl_token_2022::id(),
        &spl_token_account_keypair.pubkey(),
        &spl_mint_key,
        payer,
    )
    .unwrap();

    send_tx_with_retry(
        Rc::clone(&context),
        &[create_spl_token_account_ix, init_spl_token_account_ix],
        Some(&payer),
        &[
            &payer_keypair.insecure_clone(),
            &spl_token_account_keypair.insecure_clone(),
        ],
    )
    .await?;

    // Mint tokens
    let spl_mint_to_instruction: Instruction = spl_token_2022::instruction::mint_to(
        &spl_token_2022::ID,
        &spl_mint_key,
        &spl_token_account_keypair.pubkey(),
        &payer,
        &[&payer],
        1_000_000_000_000,
    )
    .unwrap();
    send_tx_with_retry(
        Rc::clone(&context),
        &[spl_mint_to_instruction],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    // First deposit with 0% fee
    let deposit_spl_ix: Instruction = deposit_instruction(
        &market_keypair.pubkey(),
        &payer,
        &spl_mint_key,
        1_000_000_000,
        &spl_token_account_keypair.pubkey(),
        spl_token_2022::id(),
        None,
    );
    send_tx_with_retry(
        Rc::clone(&context),
        &[deposit_spl_ix],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    // Verify full amount credited with 0% fee
    {
        let market_account: solana_sdk::account::Account = context
            .borrow_mut()
            .banks_client
            .get_account(market_keypair.pubkey())
            .await
            .unwrap()
            .unwrap();

        let market: manifest::state::MarketValue =
            manifest::program::get_dynamic_value(market_account.data.as_slice());
        let balance = market.get_trader_balance(&payer);
        assert_eq!(
            balance.0.as_u64(),
            1_000_000_000,
            "Expected 0% fee, full amount (1_000_000_000), got {}",
            balance.0.as_u64()
        );
    }

    // Schedule 15% fee for future epoch
    let set_fee_ix: Instruction =
        spl_token_2022::extension::transfer_fee::instruction::set_transfer_fee(
            &spl_token_2022::id(),
            &spl_mint_key,
            &payer,
            &[],
            1500, // 15%
            u64::MAX,
        )
        .unwrap();

    send_tx_with_retry(
        Rc::clone(&context),
        &[set_fee_ix],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    // Advance to new epoch where 15% fee is active
    let current_clock: solana_sdk::clock::Clock = context
        .borrow_mut()
        .banks_client
        .get_sysvar()
        .await
        .unwrap();
    let mut new_clock = current_clock;
    new_clock.epoch += 2;
    new_clock.slot += 1_000_000;
    context.borrow_mut().set_sysvar(&new_clock);

    // Second deposit - now 15% fee should apply
    let deposit_spl_ix_2: Instruction = deposit_instruction(
        &market_keypair.pubkey(),
        &payer,
        &spl_mint_key,
        1_000_000_000,
        &spl_token_account_keypair.pubkey(),
        spl_token_2022::id(),
        None,
    );
    send_tx_with_retry(
        Rc::clone(&context),
        &[deposit_spl_ix_2],
        Some(&payer),
        &[&payer_keypair.insecure_clone()],
    )
    .await?;

    // Verify 15% fee applied (1_000_000_000 + 850_000_000 = 1_850_000_000)
    {
        let market_account: solana_sdk::account::Account = context
            .borrow_mut()
            .banks_client
            .get_account(market_keypair.pubkey())
            .await
            .unwrap()
            .unwrap();

        let market: manifest::state::MarketValue =
            manifest::program::get_dynamic_value(market_account.data.as_slice());
        let balance = market.get_trader_balance(&payer);
        assert_eq!(
            balance.0.as_u64(),
            1_850_000_000,
            "Expected 15% fee on second deposit (1B + 850M = 1.85B), got {}",
            balance.0.as_u64()
        );
    }

    Ok(())
}
