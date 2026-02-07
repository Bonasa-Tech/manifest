use spl_associated_token_account::get_associated_token_address;
use std::{
    cell::{Ref, RefCell, RefMut},
    io::Error,
    str::FromStr,
};

use hypertree::{DataIndex, HyperTreeValueIteratorTrait};
use manifest::{
    program::{
        batch_update::{CancelOrderParams, PlaceOrderParams},
        batch_update_instruction,
        claim_seat_instruction::claim_seat_instruction,
        create_market_instructions, deposit_instruction, get_dynamic_value,
        global_add_trader_instruction,
        global_create_instruction::create_global_instruction,
        global_deposit_instruction, global_withdraw_instruction, swap_instruction,
        swap_v2_instruction, withdraw_instruction,
    },
    quantities::WrapperU64,
    state::{GlobalFixed, GlobalValue, MarketFixed, MarketValue, OrderType, RestingOrder},
    validation::{get_global_address, get_vault_address, MintAccountInfo},
};
use solana_program::{hash::Hash, pubkey::Pubkey, rent::Rent};
use solana_program_test::{processor, BanksClientError, ProgramTest, ProgramTestContext};
use solana_sdk::{
    account::Account, account_info::AccountInfo, clock::Clock, instruction::Instruction,
    program_pack::Pack, signature::Keypair, signer::Signer, system_instruction::create_account,
    transaction::Transaction,
};
use spl_token_2022::state::Mint;
use std::rc::Rc;

#[derive(PartialEq)]
pub enum Token {
    USDC = 0,
    SOL = 1,
}

#[derive(PartialEq)]
pub enum Side {
    Bid = 0,
    Ask = 1,
}

pub const RUST_LOG_DEFAULT: &str = "solana_rbpf::vm=info,\
             solana_program_runtime::stable_log=debug,\
             solana_runtime::message_processor=debug,\
             solana_runtime::system_instruction_processor=info,\
             solana_program_test=info,\
             solana_bpf_loader_program=debug";

// Not lots, just big enough numbers for tests to run.
pub const SOL_UNIT_SIZE: u64 = 1_000_000_000;
pub const USDC_UNIT_SIZE: u64 = 1_000_000;

pub struct TestFixture {
    pub context: Rc<RefCell<ProgramTestContext>>,
    pub sol_mint_fixture: MintFixture,
    pub usdc_mint_fixture: MintFixture,
    pub payer_sol_fixture: TokenAccountFixture,
    pub payer_usdc_fixture: TokenAccountFixture,
    pub market_fixture: MarketFixture,
    pub global_fixture: GlobalFixture,
    pub sol_global_fixture: GlobalFixture,
    pub second_keypair: Keypair,
}

impl TestFixture {
    pub async fn new() -> TestFixture {
        let mut program: ProgramTest = ProgramTest::new(
            "manifest",
            manifest::ID,
            processor!(manifest::process_instruction),
        );

        let second_keypair: Keypair = Keypair::new();
        program.add_account(
            second_keypair.pubkey(),
            solana_sdk::account::Account::new(
                u32::MAX as u64,
                0,
                &solana_sdk::system_program::id(),
            ),
        );

        // Add testdata for the reverse coalesce test.
        for pk in [
            "ENhU8LsaR7vDD2G1CsWcsuSGNrih9Cv5WZEk7q9kPapQ",
            "AKjfJDv4ywdpCDrj7AURuNkGA3696GTVFgrMwk4TjkKs",
            "FN9K6rTdWtRDUPmLTN2FnGvLZpHVNRN2MeRghKknSGDs",
            "8sjV1AqBFvFuADBCQHhotaRq5DFFYSjjg1jMyVWMqXvZ",
            "CNRQ2Q5YURFcQrATzYeKUWgKUoBDfqzkDrRWf21UXCVo",
            "FGQoLafigpyVb7mLa6pvsDDpDaEE3JetrzQoAggTo3n7",
        ] {
            let filename = format!("tests/testdata/{}", pk);
            let file: std::fs::File = std::fs::File::open(filename)
                .unwrap_or_else(|_| panic!("{pk} should open read only"));
            let json: serde_json::Value =
                serde_json::from_reader(file).expect("file should be proper JSON");
            program.add_account_with_base64_data(
                Pubkey::from_str(pk).unwrap(),
                u32::MAX as u64,
                Pubkey::from_str(json["result"]["value"]["owner"].as_str().unwrap()).unwrap(),
                json["result"]["value"]["data"].as_array().unwrap()[0]
                    .as_str()
                    .unwrap(),
            );
        }

        let second_payer: Pubkey = second_keypair.pubkey();
        let usdc_mint: Pubkey =
            Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
        let user_usdc_ata: Pubkey = get_associated_token_address(&second_payer, &usdc_mint);
        let mut account: solana_sdk::account::Account = solana_sdk::account::Account::new(
            u32::MAX as u64,
            spl_token::state::Account::get_packed_len(),
            &spl_token::id(),
        );
        let _ = &spl_token::state::Account {
            mint: usdc_mint,
            owner: second_payer,
            amount: 1_000_000_000_000,
            state: spl_token::state::AccountState::Initialized,
            ..spl_token::state::Account::default()
        }
        .pack_into_slice(&mut account.data);
        program.add_account(user_usdc_ata, account);

        let usdt_mint: Pubkey =
            Pubkey::from_str("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB").unwrap();
        let user_usdt_ata: Pubkey = get_associated_token_address(&second_payer, &usdt_mint);
        let mut account: solana_sdk::account::Account = solana_sdk::account::Account::new(
            u32::MAX as u64,
            spl_token::state::Account::get_packed_len(),
            &spl_token::id(),
        );
        let _ = &spl_token::state::Account {
            mint: usdt_mint,
            owner: second_payer,
            amount: 1_000_000_000_000,
            state: spl_token::state::AccountState::Initialized,
            ..spl_token::state::Account::default()
        }
        .pack_into_slice(&mut account.data);
        program.add_account(user_usdt_ata, account);

        let sol_mint: Pubkey =
            Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap();
        let user_sol_ata: Pubkey = get_associated_token_address(&second_payer, &sol_mint);
        let mut account: solana_sdk::account::Account = solana_sdk::account::Account::new(
            u32::MAX as u64,
            spl_token::state::Account::get_packed_len(),
            &spl_token::id(),
        );
        let _ = &spl_token::state::Account {
            mint: sol_mint,
            owner: second_payer,
            amount: 1_000_000_000_000,
            state: spl_token::state::AccountState::Initialized,
            ..spl_token::state::Account::default()
        }
        .pack_into_slice(&mut account.data);
        program.add_account(user_sol_ata, account);

        let context: Rc<RefCell<ProgramTestContext>> =
            Rc::new(RefCell::new(program.start_with_context().await));
        solana_logger::setup_with_default(RUST_LOG_DEFAULT);

        let usdc_mint_f: MintFixture = MintFixture::new(Rc::clone(&context), Some(6)).await;
        let sol_mint_f: MintFixture = MintFixture::new(Rc::clone(&context), Some(9)).await;
        let mut market_fixture: MarketFixture =
            MarketFixture::new(Rc::clone(&context), &sol_mint_f.key, &usdc_mint_f.key).await;

        let mut global_fixture: GlobalFixture =
            GlobalFixture::new(Rc::clone(&context), &usdc_mint_f.key).await;
        let mut sol_global_fixture: GlobalFixture =
            GlobalFixture::new(Rc::clone(&context), &sol_mint_f.key).await;

        let payer: Pubkey = context.borrow().payer.pubkey();
        let payer_sol_fixture: TokenAccountFixture =
            TokenAccountFixture::new(Rc::clone(&context), &sol_mint_f.key, &payer).await;
        let payer_usdc_fixture =
            TokenAccountFixture::new(Rc::clone(&context), &usdc_mint_f.key, &payer).await;
        market_fixture.reload().await;
        global_fixture.reload().await;
        sol_global_fixture.reload().await;

        TestFixture {
            context: Rc::clone(&context),
            usdc_mint_fixture: usdc_mint_f,
            sol_mint_fixture: sol_mint_f,
            market_fixture,
            global_fixture,
            sol_global_fixture,
            payer_sol_fixture,
            payer_usdc_fixture,
            second_keypair,
        }
    }

