//! Integration tests using solana-program-test.
//!
//! These tests verify that instructions created by the slim client
//! work correctly with the actual Manifest program.

#[cfg(test)]
mod integration_tests {
    use crate::batch_update_instruction;
    use crate::claim_seat_instruction;
    use crate::create_market_instruction;
    use crate::deposit_instruction;
    use crate::swap_instruction;
    use crate::withdraw_instruction;
    use crate::BatchUpdateParams;
    use crate::DepositParams;
    use crate::Market;
    use crate::PlaceOrderParams;
    use crate::SwapParams;
    use crate::WithdrawParams;
    use crate::MARKET_FIXED_SIZE;
    use crate::TOKEN_2022_PROGRAM_ID;
    use crate::TOKEN_PROGRAM_ID;
    use solana_pubkey::Pubkey;

    use solana_program::program_pack::Pack;
    use solana_program::system_instruction;
    use solana_program_test::{processor, ProgramTest};
    use solana_sdk::{
        instruction::Instruction as SolanaInstruction,
        pubkey::Pubkey as SolanaPubkey,
        rent::Rent,
        signature::{Keypair, Signer},
        transaction::Transaction,
    };
    use spl_token::state::Mint;

    /// Convert our Pubkey to solana_sdk::Pubkey
    fn to_solana_pubkey(pk: &Pubkey) -> SolanaPubkey {
        SolanaPubkey::new_from_array(pk.to_bytes())
    }

    /// Convert solana_sdk::Pubkey to our Pubkey
    fn from_solana_pubkey(pk: &SolanaPubkey) -> Pubkey {
        Pubkey::new_from_array(pk.to_bytes())
    }

    /// Convert our Instruction to solana_sdk::Instruction
    fn to_solana_instruction(ix: &crate::Instruction) -> SolanaInstruction {
        SolanaInstruction {
            program_id: to_solana_pubkey(&ix.program_id),
            accounts: ix
                .accounts
                .iter()
                .map(|a| solana_sdk::instruction::AccountMeta {
                    pubkey: to_solana_pubkey(&a.pubkey),
                    is_signer: a.is_signer,
                    is_writable: a.is_writable,
                })
                .collect(),
            data: ix.data.clone(),
        }
    }

    async fn setup_test() -> (
        solana_program_test::BanksClient,
        Keypair,
        solana_sdk::hash::Hash,
    ) {
        let program_test = ProgramTest::new(
            "manifest",
            manifest::ID,
            processor!(manifest::process_instruction),
        );

        program_test.start().await
    }

    async fn create_mint(
        banks_client: &mut solana_program_test::BanksClient,
        payer: &Keypair,
        recent_blockhash: solana_sdk::hash::Hash,
        decimals: u8,
    ) -> SolanaPubkey {
        let mint = Keypair::new();
        let rent = Rent::default();
        let mint_rent = rent.minimum_balance(Mint::LEN);

        let create_mint_account_ix = system_instruction::create_account(
            &payer.pubkey(),
            &mint.pubkey(),
            mint_rent,
            Mint::LEN as u64,
            &spl_token::id(),
        );

        let init_mint_ix = spl_token::instruction::initialize_mint(
            &spl_token::id(),
            &mint.pubkey(),
            &payer.pubkey(),
            None,
            decimals,
        )
        .unwrap();

        let tx = Transaction::new_signed_with_payer(
            &[create_mint_account_ix, init_mint_ix],
            Some(&payer.pubkey()),
            &[payer, &mint],
            recent_blockhash,
        );

        banks_client.process_transaction(tx).await.unwrap();

        mint.pubkey()
    }

    async fn create_token_account(
        banks_client: &mut solana_program_test::BanksClient,
        payer: &Keypair,
        recent_blockhash: solana_sdk::hash::Hash,
        mint: &SolanaPubkey,
        owner: &SolanaPubkey,
    ) -> SolanaPubkey {
        let token_account = Keypair::new();
        let rent = Rent::default();
        let account_rent = rent.minimum_balance(spl_token::state::Account::LEN);

        let create_account_ix = system_instruction::create_account(
            &payer.pubkey(),
            &token_account.pubkey(),
            account_rent,
            spl_token::state::Account::LEN as u64,
            &spl_token::id(),
        );

        let init_account_ix = spl_token::instruction::initialize_account(
            &spl_token::id(),
            &token_account.pubkey(),
            mint,
            owner,
        )
        .unwrap();

        let tx = Transaction::new_signed_with_payer(
            &[create_account_ix, init_account_ix],
            Some(&payer.pubkey()),
            &[payer, &token_account],
            recent_blockhash,
        );

        banks_client.process_transaction(tx).await.unwrap();

        token_account.pubkey()
    }

