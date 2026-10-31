use manifest::{
    deps::hypertree::{get_helper, DataIndex, HyperTreeValueIteratorTrait, RBNode, NIL},
    program::{
        batch_update::{CancelOrderParams, MarketDataTreeNodeType, PlaceOrderParams},
        batch_update_instruction, claim_seat_instruction, deposit_instruction, get_dynamic_account,
        invoke, invoke_signed, withdraw_instruction, ManifestInstruction,
    },
    quantities::WrapperU64,
    state::{
        MarketFixed, MarketRef, OrderType, RestingOrder, ADVANCED_ORDER_FEE_LAMPORTS,
        MARKET_BLOCK_SIZE,
    },
    utils::create_account,
    validation::{
        as_raw_key, next_account_info, to_program_error, AccountViewExt, EmptyAccount,
        ManifestAccountInfo, Program, Signer,
    },
};
use pinocchio::{
    account::AccountView,
    error::ProgramError,
    sysvars::{clock::Clock, rent::Rent, Sysvar},
    ProgramResult,
};
use solana_program::{
    instruction::{AccountMeta, Instruction},
    program::get_return_data,
    pubkey::Pubkey,
};
use solana_sdk_ids::system_program;
use solana_system_interface::instruction as system_instruction;

use crate::{
    instruction::CREATE_FIXED_DATA_LEN,
    state::{
        get_position_address, get_trader_address, placed_bitmap_len, Ladder, LadderOrder, Position,
        Snapshot, MAX_CREATED_AT_DRIFT_SECONDS, MAX_LADDER_BYTES, POSITION_DISCRIMINANT,
        POSITION_HEADER_SIZE, POSITION_SEED, TRADER_SEED,
    },
};

/// Orders one PlaceOrders may lay: what one Manifest batch update can report
/// back, as its return data (4 bytes + 12 per order) is capped at 1 KiB. The
/// owner also grows the market by one 80-byte block per order plus Manifest's
/// reserve of five in one resize, well inside the 10 KiB an instruction may
/// grow an account by. A transaction can carry several PlaceOrders.
pub const MAX_ORDERS_PER_PLACE: usize = 85;

/// Below this many tenths of a basis point a reverse order is ReverseTight,
/// whose spread counts in 1e-8 instead of 1e-5 (and so tops out at 6.5535
/// bps); the app picks the order type the same way.
const REVERSE_TIGHT_BELOW_E1: u16 = 66;

/// Share of its size, in percent, every placed order must still rest with.
/// Reverse orders take what they cross; an order that traded more than 1%
/// on the way in met a book far from the opening price.
const MIN_RESTING_PERCENT: u128 = 99;
/// PlaceOrders fails with this when an order rested with less than that;
/// clear of the token programs' and Manifest's own error codes.
pub const ERROR_ORDER_CROSSED: u32 = 1000;

/// Accounts PlaceOrders and ClosePosition share, see TradingAccounts.
const TRADING_ACCOUNTS: usize = 16;

/// Creates the PDA even when someone has already sent lamports to its
/// address: a plain create_account fails on a funded address, which would let
/// anyone block the next few seconds' positions for the price of a transfer.
fn create_pda(
    payer: &AccountView,
    account: &AccountView,
    system_program: &AccountView,
    program_id: &Pubkey,
    space: usize,
    seeds: &[&[u8]],
) -> ProgramResult {
    let rent = Rent::get()?;
    let current = account.lamports();
    if current == 0 {
        return create_account(
            payer,
            account,
            system_program,
            program_id,
            &rent,
            space as u64,
            seeds.iter().map(|s| s.to_vec()).collect(),
        );
    }
    let required = rent.try_minimum_balance(space)?;
    if current < required {
        invoke(
            &system_instruction::transfer(payer.pubkey(), account.pubkey(), required - current),
            &[payer, account],
        )?;
    }
    invoke_signed(
        &system_instruction::allocate(account.pubkey(), space as u64),
        &[account],
        &[seeds],
    )?;
    invoke_signed(
        &system_instruction::assign(account.pubkey(), program_id),
        &[account],
        &[seeds],
    )
}