    pub async fn try_new_for_matching_test() -> anyhow::Result<TestFixture, BanksClientError> {
        let mut test_fixture = TestFixture::new().await;
        let second_keypair = test_fixture.second_keypair.insecure_clone();

        test_fixture.claim_seat().await?;
        test_fixture
            .deposit(Token::SOL, 1_000 * SOL_UNIT_SIZE)
            .await?;
        test_fixture
            .deposit(Token::USDC, 10_000 * USDC_UNIT_SIZE)
            .await?;

        test_fixture.claim_seat_for_keypair(&second_keypair).await?;
        test_fixture
            .deposit_for_keypair(Token::SOL, 1_000 * SOL_UNIT_SIZE, &second_keypair)
            .await?;
        test_fixture
            .deposit_for_keypair(Token::USDC, 10_000 * USDC_UNIT_SIZE, &second_keypair)
            .await?;
        Ok(test_fixture)
    }

    pub async fn try_load(
        &self,
        address: &Pubkey,
    ) -> anyhow::Result<Option<Account>, BanksClientError> {
        self.context
            .borrow_mut()
            .banks_client
            .get_account(*address)
            .await
    }

    pub fn payer(&self) -> Pubkey {
        self.context.borrow().payer.pubkey()
    }

    pub fn payer_keypair(&self) -> Keypair {
        self.context.borrow().payer.insecure_clone()
    }

    pub async fn advance_time_seconds(&self, seconds: i64) {
        let mut clock: Clock = self
            .context
            .borrow_mut()
            .banks_client
            .get_sysvar()
            .await
            .unwrap();
        clock.unix_timestamp += seconds;
        clock.slot += (seconds as u64) / 2;
        self.context.borrow_mut().set_sysvar(&clock);
    }

    pub async fn create_new_market(
        &self,
        base_mint: &Pubkey,
        quote_mint: &Pubkey,
    ) -> anyhow::Result<Pubkey, BanksClientError> {
        let market_keypair: Keypair = Keypair::new();
        let payer: Pubkey = self.context.borrow().payer.pubkey();
        let payer_keypair: Keypair = self.context.borrow().payer.insecure_clone();

        let create_market_ixs: Vec<Instruction> =
            create_market_instructions(&market_keypair.pubkey(), base_mint, quote_mint, &payer)
                .unwrap();

        send_tx_with_retry(
            Rc::clone(&self.context),
            &create_market_ixs[..],
            Some(&payer),
            &[&payer_keypair, &market_keypair],
        )
        .await?;
        Ok(market_keypair.pubkey())
    }

    pub async fn claim_seat(&self) -> anyhow::Result<(), BanksClientError> {
        self.claim_seat_for_keypair(&self.payer_keypair()).await
    }

    pub async fn claim_seat_for_keypair(
        &self,
        keypair: &Keypair,
    ) -> anyhow::Result<(), BanksClientError> {
        let claim_seat_ix: Instruction =
            claim_seat_instruction(&self.market_fixture.key, &keypair.pubkey());
        send_tx_with_retry(
            Rc::clone(&self.context),
            &[claim_seat_ix],
            Some(&keypair.pubkey()),
            &[keypair],
        )
        .await
    }

    pub async fn global_add_trader(&self) -> anyhow::Result<(), BanksClientError> {
        self.global_add_trader_for_keypair(&self.payer_keypair())
            .await
    }

    pub async fn global_add_trader_for_keypair(
        &self,
        keypair: &Keypair,
    ) -> anyhow::Result<(), BanksClientError> {
        send_tx_with_retry(
            Rc::clone(&self.context),
            &[global_add_trader_instruction(
                &self.global_fixture.key,
                &keypair.pubkey(),
            )],
            Some(&keypair.pubkey()),
            &[&keypair],
        )
        .await
    }

    pub async fn global_deposit(&mut self, num_atoms: u64) -> anyhow::Result<(), BanksClientError> {
        self.global_deposit_for_keypair(&self.payer_keypair(), num_atoms)
            .await
    }

    pub async fn global_deposit_for_keypair(
        &mut self,
        keypair: &Keypair,
        num_atoms: u64,
    ) -> anyhow::Result<(), BanksClientError> {
        // Make a throw away token account
        let token_account_keypair: Keypair = Keypair::new();
        let token_account_fixture: TokenAccountFixture = TokenAccountFixture::new_with_keypair(
            Rc::clone(&self.context),
            &self.global_fixture.mint_key,
            &keypair.pubkey(),
            &token_account_keypair,
        )
        .await;
        self.usdc_mint_fixture
            .mint_to(&token_account_fixture.key, num_atoms)
            .await;
        send_tx_with_retry(
            Rc::clone(&self.context),
            &[global_deposit_instruction(
                &self.global_fixture.mint_key,
                &keypair.pubkey(),
                &token_account_fixture.key,
                &spl_token::id(),
                num_atoms,
            )],
            Some(&keypair.pubkey()),
            &[&keypair],
        )
        .await
    }

    pub async fn global_withdraw(
        &mut self,
        num_atoms: u64,
    ) -> anyhow::Result<(), BanksClientError> {
        self.global_withdraw_for_keypair(&self.payer_keypair(), num_atoms)
            .await
    }

    pub async fn global_withdraw_for_keypair(
        &mut self,
        keypair: &Keypair,
        num_atoms: u64,
    ) -> anyhow::Result<(), BanksClientError> {
        // Make a throw away token account
        let token_account_keypair: Keypair = Keypair::new();
        let token_account_fixture: TokenAccountFixture = TokenAccountFixture::new_with_keypair(
            Rc::clone(&self.context),
            &self.global_fixture.mint_key,
            &keypair.pubkey(),
            &token_account_keypair,
        )
        .await;
        self.usdc_mint_fixture
            .mint_to(&token_account_fixture.key, num_atoms)
            .await;
        send_tx_with_retry(
            Rc::clone(&self.context),
            &[global_withdraw_instruction(
                &self.global_fixture.mint_key,
                &keypair.pubkey(),
                &token_account_fixture.key,
                &spl_token::id(),
                num_atoms,
            )],
            Some(&keypair.pubkey()),
            &[&keypair],
        )
        .await
    }

