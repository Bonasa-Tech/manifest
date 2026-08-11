use std::rc::Rc;

use anyhow::Result;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_program::{rent::Rent, system_instruction, system_program};
use solana_signer::Signer;
use wrapper::instruction::ManifestWrapperInstruction;

use crate::program_test::{send_tx_with_retry, TestFixture};

#[tokio::test]
async fn collect_moves_only_lamports_above_rent_minimum() -> Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;
    let payer: Keypair = test_fixture.payer_keypair().insecure_clone();
    let collector: Keypair = Keypair::from_bytes(&[
        42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 42,
        42, 42, 42, 42, 42, 42, 42, 42, 42, 25, 127, 107, 35, 225, 108, 133, 50, 198, 171, 200, 56,
        250, 205, 94, 167, 137, 190, 12, 118, 178, 146, 3, 52, 3, 155, 250, 139, 61, 54, 141, 97,
    ])?;
    let excess_lamports: u64 = 123_456;

    let fund_instruction: Instruction =
        system_instruction::transfer(&payer.pubkey(), &test_fixture.wrapper.key, excess_lamports);
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[fund_instruction],
        Some(&payer.pubkey()),
        &[&payer],
    )
    .await?;

    let wrapper_before: u64 = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_balance(test_fixture.wrapper.key)
        .await?;
    let collector_before: u64 = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_balance(collector.pubkey())
        .await?;
    let wrapper_data_len: usize = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(test_fixture.wrapper.key)
        .await?
        .unwrap()
        .data
        .len();
    let rent_minimum: u64 = Rent::default().minimum_balance(wrapper_data_len);
    assert_eq!(wrapper_before, rent_minimum + excess_lamports);

    let collect_instruction: Instruction = Instruction {
        program_id: wrapper::id(),
        accounts: vec![
            AccountMeta::new(test_fixture.wrapper.key, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new(collector.pubkey(), true),
        ],
        data: ManifestWrapperInstruction::Collect.to_vec(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[collect_instruction],
        Some(&payer.pubkey()),
        &[&payer, &collector],
    )
    .await?;

    let wrapper_after: u64 = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_balance(test_fixture.wrapper.key)
        .await?;
    let collector_after: u64 = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_balance(collector.pubkey())
        .await?;
    assert_eq!(wrapper_after, rent_minimum);
    assert_eq!(collector_after, collector_before + excess_lamports);
    Ok(())
}