    async fn mint_tokens(
        banks_client: &mut solana_program_test::BanksClient,
        payer: &Keypair,
        recent_blockhash: solana_sdk::hash::Hash,
        mint: &SolanaPubkey,
        token_account: &SolanaPubkey,
        amount: u64,
    ) {
        let mint_to_ix = spl_token::instruction::mint_to(
            &spl_token::id(),
            mint,
            token_account,
            &payer.pubkey(),
            &[],
            amount,
        )
        .unwrap();

        let tx = Transaction::new_signed_with_payer(
            &[mint_to_ix],
            Some(&payer.pubkey()),
            &[payer],
            recent_blockhash,
        );

        banks_client.process_transaction(tx).await.unwrap();
    }

    #[tokio::test]
    async fn test_create_market() {
        let (mut banks_client, payer, recent_blockhash) = setup_test().await;

        // Create mints
        let base_mint = create_mint(&mut banks_client, &payer, recent_blockhash, 9).await;
        let quote_mint = create_mint(&mut banks_client, &payer, recent_blockhash, 6).await;

        // Create market keypair
        let market = Keypair::new();
        let rent = Rent::default();

        // Allocate market account (create_market expects exactly MARKET_FIXED_SIZE)
        let market_size: usize = MARKET_FIXED_SIZE;
        let market_rent: u64 = rent.minimum_balance(market_size);

        let create_market_account_ix: SolanaInstruction = system_instruction::create_account(
            &payer.pubkey(),
            &market.pubkey(),
            market_rent,
            market_size as u64,
            &manifest::ID,
        );

        // Create the market instruction using our slim client
        let create_market_ix = create_market_instruction(
            from_solana_pubkey(&payer.pubkey()),
            from_solana_pubkey(&market.pubkey()),
            from_solana_pubkey(&base_mint),
            from_solana_pubkey(&quote_mint),
            TOKEN_PROGRAM_ID,
            TOKEN_2022_PROGRAM_ID,
        );

        let tx = Transaction::new_signed_with_payer(
            &[
                create_market_account_ix,
                to_solana_instruction(&create_market_ix),
            ],
            Some(&payer.pubkey()),
            &[&payer, &market],
            recent_blockhash,
        );

        banks_client.process_transaction(tx).await.unwrap();

        // Verify the market was created by reading the account
        let market_account = banks_client
            .get_account(market.pubkey())
            .await
            .unwrap()
            .unwrap();

        let parsed_market = Market::try_from_bytes(&market_account.data).unwrap();
        assert_eq!(to_solana_pubkey(&parsed_market.get_base_mint()), base_mint);
        assert_eq!(
            to_solana_pubkey(&parsed_market.get_quote_mint()),
            quote_mint
        );
    }

