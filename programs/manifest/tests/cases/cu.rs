//! Compute unit measurements for each instruction.
//!
//! Every test simulates a representative transaction, prints a line of the
//! form `CU <name>: <units>` and then executes it. The numbers are only
//! meaningful when the compiled SBF program is loaded. The `test-sbf` feature
//! enables the CU-specific assertions and log checks; CI points ProgramTest at
//! the prebuilt canonical v3 artifacts through `SBF_OUT_DIR`.
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
use solana_program::{program_pack::Pack, pubkey::Pubkey, rent::Rent};
use solana_program_test::{tokio, ProgramTestContext};
use solana_signer::Signer;
use solana_system_interface::instruction as system_instruction;
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

/// Simulates `instructions`, requiring success, and prints the compute units
/// as `CU <name>: <units>` without executing anything. For comparing two
/// variants of the same instruction from one state: simulation does not
/// commit, so each sample starts from exactly the state the caller set up.
async fn measure(
    test_fixture: &TestFixture,
    name: &str,
    instructions: &[Instruction],
    payer: &Pubkey,
    signers: &[&Keypair],
) -> u64 {
    let (result, units_consumed) = simulate(test_fixture, instructions, payer, signers).await;
    if let Err(error) = result {
        panic!("{name} simulation failed: {error}");
    }
    println!("CU {name}: {units_consumed}");
    units_consumed
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

/// Creates and initializes a mint whose global PDAs derive on the first bump,
/// with the payer as its mint authority. Measurements that touch a global
/// account use these so the derivation is a single attempt.
async fn create_first_bump_globals_mint(
    test_fixture: &TestFixture,
    decimals: u8,
) -> anyhow::Result<Pubkey> {
    create_first_bump_globals_mint_with_program(test_fixture, decimals, &spl_token::id()).await
}

async fn create_first_bump_globals_mint_with_program(
    test_fixture: &TestFixture,
    decimals: u8,
    token_program: &Pubkey,
) -> anyhow::Result<Pubkey> {
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair();
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
                token_program,
            ),
            spl_token_2022::instruction::initialize_mint(
                token_program,
                &mint,
                &payer,
                None,
                decimals,
            )?,
        ],
        Some(&payer),
        &[&payer_keypair, &mint_keypair],
    )
    .await?;
    Ok(mint)
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
    )
    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
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
    // Creating a market derives both vault PDAs and caches both global
    // addresses, so the mints and the market key are all chosen to derive on
    // the first bump; otherwise this number moves by about 1,500 CU per extra
    // attempt from run to run.
    let base_mint: Pubkey = create_first_bump_globals_mint(&test_fixture, 9).await?;
    let quote_mint: Pubkey = create_first_bump_globals_mint(&test_fixture, 6).await?;
    let market_keypair: Keypair = market_keypair_with_first_bump_vaults(&base_mint, &quote_mint);

    let create_market_ixs: Vec<Instruction> =
        create_market_instructions(&market_keypair.pubkey(), &base_mint, &quote_mint, &payer)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
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
    measure_swap_with_market_seats(1, false).await
}

/// The wrapper replay passes seat hints. Fixed multi-seat fixtures exercise
/// the unhinted market lookups used by direct swaps instead.
#[tokio::test]
async fn cu_swap_market_seat_sizes_test() -> anyhow::Result<()> {
    for seats in [32, 128, 999] {
        measure_swap_with_market_seats(seats, false).await?;
    }
    Ok(())
}

#[tokio::test]
async fn cu_swap_temporary_seat_sizes_test() -> anyhow::Result<()> {
    for seats in [1, 32, 128, 999] {
        measure_swap_with_market_seats(seats, true).await?;
    }
    Ok(())
}