/// Grows the market until it has `blocks` free blocks, paid by `payer`, so
/// that the trader, which only holds the rent minimum, never pays for it.
fn expand_market(
    payer: &AccountView,
    market: &AccountView,
    system_program: &AccountView,
    blocks: u32,
) -> ProgramResult {
    let mut data = vec![ManifestInstruction::Expand as u8];
    data.extend_from_slice(&blocks.to_le_bytes());
    invoke(
        &Instruction {
            program_id: manifest::ID,
            accounts: vec![
                AccountMeta::new(*payer.pubkey(), true),
                AccountMeta::new(*market.pubkey(), false),
                AccountMeta::new_readonly(system_program::id(), false),
            ],
            data,
        },
        &[payer, market, system_program],
    )
}

pub(crate) fn process_create_position(
    program_id: &Pubkey,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let iter = &mut accounts.iter();
    let owner: Signer = Signer::new_payer(next_account_info(iter)?)?;
    let position: EmptyAccount = EmptyAccount::new(next_account_info(iter)?)?;
    let trader: &AccountView = next_account_info(iter)?;
    let market: &AccountView = next_account_info(iter)?;
    let system_program: Program = Program::new(next_account_info(iter)?, &system_program::id())?;
    let _manifest_program: Program = Program::new(next_account_info(iter)?, &manifest::ID)?;

    if data.len() < CREATE_FIXED_DATA_LEN || data.len() - CREATE_FIXED_DATA_LEN > MAX_LADDER_BYTES {
        return Err(ProgramError::InvalidInstructionData);
    }
    let created_at = i64::from_le_bytes(data[..8].try_into().unwrap());
    let price_e9 = u64::from_le_bytes(data[8..16].try_into().unwrap());
    let ladder_bytes = &data[CREATE_FIXED_DATA_LEN..];
    let ladder = Ladder::parse(ladder_bytes)?;
    let total_orders = ladder.total_orders();
    if price_e9 == 0 {
        return Err(ProgramError::InvalidArgument);
    }

    // A real Manifest market: owned by Manifest and carrying its discriminant.
    // Every order of the ladder must be one it takes, or placing would get
    // stuck on it.
    {
        let market = ManifestAccountInfo::<MarketFixed>::new(market)?;
        let fixed = market.get_fixed()?;
        ladder.check_orders(
            price_e9,
            fixed.get_base_mint_decimals(),
            fixed.get_quote_mint_decimals(),
        )?;
    }

    // The creation time is part of the address, so the client chooses it; it
    // must still be the actual time, give or take the transaction's lifetime.
    let now = Clock::get()?.unix_timestamp;
    if created_at <= 0 || (now - created_at).abs() > MAX_CREATED_AT_DRIFT_SECONDS {
        return Err(ProgramError::InvalidArgument);
    }

    let (expected, bump) = get_position_address(owner.pubkey(), market.pubkey(), created_at);
    if expected != *position.info.pubkey() {
        return Err(ProgramError::InvalidSeeds);
    }
    let (expected_trader, trader_bump) = get_trader_address(position.info.pubkey());
    if expected_trader != *trader.pubkey() {
        return Err(ProgramError::InvalidSeeds);
    }

    let created_at_bytes = created_at.to_le_bytes();
    let bump_bytes = [bump];
    create_pda(
        owner.info,
        position.info,
        system_program.as_ref(),
        program_id,
        POSITION_HEADER_SIZE + ladder_bytes.len() + placed_bitmap_len(total_orders),
        &[
            POSITION_SEED,
            owner.pubkey().as_ref(),
            market.pubkey().as_ref(),
            &created_at_bytes,
            &bump_bytes,
        ],
    )?;

    {
        let mut bytes = position.info.try_borrow_mut()?;
        let (header, rest) = bytes.split_at_mut(POSITION_HEADER_SIZE);
        *bytemuck::from_bytes_mut::<Position>(header) = Position {
            discriminant: POSITION_DISCRIMINANT,
            owner: *owner.pubkey(),
            market: *market.pubkey(),
            created_at,
            closed_at: 0,
            created_with: Snapshot {
                base_atoms: 0,
                quote_atoms: 0,
                price_e9,
            },
            closed_with: Snapshot::default(),
            total_orders: total_orders as u16,
            placed_orders: 0,
            ladder_len: ladder_bytes.len() as u16,
            bump,
            trader_bump,
        };
        rest[..ladder_bytes.len()].copy_from_slice(ladder_bytes);
    }

    // The trader is a plain system account that only ever holds the rent
    // minimum between instructions. Lamports someone sent it beforehand
    // simply count towards that.
    let rent_minimum = Rent::get()?.try_minimum_balance(0)?;
    if trader.lamports() < rent_minimum {
        invoke(
            &system_instruction::transfer(
                owner.pubkey(),
                trader.pubkey(),
                rent_minimum - trader.lamports(),
            ),
            &[owner.info, trader],
        )?;
    }

    // Two free blocks: one for the seat, one left over so Manifest does not
    // ask the trader to grow the market after claiming.
    expand_market(owner.info, market, system_program.as_ref(), 2)?;
    invoke_signed(
        &claim_seat_instruction(market.pubkey(), trader.pubkey()),
        &[trader, market, system_program.as_ref()],
        &[&[TRADER_SEED, position.info.pubkey().as_ref(), &[trader_bump]]],
    )
}