    #[tokio::test]
    async fn test_deposit_and_withdraw() {
        let (mut banks_client, payer, mut recent_blockhash) = setup_test().await;

        // Create mints
        let base_mint = create_mint(&mut banks_client, &payer, recent_blockhash, 9).await;
        let quote_mint = create_mint(&mut banks_client, &payer, recent_blockhash, 6).await;

        // Create token accounts
        let trader_base_account = create_token_account(
            &mut banks_client,
            &payer,
            recent_blockhash,
            &base_mint,
            &payer.pubkey(),
        )
        .await;

        // Mint some tokens
        mint_tokens(
            &mut banks_client,
            &payer,
            recent_blockhash,
            &base_mint,
            &trader_base_account,
            1_000_000_000,
        )
        .await;

        // Refresh blockhash
        recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();

        // Create market
        let market = Keypair::new();
        let rent = Rent::default();
        let market_size: usize = MARKET_FIXED_SIZE;
        let market_rent = rent.minimum_balance(market_size);

        let create_market_account_ix = system_instruction::create_account(
            &payer.pubkey(),
            &market.pubkey(),
            market_rent,
            market_size as u64,
            &manifest::ID,
        );

        let create_market_ix = create_market_instruction(
            from_solana_pubkey(&payer.pubkey()),
            from_solana_pubkey(&market.pubkey()),
            from_solana_pubkey(&base_mint),
            from_solana_pubkey(&quote_mint),
            TOKEN_PROGRAM_ID,
            TOKEN_2022_PROGRAM_ID,
        );

        let tx = Transaction::new_signed_with_payer(
            &[
                create_market_account_ix,
                to_solana_instruction(&create_market_ix),
            ],
            Some(&payer.pubkey()),
            &[&payer, &market],
            recent_blockhash,
        );

        banks_client.process_transaction(tx).await.unwrap();

        // Refresh blockhash
        recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();

        // Claim seat
        let claim_seat_ix = claim_seat_instruction(
            from_solana_pubkey(&payer.pubkey()),
            from_solana_pubkey(&market.pubkey()),
        );

        let tx = Transaction::new_signed_with_payer(
            &[to_solana_instruction(&claim_seat_ix)],
            Some(&payer.pubkey()),
            &[&payer],
            recent_blockhash,
        );

        banks_client.process_transaction(tx).await.unwrap();

        // Refresh blockhash
        recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();

        // Deposit
        let deposit_amount = 100_000_000u64;
        let deposit_ix = deposit_instruction(
            from_solana_pubkey(&payer.pubkey()),
            from_solana_pubkey(&market.pubkey()),
            from_solana_pubkey(&trader_base_account),
            from_solana_pubkey(&base_mint),
            TOKEN_PROGRAM_ID,
            DepositParams::new(deposit_amount),
        );

        let tx = Transaction::new_signed_with_payer(
            &[to_solana_instruction(&deposit_ix)],
            Some(&payer.pubkey()),
            &[&payer],
            recent_blockhash,
        );

        banks_client.process_transaction(tx).await.unwrap();

        // Refresh blockhash
        recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();

        // Withdraw
        let withdraw_ix = withdraw_instruction(
            from_solana_pubkey(&payer.pubkey()),
            from_solana_pubkey(&market.pubkey()),
            from_solana_pubkey(&trader_base_account),
            from_solana_pubkey(&base_mint),
            TOKEN_PROGRAM_ID,
            WithdrawParams::new(deposit_amount),
        );

        let tx = Transaction::new_signed_with_payer(
            &[to_solana_instruction(&withdraw_ix)],
            Some(&payer.pubkey()),
            &[&payer],
            recent_blockhash,
        );

        banks_client.process_transaction(tx).await.unwrap();
    }

