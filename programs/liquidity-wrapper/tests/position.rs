use liquidity_wrapper::{
    instruction::{
        cancel_orders_instruction, close_position_instruction, create_position_instruction,
        place_orders_instruction, TradingAccounts,
    },
    processor::{
        ERROR_NOT_NEAREST_FIRST, ERROR_ORDER_CROSSED, ERROR_UNSUPPORTED_MINT, MAX_ORDERS_PER_PLACE,
    },
    state::{
        get_position_address, get_trader_address, placed_bitmap_len, Ladder, Position, Price,
        POSITION_DISCRIMINANT, POSITION_HEADER_SIZE,
    },
};
use manifest::{
    deps::hypertree::HyperTreeValueIteratorTrait,
    program::{
        batch_update::PlaceOrderParams, batch_update_instruction, claim_seat_instruction,
        create_global_instruction, create_market_instructions, defrag_instruction,
        deposit_instruction, get_dynamic_value_or, global_add_trader_instruction,
        global_deposit_instruction, global_withdraw_instruction,
    },
    quantities::WrapperU64,
    state::{MarketValue, OrderType, RestingOrder},
    validation::get_global_address,
};
use solana_clock::Clock;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_program::pubkey::Pubkey;
use solana_program_test::{BanksClientError, ProgramTest, ProgramTestContext};
use solana_signer::Signer;
use solana_system_interface::instruction as system_instruction;
use solana_transaction::Transaction;
use spl_token_2022::{
    extension::{
        transfer_fee::instruction::initialize_transfer_fee_config, BaseStateWithExtensions,
        ExtensionType, StateWithExtensions,
    },
    state::{Account, Mint},
};

const BASE_DECIMALS: u8 = 8;
const QUOTE_DECIMALS: u8 = 6;
/// 50 quote tokens per base token.
const PRICE_E9: Price = e9(50_000_000_000);

/// A price given in billionths of a quote token per base token, without the
/// trailing zeros a u32 mantissa has no room for.
const fn e9(mut price: u64) -> Price {
    let mut exponent = -9;
    while price > u32::MAX as u64 {
        assert!(price % 10 == 0, "more digits than a u32 holds");
        price /= 10;
        exponent += 1;
    }
    Price::new(price as u32, exponent)
}
const WALLET_BASE: u64 = 100 * 100_000_000;
const WALLET_QUOTE: u64 = 10_000 * 1_000_000;

type BandSpec = (i16, i16, u16, u16, u64);

/// Ten bids from -5% to -0.1% at 5 bps (ReverseTight) and ten asks from
/// +0.1% to +5% at 20 bps (Reverse), 10 quote tokens each.
const BANDS: &[BandSpec] = &[
    (-500, -10, 10, 50, 10_000_000),
    (10, 500, 10, 200, 10_000_000),
];
const TOTAL: usize = 20;

fn ladder(bands: &[BandSpec]) -> Vec<u8> {
    let mut b = vec![2, bands.len() as u8, 0, 0];
    for &(lo, hi, levels, spread, size) in bands {
        b.extend_from_slice(&lo.to_le_bytes());
        b.extend_from_slice(&hi.to_le_bytes());
        b.extend_from_slice(&levels.to_le_bytes());
        b.extend_from_slice(&spread.to_le_bytes());
        b.extend_from_slice(&size.to_le_bytes());
    }
    b
}

struct Fixture {
    ctx: ProgramTestContext,
    owner: Keypair,
    market: Pubkey,
    base_mint: Pubkey,
    quote_mint: Pubkey,
    base_program: Pubkey,
    owner_base: Pubkey,
    owner_quote: Pubkey,
}

struct Opened {
    position: Pubkey,
    trader: Pubkey,
    accounts: TradingAccounts,
}

async fn setup() -> Fixture {
    setup_with(Base::Token).await
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Base {
    Token,
    /// Token-2022 without extensions.
    Token2022,
    /// Token-2022 with the extensions of a tokenised stock (xStocks, Ondo,
    /// Backpack): a transfer hook extension without a program, accounts
    /// initialised by default, a permanent delegate, a scaled UI amount and
    /// pausing.
    Stock,
    /// Token-2022 with a transfer fee config, at zero.
    TransferFee,
    /// Token-2022 with a transfer hook program.
    Hooked,
    /// Token-2022 whose accounts start frozen.
    FrozenByDefault,
    /// Token-2022, non-transferable.
    NonTransferable,
}

/// A fresh Manifest market with a base of 8 decimals (Token-2022 when
/// `base` says so) and a 6-decimal quote, and an owner
/// wallet holding both.
async fn setup_with(base: Base) -> Fixture {
    let mut program = ProgramTest::default();
    program.prefer_bpf(true);
    program.add_program("liquidity_wrapper", liquidity_wrapper::ID, None);
    program.add_program("manifest", manifest::ID, None);
    program.set_compute_max_units(1_400_000);
    let mut ctx = program.start_with_context().await;
    let owner = ctx.payer.insecure_clone();
    let base_program = if base != Base::Token {
        spl_token_2022::id()
    } else {
        spl_token::id()
    };
    let base_mint = create_mint(&mut ctx, &base_program, BASE_DECIMALS, base).await;
    let quote_mint = create_mint(&mut ctx, &spl_token::id(), QUOTE_DECIMALS, Base::Token).await;

    let market = Keypair::new();
    let ixs =
        create_market_instructions(&market.pubkey(), &base_mint, &quote_mint, &owner.pubkey())
            .unwrap();
    send(&mut ctx, &ixs, &[&market]).await.unwrap();

    let owner_base = token_account(&mut ctx, &base_program, &base_mint, &owner.pubkey()).await;
    let owner_quote = token_account(&mut ctx, &spl_token::id(), &quote_mint, &owner.pubkey()).await;
    // A frozen account takes nothing.
    if base != Base::FrozenByDefault {
        mint_to(
            &mut ctx,
            &base_program,
            &base_mint,
            &owner_base,
            WALLET_BASE,
        )
        .await;
    }
    mint_to(
        &mut ctx,
        &spl_token::id(),
        &quote_mint,
        &owner_quote,
        WALLET_QUOTE,
    )
    .await;
    Fixture {
        ctx,
        owner,
        market: market.pubkey(),
        base_mint,
        quote_mint,
        base_program,
        owner_base,
        owner_quote,
    }
}

async fn create_mint(
    ctx: &mut ProgramTestContext,
    program: &Pubkey,
    decimals: u8,
    base: Base,
) -> Pubkey {
    use spl_token_2022::{
        extension::{default_account_state, pausable, scaled_ui_amount, transfer_hook},
        instruction::{initialize_non_transferable_mint, initialize_permanent_delegate},
        state::AccountState,
    };
    let mint = Keypair::new();
    let rent = ctx.banks_client.get_rent().await.unwrap();
    let payer = ctx.payer.pubkey();
    let m = &mint.pubkey();
    let (extensions, init): (Vec<ExtensionType>, Vec<Instruction>) = match base {
        Base::Token | Base::Token2022 => (vec![], vec![]),
        Base::Stock => (
            vec![
                ExtensionType::TransferHook,
                ExtensionType::DefaultAccountState,
                ExtensionType::PermanentDelegate,
                ExtensionType::ScaledUiAmount,
                ExtensionType::Pausable,
            ],
            vec![
                transfer_hook::instruction::initialize(program, m, Some(payer), None).unwrap(),
                default_account_state::instruction::initialize_default_account_state(
                    program,
                    m,
                    &AccountState::Initialized,
                )
                .unwrap(),
                initialize_permanent_delegate(program, m, &payer).unwrap(),
                scaled_ui_amount::instruction::initialize(program, m, Some(payer), 1.0).unwrap(),
                pausable::instruction::initialize(program, m, &payer).unwrap(),
            ],
        ),
        Base::TransferFee => (
            vec![ExtensionType::TransferFeeConfig],
            vec![initialize_transfer_fee_config(program, m, Some(&payer), None, 0, 0).unwrap()],
        ),
        Base::Hooked => (
            vec![ExtensionType::TransferHook],
            vec![transfer_hook::instruction::initialize(
                program,
                m,
                Some(payer),
                Some(Pubkey::new_unique()),
            )
            .unwrap()],
        ),
        Base::FrozenByDefault => (
            vec![ExtensionType::DefaultAccountState],
            vec![
                default_account_state::instruction::initialize_default_account_state(
                    program,
                    m,
                    &AccountState::Frozen,
                )
                .unwrap(),
            ],
        ),
        Base::NonTransferable => (
            vec![ExtensionType::NonTransferable],
            vec![initialize_non_transferable_mint(program, m).unwrap()],
        ),
    };
    let len = ExtensionType::try_calculate_account_len::<Mint>(&extensions).unwrap();
    let mut ixs = vec![system_instruction::create_account(
        &payer,
        m,
        rent.minimum_balance(len),
        len as u64,
        program,
    )];
    ixs.extend(init);
    ixs.push(
        // A default account state needs a freeze authority.
        spl_token_2022::instruction::initialize_mint(program, m, &payer, Some(&payer), decimals)
            .unwrap(),
    );
    send(ctx, &ixs, &[&mint]).await.unwrap();
    mint.pubkey()
}

async fn token_account(
    ctx: &mut ProgramTestContext,
    program: &Pubkey,
    mint: &Pubkey,
    owner: &Pubkey,
) -> Pubkey {
    let account = Keypair::new();
    let rent = ctx.banks_client.get_rent().await.unwrap();
    // Whatever extensions the mint asks of its accounts.
    let mint_data = ctx
        .banks_client
        .get_account(*mint)
        .await
        .unwrap()
        .unwrap()
        .data;
    let required = ExtensionType::get_required_init_account_extensions(
        &StateWithExtensions::<Mint>::unpack(&mint_data)
            .unwrap()
            .get_extension_types()
            .unwrap(),
    );
    let len = ExtensionType::try_calculate_account_len::<Account>(&required).unwrap();
    let ixs = [
        system_instruction::create_account(
            &ctx.payer.pubkey(),
            &account.pubkey(),
            rent.minimum_balance(len),
            len as u64,
            program,
        ),
        spl_token_2022::instruction::initialize_account(program, &account.pubkey(), mint, owner)
            .unwrap(),
    ];
    send(ctx, &ixs, &[&account]).await.unwrap();
    account.pubkey()
}

async fn mint_to(
    ctx: &mut ProgramTestContext,
    program: &Pubkey,
    mint: &Pubkey,
    to: &Pubkey,
    amount: u64,
) {
    let ix =
        spl_token_2022::instruction::mint_to(program, mint, to, &ctx.payer.pubkey(), &[], amount)
            .unwrap();
    send(ctx, &[ix], &[]).await.unwrap();
}

async fn now(ctx: &mut ProgramTestContext) -> u32 {
    ctx.banks_client
        .get_sysvar::<Clock>()
        .await
        .unwrap()
        .unix_timestamp as u32
}

async fn send(
    ctx: &mut ProgramTestContext,
    ixs: &[Instruction],
    signers: &[&Keypair],
) -> Result<(), BanksClientError> {
    let blockhash = ctx.get_new_latest_blockhash().await.unwrap();
    let mut all: Vec<&Keypair> = vec![&ctx.payer];
    all.extend_from_slice(signers);
    let tx = Transaction::new_signed_with_payer(ixs, Some(&ctx.payer.pubkey()), &all, blockhash);
    ctx.banks_client.process_transaction(tx).await
}

/// Compute units the transaction takes, then sends it.
async fn measure(ctx: &mut ProgramTestContext, ixs: &[Instruction]) -> u64 {
    let blockhash = ctx.get_new_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        ixs,
        Some(&ctx.payer.pubkey()),
        &[&ctx.payer],
        blockhash,
    );
    let sim = ctx.banks_client.simulate_transaction(tx).await.unwrap();
    if let Some(Err(e)) = sim.result {
        panic!("{e:?}");
    }
    let units = sim.simulation_details.unwrap().units_consumed;
    send(ctx, ixs, &[]).await.unwrap();
    units
}