async fn measure_swap_with_market_seats(seats: u16, temporary_seat: bool) -> anyhow::Result<()> {
    use hypertree::{HyperTreeValueIteratorTrait, NIL};
    use manifest::state::{claimed_seat::ClaimedSeat, DynamicAccount, MARKET_BLOCK_SIZE};

    let mut test_fixture: TestFixture = TestFixture::new().await;
    let payer_keypair = if seats == 1 && !temporary_seat {
        test_fixture.payer_keypair()
    } else {
        Keypair::new_from_array([23; 32])
    };
    let payer = payer_keypair.pubkey();
    let (base_account, quote_account) = if seats == 1 && !temporary_seat {
        (
            test_fixture.payer_sol_fixture.key,
            test_fixture.payer_usdc_fixture.key,
        )
    } else {
        let funder = test_fixture.payer_keypair();
        send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[system_instruction::transfer(
                &funder.pubkey(),
                &payer,
                100_000_000,
            )],
            Some(&funder.pubkey()),
            &[&funder],
        )
        .await?;
        let base = TokenAccountFixture::new(
            Rc::clone(&test_fixture.context),
            &test_fixture.sol_mint_fixture.key,
            &payer,
        )
        .await;
        let quote = TokenAccountFixture::new(
            Rc::clone(&test_fixture.context),
            &test_fixture.usdc_mint_fixture.key,
            &payer,
        )
        .await;
        (base.key, quote.key)
    };
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
    if seats > 1 {
        let mut account = test_fixture
            .context
            .borrow_mut()
            .banks_client
            .get_account(market)
            .await?
            .unwrap();
        let mut state = DynamicAccount {
            fixed: *get_helper::<MarketFixed>(&account.data, 0),
            dynamic: account.data[std::mem::size_of::<MarketFixed>()..].to_vec(),
        };
        for i in 1..seats {
            state
                .dynamic
                .resize(state.dynamic.len() + MARKET_BLOCK_SIZE, 0);
            state.market_expand().unwrap();
            let mut bytes = [0; 32];
            bytes[..2].copy_from_slice(&i.to_be_bytes());
            state.claim_seat(&Pubkey::new_from_array(bytes)).unwrap();
        }
        account.data = [bytemuck::bytes_of(&state.fixed), &state.dynamic].concat();
        account.lamports = Rent::default().minimum_balance(account.data.len());
        test_fixture
            .context
            .borrow_mut()
            .set_account(&market, &AccountSharedData::from(account));
    }
    test_fixture
        .sol_mint_fixture
        .mint_to(&base_account, deposit_atoms)
        .await;
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[
            deposit_instruction(
                &market,
                &payer,
                &test_fixture.sol_mint_fixture.key,
                deposit_atoms,
                &base_account,
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

    // Exercise both a persistent seat and a different owner whose temporary
    // seat must be removed without disturbing any other seat or its index.
    let (swapper, base_account, quote_account) = if temporary_seat {
        let swapper = Keypair::new_from_array([24; 32]);
        send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[system_instruction::transfer(
                &payer,
                &swapper.pubkey(),
                10_000_000,
            )],
            Some(&payer),
            &[&payer_keypair],
        )
        .await?;
        let base = TokenAccountFixture::new(
            Rc::clone(&test_fixture.context),
            &test_fixture.sol_mint_fixture.key,
            &swapper.pubkey(),
        )
        .await;
        let quote = TokenAccountFixture::new(
            Rc::clone(&test_fixture.context),
            &test_fixture.usdc_mint_fixture.key,
            &swapper.pubkey(),
        )
        .await;
        (swapper, base.key, quote.key)
    } else {
        (payer_keypair, base_account, quote_account)
    };
    let account_before = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(market)
        .await?
        .unwrap();
    let state_before = DynamicAccount {
        fixed: get_helper::<MarketFixed>(&account_before.data, 0),
        dynamic: &account_before.data[std::mem::size_of::<MarketFixed>()..],
    };
    let seats_before: Vec<_> = state_before
        .get_claimed_seats()
        .iter::<ClaimedSeat>()
        .map(|(index, seat)| (index, seat.trader))
        .collect();
    assert_eq!(seats_before.len(), seats as usize);
    assert_eq!(
        state_before.get_trader_index(&swapper.pubkey()) == NIL,
        temporary_seat
    );
    let balances_before =
        (!temporary_seat).then(|| state_before.get_trader_balance(&swapper.pubkey()));

    // Taker buys 1 SOL with 1 USDC from the wallet, filling the single ask.
    let quote_in_atoms: u64 = 1 * USDC_UNIT_SIZE;
    test_fixture
        .usdc_mint_fixture
        .mint_to(&quote_account, quote_in_atoms)
        .await;
    let swap_ix: Instruction = swap_instruction(
        &market,
        &swapper.pubkey(),
        &test_fixture.sol_mint_fixture.key,
        &test_fixture.usdc_mint_fixture.key,
        &base_account,
        &quote_account,
        quote_in_atoms,
        1 * SOL_UNIT_SIZE,
        false,
        true,
        spl_token::id(),
        spl_token::id(),
        false,
    );
    let label = if temporary_seat {
        format!("swap_fill_1_{seats}_market_seats_temporary")
    } else if seats == 1 {
        "swap_fill_1".to_owned()
    } else {
        format!("swap_fill_1_{seats}_market_seats")
    };
    measure_and_send(
        &test_fixture,
        &label,
        &[swap_ix],
        &swapper.pubkey(),
        &[&swapper],
    )
    .await?;
    let account_after = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(market)
        .await?
        .unwrap();
    let state_after = DynamicAccount {
        fixed: get_helper::<MarketFixed>(&account_after.data, 0),
        dynamic: &account_after.data[std::mem::size_of::<MarketFixed>()..],
    };
    let seats_after: Vec<_> = state_after
        .get_claimed_seats()
        .iter::<ClaimedSeat>()
        .map(|(index, seat)| (index, seat.trader))
        .collect();
    assert_eq!(seats_after, seats_before);
    assert!(state_after.has_free_block());
    if let Some(balances) = balances_before {
        assert_eq!(state_after.get_trader_balance(&swapper.pubkey()), balances);
    } else {
        assert_eq!(state_after.get_trader_index(&swapper.pubkey()), NIL);
    }
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
    let mint: Pubkey = create_first_bump_globals_mint(&test_fixture, 6).await?;
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
/// and stores them; the second takes the cached path. Both mints and the
/// market key derive on the first bump so the uncached number is a single
/// derivation attempt per side rather than however many the fixture's random
/// mints happen to need.
#[tokio::test]
async fn cu_batch_update_with_globals_test() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair();
    let amount_atoms: u64 = 10 * USDC_UNIT_SIZE;

    let base_mint: Pubkey = create_first_bump_globals_mint(&test_fixture, 9).await?;
    let quote_mint: Pubkey = create_first_bump_globals_mint(&test_fixture, 6).await?;
    let market_keypair: Keypair = market_keypair_with_first_bump_vaults(&base_mint, &quote_mint);
    let market: Pubkey = market_keypair.pubkey();
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &create_market_instructions(&market, &base_mint, &quote_mint, &payer)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?[..],
        Some(&payer),
        &[&payer_keypair, &market_keypair],
    )
    .await?;

    // A seat on the market, both globals, and quote tokens on the global to
    // back the global bid placed below.
    let quote_token_account: TokenAccountFixture =
        TokenAccountFixture::new(Rc::clone(&test_fixture.context), &quote_mint, &payer).await;
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[
            claim_seat_instruction(&market, &payer),
            create_global_instruction(&base_mint, &payer, &spl_token::id()),
            create_global_instruction(&quote_mint, &payer, &spl_token::id()),
            spl_token::instruction::mint_to(
                &spl_token::id(),
                &quote_mint,
                &quote_token_account.key,
                &payer,
                &[&payer],
                amount_atoms,
            )?,
        ],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;
    let (quote_global, _) = get_global_address(&quote_mint);
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[
            global_add_trader_instruction(&quote_global, &payer),
            global_deposit_instruction(
                &quote_mint,
                &payer,
                &quote_token_account.key,
                &spl_token::id(),
                amount_atoms,
            ),
        ],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // Both samples are simulations of the same instruction, and simulating
    // does not commit, so both run against this same state: an empty book, an
    // untouched global, and a seat that has not traded. The only difference
    // between them is the 64 bytes of cached addresses, which is the point of
    // the comparison; sending either one first would leave the second facing a
    // resting order and a changed global balance.
    let place_global_bid_ix: Instruction = batch_update_instruction(
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
        Some(base_mint),
        None,
        Some(quote_mint),
        None,
    );

    // Creating the market cached both addresses, so this is the cached path.
    let cached_units: u64 = measure(
        &test_fixture,
        "batch_update_global_place_1[cached]",
        &[place_global_bid_ix.clone()],
        &payer,
        &[&payer_keypair],
    )
    .await;

    // Clear just the cache, to behave like a market from before it existed.
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

    let uncached_units: u64 = measure(
        &test_fixture,
        "batch_update_global_place_1[uncached]",
        &[place_global_bid_ix.clone()],
        &payer,
        &[&payer_keypair],
    )
    .await;
    // Only the BPF program meters compute, see the module docs, so this only
    // says anything under `cargo test-sbf`.
    if cfg!(feature = "test-sbf") {
        assert!(
            uncached_units > cached_units,
            "deriving both globals must cost more than comparing them",
        );
    }

    // Run it for real from the cleared state, so the cache being filled in is
    // covered too.
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[place_global_bid_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;
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

/// Use fixed trader keys and a full global account so eviction samples include
/// the same lookup depths on every build. Cover equal zero balances and a
/// funded minimum so eviction also exercises a nonzero token withdrawal.
#[tokio::test]
async fn cu_global_evict_test() -> anyhow::Result<()> {
    use manifest::{
        program::global_evict_instruction,
        quantities::{GlobalAtoms, WrapperU64},
        state::{DynamicAccount, GlobalFixed, GLOBAL_BLOCK_SIZE, MAX_GLOBAL_SEATS},
    };

    for (evictee_balance, other_balance, new_deposit, label) in [
        (0u64, 0u64, 1u64, "global_evict_equal_balances"),
        (50, 100, 101, "global_evict_nonzero_balance"),
    ] {
        let test_fixture: TestFixture = TestFixture::new().await;
        let payer_keypair: Keypair = test_fixture.payer_keypair();
        let payer: Pubkey = payer_keypair.pubkey();
        let mint: Pubkey = create_first_bump_globals_mint(&test_fixture, 6).await?;
        let evictor: Keypair = Keypair::new_from_array([7; 32]);
        let evictee: Pubkey = Pubkey::new_from_array([8; 32]);
        let evictor_token =
            TokenAccountFixture::new(Rc::clone(&test_fixture.context), &mint, &evictor.pubkey())
                .await;
        let evictee_token =
            TokenAccountFixture::new(Rc::clone(&test_fixture.context), &mint, &evictee).await;
        send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[
                system_instruction::transfer(&payer, &evictor.pubkey(), 100_000_000),
                create_global_instruction(&mint, &payer, &spl_token::id()),
                spl_token::instruction::mint_to(
                    &spl_token::id(),
                    &mint,
                    &evictor_token.key,
                    &payer,
                    &[&payer],
                    new_deposit,
                )?,
                spl_token::instruction::mint_to(
                    &spl_token::id(),
                    &mint,
                    &get_global_vault_address(&mint).0,
                    &payer,
                    &[&payer],
                    evictee_balance + other_balance * u64::from(MAX_GLOBAL_SEATS - 1),
                )?,
            ],
            Some(&payer),
            &[&payer_keypair],
        )
        .await?;

        let mut global_state = DynamicAccount {
            fixed: GlobalFixed::new_empty(&mint),
            dynamic: Vec::<u8>::new(),
        };
        for i in 0..MAX_GLOBAL_SEATS {
            global_state
                .dynamic
                .resize(global_state.dynamic.len() + 2 * GLOBAL_BLOCK_SIZE, 0);
            global_state.global_expand().unwrap();
            let trader = if i == 0 {
                evictee
            } else {
                let mut bytes = [0; 32];
                bytes[..2].copy_from_slice(&i.to_be_bytes());
                Pubkey::new_from_array(bytes)
            };
            global_state.add_trader(&trader).unwrap();
            let balance = if i == 0 {
                evictee_balance
            } else {
                other_balance
            };
            if balance != 0 {
                global_state
                    .deposit_global(&trader, GlobalAtoms::new(balance))
                    .unwrap();
            }
        }
        global_state.verify_min_balance(&evictee).unwrap();
        let data = [
            bytemuck::bytes_of(&global_state.fixed),
            &global_state.dynamic,
        ]
        .concat();
        let (global_key, _) = get_global_address(&mint);
        test_fixture.context.borrow_mut().set_account(
            &global_key,
            &AccountSharedData::from(Account {
                lamports: Rent::default().minimum_balance(data.len()),
                data,
                owner: manifest::ID,
                executable: false,
                rent_epoch: 0,
            }),
        );

        measure_and_send(
            &test_fixture,
            label,
            &[global_evict_instruction(
                &mint,
                &evictor.pubkey(),
                &evictor_token.key,
                &evictee_token.key,
                &spl_token::id(),
                new_deposit,
            )],
            &evictor.pubkey(),
            &[&evictor],
        )
        .await?;
        let account = test_fixture
            .context
            .borrow_mut()
            .banks_client
            .get_account(global_key)
            .await?
            .unwrap();
        let final_global: manifest::state::GlobalRef = DynamicAccount {
            fixed: get_helper::<GlobalFixed>(&account.data, 0),
            dynamic: &account.data[std::mem::size_of::<GlobalFixed>()..],
        };
        assert!(!final_global.has_global_seat(&evictee));
        assert!(final_global.has_global_seat(&evictor.pubkey()));
        assert_eq!(
            final_global.get_balance_atoms(&evictor.pubkey()),
            GlobalAtoms::new(new_deposit)
        );
        for i in 1..MAX_GLOBAL_SEATS {
            let mut bytes = [0; 32];
            bytes[..2].copy_from_slice(&i.to_be_bytes());
            let trader = Pubkey::new_from_array(bytes);
            assert!(final_global.has_global_seat(&trader));
            assert_eq!(
                final_global.get_balance_atoms(&trader),
                GlobalAtoms::new(other_balance)
            );
        }
        // A cached deposit index must still debit and refund the right trader.
        for (key, expected) in [
            (evictee_token.key, evictee_balance),
            (evictor_token.key, 0),
            (
                get_global_vault_address(&mint).0,
                other_balance * u64::from(MAX_GLOBAL_SEATS - 1) + new_deposit,
            ),
        ] {
            let account = test_fixture
                .context
                .borrow_mut()
                .banks_client
                .get_account(key)
                .await?
                .unwrap();
            assert_eq!(
                spl_token::state::Account::unpack(&account.data)?.amount,
                expected
            );
        }
    }
    Ok(())
}

