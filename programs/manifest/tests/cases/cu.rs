//! Compute unit measurements for each instruction.
//!
//! Every test simulates a representative transaction, prints a line of the
//! form `CU <name>: <units>` and then executes it. The numbers are only
//! meaningful when the compiled BPF program is loaded
//! (`cargo test-sbf --features "test,test-sbf"`); the native processor used by
//! plain `cargo test` does not meter compute, but the tests still exercise the
//! same instructions there.
//!
//! The program derives its vault and global PDAs on chain with
//! `find_program_address`, which costs about 1,500 CU per bump it has to try.
//! So that the numbers do not vary with the random test keys, markets and
//! mints are created with keys whose PDAs derive on the first bump.
//!
//! No test asserts a specific number, they exist so different builds of the
//! program can be compared line by line.

use std::{cell::RefMut, rc::Rc};

use hypertree::get_helper;
use manifest::{
    program::{
        batch_update::{CancelOrderParams, PlaceOrderParams},
        batch_update_instruction, claim_seat_instruction, create_global_instruction,
        create_market_instructions, deposit_instruction, expand_market_instruction,
        global_add_trader_instruction, global_deposit_instruction, global_withdraw_instruction,
        swap_instruction, withdraw_instruction,
    },
    state::{constants::NO_EXPIRATION_LAST_VALID_SLOT, MarketFixed, OrderType},
    validation::{get_global_address, get_global_vault_address, get_vault_address},
};
use solana_account::{Account, AccountSharedData};
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_program::{program_pack::Pack, pubkey::Pubkey, rent::Rent, system_instruction};
use solana_program_test::{tokio, ProgramTestContext};
use solana_signer::Signer;
use solana_transaction::Transaction;

use crate::{send_tx_with_retry, TestFixture, TokenAccountFixture, SOL_UNIT_SIZE, USDC_UNIT_SIZE};

/// Simulates `instructions` and returns the transaction result together with
/// the compute units the simulation consumed.
async fn simulate(
    test_fixture: &TestFixture,
    instructions: &[Instruction],
    payer: &Pubkey,
    signers: &[&Keypair],
) -> (Result<(), String>, u64) {
    let mut context: RefMut<ProgramTestContext> = test_fixture.context.borrow_mut();
    let blockhash: solana_program::hash::Hash = context.get_new_latest_blockhash().await.unwrap();
    let transaction: Transaction =
        Transaction::new_signed_with_payer(instructions, Some(payer), signers, blockhash);
    let simulation = context
        .banks_client
        .simulate_transaction(transaction)
        .await
        .unwrap();
    let units_consumed: u64 = simulation
        .simulation_details
        .expect("simulation details present")
        .units_consumed;
    let result: Result<(), String> = match simulation.result {
        Some(Err(error)) => Err(format!("{error:?}")),
        _ => Ok(()),
    };
    (result, units_consumed)
}

/// Simulates `instructions`, requiring success, prints the compute units as
/// `CU <name>: <units>` and then executes the transaction so that later
/// measurements in the same test see its effects.
async fn measure_and_send(
    test_fixture: &TestFixture,
    name: &str,
    instructions: &[Instruction],
    payer: &Pubkey,
    signers: &[&Keypair],
) -> anyhow::Result<u64> {
    let (result, units_consumed) = simulate(test_fixture, instructions, payer, signers).await;
    if let Err(error) = result {
        panic!("{name} simulation failed: {error}");
    }
    println!("CU {name}: {units_consumed}");
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        instructions,
        Some(payer),
        signers,
    )
    .await?;
    Ok(units_consumed)
}

/// A market keypair whose base and quote vault PDAs derive on the first bump.
fn market_keypair_with_first_bump_vaults(base_mint: &Pubkey, quote_mint: &Pubkey) -> Keypair {
    loop {
        let keypair: Keypair = Keypair::new();
        let (_, base_bump) = get_vault_address(&keypair.pubkey(), base_mint);
        let (_, quote_bump) = get_vault_address(&keypair.pubkey(), quote_mint);
        if base_bump == u8::MAX && quote_bump == u8::MAX {
            return keypair;
        }
    }
}

/// A mint keypair whose global and global vault PDAs derive on the first bump.
fn mint_keypair_with_first_bump_globals() -> Keypair {
    loop {
        let keypair: Keypair = Keypair::new();
        let (_, global_bump) = get_global_address(&keypair.pubkey());
        let (_, global_vault_bump) = get_global_vault_address(&keypair.pubkey());
        if global_bump == u8::MAX && global_vault_bump == u8::MAX {
            return keypair;
        }
    }
}