async fn lamports(ctx: &mut ProgramTestContext, address: &Pubkey) -> u64 {
    ctx.banks_client
        .get_account(*address)
        .await
        .unwrap()
        .map_or(0, |a| a.lamports)
}

async fn token_balance(ctx: &mut ProgramTestContext, address: &Pubkey) -> u64 {
    let account = ctx
        .banks_client
        .get_account(*address)
        .await
        .unwrap()
        .unwrap();
    u64::from_le_bytes(account.data[64..72].try_into().unwrap())
}

async fn load(ctx: &mut ProgramTestContext, address: &Pubkey) -> (Position, Vec<u8>) {
    let account = ctx
        .banks_client
        .get_account(*address)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(account.owner, liquidity_wrapper::ID);
    let header: Position = *bytemuck::from_bytes(&account.data[..POSITION_HEADER_SIZE]);
    (header, account.data[POSITION_HEADER_SIZE..].to_vec())
}

/// Orders the position's bitmap marks placed; `rest` is what follows the
/// header, as `load` returns it.
fn placed_count(p: &Position, rest: &[u8]) -> u32 {
    rest[p.ladder_len as usize..]
        .iter()
        .map(|b| b.count_ones())
        .sum()
}

async fn market(ctx: &mut ProgramTestContext, address: &Pubkey) -> MarketValue {
    let account = ctx
        .banks_client
        .get_account(*address)
        .await
        .unwrap()
        .unwrap();
    get_dynamic_value_or(account.data.as_slice()).unwrap()
}

/// The trader's resting orders.
async fn orders_of(f: &mut Fixture, trader: &Pubkey) -> Vec<RestingOrder> {
    let m = market(&mut f.ctx, &f.market).await;
    let index = m.get_trader_index(trader);
    m.get_bids()
        .iter::<RestingOrder>()
        .chain(m.get_asks().iter::<RestingOrder>())
        .filter(|(_, o)| o.get_trader_index() == index)
        .map(|(_, o)| *o)
        .collect()
}

/// Market indices of the trader's resting orders, ascending, as the app
/// reads them for CancelOrders.
async fn order_indices(f: &mut Fixture, trader: &Pubkey) -> Vec<u32> {
    let m = market(&mut f.ctx, &f.market).await;
    let index = m.get_trader_index(trader);
    let mut out: Vec<u32> = m
        .get_bids()
        .iter::<RestingOrder>()
        .chain(m.get_asks().iter::<RestingOrder>())
        .filter(|(_, o)| o.get_trader_index() == index)
        .map(|(i, _)| i)
        .collect();
    out.sort();
    out
}

impl Fixture {
    async fn open(&mut self, bands: &[BandSpec]) -> Opened {
        let created_at = now(&mut self.ctx).await;
        let owner = self.owner.pubkey();
        let ix = create_position_instruction(
            &owner,
            &self.market,
            &self.base_mint,
            &self.quote_mint,
            created_at,
            PRICE_E9,
            &ladder(bands),
        );
        send(&mut self.ctx, &[ix], &[]).await.unwrap();
        self.trading(&owner, created_at, self.owner_base, self.owner_quote)
            .await
    }

    /// The accounts for (owner, created_at), creating the trader's token
    /// accounts as the app does before its first placement.
    async fn trading(
        &mut self,
        owner: &Pubkey,
        created_at: u32,
        owner_base: Pubkey,
        owner_quote: Pubkey,
    ) -> Opened {
        let (position, _) = get_position_address(owner, &self.market, created_at);
        let (trader, _) = get_trader_address(&position);
        let base_program = self.base_program;
        let (base_mint, quote_mint) = (self.base_mint, self.quote_mint);
        let trader_base = token_account(&mut self.ctx, &base_program, &base_mint, &trader).await;
        let trader_quote =
            token_account(&mut self.ctx, &spl_token::id(), &quote_mint, &trader).await;
        Opened {
            position,
            trader,
            accounts: TradingAccounts {
                owner: *owner,
                position,
                market: self.market,
                base_mint,
                quote_mint,
                owner_base,
                owner_quote,
                trader_base,
                trader_quote,
                base_token_program: base_program,
                quote_token_program: spl_token::id(),
            },
        }
    }

    async fn place(&mut self, o: &Opened, orders: &[u16]) -> Result<(), BanksClientError> {
        send(
            &mut self.ctx,
            &[place_orders_instruction(&o.accounts, orders)],
            &[],
        )
        .await
    }

    /// ClosePosition with the indices of every order the seat holds, read
    /// as the app reads them.
    async fn close_ix(&mut self, o: &Opened, price: Price) -> Instruction {
        let indices = order_indices(self, &o.trader).await;
        close_position_instruction(&o.accounts, price, &indices)
    }

    /// Fresh trader token accounts, as the app creates them again before a
    /// sweep: the close took the old ones.
    async fn renew_trader_accounts(&mut self, o: &mut Opened) {
        let (base_program, base_mint, quote_mint) =
            (self.base_program, self.base_mint, self.quote_mint);
        o.accounts.trader_base =
            token_account(&mut self.ctx, &base_program, &base_mint, &o.trader).await;
        o.accounts.trader_quote =
            token_account(&mut self.ctx, &spl_token::id(), &quote_mint, &o.trader).await;
    }

    async fn wallet(&mut self) -> (u64, u64) {
        let (b, q) = (self.owner_base, self.owner_quote);
        (
            token_balance(&mut self.ctx, &b).await,
            token_balance(&mut self.ctx, &q).await,
        )
    }
}

fn all(total: usize) -> Vec<u16> {
    (0..total as u16).collect()
}

/// The ladder's order indices nearest the opening price first, both sides
/// together, as the app places them.
fn nearest_first(bands: &[BandSpec]) -> Vec<u16> {
    let bytes = ladder(bands);
    let (ladder, total) = Ladder::parse(&bytes).unwrap();
    let mut ks = all(total);
    ks.sort_by_key(|&k| {
        ladder
            .order(k as usize, PRICE_E9, BASE_DECIMALS, QUOTE_DECIMALS)
            .unwrap()
            .unwrap()
            .distance_e8
    });
    ks
}

