use std::{
    cell::{Ref, RefCell, RefMut},
    io::Error,
};

use hypertree::trace;
use manifest::{
    program::{create_global_instruction, create_market_instructions, get_dynamic_value},
    quantities::WrapperU64,
    state::{GlobalFixed, GlobalValue, MarketFixed, MarketValue},
    validation::{get_global_address, get_market_address, MintAccountInfo},
};
use solana_program::{hash::Hash, pubkey::Pubkey, rent::Rent};
use solana_program_test::{processor, BanksClientError, ProgramTest, ProgramTestContext};
use solana_sdk::{
    account::Account, account_info::AccountInfo, instruction::Instruction, program_pack::Pack,
    signature::Keypair, signer::Signer, system_instruction::create_account,
    transaction::Transaction,
};
use spl_token_2022::{
    extension::{
        transfer_fee::instruction::initialize_transfer_fee_config, transfer_hook,
        BaseStateWithExtensions, ExtensionType, StateWithExtensions,
    },
    state::Mint,
};
use std::rc::Rc;
use ui_wrapper::{
    instruction_builders::create_wrapper_instructions,
    wrapper_user::{ManifestWrapperUserFixed, WrapperUserValue},
};

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

const RUST_LOG_DEFAULT: &str = "solana_rbpf::vm=info,\
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
    pub sol_mint: MintFixture,
    pub usdc_mint: MintFixture,
    pub payer_sol: TokenAccountFixture,
    pub payer_usdc: TokenAccountFixture,
    pub market: MarketFixture,
    pub wrapper: WrapperFixture,
    pub global_fixture: GlobalFixture,
    pub sol_global_fixture: GlobalFixture,
    pub second_keypair: Keypair,
}

impl TestFixture {
    pub async fn new() -> TestFixture {
        let mut program: ProgramTest = ProgramTest::new(
            "ui_wrapper",
            ui_wrapper::ID,
            processor!(ui_wrapper::process_instruction),
        );
        // needed extra cu for logs and traces
        program.set_compute_max_units(600_000);
        program.add_program(
            "manifest",
            manifest::ID,
            processor!(manifest::process_instruction),
        );

        let second_keypair: Keypair = Keypair::new();
        program.add_account(
            second_keypair.pubkey(),
            solana_sdk::account::Account::new(SOL_UNIT_SIZE, 0, &solana_sdk::system_program::id()),
        );

        let wrapper_keypair: Keypair = Keypair::new();

        let context: Rc<RefCell<ProgramTestContext>> =
            Rc::new(RefCell::new(program.start_with_context().await));
        solana_logger::setup_with_default(RUST_LOG_DEFAULT);

        let usdc_mint_f: MintFixture = MintFixture::new(Rc::clone(&context), Some(6)).await;
        let sol_mint_f: MintFixture = MintFixture::new(Rc::clone(&context), Some(9)).await;

        let payer_pubkey: Pubkey = context.borrow().payer.pubkey();
        let payer: Keypair = context.borrow().payer.insecure_clone();
        let (market_key, _) = get_market_address(0, &usdc_mint_f.key);
        let create_market_ixs: Vec<Instruction> =
            create_market_instructions(0, 9, &usdc_mint_f.key, &payer_pubkey, 1000, 500, Pubkey::default());

        send_tx_with_retry(
            Rc::clone(&context),
            &create_market_ixs[..],
            Some(&payer_pubkey),
            &[&payer.insecure_clone()],
        )
        .await
        .unwrap();

        // Now that market is created, we can make a market fixture.
        let market_fixture: MarketFixture =
            MarketFixture::new(Rc::clone(&context), market_key).await;

        let create_wrapper_ixs: Vec<Instruction> =
            create_wrapper_instructions(&payer_pubkey, &payer_pubkey, &wrapper_keypair.pubkey())
                .unwrap();
        send_tx_with_retry(
            Rc::clone(&context),
            &create_wrapper_ixs[..],
            Some(&payer_pubkey),
            &[&payer.insecure_clone(), &wrapper_keypair],
        )
        .await
        .unwrap();

        let wrapper_fixture: WrapperFixture =
            WrapperFixture::new(Rc::clone(&context), wrapper_keypair.pubkey()).await;

        let payer_sol_fixture: TokenAccountFixture =
            TokenAccountFixture::new(Rc::clone(&context), &sol_mint_f.key, &payer_pubkey).await;
        let payer_usdc_fixture =
            TokenAccountFixture::new(Rc::clone(&context), &usdc_mint_f.key, &payer_pubkey).await;

        let global_fixture: GlobalFixture =
            GlobalFixture::new(Rc::clone(&context), &usdc_mint_f.key).await;
        let sol_global_fixture: GlobalFixture =
            GlobalFixture::new(Rc::clone(&context), &sol_mint_f.key).await;

        TestFixture {
            context: Rc::clone(&context),
            usdc_mint: usdc_mint_f,
            sol_mint: sol_mint_f,
            market: market_fixture,
            wrapper: wrapper_fixture,
            payer_sol: payer_sol_fixture,
            payer_usdc: payer_usdc_fixture,
            global_fixture,
            sol_global_fixture,
            second_keypair,
        }
    }