/// Creates a market on the fixture's SOL/USDC mints whose vault PDAs derive on
/// the first bump, returning its key.
async fn create_market_with_first_bump_vaults(
    test_fixture: &TestFixture,
) -> anyhow::Result<Pubkey> {
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair();
    let market_keypair: Keypair = market_keypair_with_first_bump_vaults(
        &test_fixture.sol_mint_fixture.key,
        &test_fixture.usdc_mint_fixture.key,
    );
    let create_market_ixs: Vec<Instruction> = create_market_instructions(
        &market_keypair.pubkey(),
        &test_fixture.sol_mint_fixture.key,
        &test_fixture.usdc_mint_fixture.key,
        &payer,
    )?;
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &create_market_ixs,
        Some(&payer),
        &[&payer_keypair, &market_keypair],
    )
    .await?;
    Ok(market_keypair.pubkey())
}

/// Cost of just reaching the dispatcher: an instruction with an unknown tag is
/// rejected before any account is touched, so all that is metered is the
/// entrypoint deserialization for the given number of accounts (plus the
/// dispatch). Measured for a range of account counts to expose the per-account
/// cost.
#[tokio::test]
async fn cu_entrypoint_only_test() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair();

    for num_extra_accounts in [0usize, 1, 2, 4, 8, 12, 16] {
        let mut accounts: Vec<AccountMeta> = vec![AccountMeta::new(payer, true)];
        for _ in 0..num_extra_accounts {
            accounts.push(AccountMeta::new_readonly(Pubkey::new_unique(), false));
        }
        let unknown_instruction: Instruction = Instruction {
            program_id: manifest::id(),
            accounts,
            data: vec![u8::MAX],
        };
        let (result, units_consumed) = simulate(
            &test_fixture,
            &[unknown_instruction],
            &payer,
            &[&payer_keypair],
        )
        .await;
        assert!(result.is_err(), "unknown instruction tag must be rejected");
        println!(
            "CU entrypoint_only[{} accounts]: {}",
            1 + num_extra_accounts,
            units_consumed
        );
    }
    Ok(())
}