#[tokio::test]
async fn create_claims_a_seat_of_its_own() {
    let mut f = setup().await;
    let o = f.open(BANDS).await;

    let (p, rest) = load(&mut f.ctx, &o.position).await;
    assert_eq!(p.discriminant, POSITION_DISCRIMINANT);
    assert_eq!(p.owner, f.owner.pubkey());
    assert_eq!(p.market, f.market);
    assert_eq!(p.closed_at, 0);
    assert_eq!(p.total_orders as usize, TOTAL);
    assert_eq!(placed_count(&p, &rest), 0);
    assert_eq!(
        (
            p.created_with.base_atoms,
            p.created_with.quote_atoms,
            p.created_with.price
        ),
        (0, 0, PRICE_E9)
    );
    assert_eq!(rest.len(), ladder(BANDS).len() + placed_bitmap_len(TOTAL));
    assert_eq!(&rest[..p.ladder_len as usize], &ladder(BANDS)[..]);

    let m = market(&mut f.ctx, &f.market).await;
    assert_ne!(
        m.get_trader_index(&o.trader),
        m.get_trader_index(&f.owner.pubkey())
    );
    assert_eq!(
        m.get_trader_key_by_index(m.get_trader_index(&o.trader)),
        &o.trader
    );
    // The trader holds exactly the rent minimum of an empty account.
    let rent = f.ctx.banks_client.get_rent().await.unwrap();
    assert_eq!(
        lamports(&mut f.ctx, &o.trader).await,
        rent.minimum_balance(0)
    );
}

#[tokio::test]
async fn placed_orders_match_the_ladder_and_take_exactly_what_they_lock() {
    let mut f = setup().await;
    let o = f.open(BANDS).await;
    let before = f.wallet().await;
    // Any batches, nearest first.
    let sequence = nearest_first(BANDS);
    let (near, far) = sequence.split_at(7);
    f.place(&o, near).await.unwrap();
    f.place(&o, far).await.unwrap();

    let ladder_bytes = ladder(BANDS);
    let (ladder, _) = Ladder::parse(&ladder_bytes).unwrap();
    let expected: Vec<_> = (0..TOTAL)
        .map(|k| {
            ladder
                .order(k, PRICE_E9, BASE_DECIMALS, QUOTE_DECIMALS)
                .unwrap()
                .unwrap()
        })
        .collect();
    let (want_base, want_quote) = expected.iter().fold((0, 0), |(b, q), o| {
        if o.is_bid {
            (b, q + o.quote_atoms)
        } else {
            (b + o.base_atoms, q)
        }
    });

    let resting = orders_of(&mut f, &o.trader).await;
    assert_eq!(resting.len(), TOTAL);
    let mut got: Vec<(bool, u64, u16, OrderType)> = resting
        .iter()
        .map(|r| {
            (
                r.get_is_bid(),
                r.get_num_base_atoms().as_u64(),
                r.get_reverse_spread(),
                r.get_order_type(),
            )
        })
        .collect();
    let mut want: Vec<(bool, u64, u16, OrderType)> = expected
        .iter()
        .map(|o| {
            if o.spread_e1 < 66 {
                (
                    o.is_bid,
                    o.base_atoms,
                    o.spread_e1 * 1000,
                    OrderType::ReverseTight,
                )
            } else {
                (o.is_bid, o.base_atoms, o.spread_e1, OrderType::Reverse)
            }
        })
        .collect();
    got.sort_by_key(|x| (x.0, x.1));
    want.sort_by_key(|x| (x.0, x.1));
    assert_eq!(got, want);
    // Ten bids below the price, ten asks above, 10 quote tokens each.
    assert_eq!(expected.iter().filter(|o| o.is_bid).count(), 10);
    for e in &expected {
        let quote =
            e.base_atoms as f64 * e.price_mantissa as f64 * 10f64.powi(e.price_exponent as i32);
        assert!((quote - 10_000_000.0).abs() < 1_000.0, "{e:?}");
    }

    // The wallet paid exactly what the orders lock, the seat keeps nothing
    // spare, and the opening snapshot says so.
    let after = f.wallet().await;
    assert_eq!(
        (before.0 - after.0, before.1 - after.1),
        (want_base, want_quote)
    );
    let m = market(&mut f.ctx, &f.market).await;
    let (seat_base, seat_quote) = m.get_trader_balance(&o.trader);
    assert_eq!((seat_base.as_u64(), seat_quote.as_u64()), (0, 0));
    let (p, rest) = load(&mut f.ctx, &o.position).await;
    assert_eq!(
        (p.created_with.base_atoms, p.created_with.quote_atoms),
        (want_base, want_quote)
    );
    assert_eq!(placed_count(&p, &rest) as usize, TOTAL);
    assert_eq!(&rest[p.ladder_len as usize..], &[0xff, 0xff, 0x0f]);
}

#[tokio::test]
async fn an_order_goes_out_at_most_once() {
    let mut f = setup().await;
    let o = f.open(BANDS).await;
    // The nearest bid.
    f.place(&o, &[9]).await.unwrap();
    let wallet = f.wallet().await;
    assert!(f.place(&o, &[9]).await.is_err());
    assert!(f.place(&o, &[8, 8]).await.is_err());
    assert!(f.place(&o, &[TOTAL as u16]).await.is_err());
    assert!(f.place(&o, &[]).await.is_err());
    // A failed batch places and takes nothing.
    assert_eq!(f.wallet().await, wallet);
    assert_eq!(orders_of(&mut f, &o.trader).await.len(), 1);
    let (p, rest) = load(&mut f.ctx, &o.position).await;
    assert_eq!(placed_count(&p, &rest), 1);
    f.place(&o, &[8]).await.unwrap();
}

#[tokio::test]
async fn the_ladder_goes_out_nearest_first() {
    let mut f = setup().await;
    let o = f.open(BANDS).await;
    let refused = |err: BanksClientError| {
        assert!(
            format!("{err:?}").contains(&format!("Custom({ERROR_NOT_NEAREST_FIRST})")),
            "{err:?}"
        );
    };
    // The farthest bid while nearer ones are out, and a batch that skips one.
    refused(f.place(&o, &[0]).await.unwrap_err());
    refused(f.place(&o, &[9, 7]).await.unwrap_err());
    // Each side on its own: the nearest ask goes out with no bid.
    f.place(&o, &[10]).await.unwrap();
    // In any order within the batch, as long as the batch closes the gap.
    f.place(&o, &[7, 9, 8]).await.unwrap();
    refused(f.place(&o, &[12]).await.unwrap_err());
    f.place(&o, &[11, 12]).await.unwrap();
    assert_eq!(orders_of(&mut f, &o.trader).await.len(), 6);
    assert_trader_paid_nothing(&mut f, &o).await;
}

#[tokio::test]
async fn overlapping_bands_go_out_by_distance_across_them() {
    let mut f = setup().await;
    // Asks at +0.1% to +1%, and at +0.5% to +5%: the second band's first
    // level (+0.725%) lies among the first band's.
    let o = f
        .open(&[
            (10, 100, 10, 200, 10_000_000),
            (50, 500, 10, 200, 10_000_000),
        ])
        .await;
    let sequence = nearest_first(&[
        (10, 100, 10, 200, 10_000_000),
        (50, 500, 10, 200, 10_000_000),
    ]);
    // The second band's first level ahead of its turn.
    let err = f.place(&o, &[0, 1, 10]).await.unwrap_err();
    assert!(format!("{err:?}").contains(&format!("Custom({ERROR_NOT_NEAREST_FIRST})")));
    for chunk in sequence.chunks(3) {
        f.place(&o, chunk).await.unwrap();
    }
    assert_eq!(orders_of(&mut f, &o.trader).await.len(), 20);
}

#[tokio::test]
async fn one_instruction_holds_at_most_the_markets_growth_limit() {
    let mut f = setup().await;
    let o = f
        .open(&[
            (-2000, -10, 100, 50, 1_000_000),
            (10, 2000, 100, 50, 1_000_000),
        ])
        .await;
    let too_many: Vec<u16> = (0..=MAX_ORDERS_PER_PLACE as u16).collect();
    assert!(f.place(&o, &too_many).await.is_err());
}

#[tokio::test]
async fn compute_units_per_batch() {
    let mut f = setup().await;
    let o = f
        .open(&[
            (-3000, -10, 150, 50, 1_000_000),
            (10, 3000, 150, 200, 1_000_000),
        ])
        .await;
    // Nearest first on both sides, as the app places them.
    let sequence = nearest_first(&[
        (-3000, -10, 150, 50, 1_000_000),
        (10, 3000, 150, 200, 1_000_000),
    ]);
    let mut rest = &sequence[..];
    for n in [1usize, 10, 20, 30, 40, 50] {
        let (batch, later) = rest.split_at(n);
        rest = later;
        let ix = place_orders_instruction(&o.accounts, batch);
        let units = measure(&mut f.ctx, &[ix]).await;
        println!("CU place {n}: {units}");
        assert!(units < 1_400_000);
    }
    // A full instruction's worth.
    let batch = &rest[..MAX_ORDERS_PER_PLACE];
    let ix = place_orders_instruction(&o.accounts, batch);
    println!(
        "CU place {}: {}",
        batch.len(),
        measure(&mut f.ctx, &[ix]).await
    );
    let resting = orders_of(&mut f, &o.trader).await.len();
    let close = f.close_ix(&o, PRICE_E9).await;
    let units = measure(&mut f.ctx, &[close]).await;
    println!("CU close with {resting} orders: {units}");
}