    pub async fn deposit(
        &mut self,
        token: Token,
        num_atoms: u64,
    ) -> anyhow::Result<(), BanksClientError> {
        self.deposit_for_keypair(token, num_atoms, &self.payer_keypair())
            .await?;
        Ok(())
    }

    pub async fn deposit_for_keypair(
        &mut self,
        token: Token,
        num_atoms: u64,
        keypair: &Keypair,
    ) -> anyhow::Result<(), BanksClientError> {
        let is_base: bool = token == Token::SOL;
        let (mint, trader_token_account) = if is_base {
            let trader_token_account: Pubkey = if keypair.pubkey() == self.payer() {
                self.payer_sol_fixture.key
            } else {
                // Make a new token account
                let token_account_keypair: Keypair = Keypair::new();
                let token_account_fixture: TokenAccountFixture =
                    TokenAccountFixture::new_with_keypair(
                        Rc::clone(&self.context),
                        &self.sol_mint_fixture.key,
                        &keypair.pubkey(),
                        &token_account_keypair,
                    )
                    .await;
                token_account_fixture.key
            };
            self.sol_mint_fixture
                .mint_to(&trader_token_account, num_atoms)
                .await;
            (&self.sol_mint_fixture.key, trader_token_account)
        } else {
            let trader_token_account: Pubkey = if keypair.pubkey() == self.payer() {
                self.payer_usdc_fixture.key
            } else {
                // Make a new token account
                let token_account_keypair: Keypair = Keypair::new();
                let token_account_fixture: TokenAccountFixture =
                    TokenAccountFixture::new_with_keypair(
                        Rc::clone(&self.context),
                        &self.usdc_mint_fixture.key,
                        &keypair.pubkey(),
                        &token_account_keypair,
                    )
                    .await;
                token_account_fixture.key
            };
            self.usdc_mint_fixture
                .mint_to(&trader_token_account, num_atoms)
                .await;
            (&self.usdc_mint_fixture.key, trader_token_account)
        };

        let deposit_ix: Instruction = deposit_instruction(
            &self.market_fixture.key,
            &keypair.pubkey(),
            mint,
            num_atoms,
            &trader_token_account,
            spl_token::id(),
            None,
        );

        send_tx_with_retry(
            Rc::clone(&self.context),
            &[deposit_ix],
            Some(&keypair.pubkey()),
            &[keypair],
        )
        .await
    }

    pub async fn withdraw(
        &mut self,
        token: Token,
        num_atoms: u64,
    ) -> anyhow::Result<(), BanksClientError> {
        self.withdraw_for_keypair(token, num_atoms, &self.payer_keypair())
            .await?;
        Ok(())
    }

    pub async fn withdraw_for_keypair(
        &mut self,
        token: Token,
        num_atoms: u64,
        keypair: &Keypair,
    ) -> anyhow::Result<(), BanksClientError> {
        let is_base: bool = token == Token::SOL;
        let (mint, trader_token_account) = if is_base {
            let trader_token_account: Pubkey = if keypair.pubkey() == self.payer() {
                self.payer_sol_fixture.key
            } else {
                // Make a new token account
                let token_account_keypair: Keypair = Keypair::new();
                let token_account_fixture: TokenAccountFixture =
                    TokenAccountFixture::new_with_keypair(
                        Rc::clone(&self.context),
                        &self.sol_mint_fixture.key,
                        &keypair.pubkey(),
                        &token_account_keypair,
                    )
                    .await;
                token_account_fixture.key
            };
            (&self.sol_mint_fixture.key, trader_token_account)
        } else {
            let trader_token_account: Pubkey = if keypair.pubkey() == self.payer() {
                self.payer_usdc_fixture.key
            } else {
                // Make a new token account
                let token_account_keypair: Keypair = Keypair::new();
                let token_account_fixture: TokenAccountFixture =
                    TokenAccountFixture::new_with_keypair(
                        Rc::clone(&self.context),
                        &self.usdc_mint_fixture.key,
                        &keypair.pubkey(),
                        &token_account_keypair,
                    )
                    .await;
                token_account_fixture.key
            };
            (&self.usdc_mint_fixture.key, trader_token_account)
        };

        let withdraw_ix: Instruction = withdraw_instruction(
            &self.market_fixture.key,
            &keypair.pubkey(),
            mint,
            num_atoms,
            &trader_token_account,
            spl_token::id(),
            None,
        );
        send_tx_with_retry(
            Rc::clone(&self.context),
            &[withdraw_ix],
            Some(&keypair.pubkey()),
            &[keypair],
        )
        .await
    }

    pub async fn place_order(
        &mut self,
        side: Side,
        base_atoms: u64,
        price_mantissa: u32,
        price_exponent: i8,
        last_valid_slot: u32,
        order_type: OrderType,
    ) -> anyhow::Result<(), BanksClientError> {
        self.place_order_for_keypair(
            side,
            base_atoms,
            price_mantissa,
            price_exponent,
            last_valid_slot,
            order_type,
            &self.payer_keypair(),
        )
        .await?;
        Ok(())
    }

    pub async fn place_order_for_keypair(
        &mut self,
        side: Side,
        base_atoms: u64,
        price_mantissa: u32,
        price_exponent: i8,
        last_valid_slot: u32,
        order_type: OrderType,
        keypair: &Keypair,
    ) -> anyhow::Result<(), BanksClientError> {
        let is_bid: bool = side == Side::Bid;
        let place_order_ix: Instruction = batch_update_instruction(
            &self.market_fixture.key,
            &keypair.pubkey(),
            None,
            vec![],
            vec![PlaceOrderParams::new(
                base_atoms,
                price_mantissa,
                price_exponent,
                is_bid,
                order_type,
                last_valid_slot,
            )],
            None,
            None,
            None,
            None,
        );
        send_tx_with_retry(
            Rc::clone(&self.context),
            &[place_order_ix],
            Some(&keypair.pubkey()),
            &[keypair],
        )
        .await
    }

    // Similar to swap, but the second_keypair is the gas/rent payer and normal
    // keypair owns the token accounts.
    pub async fn swap_v2(
        &mut self,
        in_atoms: u64,
        out_atoms: u64,
        is_base_in: bool,
        is_exact_in: bool,
    ) -> anyhow::Result<(), BanksClientError> {
        let payer: Pubkey = self.context.borrow().payer.pubkey();
        let payer_keypair: Keypair = self.context.borrow().payer.insecure_clone();

        let swap_ix: Instruction = swap_v2_instruction(
            &self.market_fixture.key,
            &self.second_keypair.pubkey(),
            &payer,
            &self.sol_mint_fixture.key,
            &self.usdc_mint_fixture.key,
            &self.payer_sol_fixture.key,
            &self.payer_usdc_fixture.key,
            in_atoms,
            out_atoms,
            is_base_in,
            is_exact_in,
            spl_token::id(),
            spl_token::id(),
            false,
        );

        send_tx_with_retry(
            Rc::clone(&self.context),
            &[swap_ix],
            Some(&self.second_keypair.pubkey()),
            &[&payer_keypair, &self.second_keypair.insecure_clone()],
        )
        .await
    }