#[tokio::test]
async fn cu_create_market_test() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair();
    let market_keypair: Keypair = market_keypair_with_first_bump_vaults(
        &test_fixture.sol_mint_fixture.key,
        &test_fixture.usdc_mint_fixture.key,
    );

    let create_market_ixs: Vec<Instruction> = create_market_instructions(
        &market_keypair.pubkey(),
        &test_fixture.sol_mint_fixture.key,
        &test_fixture.usdc_mint_fixture.key,
        &payer,
    )?;
    // Includes the system program create account instruction.
    measure_and_send(
        &test_fixture,
        "create_market",
        &create_market_ixs,
        &payer,
        &[&payer_keypair, &market_keypair],
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn cu_claim_seat_and_expand_test() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair();
    let market: Pubkey = create_market_with_first_bump_vaults(&test_fixture).await?;

    measure_and_send(
        &test_fixture,
        "claim_seat",
        &[claim_seat_instruction(&market, &payer)],
        &payer,
        &[&payer_keypair],
    )
    .await?;
    measure_and_send(
        &test_fixture,
        "expand_market",
        &[expand_market_instruction(&market, &payer)],
        &payer,
        &[&payer_keypair],
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn cu_deposit_and_withdraw_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair();
    let market: Pubkey = create_market_with_first_bump_vaults(&test_fixture).await?;
    let amount_atoms: u64 = 10 * SOL_UNIT_SIZE;

    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[claim_seat_instruction(&market, &payer)],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;
    test_fixture
        .sol_mint_fixture
        .mint_to(&test_fixture.payer_sol_fixture.key, amount_atoms)
        .await;

    measure_and_send(
        &test_fixture,
        "deposit",
        &[deposit_instruction(
            &market,
            &payer,
            &test_fixture.sol_mint_fixture.key,
            amount_atoms,
            &test_fixture.payer_sol_fixture.key,
            spl_token::id(),
            None,
        )],
        &payer,
        &[&payer_keypair],
    )
    .await?;
    measure_and_send(
        &test_fixture,
        "withdraw",
        &[withdraw_instruction(
            &market,
            &payer,
            &test_fixture.sol_mint_fixture.key,
            amount_atoms,
            &test_fixture.payer_sol_fixture.key,
            spl_token::id(),
            None,
        )],
        &payer,
        &[&payer_keypair],
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn cu_place_and_cancel_order_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair();
    let market: Pubkey = create_market_with_first_bump_vaults(&test_fixture).await?;
    let deposit_atoms: u64 = 10 * SOL_UNIT_SIZE;

    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[claim_seat_instruction(&market, &payer)],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;
    test_fixture
        .sol_mint_fixture
        .mint_to(&test_fixture.payer_sol_fixture.key, deposit_atoms)
        .await;
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[deposit_instruction(
            &market,
            &payer,
            &test_fixture.sol_mint_fixture.key,
            deposit_atoms,
            &test_fixture.payer_sol_fixture.key,
            spl_token::id(),
            None,
        )],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // Resting ask, 1 SOL at 1 USDC (price = 1e-3 quote atoms per base atom).
    let place_order_ix: Instruction = batch_update_instruction(
        &market,
        &payer,
        None,
        vec![],
        vec![PlaceOrderParams::new(
            1 * SOL_UNIT_SIZE,
            1,
            -3,
            false,
            OrderType::Limit,
            NO_EXPIRATION_LAST_VALID_SLOT,
        )],
        None,
        None,
        None,
        None,
    );
    measure_and_send(
        &test_fixture,
        "batch_update_place_1",
        &[place_order_ix],
        &payer,
        &[&payer_keypair],
    )
    .await?;

    // Five more asks at distinct prices in one instruction.
    let orders: Vec<PlaceOrderParams> = (2..7)
        .map(|price_mantissa: u32| {
            PlaceOrderParams::new(
                1 * SOL_UNIT_SIZE,
                price_mantissa,
                -3,
                false,
                OrderType::Limit,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )
        })
        .collect();
    let place_orders_ix: Instruction = batch_update_instruction(
        &market,
        &payer,
        None,
        vec![],
        orders,
        None,
        None,
        None,
        None,
    );
    measure_and_send(
        &test_fixture,
        "batch_update_place_5",
        &[place_orders_ix],
        &payer,
        &[&payer_keypair],
    )
    .await?;

    // The first order placed on a fresh market has sequence number 0.
    let cancel_order_ix: Instruction = batch_update_instruction(
        &market,
        &payer,
        None,
        vec![CancelOrderParams::new(0)],
        vec![],
        None,
        None,
        None,
        None,
    );
    measure_and_send(
        &test_fixture,
        "batch_update_cancel_1",
        &[cancel_order_ix],
        &payer,
        &[&payer_keypair],
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn cu_swap_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair();
    let market: Pubkey = create_market_with_first_bump_vaults(&test_fixture).await?;
    let deposit_atoms: u64 = 10 * SOL_UNIT_SIZE;

    // Maker: seat, deposit and a resting ask of 1 SOL at 1 USDC.
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[claim_seat_instruction(&market, &payer)],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;
    test_fixture
        .sol_mint_fixture
        .mint_to(&test_fixture.payer_sol_fixture.key, deposit_atoms)
        .await;
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[
            deposit_instruction(
                &market,
                &payer,
                &test_fixture.sol_mint_fixture.key,
                deposit_atoms,
                &test_fixture.payer_sol_fixture.key,
                spl_token::id(),
                None,
            ),
            batch_update_instruction(
                &market,
                &payer,
                None,
                vec![],
                vec![PlaceOrderParams::new(
                    1 * SOL_UNIT_SIZE,
                    1,
                    -3,
                    false,
                    OrderType::Limit,
                    NO_EXPIRATION_LAST_VALID_SLOT,
                )],
                None,
                None,
                None,
                None,
            ),
        ],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // Taker buys 1 SOL with 1 USDC from the wallet, filling the single ask.
    let quote_in_atoms: u64 = 1 * USDC_UNIT_SIZE;
    test_fixture
        .usdc_mint_fixture
        .mint_to(&test_fixture.payer_usdc_fixture.key, quote_in_atoms)
        .await;
    let swap_ix: Instruction = swap_instruction(
        &market,
        &payer,
        &test_fixture.sol_mint_fixture.key,
        &test_fixture.usdc_mint_fixture.key,
        &test_fixture.payer_sol_fixture.key,
        &test_fixture.payer_usdc_fixture.key,
        quote_in_atoms,
        1 * SOL_UNIT_SIZE,
        false,
        true,
        spl_token::id(),
        spl_token::id(),
        false,
    );
    measure_and_send(
        &test_fixture,
        "swap_fill_1",
        &[swap_ix],
        &payer,
        &[&payer_keypair],
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn cu_global_test() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair();
    let amount_atoms: u64 = 10 * USDC_UNIT_SIZE;

    // A mint whose global PDAs derive on the first bump, with the payer as its
    // mint authority, and a token account for the payer.
    let mint_keypair: Keypair = mint_keypair_with_first_bump_globals();
    let mint: Pubkey = mint_keypair.pubkey();
    let rent: Rent = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_rent()
        .await?;
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[
            system_instruction::create_account(
                &payer,
                &mint,
                rent.minimum_balance(spl_token::state::Mint::LEN),
                spl_token::state::Mint::LEN as u64,
                &spl_token::id(),
            ),
            spl_token::instruction::initialize_mint(&spl_token::id(), &mint, &payer, None, 6)?,
        ],
        Some(&payer),
        &[&payer_keypair, &mint_keypair],
    )
    .await?;
    let token_account: TokenAccountFixture =
        TokenAccountFixture::new(Rc::clone(&test_fixture.context), &mint, &payer).await;
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[
            spl_token::instruction::mint_to(
                &spl_token::id(),
                &mint,
                &token_account.key,
                &payer,
                &[&payer],
                amount_atoms,
            )?,
            create_global_instruction(&mint, &payer, &spl_token::id()),
        ],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    let (global, _) = get_global_address(&mint);
    measure_and_send(
        &test_fixture,
        "global_add_trader",
        &[global_add_trader_instruction(&global, &payer)],
        &payer,
        &[&payer_keypair],
    )
    .await?;
    measure_and_send(
        &test_fixture,
        "global_deposit",
        &[global_deposit_instruction(
            &mint,
            &payer,
            &token_account.key,
            &spl_token::id(),
            amount_atoms,
        )],
        &payer,
        &[&payer_keypair],
    )
    .await?;
    measure_and_send(
        &test_fixture,
        "global_withdraw",
        &[global_withdraw_instruction(
            &mint,
            &payer,
            &token_account.key,
            &spl_token::id(),
            amount_atoms,
        )],
        &payer,
        &[&payer_keypair],
    )
    .await?;
    Ok(())
}

/// Batch update carrying the global accounts for both sides. The first call
/// is on a market whose cached global addresses were cleared, so it derives
/// and stores them (its cost depends on how many bumps the search tries for
/// the fixture's random mints); the second call takes the cached path.
#[tokio::test]
async fn cu_batch_update_with_globals_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    test_fixture.claim_seat().await?;
    test_fixture.global_add_trader().await?;
    test_fixture.global_deposit(10 * USDC_UNIT_SIZE).await?;
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair();
    let market: Pubkey = test_fixture.market_fixture.key;

    // Clear the cache to behave like a market from before it existed.
    let mut market_account: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(market)
        .await?
        .expect("market exists");
    market_account.data[192..256].fill(0);
    test_fixture
        .context
        .borrow_mut()
        .set_account(&market, &AccountSharedData::from(market_account));

    for (name, price_mantissa) in [("uncached", 1u32), ("cached", 2u32)] {
        let place_global_bid_ix: Instruction = batch_update_instruction(
            &market,
            &payer,
            None,
            vec![],
            vec![PlaceOrderParams::new(
                10,
                price_mantissa,
                0,
                true,
                OrderType::Global,
                NO_EXPIRATION_LAST_VALID_SLOT,
            )],
            Some(test_fixture.sol_mint_fixture.key),
            None,
            Some(test_fixture.usdc_mint_fixture.key),
            None,
        );
        measure_and_send(
            &test_fixture,
            &format!("batch_update_global_place_1[{name}]"),
            &[place_global_bid_ix],
            &payer,
            &[&payer_keypair],
        )
        .await?;
    }
    let market_account: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(market)
        .await?
        .expect("market exists");
    let market_fixed: &MarketFixed = get_helper::<MarketFixed>(&market_account.data, 0_u32);
    assert_ne!(*market_fixed.get_base_global(), Pubkey::default());
    assert_ne!(*market_fixed.get_quote_global(), Pubkey::default());
    Ok(())
}