#[tokio::test]
async fn others_cannot_trade_a_position() {
    let mut f = setup().await;
    let o = f.open(BANDS).await;
    let other = Keypair::new();
    send(
        &mut f.ctx,
        &[system_instruction::transfer(
            &f.owner.pubkey(),
            &other.pubkey(),
            1_000_000_000,
        )],
        &[],
    )
    .await
    .unwrap();
    let base_program = f.base_program;
    let (base_mint, quote_mint) = (f.base_mint, f.quote_mint);
    let other_base = token_account(&mut f.ctx, &base_program, &base_mint, &other.pubkey()).await;
    let other_quote =
        token_account(&mut f.ctx, &spl_token::id(), &quote_mint, &other.pubkey()).await;
    let mut theirs = TradingAccounts {
        owner: other.pubkey(),
        owner_base: other_base,
        owner_quote: other_quote,
        ..o.accounts
    };
    let place = place_orders_instruction(&theirs, &[0]);
    assert!(send(&mut f.ctx, &[place], &[&other]).await.is_err());
    let close = close_position_instruction(&theirs, PRICE_E9, &[]);
    assert!(send(&mut f.ctx, &[close], &[&other]).await.is_err());
    let indices = order_indices(&mut f, &o.trader).await;
    let cancel =
        cancel_orders_instruction(&other.pubkey(), &o.position, &f.market, PRICE_E9, &indices);
    assert!(send(&mut f.ctx, &[cancel], &[&other]).await.is_err());

    // Nor can the owner send what it withdraws to someone else's wallet,
    // or name token accounts of the wrong mint or no token account at all.
    theirs.owner = f.owner.pubkey();
    let close = close_position_instruction(&theirs, PRICE_E9, &[]);
    let err = send(&mut f.ctx, &[close], &[]).await.unwrap_err();
    assert!(format!("{err:?}").contains("IllegalOwner"), "{err:?}");
    let swapped = TradingAccounts {
        owner_base: f.owner_quote,
        owner_quote: f.owner_base,
        ..o.accounts
    };
    let close = close_position_instruction(&swapped, PRICE_E9, &[]);
    let err = send(&mut f.ctx, &[close], &[]).await.unwrap_err();
    assert!(format!("{err:?}").contains("InvalidAccountData"), "{err:?}");
    let not_a_token_account = TradingAccounts {
        owner_base: f.market,
        ..o.accounts
    };
    let close = close_position_instruction(&not_a_token_account, PRICE_E9, &[]);
    let err = send(&mut f.ctx, &[close], &[]).await.unwrap_err();
    assert!(format!("{err:?}").contains("IllegalOwner"), "{err:?}");
}

async fn close_returns_everything(base: Base) {
    let mut f = setup_with(base).await;
    let o = f.open(BANDS).await;
    let before = f.wallet().await;
    let owner_lamports = lamports(&mut f.ctx, &f.owner.pubkey()).await;
    f.place(&o, &all(TOTAL)).await.unwrap();
    let close = f.close_ix(&o, e9(51_000_000_000)).await;
    send(&mut f.ctx, &[close], &[]).await.unwrap();

    assert!(orders_of(&mut f, &o.trader).await.is_empty());
    // Nothing traded, so everything comes back.
    assert_eq!(f.wallet().await, before);
    let (p, _) = load(&mut f.ctx, &o.position).await;
    assert!(p.closed_at > 0);
    assert_eq!(
        (
            p.closed_with.base_atoms,
            p.closed_with.quote_atoms,
            p.closed_with.price
        ),
        (
            p.created_with.base_atoms,
            p.created_with.quote_atoms,
            e9(51_000_000_000)
        )
    );
    // The trader and its token accounts are gone, their rent back with the owner.
    for account in [o.trader, o.accounts.trader_base, o.accounts.trader_quote] {
        assert_eq!(lamports(&mut f.ctx, &account).await, 0);
    }
    assert!(lamports(&mut f.ctx, &f.owner.pubkey()).await > owner_lamports - 50_000_000);

    // Nothing goes out again.
    assert!(f.place(&o, &[0]).await.is_err());
}

#[tokio::test]
async fn close_cancels_every_order_and_returns_everything() {
    close_returns_everything(Base::Token).await;
}

#[tokio::test]
async fn close_works_with_a_token_2022_base() {
    close_returns_everything(Base::Token2022).await;
}

#[tokio::test]
async fn close_works_with_a_tokenised_stock_base() {
    close_returns_everything(Base::Stock).await;
}

#[tokio::test]
async fn tokens_sent_to_the_trader_go_back_but_not_into_the_snapshot() {
    let mut f = setup().await;
    let o = f.open(BANDS).await;
    let before = f.wallet().await;
    f.place(&o, &all(TOTAL)).await.unwrap();
    let quote_mint = f.quote_mint;
    let trader_quote = o.accounts.trader_quote;
    mint_to(
        &mut f.ctx,
        &spl_token::id(),
        &quote_mint,
        &trader_quote,
        1_000_000,
    )
    .await;
    let close = f.close_ix(&o, PRICE_E9).await;
    send(&mut f.ctx, &[close], &[]).await.unwrap();

    let (p, _) = load(&mut f.ctx, &o.position).await;
    assert_eq!(p.closed_with.quote_atoms, p.created_with.quote_atoms);
    assert_eq!(f.wallet().await, (before.0, before.1 + 1_000_000));
}

#[tokio::test]
async fn fills_show_up_in_the_closing_snapshot() {
    let mut f = setup().await;
    let o = f.open(BANDS).await;
    let start = f.wallet().await;
    f.place(&o, &all(TOTAL)).await.unwrap();

    // A taker with a seat of its own lifts the asks up to 51 quote tokens.
    let taker = Keypair::new();
    send(
        &mut f.ctx,
        &[system_instruction::transfer(
            &f.owner.pubkey(),
            &taker.pubkey(),
            1_000_000_000,
        )],
        &[],
    )
    .await
    .unwrap();
    let quote_mint = f.quote_mint;
    let taker_quote =
        token_account(&mut f.ctx, &spl_token::id(), &quote_mint, &taker.pubkey()).await;
    mint_to(
        &mut f.ctx,
        &spl_token::id(),
        &quote_mint,
        &taker_quote,
        1_000_000_000,
    )
    .await;
    let take = [
        claim_seat_instruction(&f.market, &taker.pubkey()),
        deposit_instruction(
            &f.market,
            &taker.pubkey(),
            &quote_mint,
            1_000_000_000,
            &taker_quote,
            spl_token::id(),
            None,
        ),
        batch_update_instruction(
            &f.market,
            &taker.pubkey(),
            None,
            vec![],
            // 3 tokens at up to 0.51 quote atoms per base atom.
            vec![PlaceOrderParams::new(
                300_000_000,
                51,
                -2,
                true,
                OrderType::ImmediateOrCancel,
                0,
            )],
            None,
            None,
            None,
            None,
        ),
    ];
    send(&mut f.ctx, &take, &[&taker]).await.unwrap();
    // The filled asks came back as bids of the trader.
    let resting = orders_of(&mut f, &o.trader).await;
    assert!(resting.iter().filter(|r| r.get_is_bid()).count() > 10);

    let close = f.close_ix(&o, PRICE_E9).await;
    send(&mut f.ctx, &[close], &[]).await.unwrap();
    assert!(orders_of(&mut f, &o.trader).await.is_empty());
    let (p, _) = load(&mut f.ctx, &o.position).await;
    assert!(p.closed_with.base_atoms < p.created_with.base_atoms);
    assert!(p.closed_with.quote_atoms > p.created_with.quote_atoms);
    let end = f.wallet().await;
    assert_eq!(
        end,
        (
            start.0 - p.created_with.base_atoms + p.closed_with.base_atoms,
            start.1 - p.created_with.quote_atoms + p.closed_with.quote_atoms,
        )
    );
}

#[tokio::test]
async fn cancel_orders_takes_a_bounded_bite() {
    let mut f = setup().await;
    let o = f.open(BANDS).await;
    f.place(&o, &all(TOTAL)).await.unwrap();
    let indices = order_indices(&mut f, &o.trader).await;
    let cancel = cancel_orders_instruction(
        &f.owner.pubkey(),
        &o.position,
        &f.market,
        PRICE_E9,
        &indices[..5],
    );
    send(&mut f.ctx, &[cancel], &[]).await.unwrap();
    assert_eq!(orders_of(&mut f, &o.trader).await.len(), TOTAL - 5);
    let close = f.close_ix(&o, PRICE_E9).await;
    send(&mut f.ctx, &[close], &[]).await.unwrap();
    assert!(orders_of(&mut f, &o.trader).await.is_empty());
}