/// The accounts every instruction on an open position shares, checked.
struct Open<'a> {
    owner: Signer<'a>,
    position: &'a AccountView,
    trader: &'a AccountView,
    market: &'a AccountView,
    system_program: &'a AccountView,
    trader_bump: u8,
}

impl<'a> Open<'a> {
    fn load(
        program_id: &Pubkey,
        iter: &mut core::slice::Iter<'a, AccountView>,
        writable_owner: bool,
    ) -> Result<Self, ProgramError> {
        let owner_info = next_account_info(iter)?;
        let owner: Signer = if writable_owner {
            Signer::new_payer(owner_info)?
        } else {
            Signer::new(owner_info)?
        };
        let position: &AccountView = next_account_info(iter)?;
        let trader: &AccountView = next_account_info(iter)?;
        let market: &AccountView = next_account_info(iter)?;
        let system_program: &AccountView = next_account_info(iter)?;
        Program::new(system_program, &system_program::id())?;
        Program::new(next_account_info(iter)?, &manifest::ID)?;

        if !position.owned_by(as_raw_key(program_id)) {
            return Err(ProgramError::IllegalOwner);
        }
        let state: Position = {
            let bytes = position.try_borrow()?;
            if bytes.len() < POSITION_HEADER_SIZE {
                return Err(ProgramError::InvalidAccountData);
            }
            let state: Position = *bytemuck::from_bytes(&bytes[..POSITION_HEADER_SIZE]);
            if state.discriminant != POSITION_DISCRIMINANT
                || bytes.len()
                    != POSITION_HEADER_SIZE
                        + state.ladder_len as usize
                        + placed_bitmap_len(state.total_orders as usize)
            {
                return Err(ProgramError::InvalidAccountData);
            }
            state
        };
        if state.owner != *owner.pubkey() {
            return Err(ProgramError::IllegalOwner);
        }
        // Closing is final: a closed position cannot be re-closed or place
        // anything again.
        if state.is_closed() {
            return Err(ProgramError::AccountAlreadyInitialized);
        }
        if state.market != *market.pubkey() {
            return Err(ProgramError::InvalidArgument);
        }
        let expected_trader = Pubkey::create_program_address(
            &[
                TRADER_SEED,
                position.pubkey().as_ref(),
                &[state.trader_bump],
            ],
            program_id,
        )
        .map_err(|_| ProgramError::InvalidSeeds)?;
        if expected_trader != *trader.pubkey() {
            return Err(ProgramError::InvalidSeeds);
        }
        Ok(Open {
            owner,
            position,
            trader,
            market,
            system_program,
            trader_bump: state.trader_bump,
        })
    }