    pub async fn swap(
        &mut self,
        in_atoms: u64,
        out_atoms: u64,
        is_base_in: bool,
        is_exact_in: bool,
    ) -> anyhow::Result<(), BanksClientError> {
        let payer: Pubkey = self.context.borrow().payer.pubkey();
        let payer_keypair: Keypair = self.context.borrow().payer.insecure_clone();
        let swap_ix: Instruction = swap_instruction(
            &self.market_fixture.key,
            &payer,
            &self.sol_mint_fixture.key,
            &self.usdc_mint_fixture.key,
            &self.payer_sol_fixture.key,
            &self.payer_usdc_fixture.key,
            in_atoms,
            out_atoms,
            is_base_in,
            is_exact_in,
            spl_token::id(),
            spl_token::id(),
            false,
        );

        send_tx_with_retry(
            Rc::clone(&self.context),
            &[swap_ix],
            Some(&payer),
            &[&payer_keypair],
        )
        .await
    }

    pub async fn swap_with_global(
        &mut self,
        in_atoms: u64,
        out_atoms: u64,
        is_base_in: bool,
        is_exact_in: bool,
    ) -> anyhow::Result<(), BanksClientError> {
        let payer: Pubkey = self.context.borrow().payer.pubkey();
        let payer_keypair: Keypair = self.context.borrow().payer.insecure_clone();
        let swap_ix: Instruction = swap_instruction(
            &self.market_fixture.key,
            &payer,
            &self.sol_mint_fixture.key,
            &self.usdc_mint_fixture.key,
            &self.payer_sol_fixture.key,
            &self.payer_usdc_fixture.key,
            in_atoms,
            out_atoms,
            is_base_in,
            is_exact_in,
            spl_token::id(),
            spl_token::id(),
            true,
        );

        send_tx_with_retry(
            Rc::clone(&self.context),
            &[swap_ix],
            Some(&payer),
            &[&payer_keypair],
        )
        .await
    }

    pub async fn cancel_order(
        &mut self,
        order_sequence_number: u64,
    ) -> anyhow::Result<(), BanksClientError> {
        let payer: Pubkey = self.context.borrow().payer.pubkey();
        let payer_keypair: Keypair = self.context.borrow().payer.insecure_clone();
        let cancel_order_ix: Instruction = batch_update_instruction(
            &self.market_fixture.key,
            &payer,
            None,
            vec![CancelOrderParams::new(order_sequence_number)],
            vec![],
            None,
            None,
            None,
            None,
        );
        send_tx_with_retry(
            Rc::clone(&self.context),
            &[cancel_order_ix],
            Some(&payer),
            &[&payer_keypair],
        )
        .await
    }

    pub async fn batch_update_for_keypair(
        &mut self,
        trader_index_hint: Option<DataIndex>,
        cancels: Vec<CancelOrderParams>,
        orders: Vec<PlaceOrderParams>,
        keypair: &Keypair,
    ) -> anyhow::Result<(), BanksClientError> {
        let batch_update_ix: Instruction = batch_update_instruction(
            &self.market_fixture.key,
            &keypair.pubkey(),
            trader_index_hint,
            cancels,
            orders,
            None,
            None,
            None,
            None,
        );
        send_tx_with_retry(
            Rc::clone(&self.context),
            &[batch_update_ix],
            Some(&keypair.pubkey()),
            &[keypair],
        )
        .await
    }

    pub async fn batch_update_with_global_for_keypair(
        &mut self,
        trader_index_hint: Option<DataIndex>,
        cancels: Vec<CancelOrderParams>,
        orders: Vec<PlaceOrderParams>,
        keypair: &Keypair,
    ) -> anyhow::Result<(), BanksClientError> {
        let batch_update_ix: Instruction = batch_update_instruction(
            &self.market_fixture.key,
            &keypair.pubkey(),
            trader_index_hint,
            cancels,
            orders,
            Some(*self.market_fixture.market.get_base_mint()),
            None,
            Some(*self.market_fixture.market.get_quote_mint()),
            None,
        );

        send_tx_with_retry(
            Rc::clone(&self.context),
            &[batch_update_ix],
            Some(&keypair.pubkey()),
            &[keypair],
        )
        .await
    }
}

#[derive(Clone)]
pub struct MarketFixture {
    pub context: Rc<RefCell<ProgramTestContext>>,
    pub key: Pubkey,
    pub market: MarketValue,
}

impl MarketFixture {
    pub async fn new(
        context: Rc<RefCell<ProgramTestContext>>,
        base_mint: &Pubkey,
        quote_mint: &Pubkey,
    ) -> Self {
        let market_keypair: Keypair = Keypair::new();
        let payer: Pubkey = context.borrow().payer.pubkey();
        let payer_keypair: Keypair = context.borrow().payer.insecure_clone();
        let create_market_ixs: Vec<Instruction> =
            create_market_instructions(&market_keypair.pubkey(), base_mint, quote_mint, &payer)
                .unwrap();

        send_tx_with_retry(
            Rc::clone(&context),
            &create_market_ixs[..],
            Some(&payer),
            &[&payer_keypair, &market_keypair],
        )
        .await
        .unwrap();

        let context_ref: Rc<RefCell<ProgramTestContext>> = Rc::clone(&context);

        let mut lamports: u64 = 0;
        let base_mint: MintAccountInfo = MintAccountInfo {
            mint: Mint {
                mint_authority: None.into(),
                supply: 0,
                decimals: 6,
                is_initialized: true,
                freeze_authority: None.into(),
            },
            info: &AccountInfo {
                key: &Pubkey::new_unique(),
                lamports: Rc::new(RefCell::new(&mut lamports)),
                data: Rc::new(RefCell::new(&mut [])),
                owner: &Pubkey::new_unique(),
                rent_epoch: 0,
                is_signer: false,
                is_writable: false,
                executable: false,
            },
        };

        let mut lamports: u64 = 0;
        let quote_mint: MintAccountInfo = MintAccountInfo {
            mint: Mint {
                mint_authority: None.into(),
                supply: 0,
                decimals: 9,
                is_initialized: true,
                freeze_authority: None.into(),
            },
            info: &AccountInfo {
                key: &Pubkey::new_unique(),
                lamports: Rc::new(RefCell::new(&mut lamports)),
                data: Rc::new(RefCell::new(&mut [])),
                owner: &Pubkey::new_unique(),
                rent_epoch: 0,
                is_signer: false,
                is_writable: false,
                executable: false,
            },
        };
        // Dummy default value. Not valid until reload.
        MarketFixture {
            context: context_ref,
            key: market_keypair.pubkey(),
            market: MarketValue {
                fixed: MarketFixed::new_empty(&base_mint, &quote_mint, &market_keypair.pubkey()),
                dynamic: Vec::new(),
            },
        }
    }

    pub async fn reload(&mut self) {
        let market_account: Account = self
            .context
            .borrow_mut()
            .banks_client
            .get_account(self.key)
            .await
            .unwrap()
            .unwrap();

        let market: MarketValue = get_dynamic_value(market_account.data.as_slice());
        self.market = market;
    }

    pub async fn get_base_balance_atoms(&mut self, trader: &Pubkey) -> u64 {
        self.reload().await;
        self.market.get_trader_balance(trader).0.as_u64()
    }

