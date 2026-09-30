use crate::{send_tx_with_retry, Side, TestFixture, Token};
use hypertree::{HyperTreeValueIteratorTrait, NIL};
use manifest::{
    program::{
        batch_update::{CancelOrderParams, PlaceOrderParams},
        defrag_instruction, get_dynamic_value_or,
    },
    state::{OrderType, RestingOrder, MARKET_BLOCK_SIZE, MARKET_FIXED_SIZE},
};
use solana_account::{Account, AccountSharedData};
use solana_keypair::Keypair;
use solana_program::{instruction::Instruction, rent::Rent};
use solana_program_test::tokio;
use solana_signer::Signer;
use std::rc::Rc;

fn collector(f: &TestFixture) -> Keypair {
    let key = Keypair::new_from_array([42; 32]);
    f.context.borrow_mut().set_account(
        &key.pubkey(),
        &AccountSharedData::from(Account::new(
            1_000_000_000,
            0,
            &solana_sdk_ids::system_program::id(),
        )),
    );
    key
}
fn ix(f: &TestFixture, key: &Keypair) -> Instruction {
    ix_with(f, key, None)
}
fn ix_with(f: &TestFixture, key: &Keypair, limit: Option<u32>) -> Instruction {
    defrag_instruction(
        &f.market_fixture.key,
        &key.pubkey(),
        &f.sol_mint_fixture.key,
        &f.usdc_mint_fixture.key,
        spl_token::id(),
        spl_token::id(),
        limit,
    )
}
async fn run(f: &TestFixture, key: &Keypair) -> anyhow::Result<()> {
    send_tx_with_retry(
        Rc::clone(&f.context),
        &[ix(f, key)],
        Some(&f.payer()),
        &[&f.payer_keypair(), key],
    )
    .await?;
    Ok(())
}
#[tokio::test]
async fn defrag_rejects_non_collector_and_closes_empty_market_and_vaults() -> anyhow::Result<()> {
    let f = TestFixture::new().await;
    f.claim_seat().await?;
    let wrong = f.second_keypair.insecure_clone();
    assert!(run(&f, &wrong).await.is_err());
    assert!(f.try_load(&f.market_fixture.key).await?.is_some());
    let key = collector(&f);
    let base =
        manifest::validation::get_vault_address(&f.market_fixture.key, &f.sol_mint_fixture.key).0;
    let quote =
        manifest::validation::get_vault_address(&f.market_fixture.key, &f.usdc_mint_fixture.key).0;
    let total = f.try_load(&f.market_fixture.key).await?.unwrap().lamports
        + f.try_load(&base).await?.unwrap().lamports
        + f.try_load(&quote).await?.unwrap().lamports;
    run(&f, &key).await?;
    assert!(f.try_load(&f.market_fixture.key).await?.is_none());
    assert!(f.try_load(&base).await?.is_none());
    assert!(f.try_load(&quote).await?.is_none());
    assert_eq!(
        f.try_load(&key.pubkey()).await?.unwrap().lamports,
        1_000_000_000 + total
    );
    Ok(())
}
#[tokio::test]
async fn defrag_preserves_orders_and_balances_then_batch_replenishes_one_node() -> anyhow::Result<()>
{
    let mut f = TestFixture::new().await;
    f.claim_seat().await?;
    f.claim_seat_for_keypair(&f.second_keypair).await?;
    f.deposit(Token::SOL, 1000).await?;
    f.place_order(Side::Ask, 300, 1, 0, 0, OrderType::Limit)
        .await?;
    f.place_order(Side::Ask, 300, 1, 0, 0, OrderType::Limit)
        .await?;
    f.market_fixture.reload().await;
    let before: Vec<_> = f
        .market_fixture
        .market
        .get_asks()
        .iter::<RestingOrder>()
        .map(|(_, o)| (o.get_sequence_number(), o.get_num_base_atoms()))
        .collect();
    let key = collector(&f);
    run(&f, &key).await?;
    f.market_fixture.reload().await;
    assert_eq!(
        f.market_fixture
            .market
            .get_trader_index(&f.second_keypair.pubkey()),
        NIL
    );
    assert_eq!(f.market_fixture.market.free_blocks_short_of_n(2), Some(0));
    let after: Vec<_> = f
        .market_fixture
        .market
        .get_asks()
        .iter::<RestingOrder>()
        .map(|(_, o)| (o.get_sequence_number(), o.get_num_base_atoms()))
        .collect();
    assert_eq!(before, after);
    let account = f.try_load(&f.market_fixture.key).await?.unwrap();
    assert_eq!(account.data.len(), 256 + 80 * (1 + 2 + 2));
    assert_eq!(
        account.lamports,
        Rent::default().minimum_balance(account.data.len())
    );
    f.batch_update_for_keypair(
        None,
        vec![],
        vec![PlaceOrderParams::new(100, 2, 0, false, OrderType::Limit, 0)],
        &f.payer_keypair(),
    )
    .await?;
    f.market_fixture.reload().await;
    assert_eq!(f.market_fixture.market.free_blocks_short_of_n(2), Some(0));
    let mut previous_size = account.data.len() + MARKET_BLOCK_SIZE;
    assert_eq!(
        f.try_load(&f.market_fixture.key).await?.unwrap().data.len(),
        previous_size
    );

    // Cancel/replace batches grow the spare reserve one node at a time and
    // stop at five. Each replacement consumes the block its cancel releases.
    for expected_free in [3, 4, 5, 5] {
        let sequence = f
            .market_fixture
            .market
            .get_asks()
            .iter::<RestingOrder>()
            .next()
            .unwrap()
            .1
            .get_sequence_number();
        f.batch_update_for_keypair(
            None,
            vec![CancelOrderParams::new(sequence)],
            vec![PlaceOrderParams::new(100, 2, 0, false, OrderType::Limit, 0)],
            &f.payer_keypair(),
        )
        .await?;
        f.market_fixture.reload().await;
        assert_eq!(
            f.market_fixture
                .market
                .free_blocks_short_of_n(expected_free),
            Some(0)
        );
        let size = f.try_load(&f.market_fixture.key).await?.unwrap().data.len();
        assert!(size == previous_size || size == previous_size + MARKET_BLOCK_SIZE);
        previous_size = size;
    }

    // Defrag again so cancel-only and empty batches would previously have
    // charged the trader to restore five free nodes.
    run(&f, &key).await?;
    f.market_fixture.reload().await;
    let compacted = f.try_load(&f.market_fixture.key).await?.unwrap();
    let sequence = f
        .market_fixture
        .market
        .get_asks()
        .iter::<RestingOrder>()
        .next()
        .unwrap()
        .1
        .get_sequence_number();
    f.cancel_order(sequence).await?;
    f.batch_update_for_keypair(None, vec![], vec![], &f.payer_keypair())
        .await?;
    let after_cancel = f.try_load(&f.market_fixture.key).await?.unwrap();
    assert_eq!(after_cancel.data.len(), compacted.data.len());
    assert_eq!(after_cancel.lamports, compacted.lamports);
    Ok(())
}