    /// The closing price: the first CancelOrders or ClosePosition records
    /// `price_e9`, and every later one keeps what it found, so the snapshot
    /// prices the position as of when its unwinding began.
    fn closing_price(&self, price_e9: u64) -> Result<u64, ProgramError> {
        if price_e9 == 0 {
            return Err(ProgramError::InvalidArgument);
        }
        let mut bytes = self.position.try_borrow_mut()?;
        let state: &mut Position = bytemuck::from_bytes_mut(&mut bytes[..POSITION_HEADER_SIZE]);
        if state.closed_with.price_e9 == 0 {
            state.closed_with.price_e9 = price_e9;
        }
        Ok(state.closed_with.price_e9)
    }

    fn trader_seeds(&self) -> [&[u8]; 3] {
        [
            TRADER_SEED,
            self.position.pubkey().as_ref(),
            core::slice::from_ref(&self.trader_bump),
        ]
    }

    /// Cancels every resting order of the trader, found by walking the whole
    /// book; how many it found.
    fn cancel_all(&self) -> Result<usize, ProgramError> {
        let cancels: Vec<CancelOrderParams> = {
            let data = self.market.try_borrow()?;
            let market: MarketRef = get_dynamic_account(&data);
            let index: DataIndex = market.get_trader_index(self.trader.pubkey());
            if index == NIL {
                return Ok(0);
            }
            market
                .get_bids()
                .iter::<RestingOrder>()
                .chain(market.get_asks().iter::<RestingOrder>())
                .filter(|(_, order)| order.get_trader_index() == index)
                .map(|(i, order)| {
                    CancelOrderParams::new_with_hint(order.get_sequence_number(), Some(i))
                })
                .collect()
        };
        self.cancel(cancels)
    }

    /// Cancels the trader's resting orders at these market indices, which
    /// must be strictly increasing. Costs what the list does, however full
    /// the book. An index that holds no order of the trader (any more) is
    /// skipped: the order filled or moved since the client read the book.
    fn cancel_at(&self, indices: &[DataIndex]) -> Result<usize, ProgramError> {
        if indices.windows(2).any(|w| w[0] >= w[1]) {
            return Err(ProgramError::InvalidInstructionData);
        }
        let cancels: Vec<CancelOrderParams> = {
            let data = self.market.try_borrow()?;
            let market: MarketRef = get_dynamic_account(&data);
            let trader: DataIndex = market.get_trader_index(self.trader.pubkey());
            if trader == NIL {
                return Ok(0);
            }
            // The indices come from the caller: get_order_by_index trusts
            // its index, so only whole blocks inside the market that hold a
            // resting order get that far.
            indices
                .iter()
                .filter(|&&i| {
                    i % MARKET_BLOCK_SIZE as DataIndex == 0
                        && (i as usize + MARKET_BLOCK_SIZE) <= market.dynamic.len()
                })
                .map(|&i| (i, get_helper::<RBNode<RestingOrder>>(market.dynamic, i)))
                .filter(|(_, node)| {
                    node.get_payload_type() == MarketDataTreeNodeType::RestingOrder as u8
                        && node.get_value().get_trader_index() == trader
                })
                .map(|(i, node)| {
                    CancelOrderParams::new_with_hint(
                        node.get_value().get_sequence_number(),
                        Some(i),
                    )
                })
                .collect()
        };
        self.cancel(cancels)
    }