    pub async fn get_quote_balance_atoms(&mut self, trader: &Pubkey) -> u64 {
        self.reload().await;
        self.market.get_trader_balance(trader).1.as_u64()
    }

    pub async fn get_quote_volume(&mut self, trader: &Pubkey) -> u64 {
        self.reload().await;
        self.market.get_trader_voume(trader).as_u64()
    }

    pub async fn get_resting_orders(&mut self) -> Vec<RestingOrder> {
        self.reload().await;
        let mut bids_vec: Vec<RestingOrder> = self
            .market
            .get_bids()
            .iter::<RestingOrder>()
            .map(|node| *node.1)
            .collect::<Vec<RestingOrder>>();
        let asks_vec: Vec<RestingOrder> = self
            .market
            .get_asks()
            .iter::<RestingOrder>()
            .map(|node| *node.1)
            .collect::<Vec<RestingOrder>>();
        bids_vec.extend(asks_vec);
        bids_vec
    }

    /// Get vault token account balances (base_vault_balance, quote_vault_balance)
    pub async fn get_vault_balances(&mut self) -> (u64, u64) {
        self.reload().await;
        let (base_vault, _) = get_vault_address(&self.key, self.market.get_base_mint());
        let (quote_vault, _) = get_vault_address(&self.key, self.market.get_quote_mint());

        let base_vault_balance: u64 = self
            .context
            .borrow_mut()
            .banks_client
            .get_packed_account_data::<spl_token::state::Account>(base_vault)
            .await
            .map(|a| a.amount)
            .unwrap_or(0);

        let quote_vault_balance: u64 = self
            .context
            .borrow_mut()
            .banks_client
            .get_packed_account_data::<spl_token::state::Account>(quote_vault)
            .await
            .map(|a| a.amount)
            .unwrap_or(0);

        (base_vault_balance, quote_vault_balance)
    }

    /// Get total base/quote locked in orders.
    /// Returns (base_locked_in_asks, quote_locked_in_bids)
    pub async fn get_total_locked_in_orders(&mut self) -> (u64, u64) {
        self.reload().await;
        let mut base_locked: u64 = 0;
        let mut quote_locked: u64 = 0;

        // Bids lock quote (base_atoms * price)
        for (_, bid) in self.market.get_bids().iter::<RestingOrder>() {
            let locked_quote = bid
                .get_num_base_atoms()
                .checked_mul(bid.get_price(), true)
                .unwrap()
                .as_u64();
            quote_locked += locked_quote;
        }

        // Asks lock base
        for (_, ask) in self.market.get_asks().iter::<RestingOrder>() {
            base_locked += ask.get_num_base_atoms().as_u64();
        }

        (base_locked, quote_locked)
    }

    /// Verify that vault balances match seats + orders.
    /// Takes a slice of trader pubkeys to sum their seat balances.
    pub async fn verify_vault_balance(&mut self, traders: &[Pubkey]) {
        self.reload().await;

        // Sum seat balances
        let mut seats_base: u64 = 0;
        let mut seats_quote: u64 = 0;
        for trader in traders {
            seats_base += self.market.get_trader_balance(trader).0.as_u64();
            seats_quote += self.market.get_trader_balance(trader).1.as_u64();
        }

        // Get locked in orders
        let (base_in_asks, quote_in_bids) = self.get_total_locked_in_orders().await;

        // Get vault balances
        let (vault_base, vault_quote) = self.get_vault_balances().await;

        // Total expected in vault
        let expected_base = seats_base + base_in_asks;
        let expected_quote = seats_quote + quote_in_bids;

        println!(
            "Vault verification: base_vault={} expected={} (seats={} + asks={})",
            vault_base, expected_base, seats_base, base_in_asks
        );
        println!(
            "Vault verification: quote_vault={} expected={} (seats={} + bids={})",
            vault_quote, expected_quote, seats_quote, quote_in_bids
        );

        assert_eq!(
            vault_base, expected_base,
            "Base vault mismatch: vault={}, expected={} (seats={} + asks={})",
            vault_base, expected_base, seats_base, base_in_asks
        );
        assert_eq!(
            vault_quote, expected_quote,
            "Quote vault mismatch: vault={}, expected={} (seats={} + bids={})",
            vault_quote, expected_quote, seats_quote, quote_in_bids
        );
    }
}

#[derive(Clone)]
pub struct GlobalFixture {
    pub context: Rc<RefCell<ProgramTestContext>>,
    pub key: Pubkey,
    pub mint_key: Pubkey,
    pub global: GlobalValue,
}

impl GlobalFixture {
    pub async fn new_with_token_program(
        context: Rc<RefCell<ProgramTestContext>>,
        mint: &Pubkey,
        token_program: &Pubkey,
    ) -> Self {
        let (global_key, _global_bump) = get_global_address(mint);
        let payer: Pubkey = context.borrow().payer.pubkey();
        let payer_keypair: Keypair = context.borrow().payer.insecure_clone();

        let context_ref: Rc<RefCell<ProgramTestContext>> = Rc::clone(&context);

        let create_global_ix: Instruction =
            create_global_instruction(&mint, &payer, &token_program);

        send_tx_with_retry(
            Rc::clone(&context),
            &[create_global_ix],
            Some(&payer),
            &[&payer_keypair, &payer_keypair],
        )
        .await
        .unwrap();

        // Dummy default value. Not valid until reload.
        GlobalFixture {
            context: context_ref,
            key: global_key,
            mint_key: *mint,
            global: GlobalValue {
                fixed: GlobalFixed::new_empty(mint),
                dynamic: Vec::new(),
            },
        }
    }

    pub async fn new(context: Rc<RefCell<ProgramTestContext>>, mint: &Pubkey) -> Self {
        GlobalFixture::new_with_token_program(context, mint, &spl_token::id()).await
    }

    pub async fn reload(&mut self) {
        let global_account: Account = self
            .context
            .borrow_mut()
            .banks_client
            .get_account(self.key)
            .await
            .unwrap()
            .unwrap();

        let global: GlobalValue = get_dynamic_value(global_account.data.as_slice());
        self.global = global;
    }
}

#[derive(Clone)]
pub struct MintFixture {
    pub context: Rc<RefCell<ProgramTestContext>>,
    pub key: Pubkey,
    pub mint: spl_token::state::Mint,
    /// Whether this is a Token-2022 mint with extensions (requires different unpacking)
    pub is_2022_with_extensions: bool,
}

impl MintFixture {
    pub async fn new(
        context: Rc<RefCell<ProgramTestContext>>,
        mint_decimals_opt: Option<u8>,
    ) -> MintFixture {
        // Defaults to not 22.
        MintFixture::new_with_version(context, mint_decimals_opt, false).await
    }