#[tokio::test]
async fn defrag_collects_token2022_excess_without_withdrawing_tokens() -> anyhow::Result<()> {
    vault_excess(true).await
}
#[tokio::test]
async fn defrag_collects_classic_excess_without_withdrawing_tokens() -> anyhow::Result<()> {
    vault_excess(false).await
}
async fn vault_excess(token2022: bool) -> anyhow::Result<()> {
    let f = TestFixture::new().await;
    let mint =
        crate::MintFixture::new_with_version(Rc::clone(&f.context), Some(6), token2022).await;
    let token_program = if token2022 {
        spl_token_2022::id()
    } else {
        spl_token::id()
    };
    let market = f
        .create_new_market(&mint.key, &f.usdc_mint_fixture.key)
        .await?;
    let vault = manifest::validation::get_vault_address(&market, &mint.key).0;
    send_tx_with_retry(
        Rc::clone(&f.context),
        &[spl_token_2022::instruction::mint_to(
            &token_program,
            &mint.key,
            &vault,
            &f.payer(),
            &[],
            123,
        )?],
        Some(&f.payer()),
        &[&f.payer_keypair()],
    )
    .await?;
    let excess = 12345678;
    send_tx_with_retry(
        Rc::clone(&f.context),
        &[solana_system_interface::instruction::transfer(
            &f.payer(),
            &vault,
            excess,
        )],
        Some(&f.payer()),
        &[&f.payer_keypair()],
    )
    .await?;
    let key = collector(&f);
    let ix = defrag_instruction(
        &market,
        &key.pubkey(),
        &mint.key,
        &f.usdc_mint_fixture.key,
        token_program,
        spl_token::id(),
        None,
    );
    send_tx_with_retry(
        Rc::clone(&f.context),
        &[ix],
        Some(&f.payer()),
        &[&f.payer_keypair(), &key],
    )
    .await?;
    let account = f.try_load(&vault).await?.unwrap();
    assert_eq!(
        account.lamports,
        Rent::default().minimum_balance(account.data.len())
    );
    assert_eq!(
        u64::from_le_bytes(account.data[64..72].try_into().unwrap()),
        123
    );
    assert!(f.try_load(&market).await?.is_some());
    Ok(())
}

