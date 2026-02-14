use manifest::validation::get_market_address;
use solana_program_test::tokio;

use crate::TestFixture;

#[tokio::test]
async fn create_market() -> anyhow::Result<()> {
    let _test_fixture: TestFixture = TestFixture::new().await;

    Ok(())
}

#[tokio::test]
async fn create_market_fail_already_initialized() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;

    // The default TestFixture already created a market with index 0 + usdc.
    // Trying to create the same pair again should fail because the PDA already exists.
    assert!(test_fixture
        .create_new_market(
            0,
            9,
            &test_fixture.usdc_mint_fixture.key,
        )
        .await
        .is_err());

    Ok(())
}

#[tokio::test]
async fn create_market_pda_address() -> anyhow::Result<()> {
    let test_fixture: TestFixture = TestFixture::new().await;

    let (expected_market_key, _) = get_market_address(
        0, // base_mint_index used in TestFixture::new
        &test_fixture.usdc_mint_fixture.key,
    );
    assert_eq!(test_fixture.market_fixture.key, expected_market_key);

    Ok(())
}