    pub async fn new_with_version(
        context: Rc<RefCell<ProgramTestContext>>,
        mint_decimals_opt: Option<u8>,
        is_22: bool,
    ) -> MintFixture {
        let context_ref: Rc<RefCell<ProgramTestContext>> = Rc::clone(&context);
        let mint_keypair: Keypair = Keypair::new();
        let mint: spl_token::state::Mint = {
            let payer: Keypair = context.borrow().payer.insecure_clone();
            let rent: Rent = context.borrow_mut().banks_client.get_rent().await.unwrap();

            let init_account_ix: Instruction = create_account(
                &payer.pubkey(),
                &mint_keypair.pubkey(),
                rent.minimum_balance(if is_22 {
                    spl_token_2022::state::Mint::LEN
                } else {
                    spl_token::state::Mint::LEN
                }),
                if is_22 {
                    spl_token_2022::state::Mint::LEN as u64
                } else {
                    spl_token::state::Mint::LEN as u64
                },
                &{
                    if is_22 {
                        spl_token_2022::id()
                    } else {
                        spl_token::id()
                    }
                },
            );
            let init_mint_ix: Instruction = if is_22 {
                spl_token_2022::instruction::initialize_mint(
                    &spl_token_2022::id(),
                    &mint_keypair.pubkey(),
                    &payer.pubkey(),
                    None,
                    mint_decimals_opt.unwrap_or(6),
                )
                .unwrap()
            } else {
                spl_token::instruction::initialize_mint(
                    &spl_token::id(),
                    &mint_keypair.pubkey(),
                    &payer.pubkey(),
                    None,
                    mint_decimals_opt.unwrap_or(6),
                )
                .unwrap()
            };

            send_tx_with_retry(
                Rc::clone(&context),
                &[init_account_ix, init_mint_ix],
                Some(&payer.pubkey()),
                &[&payer, &mint_keypair],
            )
            .await
            .unwrap();

            let mint_account: Account = context
                .borrow_mut()
                .banks_client
                .get_account(mint_keypair.pubkey())
                .await
                .unwrap()
                .unwrap();

            // We are not actually using extensions in tests, so can leave this alone
            // https://spl.solana.com/token-2022/onchain#step-6-use-statewithextensions-instead-of-mint-and-account
            spl_token::state::Mint::unpack_unchecked(&mut mint_account.data.as_slice()).unwrap()
        };

        MintFixture {
            context: context_ref,
            key: mint_keypair.pubkey(),
            mint,
            is_2022_with_extensions: false,
        }
    }

    /// Create a Token-2022 mint with TransferFeeConfig extension.
    /// This is used for tokens like LJITSPS that have transfer fees.
    ///
    /// # Arguments
    /// * `context` - Program test context
    /// * `mint_decimals` - Number of decimal places for the mint
    /// * `transfer_fee_bps` - Transfer fee in basis points (e.g., 1000 = 10%)
    pub async fn new_with_transfer_fee(
        context: Rc<RefCell<ProgramTestContext>>,
        mint_decimals: u8,
        transfer_fee_bps: u16,
    ) -> MintFixture {
        let context_ref: Rc<RefCell<ProgramTestContext>> = Rc::clone(&context);
        let mint_keypair: Keypair = Keypair::new();

        let payer: Keypair = context.borrow().payer.insecure_clone();

        // Calculate space needed for mint with TransferFeeConfig extension
        let extension_types: Vec<spl_token_2022::extension::ExtensionType> =
            vec![spl_token_2022::extension::ExtensionType::TransferFeeConfig];
        let space: usize = spl_token_2022::extension::ExtensionType::try_calculate_account_len::<
            spl_token_2022::state::Mint,
        >(&extension_types)
        .unwrap();

        let mint_rent: u64 = solana_program::sysvar::rent::Rent::default().minimum_balance(space);

        let init_account_ix: Instruction = create_account(
            &payer.pubkey(),
            &mint_keypair.pubkey(),
            mint_rent,
            space as u64,
            &spl_token_2022::id(),
        );

        // Initialize transfer fee config before mint initialization
        let transfer_fee_ix: Instruction =
            spl_token_2022::extension::transfer_fee::instruction::initialize_transfer_fee_config(
                &spl_token_2022::id(),
                &mint_keypair.pubkey(),
                None,
                None,
                transfer_fee_bps,
                u64::MAX,
            )
            .unwrap();

        let init_mint_ix: Instruction = spl_token_2022::instruction::initialize_mint2(
            &spl_token_2022::id(),
            &mint_keypair.pubkey(),
            &payer.pubkey(),
            None,
            mint_decimals,
        )
        .unwrap();

        send_tx_with_retry(
            Rc::clone(&context),
            &[init_account_ix, transfer_fee_ix, init_mint_ix],
            Some(&payer.pubkey()),
            &[&payer, &mint_keypair],
        )
        .await
        .unwrap();

        // For Token-2022 mints with extensions, we can't use spl_token::state::Mint::unpack_unchecked
        // because the account layout is different. Instead, manually construct the Mint struct
        // with the known values.
        let mint = spl_token::state::Mint {
            mint_authority: solana_program::program_option::COption::Some(payer.pubkey()),
            supply: 0,
            decimals: mint_decimals,
            is_initialized: true,
            freeze_authority: solana_program::program_option::COption::None,
        };

        MintFixture {
            context: context_ref,
            key: mint_keypair.pubkey(),
            mint,
            is_2022_with_extensions: true,
        }
    }

    pub async fn reload(&mut self) {
        let mint_account = self
            .context
            .borrow_mut()
            .banks_client
            .get_account(self.key)
            .await
            .unwrap()
            .unwrap();

        if self.is_2022_with_extensions {
            // For Token-2022 mints with extensions, use StateWithExtensions to unpack
            let mint_with_ext = spl_token_2022::extension::StateWithExtensions::<
                spl_token_2022::state::Mint,
            >::unpack(&mint_account.data)
            .unwrap();
            let mint_2022 = mint_with_ext.base;
            // Convert to spl_token::state::Mint
            self.mint = spl_token::state::Mint {
                mint_authority: mint_2022.mint_authority,
                supply: mint_2022.supply,
                decimals: mint_2022.decimals,
                is_initialized: mint_2022.is_initialized,
                freeze_authority: mint_2022.freeze_authority,
            };
        } else {
            self.mint =
                spl_token::state::Mint::unpack_unchecked(&mut mint_account.data.as_slice())
                    .unwrap();
        }
    }

    pub async fn mint_to(&mut self, dest: &Pubkey, num_atoms: u64) {
        let payer: Keypair = self.context.borrow().payer.insecure_clone();
        send_tx_with_retry(
            Rc::clone(&self.context),
            &[self.make_mint_to_ix(dest, num_atoms)],
            Some(&payer.pubkey()),
            &[&payer],
        )
        .await
        .unwrap();

        self.reload().await
    }

    fn make_mint_to_ix(&self, dest: &Pubkey, amount: u64) -> Instruction {
        let context: Ref<ProgramTestContext> = self.context.borrow();
        let mint_to_instruction: Instruction = spl_token::instruction::mint_to(
            &spl_token::ID,
            &self.key,
            dest,
            &context.payer.pubkey(),
            &[&context.payer.pubkey()],
            amount,
        )
        .unwrap();
        mint_to_instruction
    }

    pub async fn mint_to_2022(&mut self, dest: &Pubkey, num_atoms: u64) {
        let payer: Keypair = self.context.borrow().payer.insecure_clone();
        send_tx_with_retry(
            Rc::clone(&self.context),
            &[self.make_mint_to_2022_ix(dest, num_atoms)],
            Some(&payer.pubkey()),
            &[&payer],
        )
        .await
        .unwrap();

        self.reload().await
    }