    #[tokio::test]
    async fn test_place_order() {
        let (mut banks_client, payer, mut recent_blockhash) = setup_test().await;

        // Create mints
        let base_mint = create_mint(&mut banks_client, &payer, recent_blockhash, 9).await;
        let quote_mint = create_mint(&mut banks_client, &payer, recent_blockhash, 6).await;

        // Create token accounts
        let _trader_base_account = create_token_account(
            &mut banks_client,
            &payer,
            recent_blockhash,
            &base_mint,
            &payer.pubkey(),
        )
        .await;

        let trader_quote_account = create_token_account(
            &mut banks_client,
            &payer,
            recent_blockhash,
            &quote_mint,
            &payer.pubkey(),
        )
        .await;

        // Mint tokens
        mint_tokens(
            &mut banks_client,
            &payer,
            recent_blockhash,
            &quote_mint,
            &trader_quote_account,
            1_000_000_000,
        )
        .await;

        // Refresh blockhash
        recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();

        // Create market
        let market = Keypair::new();
        let rent = Rent::default();
        let market_size: usize = MARKET_FIXED_SIZE;
        let market_rent = rent.minimum_balance(market_size);

        let create_market_account_ix = system_instruction::create_account(
            &payer.pubkey(),
            &market.pubkey(),
            market_rent,
            market_size as u64,
            &manifest::ID,
        );

        let create_market_ix = create_market_instruction(
            from_solana_pubkey(&payer.pubkey()),
            from_solana_pubkey(&market.pubkey()),
            from_solana_pubkey(&base_mint),
            from_solana_pubkey(&quote_mint),
            TOKEN_PROGRAM_ID,
            TOKEN_2022_PROGRAM_ID,
        );

        let tx = Transaction::new_signed_with_payer(
            &[
                create_market_account_ix,
                to_solana_instruction(&create_market_ix),
            ],
            Some(&payer.pubkey()),
            &[&payer, &market],
            recent_blockhash,
        );

        banks_client.process_transaction(tx).await.unwrap();

        // Refresh blockhash
        recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();

        // Claim seat
        let claim_seat_ix = claim_seat_instruction(
            from_solana_pubkey(&payer.pubkey()),
            from_solana_pubkey(&market.pubkey()),
        );

        let tx = Transaction::new_signed_with_payer(
            &[to_solana_instruction(&claim_seat_ix)],
            Some(&payer.pubkey()),
            &[&payer],
            recent_blockhash,
        );

        banks_client.process_transaction(tx).await.unwrap();

        // Refresh blockhash
        recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();

        // Deposit quote tokens (for placing a bid)
        let deposit_ix = deposit_instruction(
            from_solana_pubkey(&payer.pubkey()),
            from_solana_pubkey(&market.pubkey()),
            from_solana_pubkey(&trader_quote_account),
            from_solana_pubkey(&quote_mint),
            TOKEN_PROGRAM_ID,
            DepositParams::new(1_000_000_000),
        );

        let tx = Transaction::new_signed_with_payer(
            &[to_solana_instruction(&deposit_ix)],
            Some(&payer.pubkey()),
            &[&payer],
            recent_blockhash,
        );

        banks_client.process_transaction(tx).await.unwrap();

        // Refresh blockhash
        recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();

        // Place a bid order using batch_update
        // Bid for 0.001 base tokens at price 1.0 (mantissa=1, exp=0)
        // Cost = 0.001 * 1.0 = 0.001 quote tokens = 1_000 quote atoms
        let batch_update_params: BatchUpdateParams =
            BatchUpdateParams::new().add_order(PlaceOrderParams::new(
                1_000_000, // 0.001 base tokens (9 decimals)
                1,         // price mantissa
                0,         // price exponent
                true,      // is_bid
                crate::OrderType::Limit,
            ));

        let batch_update_ix = batch_update_instruction(
            from_solana_pubkey(&payer.pubkey()),
            from_solana_pubkey(&market.pubkey()),
            batch_update_params,
        );

        let tx = Transaction::new_signed_with_payer(
            &[to_solana_instruction(&batch_update_ix)],
            Some(&payer.pubkey()),
            &[&payer],
            recent_blockhash,
        );

        banks_client.process_transaction(tx).await.unwrap();

        // Read market and verify order was placed
        let market_account = banks_client
            .get_account(market.pubkey())
            .await
            .unwrap()
            .unwrap();

        let parsed_market = Market::try_from_bytes(&market_account.data).unwrap();
        assert!(parsed_market.get_best_bid().is_some());
    }

