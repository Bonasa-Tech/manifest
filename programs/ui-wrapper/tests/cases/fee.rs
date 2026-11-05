use std::rc::Rc;

use borsh::BorshSerialize;
use manifest::{
    state::{constants::NO_EXPIRATION_LAST_VALID_SLOT, OrderType},
    validation::get_vault_address,
};
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_program::{instruction::AccountMeta, pubkey::Pubkey, rent::Rent};
use solana_program_test::tokio;
use solana_sdk_ids::system_program;
use solana_signer::Signer;
use spl_token;
use ui_wrapper::{
    instruction::ManifestWrapperInstruction, processors::place_order::WrapperPlaceOrderParams,
};

use crate::{send_tx_with_retry, TestFixture, Token};

/// Lamports on the wrapper state over what its size needs for rent, which is
/// exactly what Collect is able to sweep.
async fn collectible_excess(test_fixture: &TestFixture) -> anyhow::Result<u64> {
    let account = test_fixture
        .try_load(&test_fixture.wrapper.key)
        .await?
        .unwrap();
    Ok(account
        .lamports
        .saturating_sub(Rent::default().minimum_balance(account.data.len())))
}

async fn place_one_order(
    test_fixture: &mut TestFixture,
    client_order_id: u64,
    base_atoms: u64,
) -> anyhow::Result<()> {
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
    let (quote_mint, trader_token_account_quote) = test_fixture
        .fund_trader_wallet(&payer_keypair, Token::USDC, 1)
        .await;
    let (quote_vault, _) = get_vault_address(&test_fixture.market.key, &quote_mint);

    let place_order_ix: Instruction = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(test_fixture.wrapper.key, false),
            AccountMeta::new(payer, true),
            AccountMeta::new(trader_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(payer, true),
        ],
        data: [
            ManifestWrapperInstruction::PlaceOrder.to_vec(),
            WrapperPlaceOrderParams::new(
                client_order_id,
                base_atoms,
                1,
                0,
                true,
                NO_EXPIRATION_LAST_VALID_SLOT,
                OrderType::Limit,
            )
            .try_to_vec()
            .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[place_order_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn create_wrapper_charges_the_fee() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;

    // The fixture creates the wrapper and nothing else has touched it, so the
    // whole surplus over rent is the one fee CreateWrapper charged.
    assert_eq!(
        collectible_excess(&test_fixture).await?,
        ui_wrapper::UI_WRAPPER_FEE_LAMPORTS
    );
    Ok(())
}

#[tokio::test]
async fn place_order_charges_the_fee_and_it_survives_expansion() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;

    let mut expected: u64 = ui_wrapper::UI_WRAPPER_FEE_LAMPORTS;
    // Each placement rests an order, which grows the wrapper. Expansion must
    // bill the payer for that rent rather than spending the fees already
    // sitting here, so the surplus climbs by exactly one fee per placement.
    for client_order_id in 1..=3u64 {
        place_one_order(&mut test_fixture, client_order_id, 1).await?;
        expected += ui_wrapper::UI_WRAPPER_FEE_LAMPORTS;
        assert_eq!(collectible_excess(&test_fixture).await?, expected);
    }
    Ok(())
}

#[tokio::test]
async fn collect_sweeps_the_fees_and_leaves_rent() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
    let collector: Keypair = Keypair::new_from_array([42; 32]);

    place_one_order(&mut test_fixture, 1, 1).await?;
    let accrued: u64 = collectible_excess(&test_fixture).await?;
    assert_eq!(accrued, 2 * ui_wrapper::UI_WRAPPER_FEE_LAMPORTS);

    let collector_before: u64 = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_balance(collector.pubkey())
        .await?;

    let collect_ix: Instruction = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(test_fixture.wrapper.key, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new(collector.pubkey(), true),
        ],
        data: ManifestWrapperInstruction::Collect.to_vec(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[collect_ix],
        Some(&payer),
        &[&payer_keypair, &collector],
    )
    .await?;

    assert_eq!(collectible_excess(&test_fixture).await?, 0);
    let collector_after: u64 = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_balance(collector.pubkey())
        .await?;
    assert_eq!(collector_after, collector_before + accrued);
    Ok(())
}

#[tokio::test]
async fn collect_rejects_a_stranger() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;
    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
    let stranger: Keypair = Keypair::new();

    let collect_ix: Instruction = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(test_fixture.wrapper.key, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new(stranger.pubkey(), true),
        ],
        data: ManifestWrapperInstruction::Collect.to_vec(),
    };
    let result = send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[collect_ix],
        Some(&payer),
        &[&payer_keypair, &stranger],
    )
    .await;
    assert!(result.is_err(), "a stranger must not collect the fees");

    assert_eq!(
        collectible_excess(&test_fixture).await?,
        ui_wrapper::UI_WRAPPER_FEE_LAMPORTS
    );
    Ok(())
}
