use std::{
    cell::{Ref, RefCell, RefMut},
    io::Error,
};

use manifest::{
    program::{create_market_instructions, get_dynamic_value},
    quantities::WrapperU64,
    state::{MarketFixed, MarketValue},
    validation::{get_market_address, MintAccountInfo},
};
use solana_program::{hash::Hash, pubkey::Pubkey, rent::Rent};
use solana_program_test::{processor, BanksClientError, ProgramTest, ProgramTestContext};
use solana_sdk::{
    account::Account, account_info::AccountInfo, instruction::Instruction, program_pack::Pack,
    signature::Keypair, signer::Signer, system_instruction::create_account,
    transaction::Transaction,
};
use spl_token_2022::state::Mint;
use std::rc::Rc;
use wrapper::instruction_builders::{
    claim_seat_instruction, create_wrapper_instructions, deposit_instruction, withdraw_instruction,
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
    pub second_keypair: Keypair,
}

impl TestFixture {
    pub async fn new() -> TestFixture {
        let mut program: ProgramTest = ProgramTest::new(
            "wrapper",
            wrapper::ID,
            processor!(wrapper::process_instruction),
        );
        program.add_program(
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

        let usdc_keypair: Keypair = Keypair::new();
        let sol_keypair: Keypair = Keypair::new();
        let wrapper_keypair: Keypair = Keypair::new();

        let context: Rc<RefCell<ProgramTestContext>> =
            Rc::new(RefCell::new(program.start_with_context().await));
        solana_logger::setup_with_default(RUST_LOG_DEFAULT);

        let usdc_mint_f: MintFixture =
            MintFixture::new(Rc::clone(&context), Some(usdc_keypair), Some(6)).await;
        let sol_mint_f: MintFixture =
            MintFixture::new(Rc::clone(&context), Some(sol_keypair), Some(9)).await;

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
            create_wrapper_instructions(&payer_pubkey, &wrapper_keypair.pubkey()).unwrap();
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

        TestFixture {
            context: Rc::clone(&context),
            usdc_mint: usdc_mint_f,
            sol_mint: sol_mint_f,
            market: market_fixture,
            wrapper: wrapper_fixture,
            payer_sol: payer_sol_fixture,
            payer_usdc: payer_usdc_fixture,
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

    pub async fn claim_seat(&self) -> anyhow::Result<(), BanksClientError> {
        self.claim_seat_for_keypair(&self.payer_keypair()).await
    }

    pub async fn claim_seat_for_keypair(
        &self,
        keypair: &Keypair,
    ) -> anyhow::Result<(), BanksClientError> {
        let wrapper_key: Pubkey = self.wrapper.key;
        self.claim_seat_for_keypair_with_wrapper(keypair, &wrapper_key)
            .await
    }

    pub async fn claim_seat_for_keypair_with_wrapper(
        &self,
        keypair: &Keypair,
        wrapper_state: &Pubkey,
    ) -> anyhow::Result<(), BanksClientError> {
        let claim_seat_ix: Instruction =
            claim_seat_instruction(&self.market.key, &keypair.pubkey(), wrapper_state);
        send_tx_with_retry(
            Rc::clone(&self.context),
            &[claim_seat_ix],
            Some(&keypair.pubkey()),
            &[&keypair.insecure_clone()],
        )
        .await
        .unwrap();
        Ok(())
    }

    pub async fn deposit(
        &mut self,
        token: Token,
        amount_atoms: u64,
    ) -> anyhow::Result<(), BanksClientError> {
        self.deposit_for_keypair(token, amount_atoms, &self.payer_keypair())
            .await?;
        Ok(())
    }

    pub async fn deposit_for_keypair(
        &mut self,
        token: Token,
        amount_atoms: u64,
        keypair: &Keypair,
    ) -> anyhow::Result<(), BanksClientError> {
        let wrapper_key: Pubkey = self.wrapper.key;
        self.deposit_for_keypair_with_wrapper(token, amount_atoms, keypair, &wrapper_key)
            .await
    }

    pub async fn deposit_for_keypair_with_wrapper(
        &mut self,
        token: Token,
        amount_atoms: u64,
        keypair: &Keypair,
        wrapper_state: &Pubkey,
    ) -> anyhow::Result<(), BanksClientError> {
        let is_base: bool = token == Token::SOL;
        let (mint, trader_token_account) = if is_base {
            let trader_token_account: Pubkey = if keypair.pubkey() == self.payer() {
                self.payer_sol.key
            } else {
                // Make a temporary token account
                let token_account_keypair: Keypair = Keypair::new();
                let token_account_fixture: TokenAccountFixture =
                    TokenAccountFixture::new_with_keypair(
                        Rc::clone(&self.context),
                        &self.sol_mint.key,
                        &keypair.pubkey(),
                        &token_account_keypair,
                    )
                    .await;
                token_account_fixture.key
            };
            self.sol_mint
                .mint_to(&trader_token_account, amount_atoms)
                .await;
            (&self.sol_mint.key, trader_token_account)
        } else {
            let trader_token_account: Pubkey = if keypair.pubkey() == self.payer() {
                self.payer_usdc.key
            } else {
                // Make a temporary token account
                let token_account_keypair: Keypair = Keypair::new();
                let token_account_fixture: TokenAccountFixture =
                    TokenAccountFixture::new_with_keypair(
                        Rc::clone(&self.context),
                        &self.usdc_mint.key,
                        &keypair.pubkey(),
                        &token_account_keypair,
                    )
                    .await;
                token_account_fixture.key
            };
            self.usdc_mint
                .mint_to(&trader_token_account, amount_atoms)
                .await;
            (&self.usdc_mint.key, trader_token_account)
        };

        let deposit_ix: Instruction = deposit_instruction(
            &self.market.key,
            &keypair.pubkey(),
            mint,
            amount_atoms,
            &trader_token_account,
            wrapper_state,
            spl_token::id(),
        );

        send_tx_with_retry(
            Rc::clone(&self.context),
            &[deposit_ix],
            Some(&keypair.pubkey()),
            &[&keypair.insecure_clone()],
        )
        .await
        .unwrap();

        Ok(())
    }

    pub async fn withdraw(
        &mut self,
        token: Token,
        amount_atoms: u64,
    ) -> anyhow::Result<(), BanksClientError> {
        self.withdraw_for_keypair(token, amount_atoms, &self.payer_keypair())
            .await?;
        Ok(())
    }

    pub async fn withdraw_for_keypair(
        &mut self,
        token: Token,
        amount_atoms: u64,
        keypair: &Keypair,
    ) -> anyhow::Result<(), BanksClientError> {
        let is_base: bool = token == Token::SOL;
        let (mint, trader_token_account) = if is_base {
            let trader_token_account: Pubkey = if keypair.pubkey() == self.payer() {
                self.payer_sol.key
            } else {
                // Make a new token account
                let token_account_keypair: Keypair = Keypair::new();
                let token_account_fixture: TokenAccountFixture =
                    TokenAccountFixture::new_with_keypair(
                        Rc::clone(&self.context),
                        &self.sol_mint.key,
                        &keypair.pubkey(),
                        &token_account_keypair,
                    )
                    .await;
                token_account_fixture.key
            };
            (&self.sol_mint.key, trader_token_account)
        } else {
            let trader_token_account: Pubkey = if keypair.pubkey() == self.payer() {
                self.payer_usdc.key
            } else {
                // Make a new token account
                let token_account_keypair: Keypair = Keypair::new();
                let token_account_fixture: TokenAccountFixture =
                    TokenAccountFixture::new_with_keypair(
                        Rc::clone(&self.context),
                        &self.usdc_mint.key,
                        &keypair.pubkey(),
                        &token_account_keypair,
                    )
                    .await;
                token_account_fixture.key
            };
            (&self.usdc_mint.key, trader_token_account)
        };

        let withdraw_ix: Instruction = withdraw_instruction(
            &self.market.key,
            &keypair.pubkey(),
            mint,
            amount_atoms,
            &trader_token_account,
            &self.wrapper.key,
            spl_token::id(),
        );

        send_tx_with_retry(
            Rc::clone(&self.context),
            &[withdraw_ix],
            Some(&keypair.pubkey()),
            &[&keypair.insecure_clone()],
        )
        .await
        .unwrap();
        Ok(())
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
}

impl WrapperFixture {
    pub async fn new(context: Rc<RefCell<ProgramTestContext>>, key: Pubkey) -> Self {
        let context_ref: Rc<RefCell<ProgramTestContext>> = Rc::clone(&context);
        WrapperFixture {
            context: context_ref,
            key,
        }
    }
}

#[derive(Clone)]
pub struct MintFixture {
    pub context: Rc<RefCell<ProgramTestContext>>,
    pub key: Pubkey,
}

impl MintFixture {
    pub async fn new(
        context: Rc<RefCell<ProgramTestContext>>,
        mint_keypair: Option<Keypair>,
        mint_decimals: Option<u8>,
    ) -> MintFixture {
        let payer_pubkey: Pubkey = context.borrow().payer.pubkey();
        let payer: Keypair = context.borrow().payer.insecure_clone();

        let mint_keypair: Keypair = mint_keypair.unwrap_or_else(Keypair::new);

        let rent: Rent = context.borrow_mut().banks_client.get_rent().await.unwrap();

        let init_account_ix: Instruction = create_account(
            &context.borrow().payer.pubkey(),
            &mint_keypair.pubkey(),
            rent.minimum_balance(spl_token::state::Mint::LEN),
            spl_token::state::Mint::LEN as u64,
            &spl_token::id(),
        );
        let init_mint_ix: Instruction = spl_token::instruction::initialize_mint(
            &spl_token::id(),
            &mint_keypair.pubkey(),
            &context.borrow().payer.pubkey(),
            None,
            mint_decimals.unwrap_or(6),
        )
        .unwrap();

        send_tx_with_retry(
            Rc::clone(&context),
            &[init_account_ix, init_mint_ix],
            Some(&payer_pubkey),
            &[&mint_keypair.insecure_clone(), &payer],
        )
        .await
        .unwrap();

        let context_ref: Rc<RefCell<ProgramTestContext>> = Rc::clone(&context);
        MintFixture {
            context: context_ref,
            key: mint_keypair.pubkey(),
        }
    }

    pub async fn mint_to(&mut self, dest: &Pubkey, native_amount: u64) {
        let payer_keypair: Keypair = self.context.borrow().payer.insecure_clone();
        let mint_to_ix: Instruction = self.make_mint_to_ix(dest, native_amount);
        send_tx_with_retry(
            Rc::clone(&self.context),
            &[mint_to_ix],
            Some(&payer_keypair.pubkey()),
            &[&payer_keypair],
        )
        .await
        .unwrap();
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
}

pub struct TokenAccountFixture {
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

    pub async fn new_with_keypair(
        context: Rc<RefCell<ProgramTestContext>>,
        mint_pk: &Pubkey,
        owner_pk: &Pubkey,
        keypair: &Keypair,
    ) -> Self {
        let rent: Rent = context.borrow_mut().banks_client.get_rent().await.unwrap();
        let instructions: [Instruction; 2] = Self::create_ixs(
            rent,
            mint_pk,
            &context.borrow().payer.pubkey(),
            owner_pk,
            keypair,
        )
        .await;

        let payer_keypair: Keypair = context.borrow().payer.insecure_clone();
        send_tx_with_retry(
            Rc::clone(&context),
            &instructions,
            Some(&payer_keypair.pubkey()),
            &[&payer_keypair, keypair],
        )
        .await
        .unwrap();

        Self {
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
}

pub(crate) async fn send_tx_with_retry(
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