#[tokio::test]
async fn orders_a_close_missed_are_swept_into_the_snapshot() {
    let mut f = setup().await;
    let mut o = f.open(BANDS).await;
    let before = f.wallet().await;
    f.place(&o, &all(TOTAL)).await.unwrap();
    let indices = order_indices(&mut f, &o.trader).await;

    // A close given only some of the seat's orders, as when one flipped in
    // or moved after the client read the book, leaves the rest resting.
    let close = close_position_instruction(&o.accounts, e9(51_000_000_000), &indices[..5]);
    send(&mut f.ctx, &[close], &[]).await.unwrap();
    assert_eq!(orders_of(&mut f, &o.trader).await.len(), TOTAL - 5);
    let (first, _) = load(&mut f.ctx, &o.position).await;
    assert!(first.closed_at > 0);
    assert!(f.place(&o, &[0]).await.is_err());

    // They can still be cancelled, and a second close sweeps them in.
    let cancel = cancel_orders_instruction(
        &f.owner.pubkey(),
        &o.position,
        &f.market,
        PRICE_E9,
        &indices[5..10],
    );
    send(&mut f.ctx, &[cancel], &[]).await.unwrap();
    // The close took the trader's token accounts, which the cancelled
    // orders' funds need on their way out.
    let close = f.close_ix(&o, e9(49_000_000_000)).await;
    let err = send(&mut f.ctx, &[close], &[]).await.unwrap_err();
    assert!(
        format!("{err:?}").contains("UninitializedAccount"),
        "{err:?}"
    );
    f.renew_trader_accounts(&mut o).await;
    let close = f.close_ix(&o, e9(49_000_000_000)).await;
    send(&mut f.ctx, &[close], &[]).await.unwrap();
    assert!(orders_of(&mut f, &o.trader).await.is_empty());
    assert_eq!(f.wallet().await, before);
    let (p, _) = load(&mut f.ctx, &o.position).await;
    assert_eq!(p.closed_at, first.closed_at);
    assert_eq!(
        (
            p.closed_with.base_atoms,
            p.closed_with.quote_atoms,
            p.closed_with.price
        ),
        (
            p.created_with.base_atoms,
            p.created_with.quote_atoms,
            e9(51_000_000_000)
        )
    );
    for account in [o.trader, o.accounts.trader_base, o.accounts.trader_quote] {
        assert_eq!(lamports(&mut f.ctx, &account).await, 0);
    }

    // Sweeping an empty seat needs no trader token accounts, and changes
    // nothing.
    let close = f.close_ix(&o, PRICE_E9).await;
    send(&mut f.ctx, &[close], &[]).await.unwrap();
    assert_eq!(load(&mut f.ctx, &o.position).await.0, p);
}

/// Deep bids of one outside maker, `batches` of 25.
async fn crowd_the_book(f: &mut Fixture, batches: u32) {
    let (maker, _) = outside_bid_of(f, 100_000, 1, -3, OrderType::Limit).await;
    for _ in 0..batches {
        let ix = batch_update_instruction(
            &f.market,
            &maker.pubkey(),
            None,
            vec![],
            (0..25)
                .map(|_| PlaceOrderParams::new(100_000, 1, -3, true, OrderType::Limit, 0))
                .collect(),
            None,
            None,
            None,
            None,
        );
        send(&mut f.ctx, &[ix], &[&maker]).await.unwrap();
    }
}

#[tokio::test]
async fn closing_costs_what_the_seat_holds_not_the_book() {
    let mut units = vec![];
    for batches in [0, 20] {
        let mut f = setup().await;
        let o = f.open(BANDS).await;
        f.place(&o, &all(TOTAL)).await.unwrap();
        crowd_the_book(&mut f, batches).await;
        let book = {
            let m = market(&mut f.ctx, &f.market).await;
            m.get_bids().iter::<RestingOrder>().count()
                + m.get_asks().iter::<RestingOrder>().count()
        };
        let close = f.close_ix(&o, PRICE_E9).await;
        let cu = measure(&mut f.ctx, &[close]).await;
        println!("CU close of {TOTAL} orders on a book of {book}: {cu}");
        units.push(cu);
    }
    // Only the tree's depth grows with the book: a few hundred units per
    // order, where walking 500 more orders would cost tens of thousands.
    assert!(units[1] < units[0] + 15_000, "{units:?}");
}

#[tokio::test]
async fn cancel_orders_skips_indices_without_an_order_of_the_position() {
    let mut f = setup().await;
    let o = f.open(BANDS).await;
    f.place(&o, &all(TOTAL)).await.unwrap();
    // Someone else's resting bid.
    outside_bid(&mut f, 100_000, 1, -3).await;
    let ours = order_indices(&mut f, &o.trader).await;
    let m = market(&mut f.ctx, &f.market).await;
    let theirs: Vec<u32> = m
        .get_bids()
        .iter::<RestingOrder>()
        .map(|(i, _)| i)
        .filter(|i| !ours.contains(i))
        .collect();
    assert_eq!(theirs.len(), 1);

    // Already cancelled, someone else's, unaligned and out of bounds: all
    // skipped, the two of ours among them cancelled.
    let first = cancel_orders_instruction(
        &f.owner.pubkey(),
        &o.position,
        &f.market,
        PRICE_E9,
        &ours[..1],
    );
    send(&mut f.ctx, &[first], &[]).await.unwrap();
    let mut mixed = vec![
        ours[0],
        ours[1],
        ours[2],
        theirs[0],
        theirs[0] + 1,
        u32::MAX - 79,
    ];
    mixed.sort();
    let cancel =
        cancel_orders_instruction(&f.owner.pubkey(), &o.position, &f.market, PRICE_E9, &mixed);
    send(&mut f.ctx, &[cancel], &[]).await.unwrap();
    assert_eq!(orders_of(&mut f, &o.trader).await.len(), TOTAL - 3);
    let m = market(&mut f.ctx, &f.market).await;
    assert_eq!(
        m.get_bids()
            .iter::<RestingOrder>()
            .filter(|(i, _)| *i == theirs[0])
            .count(),
        1
    );

    // Not strictly increasing.
    let twice = cancel_orders_instruction(
        &f.owner.pubkey(),
        &o.position,
        &f.market,
        PRICE_E9,
        &[ours[3], ours[3]],
    );
    assert!(send(&mut f.ctx, &[twice], &[]).await.is_err());
    let backwards = cancel_orders_instruction(
        &f.owner.pubkey(),
        &o.position,
        &f.market,
        PRICE_E9,
        &[ours[4], ours[3]],
    );
    assert!(send(&mut f.ctx, &[backwards], &[]).await.is_err());
}

#[tokio::test]
async fn same_owner_market_and_time_cannot_be_created_twice() {
    let mut f = setup().await;
    let created_at = now(&mut f.ctx).await;
    let ix = create_position_instruction(
        &f.owner.pubkey(),
        &f.market,
        &f.base_mint,
        &f.quote_mint,
        created_at,
        PRICE_E9,
        &ladder(BANDS),
    );
    send(&mut f.ctx, std::slice::from_ref(&ix), &[])
        .await
        .unwrap();
    assert!(send(&mut f.ctx, &[ix], &[]).await.is_err());
}

#[tokio::test]
async fn low_prices_give_up_digits_to_stay_in_manifests_range() {
    let mut f = setup().await;
    // 4e-8 quote tokens per base token, down to -90%: 4e-11 quote atoms per
    // base atom at the lowest level, which an exponent of -18 still holds.
    let bands = [(-9000, 1000, 10, 50, 1_000_000)];
    let bytes = ladder(&bands);
    let (l, _) = Ladder::parse(&bytes).unwrap();
    let lowest = l
        .order(0, e9(40), BASE_DECIMALS, QUOTE_DECIMALS)
        .unwrap()
        .unwrap();
    assert_eq!(lowest.price_exponent, -18);
    assert!(lowest.price_mantissa > 0);
    let ix = create_position_instruction(
        &f.owner.pubkey(),
        &f.market,
        &f.base_mint,
        &f.quote_mint,
        now(&mut f.ctx).await,
        e9(40),
        &bytes,
    );
    send(&mut f.ctx, &[ix], &[]).await.unwrap();
}

#[test]
fn a_cheap_token_keeps_its_price() {
    // 3.012345e-8 quote tokens per base token, a pump.fun price in SOL. In
    // billionths it would have been 30, 0.4% off.
    let price = Price::new(3_012_345, -14);
    let bytes = ladder(BANDS);
    let (l, total) = Ladder::parse(&bytes).unwrap();
    for k in 0..total {
        let o = l
            .order(k, price, BASE_DECIMALS, QUOTE_DECIMALS)
            .unwrap()
            .unwrap();
        let rel = o.distance_e8 as f64 / 1e8 * if o.is_bid { -1.0 } else { 1.0 };
        let want =
            3.012345e-8 * (1.0 + rel) * 10f64.powi(QUOTE_DECIMALS as i32 - BASE_DECIMALS as i32);
        let got = o.price_mantissa as f64 * 10f64.powi(o.price_exponent as i32);
        assert!(
            (got / want - 1.0).abs() < 1e-8,
            "order {k}: {got} vs {want}"
        );
    }
}