#[tokio::test]
async fn defrag_large_empty_seat_population_fits_one_instruction() -> anyhow::Result<()> {
    use manifest::quantities::WrapperU64;
    use solana_compute_budget_interface::ComputeBudgetInstruction;
    use solana_program::pubkey::Pubkey;
    use solana_transaction::Transaction;
    let mut f = TestFixture::new().await;
    let key = collector(&f);
    let mut value = f.market_fixture.market.clone();
    value.dynamic.resize(80 * 13_002, 0);
    value.market_expand_n(13_001)?;
    for i in 0..13_000u32 {
        let mut bytes = [0u8; 32];
        bytes[..4].copy_from_slice(&i.to_le_bytes());
        bytes[31] = 1;
        let trader = Pubkey::new_from_array(bytes);
        value.claim_seat(&trader)?;
        if i < 1000 {
            value.deposit(value.get_trader_index(&trader), 1, true)?;
        }
    }
    let data = [bytemuck::bytes_of(&value.fixed), &value.dynamic].concat();
    f.context.borrow_mut().set_account(
        &f.market_fixture.key,
        &AccountSharedData::from(Account {
            lamports: Rent::default().minimum_balance(data.len()),
            data,
            owner: manifest::id(),
            executable: false,
            rent_epoch: 0,
        }),
    );
    let vault =
        manifest::validation::get_vault_address(&f.market_fixture.key, &f.sol_mint_fixture.key).0;
    f.sol_mint_fixture.mint_to(&vault, 1000).await;
    let payer = f.payer_keypair();
    let blockhash = f
        .context
        .borrow_mut()
        .banks_client
        .get_latest_blockhash()
        .await?;
    let transaction = Transaction::new_signed_with_payer(
        &[
            ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
            ix(&f, &key),
        ],
        Some(&payer.pubkey()),
        &[&payer, &key],
        blockhash,
    );
    let result = f
        .context
        .borrow_mut()
        .banks_client
        .simulate_transaction(transaction)
        .await?;
    println!(
        "CU large defrag: {}",
        result.simulation_details.as_ref().unwrap().units_consumed
    );
    assert!(result.result.as_ref().unwrap().is_ok(), "{result:?}");
    // Pin the headroom. This is the largest shape any live market is near, and
    // without an assertion a change that takes it from comfortable to marginal
    // lands silently; the 1.4M ceiling only catches it once it is too late.
    let units: u64 = result.simulation_details.as_ref().unwrap().units_consumed;
    assert!(
        units < 1_250_000,
        "large defrag regressed to {units} CU against a 1.4M ceiling"
    );
    assert_eq!(
        value
            .get_trader_balance(&Pubkey::new_from_array({
                let mut b = [0; 32];
                b[31] = 1;
                b
            }))
            .0
            .as_u64(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn defrag_leaves_wrapped_sol_principal_reserve_and_unsynced_lamports() -> anyhow::Result<()> {
    use solana_program::program_pack::Pack;
    let f = TestFixture::new().await;
    let native = spl_token::native_mint::id();
    let mut mint_data = vec![0; spl_token::state::Mint::LEN];
    spl_token::state::Mint::pack(
        spl_token::state::Mint {
            decimals: 9,
            is_initialized: true,
            ..Default::default()
        },
        &mut mint_data,
    )?;
    f.context.borrow_mut().set_account(
        &native,
        &AccountSharedData::from(Account {
            lamports: Rent::default().minimum_balance(mint_data.len()),
            data: mint_data,
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        }),
    );
    let market = f
        .create_new_market(&native, &f.usdc_mint_fixture.key)
        .await?;
    let vault = manifest::validation::get_vault_address(&market, &native).0;
    send_tx_with_retry(
        Rc::clone(&f.context),
        &[
            solana_system_interface::instruction::transfer(&f.payer(), &vault, 1_000_000),
            spl_token::instruction::sync_native(&spl_token::id(), &vault)?,
            solana_system_interface::instruction::transfer(&f.payer(), &vault, 12_345),
        ],
        Some(&f.payer()),
        &[&f.payer_keypair()],
    )
    .await?;
    let before = f.try_load(&vault).await?.unwrap();
    let key = collector(&f);
    send_tx_with_retry(
        Rc::clone(&f.context),
        &[defrag_instruction(
            &market,
            &key.pubkey(),
            &native,
            &f.usdc_mint_fixture.key,
            spl_token::id(),
            spl_token::id(),
            None,
        )],
        Some(&f.payer()),
        &[&f.payer_keypair(), &key],
    )
    .await?;
    let after = f.try_load(&vault).await?.unwrap();
    assert_eq!(after.data, before.data);
    assert_eq!(after.lamports, before.lamports);
    assert!(f.try_load(&market).await?.is_some());
    Ok(())
}

#[tokio::test]
async fn defrag_large_free_list_fits_one_instruction() -> anyhow::Result<()> {
    use solana_compute_budget_interface::ComputeBudgetInstruction;
    let f = TestFixture::new().await;
    let key = collector(&f);
    let mut value = f.market_fixture.market.clone();
    value.dynamic.resize(80 * 13_002, 0);
    value.market_expand_n(13_001)?;
    let data = [bytemuck::bytes_of(&value.fixed), &value.dynamic].concat();
    f.context.borrow_mut().set_account(
        &f.market_fixture.key,
        &AccountSharedData::from(Account {
            lamports: Rent::default().minimum_balance(data.len()),
            data,
            owner: manifest::id(),
            executable: false,
            rent_epoch: 0,
        }),
    );
    send_tx_with_retry(
        Rc::clone(&f.context),
        &[
            ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
            ix(&f, &key),
        ],
        Some(&f.payer()),
        &[&f.payer_keypair(), &key],
    )
    .await?;
    assert!(f.try_load(&f.market_fixture.key).await?.is_none());
    Ok(())
}

/// A budget the operator can pick offline has to be safe at every step: each
/// run leaves a market that is valid and no larger, and repeating converges on
/// the layout one unbounded run would reach.
#[tokio::test]
async fn bounded_defrag_makes_monotone_progress_and_converges() -> anyhow::Result<()> {
    use manifest::quantities::WrapperU64;
    use solana_program::pubkey::Pubkey;
    const SEATS: u32 = 120;
    const FUNDED: u32 = 10;
    let mut f = TestFixture::new().await;
    let key = collector(&f);
    let mut value = f.market_fixture.market.clone();
    value
        .dynamic
        .resize(MARKET_BLOCK_SIZE * (SEATS as usize + 2), 0);
    value.market_expand_n(SEATS + 1)?;
    let trader = |i: u32| {
        let mut bytes = [0u8; 32];
        bytes[..4].copy_from_slice(&i.to_le_bytes());
        bytes[31] = 3;
        Pubkey::new_from_array(bytes)
    };
    for i in 0..SEATS {
        value.claim_seat(&trader(i))?;
        if i < FUNDED {
            value.deposit(value.get_trader_index(&trader(i)), 1, true)?;
        }
    }
    let data = [bytemuck::bytes_of(&value.fixed), &value.dynamic].concat();
    f.context.borrow_mut().set_account(
        &f.market_fixture.key,
        &AccountSharedData::from(Account {
            lamports: Rent::default().minimum_balance(data.len()),
            data,
            owner: manifest::id(),
            executable: false,
            rent_epoch: 0,
        }),
    );
    let vault =
        manifest::validation::get_vault_address(&f.market_fixture.key, &f.sol_mint_fixture.key).0;
    f.sol_mint_fixture.mint_to(&vault, 1000).await;

    let mut previous = f.try_load(&f.market_fixture.key).await?.unwrap().data.len();
    let mut runs = 0;
    loop {
        send_tx_with_retry(
            Rc::clone(&f.context),
            &[ix_with(&f, &key, Some(10))],
            Some(&f.payer()),
            &[&f.payer_keypair(), &key],
        )
        .await?;
        let account = f.try_load(&f.market_fixture.key).await?.unwrap();
        assert!(
            account.data.len() <= previous,
            "a bounded run grew the market from {previous} to {}",
            account.data.len()
        );
        // Every funded seat is still readable and still holds its deposit.
        let market: manifest::state::MarketValue =
            get_dynamic_value_or(account.data.as_slice()).unwrap();
        for i in 0..FUNDED {
            assert_eq!(
                market.get_trader_balance(&trader(i)).0.as_u64(),
                1,
                "funded seat {i} lost its balance after {runs} runs"
            );
        }
        runs += 1;
        assert!(runs < 200, "bounded defrag did not converge");
        if account.data.len() == previous {
            break;
        }
        previous = account.data.len();
    }
    assert!(runs > 1, "test did not exercise the bounded path");
    assert_eq!(
        previous,
        MARKET_FIXED_SIZE + MARKET_BLOCK_SIZE * (FUNDED as usize + 2)
    );
    Ok(())
}