    fn make_mint_to_2022_ix(&self, dest: &Pubkey, amount: u64) -> Instruction {
        let context: Ref<ProgramTestContext> = self.context.borrow();
        let mint_to_instruction: Instruction = spl_token_2022::instruction::mint_to(
            &spl_token_2022::ID,
            &self.key,
            dest,
            &context.payer.pubkey(),
            &[&context.payer.pubkey()],
            amount,
        )
        .unwrap();
        mint_to_instruction
    }
}

pub struct TokenAccountFixture {
    context: Rc<RefCell<ProgramTestContext>>,
    pub key: Pubkey,
}

impl TokenAccountFixture {
    async fn create_ixs(
        rent: Rent,
        mint_pk: &Pubkey,
        payer_pk: &Pubkey,
        owner_pk: &Pubkey,
        keypair: &Keypair,
    ) -> [Instruction; 2] {
        let init_account_ix: Instruction = create_account(
            payer_pk,
            &keypair.pubkey(),
            rent.minimum_balance(spl_token::state::Account::LEN),
            spl_token::state::Account::LEN as u64,
            &spl_token::id(),
        );

        let init_token_ix: Instruction = spl_token::instruction::initialize_account(
            &spl_token::id(),
            &keypair.pubkey(),
            mint_pk,
            owner_pk,
        )
        .unwrap();

        [init_account_ix, init_token_ix]
    }
    async fn create_ixs_2022(
        rent: Rent,
        mint_pk: &Pubkey,
        payer_pk: &Pubkey,
        owner_pk: &Pubkey,
        keypair: &Keypair,
    ) -> [Instruction; 2] {
        let init_account_ix: Instruction = create_account(
            payer_pk,
            &keypair.pubkey(),
            rent.minimum_balance(spl_token_2022::state::Account::LEN),
            spl_token_2022::state::Account::LEN as u64,
            &spl_token_2022::id(),
        );

        let init_token_ix: Instruction = spl_token_2022::instruction::initialize_account(
            &spl_token_2022::id(),
            &keypair.pubkey(),
            mint_pk,
            owner_pk,
        )
        .unwrap();

        [init_account_ix, init_token_ix]
    }

    pub async fn new_with_keypair_2022(
        context: Rc<RefCell<ProgramTestContext>>,
        mint_pk: &Pubkey,
        owner_pk: &Pubkey,
        keypair: &Keypair,
    ) -> Self {
        let rent: Rent = context.borrow_mut().banks_client.get_rent().await.unwrap();
        let payer: Pubkey = context.borrow().payer.pubkey();
        let payer_keypair: Keypair = context.borrow().payer.insecure_clone();
        let instructions: [Instruction; 2] =
            Self::create_ixs_2022(rent, mint_pk, &payer, owner_pk, keypair).await;

        send_tx_with_retry(
            Rc::clone(&context),
            &instructions[..],
            Some(&payer),
            &[&payer_keypair, keypair],
        )
        .await
        .unwrap();

        let context_ref: Rc<RefCell<ProgramTestContext>> = context.clone();
        Self {
            context: context_ref.clone(),
            key: keypair.pubkey(),
        }
    }

    pub async fn new_with_keypair(
        context: Rc<RefCell<ProgramTestContext>>,
        mint_pk: &Pubkey,
        owner_pk: &Pubkey,
        keypair: &Keypair,
    ) -> Self {
        let rent: Rent = context.borrow_mut().banks_client.get_rent().await.unwrap();
        let payer: Pubkey = context.borrow().payer.pubkey();
        let payer_keypair: Keypair = context.borrow().payer.insecure_clone();
        let instructions: [Instruction; 2] =
            Self::create_ixs(rent, mint_pk, &payer, owner_pk, keypair).await;

        let _ = send_tx_with_retry(
            Rc::clone(&context),
            &instructions[..],
            Some(&payer),
            &[&payer_keypair, keypair],
        )
        .await;

        let context_ref: Rc<RefCell<ProgramTestContext>> = context.clone();
        Self {
            context: context_ref.clone(),
            key: keypair.pubkey(),
        }
    }

    pub async fn new(
        context: Rc<RefCell<ProgramTestContext>>,
        mint_pk: &Pubkey,
        owner_pk: &Pubkey,
    ) -> TokenAccountFixture {
        let keypair: Keypair = Keypair::new();
        TokenAccountFixture::new_with_keypair(context, mint_pk, owner_pk, &keypair).await
    }

    pub async fn balance_atoms(&self) -> u64 {
        let token_account: spl_token::state::Account =
            get_and_deserialize(self.context.clone(), self.key).await;

        token_account.amount
    }
}

pub async fn get_and_deserialize<T: Pack>(
    context: Rc<RefCell<ProgramTestContext>>,
    pubkey: Pubkey,
) -> T {
    let mut context: RefMut<ProgramTestContext> = context.borrow_mut();
    loop {
        let account_or: Result<Option<Account>, BanksClientError> =
            context.banks_client.get_account(pubkey).await;
        if !account_or.is_ok() {
            continue;
        }
        let account_opt: Option<Account> = account_or.unwrap();
        if account_opt.is_none() {
            continue;
        }
        return T::unpack_unchecked(&mut account_opt.unwrap().data.as_slice()).unwrap();
    }
}

pub async fn send_tx_with_retry(
    context: Rc<RefCell<ProgramTestContext>>,
    instructions: &[Instruction],
    payer: Option<&Pubkey>,
    signers: &[&Keypair],
) -> Result<(), BanksClientError> {
    let mut context: RefMut<ProgramTestContext> = context.borrow_mut();

    loop {
        let blockhash_or: Result<Hash, Error> = context.get_new_latest_blockhash().await;
        if blockhash_or.is_err() {
            continue;
        }
        let tx: Transaction =
            Transaction::new_signed_with_payer(instructions, payer, signers, blockhash_or.unwrap());
        let result: Result<(), BanksClientError> =
            context.banks_client.process_transaction(tx).await;
        if result.is_ok() {
            break;
        }
        let error: BanksClientError = result.err().unwrap();
        match error {
            BanksClientError::RpcError(_rpc_err) => {
                // Retry on rpc errors.
                continue;
            }
            BanksClientError::Io(_io_err) => {
                // Retry on io errors.
                continue;
            }
            _ => {
                println!("Unexpected error: {:?}", error);
                return Err(error);
            }
        }
    }
    Ok(())
}