#[tokio::test]
async fn creation_checks_time_market_and_ladder() {
    let mut f = setup().await;
    let t = now(&mut f.ctx).await;
    let owner = f.owner.pubkey();
    let (base_mint, quote_mint) = (f.base_mint, f.quote_mint);
    let create = |market: &Pubkey, at: u32, price: Price, ladder: &[u8]| {
        create_position_instruction(&owner, market, &base_mint, &quote_mint, at, price, ladder)
    };
    let good = ladder(BANDS);
    let not_a_market = f.base_mint;
    let mut v1 = good.clone();
    v1[0] = 1;
    let cases: Vec<(&str, Instruction)> = vec![
        ("too early", create(&f.market, t - 3600, PRICE_E9, &good)),
        ("too late", create(&f.market, t + 3600, PRICE_E9, &good)),
        ("not a market", create(&not_a_market, t, PRICE_E9, &good)),
        ("no price", create(&f.market, t, e9(0), &good)),
        (
            "price exponent out of range",
            create(&f.market, t, Price::new(5, 21), &good),
        ),
        ("old ladder", create(&f.market, t, PRICE_E9, &v1)),
        (
            "truncated",
            create(&f.market, t, PRICE_E9, &good[..good.len() - 1]),
        ),
        (
            "no levels",
            create(&f.market, t, PRICE_E9, &ladder(&[(-500, -10, 0, 50, 1)])),
        ),
        (
            "empty band",
            create(&f.market, t, PRICE_E9, &ladder(&[(10, 10, 1, 50, 1)])),
        ),
        (
            "below zero",
            create(&f.market, t, PRICE_E9, &ladder(&[(-10_000, -10, 1, 50, 1)])),
        ),
        (
            "too many",
            create(&f.market, t, PRICE_E9, &ladder(&[(-500, -10, 1025, 50, 1)])),
        ),
        (
            "order too big",
            create(
                &f.market,
                t,
                e9(1_000_000),
                &ladder(&[(-9000, -10, 10, 50, u64::MAX)]),
            ),
        ),
    ];
    for (name, ix) in cases {
        assert!(send(&mut f.ctx, &[ix], &[]).await.is_err(), "{name}");
    }
    // The position address must be the one derived from the data.
    let mut wrong = create(&f.market, t, PRICE_E9, &good);
    wrong.accounts[1].pubkey = get_position_address(&owner, &f.market, t - 1).0;
    assert!(send(&mut f.ctx, &[wrong], &[]).await.is_err());
    // As must the trader's.
    let mut wrong = create(&f.market, t, PRICE_E9, &good);
    wrong.accounts[2].pubkey = get_trader_address(&Pubkey::new_unique()).0;
    assert!(send(&mut f.ctx, &[wrong], &[]).await.is_err());
}

#[tokio::test]
async fn lamports_sent_beforehand_do_not_block_creation() {
    for gift in [1_000_000u64, 100_000_000] {
        let mut f = setup().await;
        let t = now(&mut f.ctx).await;
        let (position, _) = get_position_address(&f.owner.pubkey(), &f.market, t);
        let (trader, _) = get_trader_address(&position);
        let griefer = Keypair::new();
        send(
            &mut f.ctx,
            &[system_instruction::transfer(
                &f.owner.pubkey(),
                &griefer.pubkey(),
                1_000_000_000,
            )],
            &[],
        )
        .await
        .unwrap();
        let gifts = [
            system_instruction::transfer(&griefer.pubkey(), &position, gift),
            system_instruction::transfer(&griefer.pubkey(), &trader, gift),
        ];
        send(&mut f.ctx, &gifts, &[&griefer]).await.unwrap();

        let ix = create_position_instruction(
            &f.owner.pubkey(),
            &f.market,
            &f.base_mint,
            &f.quote_mint,
            t,
            PRICE_E9,
            &ladder(BANDS),
        );
        send(&mut f.ctx, &[ix], &[]).await.unwrap();
        let (p, _) = load(&mut f.ctx, &position).await;
        assert_eq!(p.total_orders as usize, TOTAL);
        let o = f
            .trading(&f.owner.pubkey(), t, f.owner_base, f.owner_quote)
            .await;
        f.place(&o, &all(TOTAL)).await.unwrap();
    }
}

/// The app closes in bites of this many cancels: one ClosePosition runs out
/// of heap past about 300 resting orders.
const CANCEL_BITE: usize = 180;

#[tokio::test]
async fn full_ladder_closes_after_bounded_cancels() {
    let mut f = setup().await;
    let o = f
        .open(&[
            (-5000, -10, 512, 50, 1_000_000),
            (10, 5000, 512, 200, 1_000_000),
        ])
        .await;
    let sequence = nearest_first(&[
        (-5000, -10, 512, 50, 1_000_000),
        (10, 5000, 512, 200, 1_000_000),
    ]);
    for chunk in sequence.chunks(MAX_ORDERS_PER_PLACE) {
        f.place(&o, chunk).await.unwrap();
    }
    let close = f.close_ix(&o, PRICE_E9).await;
    assert!(send(&mut f.ctx, std::slice::from_ref(&close), &[])
        .await
        .is_err());
    while orders_of(&mut f, &o.trader).await.len() > CANCEL_BITE {
        let indices = order_indices(&mut f, &o.trader).await;
        let c = cancel_orders_instruction(
            &f.owner.pubkey(),
            &o.position,
            &f.market,
            PRICE_E9,
            &indices[..CANCEL_BITE],
        );
        send(&mut f.ctx, &[c], &[]).await.unwrap();
    }
    send(&mut f.ctx, &[close], &[]).await.unwrap();
    assert!(orders_of(&mut f, &o.trader).await.is_empty());
}

/// An outside trader's resting bid of `base_atoms` at `mantissa`·10^`exponent`
/// quote atoms per base atom; a global order backed by the quote mint's
/// global account when `global`.
async fn outside_bid(f: &mut Fixture, base_atoms: u64, mantissa: u32, exponent: i8) {
    outside_bid_of(f, base_atoms, mantissa, exponent, OrderType::Limit).await;
}

async fn outside_bid_of(
    f: &mut Fixture,
    base_atoms: u64,
    mantissa: u32,
    exponent: i8,
    order_type: OrderType,
) -> (Keypair, Pubkey) {
    let global = order_type == OrderType::Global;
    let maker = Keypair::new();
    send(
        &mut f.ctx,
        &[system_instruction::transfer(
            &f.owner.pubkey(),
            &maker.pubkey(),
            1_000_000_000,
        )],
        &[],
    )
    .await
    .unwrap();
    let quote_mint = f.quote_mint;
    let maker_quote =
        token_account(&mut f.ctx, &spl_token::id(), &quote_mint, &maker.pubkey()).await;
    mint_to(
        &mut f.ctx,
        &spl_token::id(),
        &quote_mint,
        &maker_quote,
        1_000_000_000,
    )
    .await;
    let mut ixs = vec![claim_seat_instruction(&f.market, &maker.pubkey())];
    if global {
        let (global_account, _) = get_global_address(&quote_mint);
        ixs.extend([
            create_global_instruction(&quote_mint, &maker.pubkey(), &spl_token::id()),
            global_add_trader_instruction(&global_account, &maker.pubkey()),
            global_deposit_instruction(
                &quote_mint,
                &maker.pubkey(),
                &maker_quote,
                &spl_token::id(),
                1_000_000_000,
            ),
        ]);
    } else {
        ixs.push(deposit_instruction(
            &f.market,
            &maker.pubkey(),
            &quote_mint,
            1_000_000_000,
            &maker_quote,
            spl_token::id(),
            None,
        ));
    }
    ixs.push(batch_update_instruction(
        &f.market,
        &maker.pubkey(),
        None,
        vec![],
        vec![PlaceOrderParams::new(
            base_atoms,
            mantissa,
            exponent,
            true,
            order_type,
            // A reverse order's spread, 1% here.
            if order_type == OrderType::Reverse {
                1_000
            } else {
                0
            },
        )],
        None,
        None,
        global.then_some(quote_mint),
        global.then_some(spl_token::id()),
    ));
    send(&mut f.ctx, &ixs, &[&maker]).await.unwrap();
    (maker, maker_quote)
}

#[tokio::test]
async fn an_order_that_trades_more_than_one_percent_fails_the_batch() {
    // The asks closest to the price are 0.2 base tokens each (2e7 atoms).
    let first_ask = (TOTAL / 2) as u16;
    // A bid 1% above the price takes the nearest ask whole.
    let mut f = setup().await;
    let o = f.open(BANDS).await;
    outside_bid(&mut f, 100_000_000, 505, -3).await;
    let err = f.place(&o, &[first_ask]).await.unwrap_err();
    assert!(
        format!("{err:?}").contains(&format!("Custom({ERROR_ORDER_CROSSED})")),
        "{err:?}"
    );
    let (p, rest) = load(&mut f.ctx, &o.position).await;
    assert_eq!(placed_count(&p, &rest), 0);

    // One that takes 0.5% of it lets it rest with the rest.
    let mut f = setup().await;
    let o = f.open(BANDS).await;
    outside_bid(&mut f, 100_000, 505, -3).await;
    f.place(&o, &[first_ask]).await.unwrap();
    let resting = orders_of(&mut f, &o.trader).await;
    assert_eq!(resting.len(), 1);
    assert!(resting[0].get_num_base_atoms().as_u64() < 20_000_000);
}