    fn cancel(&self, cancels: Vec<CancelOrderParams>) -> Result<usize, ProgramError> {
        let found = cancels.len();
        if found > 0 {
            invoke_signed(
                &batch_update_instruction(
                    self.market.pubkey(),
                    self.trader.pubkey(),
                    None,
                    cancels,
                    vec![],
                    None,
                    None,
                    None,
                    None,
                ),
                &[self.trader, self.market, self.system_program],
                &[&self.trader_seeds()],
            )?;
        }
        Ok(found)
    }
}

/// One side of the market as the trading instructions see it.
struct Side<'a> {
    mint: &'a AccountView,
    owner_token: &'a AccountView,
    trader_token: &'a AccountView,
    vault: &'a AccountView,
    token_program: &'a AccountView,
    decimals: u8,
}

/// The accounts PlaceOrders and ClosePosition take, checked.
struct Trading<'a> {
    open: Open<'a>,
    base: Side<'a>,
    quote: Side<'a>,
}

fn token_amount(account: &AccountView) -> Result<u64, ProgramError> {
    let data = account.try_borrow()?;
    if data.len() < 72 {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(u64::from_le_bytes(data[64..72].try_into().unwrap()))
}

impl<'a> Trading<'a> {
    fn load(program_id: &Pubkey, accounts: &'a [AccountView]) -> Result<Self, ProgramError> {
        let iter = &mut accounts.iter();
        let open = Open::load(program_id, iter, true)?;
        let base_mint = next_account_info(iter)?;
        let quote_mint = next_account_info(iter)?;
        let owner_base = next_account_info(iter)?;
        let owner_quote = next_account_info(iter)?;
        let trader_base = next_account_info(iter)?;
        let trader_quote = next_account_info(iter)?;
        let base_vault = next_account_info(iter)?;
        let quote_vault = next_account_info(iter)?;
        let base_token_program = next_account_info(iter)?;
        let quote_token_program = next_account_info(iter)?;

        let market = ManifestAccountInfo::<MarketFixed>::new(open.market)?;
        let fixed = market.get_fixed()?;
        if fixed.get_base_mint() != base_mint.pubkey()
            || fixed.get_quote_mint() != quote_mint.pubkey()
            || fixed.get_base_vault() != base_vault.pubkey()
            || fixed.get_quote_vault() != quote_vault.pubkey()
        {
            return Err(ProgramError::InvalidArgument);
        }
        // Withdrawals leave through the owner's token accounts, so they must
        // be the owner's. Mints and balances the token program checks.
        for account in [owner_base, owner_quote] {
            let data = account.try_borrow()?;
            if data.len() < 72 || data[32..64] != *open.owner.pubkey().as_ref() {
                return Err(ProgramError::IllegalOwner);
            }
        }
        for program in [base_token_program, quote_token_program] {
            if *program.pubkey() != spl_token_2022::id() && *program.pubkey() != spl_token::id() {
                return Err(ProgramError::IncorrectProgramId);
            }
        }
        let (base_decimals, quote_decimals) = (
            fixed.get_base_mint_decimals(),
            fixed.get_quote_mint_decimals(),
        );
        drop(fixed);
        Ok(Trading {
            open,
            base: Side {
                mint: base_mint,
                owner_token: owner_base,
                trader_token: trader_base,
                vault: base_vault,
                token_program: base_token_program,
                decimals: base_decimals,
            },
            quote: Side {
                mint: quote_mint,
                owner_token: owner_quote,
                trader_token: trader_quote,
                vault: quote_vault,
                token_program: quote_token_program,
                decimals: quote_decimals,
            },
        })
    }

    /// Owner's wallet → trader's token account → seat.
    fn deposit(&self, side: &Side, amount: u64) -> ProgramResult {
        if amount == 0 {
            return Ok(());
        }
        let owner = self.open.owner.info;
        invoke(
            &spl_token_2022::instruction::transfer_checked(
                side.token_program.pubkey(),
                side.owner_token.pubkey(),
                side.mint.pubkey(),
                side.trader_token.pubkey(),
                owner.pubkey(),
                &[],
                amount,
                side.decimals,
            )
            .map_err(to_program_error)?,
            &[side.owner_token, side.mint, side.trader_token, owner],
        )?;
        invoke_signed(
            &deposit_instruction(
                self.open.market.pubkey(),
                self.open.trader.pubkey(),
                side.mint.pubkey(),
                amount,
                side.trader_token.pubkey(),
                *side.token_program.pubkey(),
                None,
            ),
            &[
                self.open.trader,
                self.open.market,
                side.trader_token,
                side.vault,
                side.token_program,
                side.mint,
            ],
            &[&self.open.trader_seeds()],
        )
    }

    /// Seat → trader's token account → owner's wallet, then closes the
    /// trader's token account into the owner. Tokens sent straight to that
    /// account go back with the rest but are not the position's: the seat
    /// balance is what it returns.
    fn withdraw_all(&self, side: &Side, seat_balance: u64) -> Result<u64, ProgramError> {
        let seeds = self.open.trader_seeds();
        let (trader, owner) = (self.open.trader, self.open.owner.info);
        if seat_balance > 0 {
            invoke_signed(
                &withdraw_instruction(
                    self.open.market.pubkey(),
                    trader.pubkey(),
                    side.mint.pubkey(),
                    seat_balance,
                    side.trader_token.pubkey(),
                    *side.token_program.pubkey(),
                    None,
                ),
                &[
                    trader,
                    self.open.market,
                    side.trader_token,
                    side.vault,
                    side.token_program,
                    side.mint,
                ],
                &[&seeds],
            )?;
        }
        let amount = token_amount(side.trader_token)?;
        if amount > 0 {
            invoke_signed(
                &spl_token_2022::instruction::transfer_checked(
                    side.token_program.pubkey(),
                    side.trader_token.pubkey(),
                    side.mint.pubkey(),
                    side.owner_token.pubkey(),
                    trader.pubkey(),
                    &[],
                    amount,
                    side.decimals,
                )
                .map_err(to_program_error)?,
                &[side.trader_token, side.mint, side.owner_token, trader],
                &[&seeds],
            )?;
        }
        invoke_signed(
            &spl_token_2022::instruction::close_account(
                side.token_program.pubkey(),
                side.trader_token.pubkey(),
                owner.pubkey(),
                trader.pubkey(),
                &[],
            )
            .map_err(to_program_error)?,
            &[side.trader_token, owner, trader],
            &[&seeds],
        )?;
        Ok(seat_balance)
    }
}

pub(crate) fn process_place_orders(
    program_id: &Pubkey,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let ctx = Trading::load(program_id, accounts)?;
    // Each mint's global account and global vault: with them an order that
    // crosses a global order trades with it (dust within the 1% below)
    // instead of failing the batch. Manifest checks them, and skips a mint
    // without a global account.
    let [base_global, base_global_vault, quote_global, quote_global_vault] =
        &accounts[TRADING_ACCOUNTS..]
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if data.is_empty() || !data.len().is_multiple_of(2) || data.len() / 2 > MAX_ORDERS_PER_PLACE {
        return Err(ProgramError::InvalidInstructionData);
    }

    // Mark the orders placed and work out what they need, before any call
    // out, so the position is not borrowed across them.
    let mut orders: Vec<LadderOrder> = Vec::with_capacity(data.len() / 2);
    let (mut need_base, mut need_quote) = (0u64, 0u64);
    {
        let mut bytes = ctx.open.position.try_borrow_mut()?;
        let (header, rest) = bytes.split_at_mut(POSITION_HEADER_SIZE);
        let state: &mut Position = bytemuck::from_bytes_mut(header);
        // Once closing began, the ladder only shrinks.
        if state.is_closing() {
            return Err(ProgramError::AccountAlreadyInitialized);
        }
        let (ladder_bytes, placed) = rest.split_at_mut(state.ladder_len as usize);
        let ladder = Ladder::checked(ladder_bytes);
        for k in data.chunks_exact(2) {
            let k = u16::from_le_bytes([k[0], k[1]]) as usize;
            if k >= state.total_orders as usize {
                return Err(ProgramError::InvalidArgument);
            }
            let (byte, bit) = (k / 8, 1u8 << (k % 8));
            if placed[byte] & bit != 0 {
                return Err(ProgramError::InvalidArgument);
            }
            placed[byte] |= bit;
            // Orders that round to nothing count as placed without an order.
            if let Some(order) = ladder.order(
                k,
                state.created_with.price_e9,
                ctx.base.decimals,
                ctx.quote.decimals,
            )? {
                let need = if order.is_bid {
                    &mut need_quote
                } else {
                    &mut need_base
                };
                *need = need
                    .checked_add(if order.is_bid {
                        order.quote_atoms
                    } else {
                        order.base_atoms
                    })
                    .ok_or(ProgramError::ArithmeticOverflow)?;
                orders.push(order);
            }
        }
        state.placed_orders += (data.len() / 2) as u16;
        state.created_with.base_atoms = state
            .created_with
            .base_atoms
            .checked_add(need_base)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        state.created_with.quote_atoms = state
            .created_with
            .quote_atoms
            .checked_add(need_quote)
            .ok_or(ProgramError::ArithmeticOverflow)?;
    }

    ctx.deposit(&ctx.base, need_base)?;
    ctx.deposit(&ctx.quote, need_quote)?;
    if orders.is_empty() {
        return Ok(());
    }

    let (owner, trader, market) = (ctx.open.owner.info, ctx.open.trader, ctx.open.market);
    // Room for every order plus the five spare blocks Manifest tops its
    // reserve up to after a batch, so the batch itself never grows the market.
    let system_program = ctx.open.system_program;
    expand_market(owner, market, system_program, orders.len() as u32 + 5)?;
    // Manifest charges a batch with reverse orders a flat fee from its payer.
    // Taking someone's reverse order needs no block beyond these: an order
    // that leaves a maker partly filled is used up itself and fails the
    // resting check, and a maker it empties flips into the block it frees.
    invoke(
        &system_instruction::transfer(owner.pubkey(), trader.pubkey(), ADVANCED_ORDER_FEE_LAMPORTS),
        &[owner, trader],
    )?;
    let params: Vec<PlaceOrderParams> = orders
        .iter()
        .map(|o| {
            let (order_type, spread) = if o.spread_e1 < REVERSE_TIGHT_BELOW_E1 {
                (OrderType::ReverseTight, o.spread_e1 as u32 * 1000)
            } else {
                (OrderType::Reverse, o.spread_e1 as u32)
            };
            // Reverse orders carry their spread where others carry an expiry.
            PlaceOrderParams::new(
                o.base_atoms,
                o.price_mantissa,
                o.price_exponent,
                o.is_bid,
                order_type,
                spread,
            )
        })
        .collect();
    let (base, quote) = (&ctx.base, &ctx.quote);
    let instruction = batch_update_instruction(
        market.pubkey(),
        trader.pubkey(),
        None,
        vec![],
        params,
        Some(*base.mint.pubkey()),
        Some(*base.token_program.pubkey()),
        Some(*quote.mint.pubkey()),
        Some(*quote.token_program.pubkey()),
    );
    let trader_seeds = ctx.open.trader_seeds();
    // Manifest's order per side: mint, global, global vault, market vault,
    // token program.
    invoke_signed(
        &instruction,
        &[
            trader,
            market,
            system_program,
            base.mint,
            base_global,
            base_global_vault,
            base.vault,
            base.token_program,
            quote.mint,
            quote_global,
            quote_global_vault,
            quote.vault,
            quote.token_program,
        ],
        &[&trader_seeds],
    )?;
    check_resting(market, trader, &orders)
}

/// Every order of the batch rests with at least MIN_RESTING_PERCENT of its
/// size, read back where Manifest reports it rests (sequence number, index).
fn check_resting(
    market: &AccountView,
    trader: &AccountView,
    orders: &[LadderOrder],
) -> ProgramResult {
    let (program, result) = get_return_data().ok_or(ProgramError::InvalidAccountData)?;
    if program != manifest::ID || result.len() != 4 + 12 * orders.len() {
        return Err(ProgramError::InvalidAccountData);
    }
    let data = market.try_borrow()?;
    let market: MarketRef = get_dynamic_account(&data);
    let trader_index: DataIndex = market.get_trader_index(trader.pubkey());
    for (order, record) in orders.iter().zip(result[4..].chunks_exact(12)) {
        let sequence_number = u64::from_le_bytes(record[..8].try_into().unwrap());
        let index = DataIndex::from_le_bytes(record[8..].try_into().unwrap());
        let resting = if index == NIL {
            0
        } else {
            let resting = market.get_order_by_index(index);
            if resting.get_sequence_number() != sequence_number
                || resting.get_trader_index() != trader_index
            {
                return Err(ProgramError::InvalidAccountData);
            }
            resting.get_num_base_atoms().as_u64()
        };
        if (resting as u128) * 100 < order.base_atoms as u128 * MIN_RESTING_PERCENT {
            return Err(ProgramError::Custom(ERROR_ORDER_CROSSED));
        }
    }
    Ok(())
}

pub(crate) fn process_close_position(
    program_id: &Pubkey,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let ctx = Trading::load(program_id, accounts)?;
    if data.len() != 8 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let price_e9 = ctx
        .open
        .closing_price(u64::from_le_bytes(data.try_into().unwrap()))?;

    ctx.open.cancel_all()?;
    let (base, quote) = {
        let data = ctx.open.market.try_borrow()?;
        let market: MarketRef = get_dynamic_account(&data);
        let (base, quote) = market.get_trader_balance(ctx.open.trader.pubkey());
        (base.as_u64(), quote.as_u64())
    };
    let base = ctx.withdraw_all(&ctx.base, base)?;
    let quote = ctx.withdraw_all(&ctx.quote, quote)?;

    // Everything the trader still holds goes back to the owner; the account
    // disappears with its last lamport.
    let (owner, trader) = (ctx.open.owner.info, ctx.open.trader);
    invoke_signed(
        &system_instruction::transfer(trader.pubkey(), owner.pubkey(), trader.lamports()),
        &[trader, owner],
        &[&ctx.open.trader_seeds()],
    )?;

    let now = Clock::get()?.unix_timestamp;
    let mut bytes = ctx.open.position.try_borrow_mut()?;
    let state: &mut Position = bytemuck::from_bytes_mut(&mut bytes[..POSITION_HEADER_SIZE]);
    state.closed_at = now.max(state.created_at).max(1);
    state.closed_with = Snapshot {
        base_atoms: base,
        quote_atoms: quote,
        price_e9,
    };
    Ok(())
}

pub(crate) fn process_cancel_orders(
    program_id: &Pubkey,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let open = Open::load(program_id, &mut accounts.iter(), false)?;
    if data.len() < 8 || (data.len() - 8) % 4 != 0 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let (price_e9, indices) = data.split_at(8);
    open.closing_price(u64::from_le_bytes(price_e9.try_into().unwrap()))?;
    let indices: Vec<DataIndex> = indices
        .chunks_exact(4)
        .map(|b| DataIndex::from_le_bytes(b.try_into().unwrap()))
        .collect();
    open.cancel_at(&indices)?;
    Ok(())
}