/// Fixed keys and balances keep lookup depths and rebalancing identical between
/// builds. Exercise balance changes that move a deposit across the other keys.
#[tokio::test]
async fn cu_global_tree_sizes_test() -> anyhow::Result<()> {
    use manifest::{
        quantities::{GlobalAtoms, WrapperU64},
        state::{validate_global_dynamic, DynamicAccount, GlobalFixed, GLOBAL_BLOCK_SIZE},
    };

    let mut test_fixture = TestFixture::new().await;
    let payer_keypair = test_fixture.payer_keypair();
    let payer = payer_keypair.pubkey();
    let trader_keypair = Keypair::new_from_array([19; 32]);
    let trader = trader_keypair.pubkey();
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[system_instruction::transfer(&payer, &trader, 100_000_000)],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    for (seats, token_program) in [
        (1u16, spl_token::id()),
        (32, spl_token::id()),
        (128, spl_token::id()),
        (999, spl_token::id()),
        (32, spl_token_2022::id()),
    ] {
        let is_token22 = token_program == spl_token_2022::id();
        let suffix = if is_token22 { "_token22" } else { "" };
        let quote_token_program = is_token22.then_some(token_program);
        if seats > manifest::state::MAX_GLOBAL_SEATS {
            continue;
        }
        let mint =
            create_first_bump_globals_mint_with_program(&test_fixture, 6, &token_program).await?;
        let token_account = if is_token22 {
            TokenAccountFixture::new_with_keypair_2022(
                Rc::clone(&test_fixture.context),
                &mint,
                &trader,
                &Keypair::new(),
            )
            .await
        } else {
            TokenAccountFixture::new(Rc::clone(&test_fixture.context), &mint, &trader).await
        };
        send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[create_global_instruction(&mint, &payer, &token_program)],
            Some(&payer),
            &[&payer_keypair],
        )
        .await?;
        let mut global_state = DynamicAccount {
            fixed: GlobalFixed::new_empty(&mint),
            dynamic: Vec::<u8>::new(),
        };
        let mut balances = Vec::new();
        for i in 0..seats {
            global_state
                .dynamic
                .resize(global_state.dynamic.len() + 2 * GLOBAL_BLOCK_SIZE, 0);
            global_state.global_expand().unwrap();
            let key = if i == seats / 2 {
                trader
            } else {
                let mut bytes = [0; 32];
                bytes[..2].copy_from_slice(&i.to_be_bytes());
                Pubkey::new_from_array(bytes)
            };
            let balance = if key == trader {
                7
            } else {
                (u64::from(i) + 1) * 100
            };
            global_state.add_trader(&key).unwrap();
            global_state
                .deposit_global(&key, GlobalAtoms::new(balance))
                .unwrap();
            balances.push((key, balance));
        }
        validate_global_dynamic(&global_state.fixed, &global_state.dynamic).unwrap();
        let data = [
            bytemuck::bytes_of(&global_state.fixed),
            &global_state.dynamic,
        ]
        .concat();
        let (global_key, _) = get_global_address(&mint);
        test_fixture.context.borrow_mut().set_account(
            &global_key,
            &AccountSharedData::from(Account {
                lamports: Rent::default().minimum_balance(data.len()),
                data,
                owner: manifest::ID,
                executable: false,
                rent_epoch: 0,
            }),
        );
        let (vault, _) = get_global_vault_address(&mint);
        send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[
                spl_token_2022::instruction::mint_to(
                    &token_program,
                    &mint,
                    &vault,
                    &payer,
                    &[&payer],
                    balances.iter().map(|(_, amount)| *amount).sum(),
                )?,
                spl_token_2022::instruction::mint_to(
                    &token_program,
                    &mint,
                    &token_account.key,
                    &payer,
                    &[&payer],
                    1_000,
                )?,
            ],
            Some(&payer),
            &[&payer_keypair],
        )
        .await?;
        measure_and_send(
            &test_fixture,
            &format!("global_deposit_{seats}_seats{suffix}"),
            &[global_deposit_instruction(
                &mint,
                &trader,
                &token_account.key,
                &token_program,
                1_000,
            )],
            &trader,
            &[&trader_keypair],
        )
        .await?;
        measure_and_send(
            &test_fixture,
            &format!("global_withdraw_{seats}_seats{suffix}"),
            &[global_withdraw_instruction(
                &mint,
                &trader,
                &token_account.key,
                &token_program,
                1_000,
            )],
            &trader,
            &[&trader_keypair],
        )
        .await?;
        let account = test_fixture
            .context
            .borrow_mut()
            .banks_client
            .get_account(global_key)
            .await?
            .unwrap();
        let final_global: manifest::state::GlobalRef = DynamicAccount {
            fixed: get_helper::<GlobalFixed>(&account.data, 0),
            dynamic: &account.data[std::mem::size_of::<GlobalFixed>()..],
        };
        validate_global_dynamic(final_global.fixed, final_global.dynamic).unwrap();
        for (key, balance) in balances {
            assert!(final_global.has_global_seat(&key));
            assert_eq!(
                final_global.get_balance_atoms(&key),
                GlobalAtoms::new(balance)
            );
        }
        // Match against this same deposit tree, so the measurement includes
        // trader lookup and reduction at each of the four tree sizes.
        let taker_keypair = Keypair::new_from_array([20; 32]);
        let taker = taker_keypair.pubkey();
        let base_mint = test_fixture.sol_mint_fixture.key;
        let market_keypair = market_keypair_with_first_bump_vaults(&base_mint, &mint);
        let market = market_keypair.pubkey();
        send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &create_market_instructions(&market, &base_mint, &mint, &payer)
                .map_err(|error| anyhow::anyhow!("{error:?}"))?,
            Some(&payer),
            &[&payer_keypair, &market_keypair],
        )
        .await?;
        send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[system_instruction::transfer(&payer, &taker, 10_000_000)],
            Some(&payer),
            &[&payer_keypair],
        )
        .await?;
        let taker_token =
            TokenAccountFixture::new(Rc::clone(&test_fixture.context), &base_mint, &taker).await;
        test_fixture
            .sol_mint_fixture
            .mint_to(&taker_token.key, 5)
            .await;
        send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[claim_seat_instruction(&market, &trader)],
            Some(&trader),
            &[&trader_keypair],
        )
        .await?;
        send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[
                claim_seat_instruction(&market, &taker),
                deposit_instruction(
                    &market,
                    &taker,
                    &base_mint,
                    5,
                    &taker_token.key,
                    spl_token::id(),
                    None,
                ),
            ],
            Some(&taker),
            &[&taker_keypair],
        )
        .await?;
        send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[batch_update_instruction(
                &market,
                &trader,
                None,
                vec![],
                vec![PlaceOrderParams::new(
                    5,
                    1,
                    0,
                    true,
                    OrderType::Global,
                    NO_EXPIRATION_LAST_VALID_SLOT,
                )],
                None,
                None,
                Some(mint),
                quote_token_program,
            )],
            Some(&trader),
            &[&trader_keypair],
        )
        .await?;
        measure_and_send(
            &test_fixture,
            &format!("global_match_{seats}_seats{suffix}"),
            &[batch_update_instruction(
                &market,
                &taker,
                None,
                vec![],
                vec![PlaceOrderParams::new(
                    5,
                    1,
                    0,
                    false,
                    OrderType::ImmediateOrCancel,
                    NO_EXPIRATION_LAST_VALID_SLOT,
                )],
                None,
                None,
                Some(mint),
                quote_token_program,
            )],
            &taker,
            &[&taker_keypair],
        )
        .await?;
        let account = test_fixture
            .context
            .borrow_mut()
            .banks_client
            .get_account(global_key)
            .await?
            .unwrap();
        let final_global: manifest::state::GlobalRef = DynamicAccount {
            fixed: get_helper::<GlobalFixed>(&account.data, 0),
            dynamic: &account.data[std::mem::size_of::<GlobalFixed>()..],
        };
        assert_eq!(final_global.get_balance_atoms(&trader), GlobalAtoms::new(2));
    }
    Ok(())
}