#[tokio::test]
async fn global_accounts_let_orders_sweep_global_dust() {
    let first_ask = (TOTAL / 2) as u16;
    let mut f = setup().await;
    let o = f.open(BANDS).await;
    // The global accounts are not optional.
    let mut ix = place_orders_instruction(&o.accounts, &[first_ask]);
    ix.accounts.truncate(16);
    assert!(send(&mut f.ctx, &[ix], &[]).await.is_err());
    // A global bid 1% above the price for 0.5% of the nearest ask.
    outside_bid_of(&mut f, 100_000, 505, -3, OrderType::Global).await;
    // The ask takes it and rests with the rest.
    f.place(&o, &[first_ask]).await.unwrap();
    let resting = orders_of(&mut f, &o.trader).await;
    assert_eq!(resting.len(), 1);
    assert!(resting[0].get_num_base_atoms().as_u64() < 20_000_000);
    // Mints without a global account are skipped: the base has none here.
    f.place(&o, &[first_ask + 1]).await.unwrap();
}

#[tokio::test]
async fn taking_a_reverse_order_costs_the_trader_nothing() {
    let first_ask = (TOTAL / 2) as u16;
    let mut f = setup().await;
    let o = f.open(BANDS).await;
    // A reverse bid 1% above the price for 0.5% of the nearest ask: the ask
    // empties it and it flips into an ask of its own.
    outside_bid_of(&mut f, 100_000, 505, -3, OrderType::Reverse).await;
    f.place(&o, &[first_ask]).await.unwrap();
    assert_eq!(orders_of(&mut f, &o.trader).await.len(), 1);
    let m = market(&mut f.ctx, &f.market).await;
    assert_eq!(m.get_asks().iter::<RestingOrder>().count(), 2);
    // The flip took the block the bid freed: Manifest did not grow the market
    // at the trader's expense.
    let rent = f.ctx.banks_client.get_rent().await.unwrap();
    assert_eq!(
        lamports(&mut f.ctx, &o.trader).await,
        rent.minimum_balance(0)
    );
}

// Manifest grows the market from the batch's payer, the trader, whenever a
// placement finds no free block or the reserve ends below five; the trader
// holds only its rent minimum, so either would fail PlaceOrders. The tests
// below take the book through what frees and fills blocks during matching
// and check the trader never pays.

/// The trader holds exactly the rent minimum of an empty account.
async fn assert_trader_paid_nothing(f: &mut Fixture, o: &Opened) {
    let rent = f.ctx.banks_client.get_rent().await.unwrap();
    assert_eq!(
        lamports(&mut f.ctx, &o.trader).await,
        rent.minimum_balance(0)
    );
}

/// Someone else on the market: a seat with plenty of both tokens on it.
async fn other_trader(f: &mut Fixture) -> Keypair {
    let t = Keypair::new();
    let owner = f.owner.pubkey();
    send(
        &mut f.ctx,
        &[system_instruction::transfer(
            &owner,
            &t.pubkey(),
            1_000_000_000,
        )],
        &[],
    )
    .await
    .unwrap();
    let (base_mint, quote_mint, base_program) = (f.base_mint, f.quote_mint, f.base_program);
    let t_base = token_account(&mut f.ctx, &base_program, &base_mint, &t.pubkey()).await;
    let t_quote = token_account(&mut f.ctx, &spl_token::id(), &quote_mint, &t.pubkey()).await;
    mint_to(&mut f.ctx, &base_program, &base_mint, &t_base, WALLET_BASE).await;
    mint_to(
        &mut f.ctx,
        &spl_token::id(),
        &quote_mint,
        &t_quote,
        WALLET_QUOTE,
    )
    .await;
    let ixs = [
        claim_seat_instruction(&f.market, &t.pubkey()),
        deposit_instruction(
            &f.market,
            &t.pubkey(),
            &base_mint,
            WALLET_BASE,
            &t_base,
            base_program,
            None,
        ),
        deposit_instruction(
            &f.market,
            &t.pubkey(),
            &quote_mint,
            WALLET_QUOTE,
            &t_quote,
            spl_token::id(),
            None,
        ),
    ];
    send(&mut f.ctx, &ixs, &[&t]).await.unwrap();
    t
}

async fn other_orders(
    f: &mut Fixture,
    t: &Keypair,
    orders: Vec<PlaceOrderParams>,
) -> Result<(), BanksClientError> {
    let ix = batch_update_instruction(
        &f.market,
        &t.pubkey(),
        None,
        vec![],
        orders,
        None,
        None,
        None,
        None,
    );
    send(&mut f.ctx, &[ix], &[t]).await
}

/// A reverse order's spread of 1%, in the units its type takes.
fn reverse(
    base_atoms: u64,
    mantissa: u32,
    exponent: i8,
    is_bid: bool,
    tight: bool,
) -> PlaceOrderParams {
    if tight {
        PlaceOrderParams::new(
            base_atoms,
            mantissa,
            exponent,
            is_bid,
            OrderType::ReverseTight,
            1_000_000,
        )
    } else {
        PlaceOrderParams::new(
            base_atoms,
            mantissa,
            exponent,
            is_bid,
            OrderType::Reverse,
            1_000,
        )
    }
}

fn is_crossed(err: &BanksClientError) -> bool {
    format!("{err:?}").contains(&format!("Custom({ERROR_ORDER_CROSSED})"))
}

#[tokio::test]
async fn more_flips_in_one_batch_than_the_spare_blocks() {
    let mut f = setup().await;
    let o = f.open(BANDS).await;
    // Twelve reverse bids, alternating plain and tight, 2% above the price
    // and 1/20th of a percent of an ask each: the nearest ask empties all of
    // them, twelve flips against five spare blocks.
    let t = other_trader(&mut f).await;
    let bids = (0..12)
        .map(|i| reverse(10_000, 5_100 + i, -4, true, i % 2 == 1))
        .collect();
    other_orders(&mut f, &t, bids).await.unwrap();
    let asks: Vec<u16> = (TOTAL as u16 / 2..TOTAL as u16).collect();
    f.place(&o, &asks).await.unwrap();
    let m = market(&mut f.ctx, &f.market).await;
    assert_eq!(m.get_bids().iter::<RestingOrder>().count(), 0);
    assert_eq!(m.get_asks().iter::<RestingOrder>().count(), 10 + 12);
    assert_trader_paid_nothing(&mut f, &o).await;
}

#[tokio::test]
async fn emptying_a_maker_with_the_last_of_an_order_fails_as_crossed() {
    let mut f = setup().await;
    let o = f.open(BANDS).await;
    // A reverse bid 2% above the price larger than the nearest ask: the ask
    // goes whole and the bid flips its part, a block more than the batch.
    let t = other_trader(&mut f).await;
    other_orders(&mut f, &t, vec![reverse(50_000_000, 51, -2, true, false)])
        .await
        .unwrap();
    let err = f.place(&o, &[TOTAL as u16 / 2]).await.unwrap_err();
    assert!(is_crossed(&err), "{err:?}");
    assert_trader_paid_nothing(&mut f, &o).await;
}

#[tokio::test]
async fn expired_orders_in_the_way_are_swept_for_free() {
    let mut f = setup().await;
    let o = f.open(BANDS).await;
    // Bids 2% above the price that expire before the asks go out; each is
    // big enough that taking it would fail the batch.
    let slot = f.ctx.banks_client.get_root_slot().await.unwrap();
    let t = other_trader(&mut f).await;
    let bids = (0..8)
        .map(|_| {
            PlaceOrderParams::new(50_000_000, 51, -2, true, OrderType::Limit, slot as u32 + 50)
        })
        .collect();
    other_orders(&mut f, &t, bids).await.unwrap();
    f.ctx.warp_to_slot(slot + 100).unwrap();
    let asks: Vec<u16> = (TOTAL as u16 / 2..TOTAL as u16).collect();
    f.place(&o, &asks).await.unwrap();
    assert_eq!(orders_of(&mut f, &o.trader).await.len(), 10);
    assert_trader_paid_nothing(&mut f, &o).await;
}

#[tokio::test]
async fn an_unbacked_global_order_in_the_way_is_removed_for_free() {
    let first_ask = (TOTAL / 2) as u16;
    let mut f = setup().await;
    let o = f.open(BANDS).await;
    // A global bid 1% above the price whose funds are then withdrawn.
    let (maker, maker_quote) = outside_bid_of(&mut f, 20_000_000, 505, -3, OrderType::Global).await;
    let ix = global_withdraw_instruction(
        &f.quote_mint,
        &maker.pubkey(),
        &maker_quote,
        &spl_token::id(),
        1_000_000_000,
    );
    send(&mut f.ctx, &[ix], &[&maker]).await.unwrap();
    f.place(&o, &[first_ask]).await.unwrap();
    // The bid is gone without a trade, the ask rests whole.
    assert_eq!(orders_of(&mut f, &o.trader).await.len(), 1);
    let m = market(&mut f.ctx, &f.market).await;
    assert_eq!(m.get_bids().iter::<RestingOrder>().count(), 0);
    // The global's gas prepayment comes back to the trader, and leaves with
    // it at close.
    let rent = f.ctx.banks_client.get_rent().await.unwrap();
    assert!(lamports(&mut f.ctx, &o.trader).await >= rent.minimum_balance(0));
    let close = f.close_ix(&o, PRICE_E9).await;
    send(&mut f.ctx, &[close], &[]).await.unwrap();
    assert_eq!(lamports(&mut f.ctx, &o.trader).await, 0);
    // No quote came in: the ask never traded with the bid.
    assert_eq!(
        load(&mut f.ctx, &o.position)
            .await
            .0
            .closed_with
            .quote_atoms,
        0
    );
}