    #[tokio::test]
    async fn test_swap() {
        let (mut banks_client, payer, mut recent_blockhash) = setup_test().await;

        // Create mints
        let base_mint = create_mint(&mut banks_client, &payer, recent_blockhash, 9).await;
        let quote_mint = create_mint(&mut banks_client, &payer, recent_blockhash, 6).await;

        // Create token accounts for market maker
        let maker_base_account = create_token_account(
            &mut banks_client,
            &payer,
            recent_blockhash,
            &base_mint,
            &payer.pubkey(),
        )
        .await;

        let maker_quote_account = create_token_account(
            &mut banks_client,
            &payer,
            recent_blockhash,
            &quote_mint,
            &payer.pubkey(),
        )
        .await;

        // Mint tokens to maker
        mint_tokens(
            &mut banks_client,
            &payer,
            recent_blockhash,
            &base_mint,
            &maker_base_account,
            10_000_000_000_000, // 10k base tokens
        )
        .await;
        mint_tokens(
            &mut banks_client,
            &payer,
            recent_blockhash,
            &quote_mint,
            &maker_quote_account,
            10_000_000_000, // 10k quote tokens (USDC)
        )
        .await;

        // Refresh blockhash
        recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();

        // Create market
        let market = Keypair::new();
        let rent = Rent::default();
        let market_size: usize = MARKET_FIXED_SIZE;
        let market_rent = rent.minimum_balance(market_size);

        let create_market_account_ix = system_instruction::create_account(
            &payer.pubkey(),
            &market.pubkey(),
            market_rent,
            market_size as u64,
            &manifest::ID,
        );

        let create_market_ix = create_market_instruction(
            from_solana_pubkey(&payer.pubkey()),
            from_solana_pubkey(&market.pubkey()),
            from_solana_pubkey(&base_mint),
            from_solana_pubkey(&quote_mint),
            TOKEN_PROGRAM_ID,
            TOKEN_2022_PROGRAM_ID,
        );

        let tx = Transaction::new_signed_with_payer(
            &[
                create_market_account_ix,
                to_solana_instruction(&create_market_ix),
            ],
            Some(&payer.pubkey()),
            &[&payer, &market],
            recent_blockhash,
        );

        banks_client.process_transaction(tx).await.unwrap();

        // Refresh blockhash
        recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();

        // Claim seat
        let claim_seat_ix = claim_seat_instruction(
            from_solana_pubkey(&payer.pubkey()),
            from_solana_pubkey(&market.pubkey()),
        );

        let tx = Transaction::new_signed_with_payer(
            &[to_solana_instruction(&claim_seat_ix)],
            Some(&payer.pubkey()),
            &[&payer],
            recent_blockhash,
        );

        banks_client.process_transaction(tx).await.unwrap();

        // Refresh blockhash
        recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();

        // Deposit base tokens
        let deposit_base_ix = deposit_instruction(
            from_solana_pubkey(&payer.pubkey()),
            from_solana_pubkey(&market.pubkey()),
            from_solana_pubkey(&maker_base_account),
            from_solana_pubkey(&base_mint),
            TOKEN_PROGRAM_ID,
            DepositParams::new(1_000_000_000_000),
        );

        let tx = Transaction::new_signed_with_payer(
            &[to_solana_instruction(&deposit_base_ix)],
            Some(&payer.pubkey()),
            &[&payer],
            recent_blockhash,
        );

        banks_client.process_transaction(tx).await.unwrap();

        // Refresh blockhash
        recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();

        // Place an ask order
        let batch_update_params = BatchUpdateParams::new().add_order(PlaceOrderParams::new(
            1_000_000_000_000, // 1000 base tokens
            150,               // mantissa for price 150 USDC/SOL
            0,                 // exponent
            false,             // is_bid (this is an ask)
            crate::OrderType::Limit,
        ));

        let batch_update_ix = batch_update_instruction(
            from_solana_pubkey(&payer.pubkey()),
            from_solana_pubkey(&market.pubkey()),
            batch_update_params,
        );

        let tx = Transaction::new_signed_with_payer(
            &[to_solana_instruction(&batch_update_ix)],
            Some(&payer.pubkey()),
            &[&payer],
            recent_blockhash,
        );

        banks_client.process_transaction(tx).await.unwrap();

        // Refresh blockhash
        recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();

        // Create a taker and swap against the order
        let taker = Keypair::new();

        // Fund taker
        let fund_taker_ix =
            system_instruction::transfer(&payer.pubkey(), &taker.pubkey(), 1_000_000_000);

        let tx = Transaction::new_signed_with_payer(
            &[fund_taker_ix],
            Some(&payer.pubkey()),
            &[&payer],
            recent_blockhash,
        );

        banks_client.process_transaction(tx).await.unwrap();

        // Refresh blockhash
        recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();

        // Create taker token accounts
        let taker_base_account = create_token_account(
            &mut banks_client,
            &payer,
            recent_blockhash,
            &base_mint,
            &taker.pubkey(),
        )
        .await;

        let taker_quote_account = create_token_account(
            &mut banks_client,
            &payer,
            recent_blockhash,
            &quote_mint,
            &taker.pubkey(),
        )
        .await;

        // Mint quote to taker for the swap
        mint_tokens(
            &mut banks_client,
            &payer,
            recent_blockhash,
            &quote_mint,
            &taker_quote_account,
            1_000_000_000, // 1000 USDC
        )
        .await;

        // Refresh blockhash
        recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();

        // Swap: taker buys base with quote
        let swap_ix = swap_instruction(
            from_solana_pubkey(&taker.pubkey()),
            from_solana_pubkey(&market.pubkey()),
            from_solana_pubkey(&taker_base_account),
            from_solana_pubkey(&taker_quote_account),
            from_solana_pubkey(&base_mint),
            from_solana_pubkey(&quote_mint),
            TOKEN_PROGRAM_ID,
            None,  // same token program
            false, // no base mint needed (not token-2022)
            false, // no quote mint needed (not token-2022)
            SwapParams::new(
                150_000_000, // 150 USDC in
                0,           // min base out (we don't care for the test)
                false,       // is_base_in (we're swapping quote in)
                true,        // is_exact_in
            ),
        );

        let tx = Transaction::new_signed_with_payer(
            &[to_solana_instruction(&swap_ix)],
            Some(&taker.pubkey()),
            &[&taker],
            recent_blockhash,
        );

        banks_client.process_transaction(tx).await.unwrap();
    }
}