/// Verify that vault balances match the sum of trader seat balances plus amounts locked in orders.
/// This is a standalone helper that works with a raw context and market key.
///
/// # Arguments
/// * `context` - The program test context
/// * `market_key` - The market pubkey
/// * `traders` - List of trader pubkeys whose seat balances should be summed
pub async fn verify_vault_balance(
    context: Rc<RefCell<ProgramTestContext>>,
    market_key: &Pubkey,
    traders: &[Pubkey],
) {
    use manifest::program::get_dynamic_value;
    use manifest::state::RestingOrder;

    // Get market data
    let market_account: Account = context
        .borrow_mut()
        .banks_client
        .get_account(*market_key)
        .await
        .unwrap()
        .unwrap();
    let market: manifest::state::MarketValue = get_dynamic_value(market_account.data.as_slice());

    // Sum seat balances for all traders
    let mut seats_base: u64 = 0;
    let mut seats_quote: u64 = 0;
    for trader in traders {
        let balance = market.get_trader_balance(trader);
        seats_base += balance.0.as_u64();
        seats_quote += balance.1.as_u64();
    }

    // Get amounts locked in orders
    let mut base_in_asks: u64 = 0;
    let mut quote_in_bids: u64 = 0;

    for (_, bid) in market.get_bids().iter::<RestingOrder>() {
        let locked_quote = bid
            .get_num_base_atoms()
            .checked_mul(bid.get_price(), true)
            .unwrap()
            .as_u64();
        quote_in_bids += locked_quote;
    }

    for (_, ask) in market.get_asks().iter::<RestingOrder>() {
        base_in_asks += ask.get_num_base_atoms().as_u64();
    }

    // Get vault balances
    let (base_vault, _) = get_vault_address(market_key, market.get_base_mint());
    let (quote_vault, _) = get_vault_address(market_key, market.get_quote_mint());

    let vault_base: u64 = context
        .borrow_mut()
        .banks_client
        .get_packed_account_data::<spl_token::state::Account>(base_vault)
        .await
        .map(|a| a.amount)
        .unwrap_or(0);

    let vault_quote: u64 = context
        .borrow_mut()
        .banks_client
        .get_packed_account_data::<spl_token::state::Account>(quote_vault)
        .await
        .map(|a| a.amount)
        .unwrap_or(0);

    let expected_base = seats_base + base_in_asks;
    let expected_quote = seats_quote + quote_in_bids;

    println!(
        "Vault verification: base_vault={} expected={} (seats={} + asks={})",
        vault_base, expected_base, seats_base, base_in_asks
    );
    println!(
        "Vault verification: quote_vault={} expected={} (seats={} + bids={})",
        vault_quote, expected_quote, seats_quote, quote_in_bids
    );

    assert_eq!(
        vault_base, expected_base,
        "Base vault mismatch: vault={}, expected={} (seats={} + asks={})",
        vault_base, expected_base, seats_base, base_in_asks
    );
    assert_eq!(
        vault_quote, expected_quote,
        "Quote vault mismatch: vault={}, expected={} (seats={} + bids={})",
        vault_quote, expected_quote, seats_quote, quote_in_bids
    );

    println!("Vault verification passed!");
}

/// Create a market with the given base and quote mints.
/// Returns the market keypair.
pub async fn create_market_with_mints(
    context: Rc<RefCell<ProgramTestContext>>,
    base_mint: &Pubkey,
    quote_mint: &Pubkey,
) -> Result<Keypair, BanksClientError> {
    let market_keypair = Keypair::new();
    let payer_keypair = context.borrow().payer.insecure_clone();
    let payer = payer_keypair.pubkey();

    let create_market_ixs: Vec<Instruction> =
        create_market_instructions(&market_keypair.pubkey(), base_mint, quote_mint, &payer)
            .unwrap();

    send_tx_with_retry(
        Rc::clone(&context),
        &create_market_ixs[..],
        Some(&payer),
        &[&payer_keypair, &market_keypair],
    )
    .await?;

    Ok(market_keypair)
}

/// Create a Token-2022 token account for a mint with transfer fee extension.
/// Returns the token account keypair.
pub async fn create_token_2022_account(
    context: Rc<RefCell<ProgramTestContext>>,
    mint: &Pubkey,
    owner: &Pubkey,
) -> Result<Keypair, BanksClientError> {
    let token_account_keypair = Keypair::new();
    let payer_keypair = context.borrow().payer.insecure_clone();
    let payer = payer_keypair.pubkey();

    let rent: Rent = context.borrow_mut().banks_client.get_rent().await.unwrap();
    // Token-2022 accounts with transfer fee need extra space
    let account_size = spl_token_2022::state::Account::LEN + 13;

    let create_account_ix = create_account(
        &payer,
        &token_account_keypair.pubkey(),
        rent.minimum_balance(account_size),
        account_size as u64,
        &spl_token_2022::id(),
    );

    let init_account_ix = spl_token_2022::instruction::initialize_account(
        &spl_token_2022::id(),
        &token_account_keypair.pubkey(),
        mint,
        owner,
    )
    .unwrap();

    send_tx_with_retry(
        Rc::clone(&context),
        &[create_account_ix, init_account_ix],
        Some(&payer),
        &[&payer_keypair, &token_account_keypair],
    )
    .await?;

    Ok(token_account_keypair)
}

/// Create a regular SPL token account.
/// Returns the token account keypair.
pub async fn create_spl_token_account(
    context: Rc<RefCell<ProgramTestContext>>,
    mint: &Pubkey,
    owner: &Pubkey,
) -> Result<Keypair, BanksClientError> {
    let token_account_keypair = Keypair::new();
    let payer_keypair = context.borrow().payer.insecure_clone();
    let payer = payer_keypair.pubkey();

    let rent: Rent = context.borrow_mut().banks_client.get_rent().await.unwrap();

    let create_account_ix = create_account(
        &payer,
        &token_account_keypair.pubkey(),
        rent.minimum_balance(spl_token::state::Account::LEN),
        spl_token::state::Account::LEN as u64,
        &spl_token::id(),
    );

    let init_account_ix = spl_token::instruction::initialize_account(
        &spl_token::id(),
        &token_account_keypair.pubkey(),
        mint,
        owner,
    )
    .unwrap();

    send_tx_with_retry(
        Rc::clone(&context),
        &[create_account_ix, init_account_ix],
        Some(&payer),
        &[&payer_keypair, &token_account_keypair],
    )
    .await?;

    Ok(token_account_keypair)
}

/// Mint Token-2022 tokens to a token account.
pub async fn mint_token_2022(
    context: Rc<RefCell<ProgramTestContext>>,
    mint: &Pubkey,
    token_account: &Pubkey,
    amount: u64,
) -> Result<(), BanksClientError> {
    let payer_keypair = context.borrow().payer.insecure_clone();
    let payer = payer_keypair.pubkey();

    let mint_to_ix = spl_token_2022::instruction::mint_to(
        &spl_token_2022::id(),
        mint,
        token_account,
        &payer,
        &[&payer],
        amount,
    )
    .unwrap();

    send_tx_with_retry(
        Rc::clone(&context),
        &[mint_to_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await
}

/// Expand a market to add more free blocks for orders.
/// Calls expand_market_instruction `count` times.
pub async fn expand_market(
    context: Rc<RefCell<ProgramTestContext>>,
    market: &Pubkey,
    count: u32,
) -> Result<(), BanksClientError> {
    use manifest::program::expand_market_instruction;

    let payer_keypair = context.borrow().payer.insecure_clone();
    let payer = payer_keypair.pubkey();

    for _ in 0..count {
        let expand_ix = expand_market_instruction(market, &payer);
        send_tx_with_retry(
            Rc::clone(&context),
            &[expand_ix],
            Some(&payer),
            &[&payer_keypair],
        )
        .await?;
    }

    Ok(())
}