/// A small deterministic generator, so a failing run can be replayed.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Others keep trading around and through the ladder while it goes out in
/// random bites: limit, post-only and reverse orders, mostly small, some
/// large, across ±6% of the price, filling ours and making them flip into the
/// way of later bites. Every bite either goes out or fails as crossed, the
/// trader never pays, and the position closes at the end.
#[tokio::test]
async fn random_trading_around_the_ladder_never_charges_the_trader() {
    for seed in 1..=6u64 {
        let mut rng = Lcg(seed);
        let mut f = setup().await;
        let o = f.open(BANDS).await;
        let others = [
            other_trader(&mut f).await,
            other_trader(&mut f).await,
            other_trader(&mut f).await,
        ];
        let sequence = nearest_first(BANDS);
        let mut unplaced = sequence.clone();
        let (mut went_out, mut crossed) = (0, 0);
        for _ in 0..40 {
            if rng.below(3) > 0 {
                let t = &others[rng.below(3) as usize];
                let orders = (0..1 + rng.below(4))
                    .map(|_| {
                        // Quote atoms per base atom: 0.5 · (1 ± 6%), in 1e-6.
                        let mantissa = (470_000 + rng.below(60_001)) as u32;
                        let is_bid = rng.below(2) == 0;
                        let base_atoms = if rng.below(5) == 0 {
                            10_000_000 + rng.below(40_000_000)
                        } else {
                            1_000 + rng.below(200_000)
                        };
                        match rng.below(4) {
                            0 => PlaceOrderParams::new(
                                base_atoms,
                                mantissa,
                                -6,
                                is_bid,
                                OrderType::Limit,
                                0,
                            ),
                            1 => PlaceOrderParams::new(
                                base_atoms,
                                mantissa,
                                -6,
                                is_bid,
                                OrderType::PostOnly,
                                0,
                            ),
                            k => reverse(base_atoms, mantissa, -6, is_bid, k == 3),
                        }
                    })
                    .collect();
                // Post-only orders that would cross are refused; that is
                // their business.
                let _ = other_orders(&mut f, t, orders).await;
            } else if !unplaced.is_empty() {
                // The nearest few, in a random order.
                let n = (1 + rng.below(6) as usize).min(unplaced.len());
                let mut bite: Vec<u16> = unplaced.drain(..n).collect();
                for i in (1..n).rev() {
                    bite.swap(i, rng.below(i as u64 + 1) as usize);
                }
                match f.place(&o, &bite).await {
                    Ok(()) => went_out += 1,
                    Err(err) => {
                        assert!(is_crossed(&err), "seed {seed}: {err:?}");
                        crossed += 1;
                        // Back in front, nearest first again.
                        bite.append(&mut unplaced);
                        unplaced = sequence
                            .iter()
                            .copied()
                            .filter(|k| bite.contains(k))
                            .collect();
                    }
                }
                assert_trader_paid_nothing(&mut f, &o).await;
            }
        }
        eprintln!(
            "seed {seed}: {went_out} bites out, {crossed} crossed, {} unplaced",
            unplaced.len()
        );
        let close = f.close_ix(&o, PRICE_E9).await;
        send(&mut f.ctx, &[close], &[]).await.unwrap();
        assert_eq!(lamports(&mut f.ctx, &o.trader).await, 0);
    }
}

#[tokio::test]
async fn the_first_cancel_records_the_closing_price() {
    let mut f = setup().await;
    let o = f.open(BANDS).await;
    f.place(&o, &all(TOTAL - 1)).await.unwrap();
    let indices = order_indices(&mut f, &o.trader).await;
    let owner = f.owner.pubkey();

    // A zero price is refused, by either instruction.
    let cancel = cancel_orders_instruction(&owner, &o.position, &f.market, e9(0), &indices[..1]);
    assert!(send(&mut f.ctx, &[cancel], &[]).await.is_err());
    let close = f.close_ix(&o, e9(0)).await;
    assert!(send(&mut f.ctx, &[close], &[]).await.is_err());

    // The first cancel's price stands; later ones and the close keep it.
    let cancel = cancel_orders_instruction(
        &owner,
        &o.position,
        &f.market,
        e9(51_000_000_000),
        &indices[..1],
    );
    send(&mut f.ctx, &[cancel], &[]).await.unwrap();
    let p = load(&mut f.ctx, &o.position).await.0;
    assert_eq!((p.closed_at, p.closed_with.price), (0, e9(51_000_000_000)));
    // Closing began: the rest of the ladder no longer goes out.
    assert!(f.place(&o, &[TOTAL as u16 - 1]).await.is_err());
    let cancel = cancel_orders_instruction(
        &owner,
        &o.position,
        &f.market,
        e9(49_000_000_000),
        &indices[1..2],
    );
    send(&mut f.ctx, &[cancel], &[]).await.unwrap();
    let close = f.close_ix(&o, e9(48_000_000_000)).await;
    send(&mut f.ctx, &[close], &[]).await.unwrap();
    let p = load(&mut f.ctx, &o.position).await.0;
    assert!(p.closed_at > 0);
    assert_eq!(p.closed_with.price, e9(51_000_000_000));
}

/// Runs a Manifest defrag on the market as its test collector, reclaiming
/// every empty seat.
async fn defrag(f: &mut Fixture) {
    let collector = Keypair::new_from_array([42; 32]);
    f.ctx.set_account(
        &collector.pubkey(),
        &solana_account::AccountSharedData::from(solana_account::Account::new(
            1_000_000_000,
            0,
            &solana_sdk_ids::system_program::id(),
        )),
    );
    let ix = defrag_instruction(
        &f.market,
        &collector.pubkey(),
        &f.base_mint,
        &f.quote_mint,
        f.base_program,
        spl_token::id(),
        None,
    );
    send(&mut f.ctx, &[ix], &[&collector]).await.unwrap();
}

async fn has_seat(f: &mut Fixture, trader: &Pubkey) -> bool {
    market(&mut f.ctx, &f.market).await.get_trader_index(trader) != manifest::deps::hypertree::NIL
}

#[tokio::test]
async fn a_seat_reclaimed_before_placing_is_claimed_again() {
    let mut f = setup().await;
    let o = f.open(BANDS).await;
    let before = f.wallet().await;
    defrag(&mut f).await;
    assert!(!has_seat(&mut f, &o.trader).await);

    f.place(&o, &all(TOTAL)).await.unwrap();
    assert!(has_seat(&mut f, &o.trader).await);
    assert_eq!(orders_of(&mut f, &o.trader).await.len(), TOTAL);
    assert_trader_paid_nothing(&mut f, &o).await;

    let close = f.close_ix(&o, PRICE_E9).await;
    send(&mut f.ctx, &[close], &[]).await.unwrap();
    assert_eq!(f.wallet().await, before);
}

#[tokio::test]
async fn a_position_whose_seat_was_reclaimed_still_closes() {
    let mut f = setup().await;
    let o = f.open(BANDS).await;
    defrag(&mut f).await;
    assert!(!has_seat(&mut f, &o.trader).await);

    let close = f.close_ix(&o, PRICE_E9).await;
    send(&mut f.ctx, &[close], &[]).await.unwrap();
    let (p, _) = load(&mut f.ctx, &o.position).await;
    assert!(p.closed_at > 0);
    assert_eq!(
        (p.closed_with.base_atoms, p.closed_with.quote_atoms),
        (0, 0)
    );
    for account in [o.trader, o.accounts.trader_base, o.accounts.trader_quote] {
        assert_eq!(lamports(&mut f.ctx, &account).await, 0);
    }
}

#[tokio::test]
async fn mints_the_position_cannot_trade_are_refused() {
    // A transfer fee even at zero: its authority can raise it at any time.
    for base in [
        Base::TransferFee,
        Base::Hooked,
        Base::FrozenByDefault,
        Base::NonTransferable,
    ] {
        let mut f = setup_with(base).await;
        let created_at = now(&mut f.ctx).await;
        let ix = create_position_instruction(
            &f.owner.pubkey(),
            &f.market,
            &f.base_mint,
            &f.quote_mint,
            created_at,
            PRICE_E9,
            &ladder(BANDS),
        );
        let err = send(&mut f.ctx, &[ix], &[]).await.unwrap_err();
        assert!(
            format!("{err:?}").contains(&format!("Custom({ERROR_UNSUPPORTED_MINT})")),
            "{base:?}: {err:?}"
        );
        // Nor does naming another mint than the market's get past it.
        let ix = create_position_instruction(
            &f.owner.pubkey(),
            &f.market,
            &f.quote_mint,
            &f.quote_mint,
            created_at,
            PRICE_E9,
            &ladder(BANDS),
        );
        let err = send(&mut f.ctx, &[ix], &[]).await.unwrap_err();
        assert!(format!("{err:?}").contains("InvalidArgument"), "{err:?}");
    }
}