    pub async fn new_with_extensions(transfer_fee: bool, transfer_hook: bool) -> TestFixture {
        let mut program: ProgramTest = ProgramTest::new(
            "ui_wrapper",
            ui_wrapper::ID,
            processor!(ui_wrapper::process_instruction),
        );
        // needed extra cu for logs and traces
        program.set_compute_max_units(600_000);
        program.add_program(
            "manifest",
            manifest::ID,
            processor!(manifest::process_instruction),
        );

        let second_keypair: Keypair = Keypair::new();
        program.add_account(
            second_keypair.pubkey(),
            solana_sdk::account::Account::new(SOL_UNIT_SIZE, 0, &solana_sdk::system_program::id()),
        );

        let wrapper_keypair: Keypair = Keypair::new();

        let context: Rc<RefCell<ProgramTestContext>> =
            Rc::new(RefCell::new(program.start_with_context().await));
        solana_logger::setup_with_default(RUST_LOG_DEFAULT);

        let usdc_mint_f: MintFixture = MintFixture::new_with_version(
            Rc::clone(&context),
            Some(6),
            true,
            transfer_fee,
            transfer_hook,
        )
        .await;
        let sol_mint_f: MintFixture = MintFixture::new(Rc::clone(&context), Some(9)).await;

        let payer_pubkey: Pubkey = context.borrow().payer.pubkey();
        let payer: Keypair = context.borrow().payer.insecure_clone();
        let (market_key, _) = get_market_address(0, &usdc_mint_f.key);
        let create_market_ixs: Vec<Instruction> =
            create_market_instructions(0, 9, &usdc_mint_f.key, &payer_pubkey, 1000, 500, Pubkey::default());

        send_tx_with_retry(
            Rc::clone(&context),
            &create_market_ixs[..],
            Some(&payer_pubkey),
            &[&payer.insecure_clone()],
        )
        .await
        .unwrap();

        // Now that market is created, we can make a market fixture.
        let market_fixture: MarketFixture =
            MarketFixture::new(Rc::clone(&context), market_key).await;

        let create_wrapper_ixs: Vec<Instruction> =
            create_wrapper_instructions(&payer_pubkey, &payer_pubkey, &wrapper_keypair.pubkey())
                .unwrap();
        send_tx_with_retry(
            Rc::clone(&context),
            &create_wrapper_ixs[..],
            Some(&payer_pubkey),
            &[&payer.insecure_clone(), &wrapper_keypair],
        )
        .await
        .unwrap();

        let wrapper_fixture: WrapperFixture =
            WrapperFixture::new(Rc::clone(&context), wrapper_keypair.pubkey()).await;

        let payer_sol_fixture: TokenAccountFixture =
            TokenAccountFixture::new(Rc::clone(&context), &sol_mint_f.key, &payer_pubkey).await;
        let payer_usdc_fixture =
            TokenAccountFixture::new_2022(Rc::clone(&context), &usdc_mint_f.key, &payer_pubkey)
                .await;

        let global_fixture: GlobalFixture = GlobalFixture::new_with_token_program(
            Rc::clone(&context),
            &usdc_mint_f.key,
            &spl_token_2022::id(),
        )
        .await;
        let sol_global_fixture: GlobalFixture =
            GlobalFixture::new(Rc::clone(&context), &sol_mint_f.key).await;

        TestFixture {
            context: Rc::clone(&context),
            usdc_mint: usdc_mint_f,
            sol_mint: sol_mint_f,
            market: market_fixture,
            wrapper: wrapper_fixture,
            payer_sol: payer_sol_fixture,
            payer_usdc: payer_usdc_fixture,
            global_fixture,
            sol_global_fixture,
            second_keypair,
        }
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

    pub async fn fund_token_account(&self, mint_pk: &Pubkey, owner_pk: &Pubkey) -> Pubkey {
        let token_account_keypair: Keypair = Keypair::new();
        let token_account_fixture: TokenAccountFixture = TokenAccountFixture::new_with_keypair(
            Rc::clone(&self.context),
            mint_pk,
            owner_pk,
            &token_account_keypair,
        )
        .await;
        token_account_fixture.key
    }

    pub async fn fund_token_account_2022(&self, mint_pk: &Pubkey, owner_pk: &Pubkey) -> Pubkey {
        let token_account_keypair: Keypair = Keypair::new();
        let token_account_fixture: TokenAccountFixture =
            TokenAccountFixture::new_with_keypair_2022(
                Rc::clone(&self.context),
                mint_pk,
                owner_pk,
                &token_account_keypair,
            )
            .await;
        token_account_fixture.key
    }
    /// returns (mint, trader_token_account)
    pub async fn fund_trader_wallet(
        &mut self,
        keypair: &Keypair,
        token: Token,
        amount_atoms: u64,
    ) -> (Pubkey, Pubkey) {
        let is_base: bool = token == Token::SOL;
        trace!(
            "fund_trader_wallet {} {amount_atoms}",
            if is_base { "SOL" } else { "USDC" }
        );
        let (mint, trader_token_account) = if is_base {
            let trader_token_account: Pubkey = if keypair.pubkey() == self.payer() {
                self.payer_sol.key
            } else {
                // Make a temporary token account
                self.fund_token_account(&self.sol_mint.key, &keypair.pubkey())
                    .await
            };
            self.sol_mint
                .mint_to(&trader_token_account, amount_atoms)
                .await;
            (self.sol_mint.key.clone(), trader_token_account)
        } else {
            let trader_token_account: Pubkey = if keypair.pubkey() == self.payer() {
                self.payer_usdc.key
            } else {
                // Make a temporary token account

                self.fund_token_account(&self.usdc_mint.key, &keypair.pubkey())
                    .await
            };
            self.usdc_mint
                .mint_to(&trader_token_account, amount_atoms)
                .await;

            (self.usdc_mint.key.clone(), trader_token_account)
        };

        (mint, trader_token_account)
    }

    pub async fn fund_trader_wallet_2022(
        &mut self,
        keypair: &Keypair,
        token: Token,
        amount_atoms: u64,
    ) -> (Pubkey, Pubkey) {
        let is_base: bool = token == Token::SOL;
        trace!(
            "fund_trader_wallet_2022 {} {amount_atoms}",
            if is_base { "SOL" } else { "USDC" }
        );
        let (mint, trader_token_account) = if is_base {
            let trader_token_account: Pubkey = if keypair.pubkey() == self.payer() {
                self.payer_sol.key
            } else {
                // Make a temporary token account
                self.fund_token_account_2022(&self.sol_mint.key, &keypair.pubkey())
                    .await
            };
            self.sol_mint
                .mint_to_2022(&trader_token_account, amount_atoms)
                .await;
            (self.sol_mint.key.clone(), trader_token_account)
        } else {
            let trader_token_account: Pubkey = if keypair.pubkey() == self.payer() {
                self.payer_usdc.key
            } else {
                // Make a temporary token account
                self.fund_token_account_2022(&self.usdc_mint.key, &keypair.pubkey())
                    .await
            };
            self.usdc_mint
                .mint_to_2022(&trader_token_account, amount_atoms)
                .await;
            (self.usdc_mint.key.clone(), trader_token_account)
        };

        (mint, trader_token_account)
    }
}

#[derive(Clone)]
pub struct MarketFixture {
    pub context: Rc<RefCell<ProgramTestContext>>,
    pub key: Pubkey,
    pub market: MarketValue,
}

impl MarketFixture {
    pub async fn new(context: Rc<RefCell<ProgramTestContext>>, key: Pubkey) -> Self {
        let context_ref: Rc<RefCell<ProgramTestContext>> = Rc::clone(&context);

        // Just needed for storing the decimals. The rest can be blank.
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
            key,
            market: MarketValue {
                fixed: MarketFixed::new_empty(&base_mint, &quote_mint, &key),
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
}

#[derive(Clone)]
pub struct WrapperFixture {
    pub context: Rc<RefCell<ProgramTestContext>>,
    pub key: Pubkey,
    pub wrapper: WrapperUserValue,
}

impl WrapperFixture {
    pub async fn new(context: Rc<RefCell<ProgramTestContext>>, key: Pubkey) -> Self {
        let context_ref: Rc<RefCell<ProgramTestContext>> = Rc::clone(&context);
        WrapperFixture {
            context: context_ref,
            key,
            wrapper: WrapperUserValue {
                fixed: ManifestWrapperUserFixed::new_empty(&key),
                dynamic: Vec::new(),
            },
        }
    }

    pub async fn reload(&mut self) {
        let wrapper_account: Account = self
            .context
            .borrow_mut()
            .banks_client
            .get_account(self.key)
            .await
            .unwrap()
            .unwrap();

        let wrapper: WrapperUserValue = get_dynamic_value(wrapper_account.data.as_slice());
        self.wrapper = wrapper;
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
    pub is_22: bool,
    pub vanilla_mint: Option<spl_token::state::Mint>,
    pub extension_mint: Option<spl_token_2022::state::Mint>,
}

impl MintFixture {
    pub async fn new(
        context: Rc<RefCell<ProgramTestContext>>,
        mint_decimals_opt: Option<u8>,
    ) -> MintFixture {
        // Defaults to not 22.
        MintFixture::new_with_version(context, mint_decimals_opt, false, false, false).await
    }

    pub async fn new_with_version(
        context: Rc<RefCell<ProgramTestContext>>,
        mint_decimals_opt: Option<u8>,
        is_22: bool,
        transfer_fee: bool,
        transfer_hook: bool,
    ) -> MintFixture {
        let context_ref: Rc<RefCell<ProgramTestContext>> = Rc::clone(&context);
        let mint_keypair: Keypair = Keypair::new();
        let payer: Keypair = context.borrow().payer.insecure_clone();
        let rent: Rent = context.borrow_mut().banks_client.get_rent().await.unwrap();
        let space = if is_22 {
            let mut extensions = Vec::new();
            if transfer_fee {
                extensions.push(ExtensionType::TransferFeeConfig);
            }
            if transfer_hook {
                extensions.push(ExtensionType::TransferHook);
            }
            ExtensionType::try_calculate_account_len::<Mint>(&extensions).unwrap()
        } else {
            spl_token::state::Mint::LEN
        };

        let mut instructions = Vec::new();

        instructions.push(create_account(
            &payer.pubkey(),
            &mint_keypair.pubkey(),
            rent.minimum_balance(space),
            space as u64,
            &{
                if is_22 {
                    spl_token_2022::id()
                } else {
                    spl_token::id()
                }
            },
        ));
        if is_22 {
            if transfer_fee {
                instructions.push(
                    initialize_transfer_fee_config(
                        &spl_token_2022::id(),
                        &mint_keypair.pubkey(),
                        None,
                        None,
                        100,
                        100,
                    )
                    .unwrap(),
                );
            }
            if transfer_hook {
                instructions.push(
                    transfer_hook::instruction::initialize(
                        &spl_token_2022::id(),
                        &mint_keypair.pubkey(),
                        Some(payer.pubkey()),
                        None,
                    )
                    .unwrap(),
                );
            }
            instructions.push(
                spl_token_2022::instruction::initialize_mint(
                    &spl_token_2022::id(),
                    &mint_keypair.pubkey(),
                    &payer.pubkey(),
                    None,
                    mint_decimals_opt.unwrap_or(6),
                )
                .unwrap(),
            );
        } else {
            instructions.push(
                spl_token::instruction::initialize_mint(
                    &spl_token::id(),
                    &mint_keypair.pubkey(),
                    &payer.pubkey(),
                    None,
                    mint_decimals_opt.unwrap_or(6),
                )
                .unwrap(),
            );
        };

        send_tx_with_retry(
            Rc::clone(&context),
            &instructions,
            Some(&payer.pubkey()),
            &[&payer, &mint_keypair],
        )
        .await
        .unwrap();

        let mut result = MintFixture {
            context: context_ref,
            key: mint_keypair.pubkey(),
            is_22,
            vanilla_mint: None,
            extension_mint: None,
        };
        result.reload().await;
        result
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

        if self.is_22 {
            self.extension_mint = Some(
                StateWithExtensions::<Mint>::unpack(&mut mint_account.data.clone().as_slice())
                    .unwrap()
                    .base,
            )
        } else {
            self.vanilla_mint = Some(
                spl_token::state::Mint::unpack_unchecked(&mut mint_account.data.as_slice())
                    .unwrap(),
            );
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
        space: usize,
    ) -> [Instruction; 2] {
        let init_account_ix: Instruction = create_account(
            payer_pk,
            &keypair.pubkey(),
            rent.minimum_balance(space),
            space as u64,
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
        let mint_account = context
            .borrow_mut()
            .banks_client
            .get_account(*mint_pk)
            .await
            .unwrap()
            .unwrap();

        let mut mint_account_data = mint_account.data.clone();
        let mint_with_extensions =
            StateWithExtensions::<Mint>::unpack(mint_account_data.as_mut_slice()).unwrap();
        let mint_extensions = mint_with_extensions.get_extension_types().unwrap();
        let account_extensions =
            ExtensionType::get_required_init_account_extensions(&mint_extensions);
        let space = ExtensionType::try_calculate_account_len::<spl_token_2022::state::Account>(
            &account_extensions,
        )
        .unwrap();

        let instructions: [Instruction; 2] =
            Self::create_ixs_2022(rent, mint_pk, &payer, owner_pk, keypair, space).await;

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

    pub async fn new(
        context: Rc<RefCell<ProgramTestContext>>,
        mint_pk: &Pubkey,
        owner_pk: &Pubkey,
    ) -> TokenAccountFixture {
        let keypair: Keypair = Keypair::new();
        TokenAccountFixture::new_with_keypair(context, mint_pk, owner_pk, &keypair).await
    }

    pub async fn new_2022(
        context: Rc<RefCell<ProgramTestContext>>,
        mint_pk: &Pubkey,
        owner_pk: &Pubkey,
    ) -> TokenAccountFixture {
        let keypair: Keypair = Keypair::new();
        TokenAccountFixture::new_with_keypair_2022(context, mint_pk, owner_pk, &keypair).await
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
