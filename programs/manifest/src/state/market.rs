#[cfg(feature = "certora")]
use {crate::certora::hooks::*, hook_macro::cvt_hook_end, nondet::nondet};

use bytemuck::{Pod, Zeroable};
use hypertree::{
    get_helper, get_mut_helper, is_not_nil, trace, DataIndex, FreeList, FreeListNode, Get, PodBool,
    RBNode, NIL,
};
#[cfg(not(feature = "certora"))]
use hypertree::{
    HyperTreeReadOperations, HyperTreeValueIteratorTrait, HyperTreeWriteOperations, RedBlackTree,
    RedBlackTreeReadOnly,
};
use shank::ShankType;
use solana_program::{entrypoint::ProgramResult, program_error::ProgramError, pubkey::Pubkey};
use static_assertions::const_assert_eq;
use std::mem::size_of;

use crate::{
    logs::{emit_stack, FillLog},
    program::{batch_update::MarketDataTreeNodeType, ManifestError},
    quantities::{BaseAtoms, GlobalAtoms, QuoteAtoms, QuoteAtomsPerBaseAtom, WrapperU64},
    require,
    state::{
        utils::{assert_can_take, remove_from_global, try_to_move_global_tokens},
        OrderType,
    },
    validation::{
        get_vault_address, loaders::GlobalTradeAccounts, ManifestAccount, MintAccountInfo,
    },
};

use super::{
    claimed_seat::ClaimedSeat,
    constants::{MARKET_BLOCK_SIZE, MARKET_FIXED_SIZE},
    order_type_can_rest,
    utils::{
        assert_already_has_seat, assert_not_already_expired, can_back_order, get_now_slot,
        try_to_add_to_global,
    },
    DerefOrBorrow, DerefOrBorrowMut, DynamicAccount, RestingOrder, MARKET_FIXED_DISCRIMINANT,
    MARKET_FREE_LIST_BLOCK_SIZE, NO_EXPIRATION_LAST_VALID_SLOT,
};

#[path = "market_helpers.rs"]
pub mod market_helpers;
use market_helpers::*;

#[path = "cvt_munge.rs"]
#[cfg(feature = "certora")]
mod cvt_munge;
#[cfg(feature = "certora")]
pub use cvt_munge::*;

#[cfg(not(feature = "certora"))]
mod helpers {
    use super::*;
    /// Read a `RBNode<ClaimedSeat>` in an array of data at a given index.
    pub fn get_helper_seat(data: &[u8], index: DataIndex) -> &RBNode<ClaimedSeat> {
        get_helper::<RBNode<ClaimedSeat>>(data, index)
    }
    /// Read a `RBNode<ClaimedSeat>` in an array of data at a given index.
    pub fn get_mut_helper_seat(data: &mut [u8], index: DataIndex) -> &mut RBNode<ClaimedSeat> {
        get_mut_helper::<RBNode<ClaimedSeat>>(data, index)
    }

    pub fn get_helper_order(data: &[u8], index: DataIndex) -> &RBNode<RestingOrder> {
        get_helper::<RBNode<RestingOrder>>(data, index)
    }
    pub fn get_mut_helper_order(data: &mut [u8], index: DataIndex) -> &mut RBNode<RestingOrder> {
        get_mut_helper::<RBNode<RestingOrder>>(data, index)
    }

    pub fn get_helper_bid_order(data: &[u8], index: DataIndex) -> &RBNode<RestingOrder> {
        get_helper::<RBNode<RestingOrder>>(data, index)
    }
    pub fn get_mut_helper_bid_order(
        data: &mut [u8],
        index: DataIndex,
    ) -> &mut RBNode<RestingOrder> {
        get_mut_helper::<RBNode<RestingOrder>>(data, index)
    }

    pub fn get_helper_ask_order(data: &[u8], index: DataIndex) -> &RBNode<RestingOrder> {
        get_helper::<RBNode<RestingOrder>>(data, index)
    }
    pub fn get_mut_helper_ask_order(
        data: &mut [u8],
        index: DataIndex,
    ) -> &mut RBNode<RestingOrder> {
        get_mut_helper::<RBNode<RestingOrder>>(data, index)
    }
}

#[cfg(not(feature = "certora"))]
pub use helpers::*;

#[derive(Clone)]
pub struct AddOrderToMarketArgs<'a, 'info> {
    pub market: Pubkey,
    pub trader_index: DataIndex,
    pub num_base_atoms: BaseAtoms,
    pub price: QuoteAtomsPerBaseAtom,
    pub is_bid: bool,
    pub last_valid_slot: u32,
    pub order_type: OrderType,
    pub global_trade_accounts_opts: &'a [Option<GlobalTradeAccounts<'a, 'info>>; 2],
    pub current_slot: Option<u32>,
}

pub struct AddOrderToMarketResult {
    pub order_sequence_number: u64,
    pub order_index: DataIndex,
    pub base_atoms_traded: BaseAtoms,
    pub quote_atoms_traded: QuoteAtoms,
}

#[repr(C, packed)]
#[derive(Default, Copy, Clone, Pod, Zeroable)]
pub struct MarketUnusedFreeListPadding {
    _padding: [u64; 9],
    _padding2: [u8; 4],
}
// 4 bytes are for the free list, rest is payload.
const_assert_eq!(
    size_of::<MarketUnusedFreeListPadding>(),
    MARKET_FREE_LIST_BLOCK_SIZE
);
// Does not need to align to word boundaries because does not deserialize.

#[repr(C)]
#[derive(Default, Copy, Clone, Zeroable, Pod, ShankType)]
pub struct MarketFixed {
    /// Discriminant for identifying this type of account.
    pub discriminant: u64,

    /// Version
    version: u8,
    /// Index identifying the base asset (e.g., 0=SOL, 1=ETH).
    /// Used in PDA seeds: [b"market", &[base_mint_index], quote_mint]
    base_mint_index: u8,
    base_mint_decimals: u8,
    quote_mint_decimals: u8,
    _padding1: [u8; 4],

    /// Quote mint (USDC)
    quote_mint: Pubkey,

    /// The sequence number of the next order.
    order_sequence_number: u64,

    /// Num bytes allocated as RestingOrder or ClaimedSeat or FreeList. Does not
    /// include the fixed bytes.
    num_bytes_allocated: u32,

    /// Red-black tree root representing the bids in the order book.
    bids_root_index: DataIndex,
    bids_best_index: DataIndex,

    /// Red-black tree root representing the asks in the order book.
    asks_root_index: DataIndex,
    asks_best_index: DataIndex,

    /// Red-black tree root representing the seats
    claimed_seats_root_index: DataIndex,

    /// LinkedList representing all free blocks that could be used for ClaimedSeats or RestingOrders
    free_list_head_index: DataIndex,

    _padding2: [u32; 1],

    /// Quote volume traded over lifetime, can overflow. This is for
    /// informational and monitoring purposes only. This is not guaranteed to
    /// be maintained. It does not secure any value in manifest.
    /// Use at your own risk.
    quote_volume: QuoteAtoms,

    // These are not included in the normal usage because they are informational
    // only and not worth the CU.
    #[cfg(feature = "certora")]
    /// Base tokens reserved for seats
    withdrawable_base_atoms: BaseAtoms,
    #[cfg(feature = "certora")]
    /// Quote tokens reserved for seats
    withdrawable_quote_atoms: QuoteAtoms,
    #[cfg(feature = "certora")]
    /// Base tokens reserved for non-global orders
    pub orderbook_base_atoms: BaseAtoms,
    #[cfg(feature = "certora")]
    /// Quote tokens reserved for non-global orders
    pub orderbook_quote_atoms: QuoteAtoms,
    #[cfg(feature = "certora")]
    /// Initial margin in basis points (e.g., 1000 = 10% = 10x leverage)
    initial_margin_bps: u64,
    #[cfg(feature = "certora")]
    /// Maintenance margin in basis points (e.g., 500 = 5%)
    maintenance_margin_bps: u64,
    #[cfg(feature = "certora")]
    pyth_feed_account: Pubkey,
    #[cfg(feature = "certora")]
    oracle_price_mantissa: u64,
    #[cfg(feature = "certora")]
    oracle_price_expo_and_pad: u64,
    #[cfg(feature = "certora")]
    last_funding_timestamp: u64,
    #[cfg(feature = "certora")]
    cumulative_funding: u64,
    #[cfg(feature = "certora")]
    insurance_fund_balance: u64,
    #[cfg(feature = "certora")]
    taker_fee_bps: u64,
    #[cfg(feature = "certora")]
    liquidation_buffer_bps: u64,
    #[cfg(feature = "certora")]
    _padding3: [u64; 3],

    /// Initial margin in basis points (e.g., 1000 = 10% = 10x leverage)
    #[cfg(not(feature = "certora"))]
    initial_margin_bps: u64,
    /// Maintenance margin in basis points (e.g., 500 = 5%)
    #[cfg(not(feature = "certora"))]
    maintenance_margin_bps: u64,
    /// Total open long position in base atoms across all traders
    #[cfg(not(feature = "certora"))]
    total_long_base_atoms: u64,
    /// Total open short position in base atoms across all traders
    #[cfg(not(feature = "certora"))]
    total_short_base_atoms: u64,
    /// Pyth price feed account expected for this market
    #[cfg(not(feature = "certora"))]
    pyth_feed_account: Pubkey,
    /// Cached oracle price mantissa (updated by CrankFunding)
    #[cfg(not(feature = "certora"))]
    oracle_price_mantissa: u64,
    /// Cached oracle price exponent in lower 32 bits (i32), upper 32 bits padding
    #[cfg(not(feature = "certora"))]
    oracle_price_expo_and_pad: u64,
    /// Unix timestamp of last funding crank
    #[cfg(not(feature = "certora"))]
    last_funding_timestamp: u64,
    /// Cumulative funding per base atom, scaled by 1e9. Stored as u64, interpret as i64.
    #[cfg(not(feature = "certora"))]
    cumulative_funding: u64,
    /// Insurance fund balance (virtual USDC ledger for bad debt coverage)
    #[cfg(not(feature = "certora"))]
    insurance_fund_balance: u64,
    /// Taker fee in basis points (e.g., 5 = 0.05%)
    #[cfg(not(feature = "certora"))]
    taker_fee_bps: u64,
    /// Buffer above maintenance margin to target after partial liquidation (basis points)
    #[cfg(not(feature = "certora"))]
    liquidation_buffer_bps: u64,
    #[cfg(not(feature = "certora"))]
    _padding3: [u64; 5],
}
const_assert_eq!(
    size_of::<MarketFixed>(),
    8 +   // discriminant
    1 +   // version
    1 +   // base_mint_index
    1 +   // base_mint_decimals
    1 +   // quote_mint_decimals
    4 +   // padding1
    32 +  // quote_mint
    8 +   // order_sequence_number
    4 +   // num_bytes_allocated
    4 +   // bids_root_index
    4 +   // bids_best_index
    4 +   // asks_root_index
    4 +   // asks_best_index
    4 +   // claimed_seats_root_index
    4 +   // free_list_head_index
    4 +   // padding2
    8 +   // quote_volume
    160   // perps + padding
);
const_assert_eq!(size_of::<MarketFixed>(), MARKET_FIXED_SIZE);
const_assert_eq!(size_of::<MarketFixed>() % 8, 0);
impl Get for MarketFixed {}

impl MarketFixed {
    pub fn new_empty(
        base_mint_index: u8,
        base_mint_decimals: u8,
        quote_mint: &MintAccountInfo,
    ) -> Self {
        MarketFixed {
            discriminant: MARKET_FIXED_DISCRIMINANT,
            version: 0,
            base_mint_index,
            base_mint_decimals,
            quote_mint_decimals: quote_mint.mint.decimals,
            _padding1: [0; 4],
            quote_mint: *quote_mint.info.key,
            order_sequence_number: 0,
            num_bytes_allocated: 0,
            bids_root_index: NIL,
            bids_best_index: NIL,
            asks_root_index: NIL,
            asks_best_index: NIL,
            claimed_seats_root_index: NIL,
            #[cfg(not(feature = "certora"))]
            free_list_head_index: NIL,
            #[cfg(feature = "certora")]
            // non NIL
            free_list_head_index: 0,
            _padding2: [0; 1],
            quote_volume: QuoteAtoms::ZERO,
            #[cfg(not(feature = "certora"))]
            initial_margin_bps: 0,
            #[cfg(not(feature = "certora"))]
            maintenance_margin_bps: 0,
            #[cfg(not(feature = "certora"))]
            total_long_base_atoms: 0,
            #[cfg(not(feature = "certora"))]
            total_short_base_atoms: 0,
            #[cfg(not(feature = "certora"))]
            pyth_feed_account: Pubkey::default(),
            #[cfg(not(feature = "certora"))]
            oracle_price_mantissa: 0,
            #[cfg(not(feature = "certora"))]
            oracle_price_expo_and_pad: 0,
            #[cfg(not(feature = "certora"))]
            last_funding_timestamp: 0,
            #[cfg(not(feature = "certora"))]
            cumulative_funding: 0,
            #[cfg(not(feature = "certora"))]
            insurance_fund_balance: 0,
            #[cfg(not(feature = "certora"))]
            taker_fee_bps: 0,
            #[cfg(not(feature = "certora"))]
            liquidation_buffer_bps: 0,
            #[cfg(not(feature = "certora"))]
            _padding3: [0; 5],
            #[cfg(feature = "certora")]
            withdrawable_base_atoms: BaseAtoms::new(0),
            #[cfg(feature = "certora")]
            withdrawable_quote_atoms: QuoteAtoms::new(0),
            #[cfg(feature = "certora")]
            orderbook_base_atoms: BaseAtoms::new(0),
            #[cfg(feature = "certora")]
            orderbook_quote_atoms: QuoteAtoms::new(0),
            #[cfg(feature = "certora")]
            initial_margin_bps: 0,
            #[cfg(feature = "certora")]
            maintenance_margin_bps: 0,
            #[cfg(feature = "certora")]
            pyth_feed_account: Pubkey::default(),
            #[cfg(feature = "certora")]
            oracle_price_mantissa: 0,
            #[cfg(feature = "certora")]
            oracle_price_expo_and_pad: 0,
            #[cfg(feature = "certora")]
            last_funding_timestamp: 0,
            #[cfg(feature = "certora")]
            cumulative_funding: 0,
            #[cfg(feature = "certora")]
            insurance_fund_balance: 0,
            #[cfg(feature = "certora")]
            taker_fee_bps: 0,
            #[cfg(feature = "certora")]
            liquidation_buffer_bps: 0,
            #[cfg(feature = "certora")]
            _padding3: [0; 3],
        }
    }

    #[cfg(feature = "certora")]
    /** All fields are set to nondeterministic values except indexes to the tree **/
    pub fn new_nondet() -> Self {
        let claimed_seats_root_index = nondet();
        cvt::cvt_assume!(claimed_seats_root_index == NIL);
        MarketFixed {
            discriminant: MARKET_FIXED_DISCRIMINANT,
            version: 0,
            base_mint_index: nondet(),
            base_mint_decimals: nondet(),
            quote_mint_decimals: nondet(),
            _padding1: [0; 4],
            quote_mint: nondet(),
            order_sequence_number: nondet(),
            num_bytes_allocated: nondet(),
            bids_root_index: NIL,
            bids_best_index: NIL,
            asks_root_index: NIL,
            asks_best_index: NIL,
            claimed_seats_root_index,
            free_list_head_index: 0,
            _padding2: [0; 1],
            quote_volume: QuoteAtoms::ZERO,
            withdrawable_base_atoms: BaseAtoms::new(nondet()),
            withdrawable_quote_atoms: QuoteAtoms::new(nondet()),
            orderbook_base_atoms: BaseAtoms::new(nondet()),
            orderbook_quote_atoms: QuoteAtoms::new(nondet()),
            initial_margin_bps: 0,
            maintenance_margin_bps: 0,
            pyth_feed_account: Pubkey::default(),
            oracle_price_mantissa: 0,
            oracle_price_expo_and_pad: 0,
            last_funding_timestamp: 0,
            cumulative_funding: 0,
            insurance_fund_balance: 0,
            taker_fee_bps: 0,
            liquidation_buffer_bps: 0,
            _padding3: [0; 3],
        }
    }

    pub fn get_base_mint_index(&self) -> u8 {
        self.base_mint_index
    }
    pub fn get_quote_mint(&self) -> &Pubkey {
        &self.quote_mint
    }
    pub fn get_base_mint_decimals(&self) -> u8 {
        self.base_mint_decimals
    }
    pub fn get_quote_mint_decimals(&self) -> u8 {
        self.quote_mint_decimals
    }
    pub fn get_quote_volume(&self) -> QuoteAtoms {
        self.quote_volume
    }

    // Used only in this file to construct iterator
    pub(crate) fn get_bids_root_index(&self) -> DataIndex {
        self.bids_root_index
    }
    pub(crate) fn get_asks_root_index(&self) -> DataIndex {
        self.asks_root_index
    }
    pub(crate) fn get_bids_best_index(&self) -> DataIndex {
        self.bids_best_index
    }
    pub(crate) fn get_asks_best_index(&self) -> DataIndex {
        self.asks_best_index
    }
    pub(crate) fn get_claimed_seats_root_index(&self) -> DataIndex {
        self.claimed_seats_root_index
    }

    #[cfg(feature = "certora")]
    pub fn get_withdrawable_base_atoms(&self) -> BaseAtoms {
        self.withdrawable_base_atoms
    }
    #[cfg(feature = "certora")]
    pub fn get_withdrawable_quote_atoms(&self) -> QuoteAtoms {
        self.withdrawable_quote_atoms
    }
    #[cfg(feature = "certora")]
    pub fn get_orderbook_base_atoms(&self) -> BaseAtoms {
        self.orderbook_base_atoms
    }
    #[cfg(feature = "certora")]
    pub fn get_orderbook_quote_atoms(&self) -> QuoteAtoms {
        self.orderbook_quote_atoms
    }

    // Used in benchmark
    pub fn has_free_block(&self) -> bool {
        self.free_list_head_index != NIL
    }

    pub fn get_initial_margin_bps(&self) -> u64 {
        self.initial_margin_bps
    }
    pub fn get_maintenance_margin_bps(&self) -> u64 {
        self.maintenance_margin_bps
    }
    #[cfg(not(feature = "certora"))]
    pub fn get_total_long_base_atoms(&self) -> u64 {
        self.total_long_base_atoms
    }
    #[cfg(not(feature = "certora"))]
    pub fn get_total_short_base_atoms(&self) -> u64 {
        self.total_short_base_atoms
    }
    pub fn set_perps_params(
        &mut self,
        initial_margin_bps: u64,
        maintenance_margin_bps: u64,
    ) {
        self.initial_margin_bps = initial_margin_bps;
        self.maintenance_margin_bps = maintenance_margin_bps;
    }
    #[cfg(not(feature = "certora"))]
    pub fn set_total_long_base_atoms(&mut self, val: u64) {
        self.total_long_base_atoms = val;
    }
    #[cfg(not(feature = "certora"))]
    pub fn set_total_short_base_atoms(&mut self, val: u64) {
        self.total_short_base_atoms = val;
    }

    pub fn get_pyth_feed(&self) -> &Pubkey {
        &self.pyth_feed_account
    }
    pub fn set_pyth_feed(&mut self, feed: Pubkey) {
        self.pyth_feed_account = feed;
    }
    pub fn get_oracle_price_mantissa(&self) -> u64 {
        self.oracle_price_mantissa
    }
    pub fn get_oracle_price_expo(&self) -> i32 {
        self.oracle_price_expo_and_pad as i32
    }
    pub fn set_oracle_price(&mut self, mantissa: u64, expo: i32) {
        self.oracle_price_mantissa = mantissa;
        self.oracle_price_expo_and_pad = expo as u32 as u64;
    }
    pub fn get_last_funding_timestamp(&self) -> i64 {
        self.last_funding_timestamp as i64
    }
    pub fn set_last_funding_timestamp(&mut self, ts: i64) {
        self.last_funding_timestamp = ts as u64;
    }
    pub fn get_cumulative_funding(&self) -> i64 {
        self.cumulative_funding as i64
    }
    pub fn set_cumulative_funding(&mut self, val: i64) {
        self.cumulative_funding = val as u64;
    }

    pub fn get_insurance_fund_balance(&self) -> u64 {
        self.insurance_fund_balance
    }
    pub fn set_insurance_fund_balance(&mut self, val: u64) {
        self.insurance_fund_balance = val;
    }
    pub fn add_to_insurance_fund(&mut self, amount: u64) {
        self.insurance_fund_balance = self.insurance_fund_balance.saturating_add(amount);
    }
    /// Draw from insurance fund, returns the amount actually drawn (capped by balance).
    pub fn draw_from_insurance_fund(&mut self, amount: u64) -> u64 {
        let drawn = amount.min(self.insurance_fund_balance);
        self.insurance_fund_balance = self.insurance_fund_balance.saturating_sub(drawn);
        drawn
    }
    pub fn get_taker_fee_bps(&self) -> u64 {
        self.taker_fee_bps
    }
    pub fn set_taker_fee_bps(&mut self, val: u64) {
        self.taker_fee_bps = val;
    }
    pub fn get_liquidation_buffer_bps(&self) -> u64 {
        self.liquidation_buffer_bps
    }
    pub fn set_liquidation_buffer_bps(&mut self, val: u64) {
        self.liquidation_buffer_bps = val;
    }
}

impl ManifestAccount for MarketFixed {
    fn verify_discriminant(&self) -> ProgramResult {
        require!(
            self.discriminant == MARKET_FIXED_DISCRIMINANT,
            ProgramError::InvalidAccountData,
            "Invalid market discriminant actual: {} expected: {}",
            self.discriminant,
            MARKET_FIXED_DISCRIMINANT
        )?;
        Ok(())
    }
}

/// Fully owned Market, used in clients that can copy.
pub type MarketValue = DynamicAccount<MarketFixed, Vec<u8>>;
/// Full market reference type.
pub type MarketRef<'a> = DynamicAccount<&'a MarketFixed, &'a [u8]>;
/// Full market reference type.
pub type MarketRefMut<'a> = DynamicAccount<&'a mut MarketFixed, &'a mut [u8]>;

#[cfg(not(feature = "certora"))]
mod types {
    use super::*;
    pub type ClaimedSeatTree<'a> = RedBlackTree<'a, ClaimedSeat>;
    pub type ClaimedSeatTreeReadOnly<'a> = RedBlackTreeReadOnly<'a, ClaimedSeat>;
    pub type Bookside<'a> = RedBlackTree<'a, RestingOrder>;
    pub type BooksideReadOnly<'a> = RedBlackTreeReadOnly<'a, RestingOrder>;
}
#[cfg(not(feature = "certora"))]
pub use types::*;

#[cfg(all(feature = "certora"))]
mod cvt_mock_types {
    use super::*;
    pub type ClaimedSeatTree<'a> = CvtClaimedSeatTree<'a>;
    pub type ClaimedSeatTreeReadOnly<'a> = CvtClaimedSeatTreeReadOnly<'a>;
    pub type Bookside<'a> = CvtBookside<'a>;
    pub type BooksideReadOnly<'a> = CvtBooksideReadOnly<'a>;
}
#[cfg(feature = "certora")]
pub use cvt_mock_types::*;

// This generic impl covers MarketRef, MarketRefMut and other
// DynamicAccount variants that allow read access.
impl<Fixed: DerefOrBorrow<MarketFixed>, Dynamic: DerefOrBorrow<[u8]>>
    DynamicAccount<Fixed, Dynamic>
{
    fn borrow_market(&self) -> MarketRef {
        MarketRef {
            fixed: self.fixed.deref_or_borrow(),
            dynamic: self.dynamic.deref_or_borrow(),
        }
    }

    pub fn get_base_mint_index(&self) -> u8 {
        let DynamicAccount { fixed, .. } = self.borrow_market();
        fixed.get_base_mint_index()
    }

    pub fn get_quote_mint(&self) -> &Pubkey {
        let DynamicAccount { fixed, .. } = self.borrow_market();
        fixed.get_quote_mint()
    }

    pub fn has_free_block(&self) -> bool {
        let DynamicAccount { fixed, .. } = self.borrow_market();
        let free_list_head_index: DataIndex = fixed.free_list_head_index;
        return free_list_head_index != NIL;
    }

    pub fn has_two_free_blocks(&self) -> bool {
        let DynamicAccount { fixed, dynamic } = self.borrow_market();
        let free_list_head_index: DataIndex = fixed.free_list_head_index;
        if free_list_head_index == NIL {
            return false;
        }
        let free_list_head: &FreeListNode<MarketUnusedFreeListPadding> =
            get_helper::<FreeListNode<MarketUnusedFreeListPadding>>(dynamic, free_list_head_index);
        free_list_head.has_next()
    }

    /**
    Returns None if length of the free list is larger or equal n.
    If less than n blocks are free, returns Some(n - length).
    */
    pub fn free_blocks_short_of_n(&self, mut n: u32) -> Option<u32> {
        let DynamicAccount { fixed, dynamic } = self.borrow_market();
        let mut current_index: DataIndex = fixed.free_list_head_index;

        while n > 0 {
            if current_index == NIL {
                return Some(n);
            }

            let current_node =
                get_helper::<FreeListNode<MarketUnusedFreeListPadding>>(dynamic, current_index);
            current_index = current_node.get_next_index();
            n -= 1;
        }

        if current_index == NIL {
            Some(0)
        } else {
            None
        }
    }

    pub fn impact_quote_atoms(
        &self,
        is_bid: bool,
        limit_base_atoms: BaseAtoms,
        global_trade_accounts_opts: &[Option<GlobalTradeAccounts>; 2],
    ) -> Result<QuoteAtoms, ProgramError> {
        let now_slot: u32 = get_now_slot();
        self.impact_quote_atoms_with_slot(
            is_bid,
            limit_base_atoms,
            global_trade_accounts_opts,
            now_slot,
        )
    }

    pub fn impact_quote_atoms_with_slot(
        &self,
        is_bid: bool,
        limit_base_atoms: BaseAtoms,
        global_trade_accounts_opts: &[Option<GlobalTradeAccounts>; 2],
        now_slot: u32,
    ) -> Result<QuoteAtoms, ProgramError> {
        let book: BooksideReadOnly = if is_bid {
            self.get_asks()
        } else {
            self.get_bids()
        };

        let mut total_matched_quote_atoms: QuoteAtoms = QuoteAtoms::ZERO;
        let mut remaining_base_atoms: BaseAtoms = limit_base_atoms;
        for (_, resting_order) in book.iter::<RestingOrder>() {
            // Skip expired orders
            if resting_order.is_expired(now_slot) {
                continue;
            }
            let matched_price: QuoteAtomsPerBaseAtom = resting_order.get_price();

            // Either fill the entire resting order, or only the
            // remaining_base_atoms, in which case, this is the last iteration
            let matched_base_atoms: BaseAtoms =
                resting_order.get_num_base_atoms().min(remaining_base_atoms);
            let did_fully_match_resting_order: bool =
                remaining_base_atoms >= resting_order.get_num_base_atoms();

            // Number of quote atoms matched exactly. Round in taker favor if
            // fully matching.
            let matched_quote_atoms: QuoteAtoms = matched_price.checked_quote_for_base(
                matched_base_atoms,
                is_bid != did_fully_match_resting_order,
            )?;

            // Stop walking if missing the needed global account.
            if self.is_missing_global_account(&resting_order, is_bid, global_trade_accounts_opts) {
                break;
            }

            // Skip unbacked global orders.
            if self.is_unbacked_global_order(
                &resting_order,
                is_bid,
                global_trade_accounts_opts,
                matched_base_atoms,
                matched_quote_atoms,
            ) {
                continue;
            }

            total_matched_quote_atoms =
                total_matched_quote_atoms.checked_add(matched_quote_atoms)?;

            if !did_fully_match_resting_order {
                break;
            }

            // prepare for next iteration
            remaining_base_atoms = remaining_base_atoms.checked_sub(matched_base_atoms)?;
        }

        // Note that when there are not enough orders on the market to use up or
        // to receive the desired number of base atoms, this returns just the
        // full amount on the bookside without differentiating that return.

        return Ok(total_matched_quote_atoms);
    }

    // Simplified version for certora. Those checks are actually stronger than
    // needed since it shows invariants hold on swap even when impact_base_atoms
    // returns a wrong value.
    #[cfg(feature = "certora")]
    pub fn impact_base_atoms(
        &self,
        is_bid: bool,
        limit_quote_atoms: QuoteAtoms,
        global_trade_accounts_opts: &[Option<GlobalTradeAccounts>; 2],
    ) -> Result<BaseAtoms, ProgramError> {
        crate::certora::summaries::impact_base_atoms::impact_base_atoms(
            self,
            is_bid,
            limit_quote_atoms,
            global_trade_accounts_opts,
        )
    }

    #[cfg(not(feature = "certora"))]
    pub fn impact_base_atoms(
        &self,
        is_bid: bool,
        limit_quote_atoms: QuoteAtoms,
        global_trade_accounts_opts: &[Option<GlobalTradeAccounts>; 2],
    ) -> Result<BaseAtoms, ProgramError> {
        let now_slot: u32 = get_now_slot();
        self.impact_base_atoms_with_slot(
            is_bid,
            limit_quote_atoms,
            global_trade_accounts_opts,
            now_slot,
        )
    }

    /// How many base atoms you get when you trade in limit_quote_atoms.
    #[cfg(not(feature = "certora"))]
    pub fn impact_base_atoms_with_slot(
        &self,
        is_bid: bool,
        limit_quote_atoms: QuoteAtoms,
        global_trade_accounts_opts: &[Option<GlobalTradeAccounts>; 2],
        now_slot: u32,
    ) -> Result<BaseAtoms, ProgramError> {
        let book: RedBlackTreeReadOnly<'_, RestingOrder> = if is_bid {
            self.get_asks()
        } else {
            self.get_bids()
        };

        let mut total_matched_base_atoms: BaseAtoms = BaseAtoms::ZERO;
        let mut remaining_quote_atoms: QuoteAtoms = limit_quote_atoms;

        for (_, resting_order) in book.iter::<RestingOrder>() {
            // Skip expired orders.
            if resting_order.is_expired(now_slot) {
                continue;
            }

            let matched_price: QuoteAtomsPerBaseAtom = resting_order.get_price();
            // base_atoms_limit is the number of base atoms that you get if you
            // were to trade all of the remaining quote atoms at the current
            // price. Rounding is done in the taker favor because at the limit,
            // it is a full match. So if you are checking against asks with 100
            // quote remaining against price 1.001, then the answer should be
            // 100, because the rounding is in favor of the taker. It takes 100
            // base atoms to exhaust 100 quote atoms at that price.
            let base_atoms_limit: BaseAtoms =
                matched_price.checked_base_for_quote(remaining_quote_atoms, !is_bid)?;
            // Either fill the entire resting order, or only the
            // base_atoms_limit, in which case, this is the last iteration.
            let matched_base_atoms: BaseAtoms =
                resting_order.get_num_base_atoms().min(base_atoms_limit);
            let did_fully_match_resting_order: bool =
                base_atoms_limit >= resting_order.get_num_base_atoms();
            // Number of quote atoms matched exactly. Round in taker favor if
            // fully matching.
            let matched_quote_atoms: QuoteAtoms = matched_price.checked_quote_for_base(
                matched_base_atoms,
                is_bid != did_fully_match_resting_order,
            )?;

            // Stop walking if missing the needed global account.
            if self.is_missing_global_account(resting_order, is_bid, global_trade_accounts_opts) {
                break;
            }

            // Skip unbacked global orders.
            if self.is_unbacked_global_order(
                &resting_order,
                is_bid,
                global_trade_accounts_opts,
                matched_base_atoms,
                matched_quote_atoms,
            ) {
                continue;
            }

            total_matched_base_atoms = total_matched_base_atoms.checked_add(matched_base_atoms)?;

            if !did_fully_match_resting_order {
                break;
            }

            // Prepare for next iteration
            remaining_quote_atoms = remaining_quote_atoms.checked_sub(matched_quote_atoms)?;

            // we can match exactly in base atoms but also deplete all quote atoms at the same time
            if remaining_quote_atoms == QuoteAtoms::ZERO {
                break;
            }
        }

        // Note that when there are not enough orders on the market to use up or
        // to receive the desired number of quote atoms, this returns just the
        // full amount on the bookside without differentiating that return.

        return Ok(total_matched_base_atoms);
    }

    #[cfg(not(feature = "certora"))]
    pub fn get_order_by_index(&self, index: DataIndex) -> &RestingOrder {
        let DynamicAccount { dynamic, .. } = self.borrow_market();
        &get_helper::<RBNode<RestingOrder>>(dynamic, index).get_value()
    }

    #[cfg(feature = "certora")]
    pub fn get_order_by_index(&self, index: DataIndex) -> &RestingOrder {
        let DynamicAccount { dynamic, .. } = self.borrow_market();
        &get_helper_order(dynamic, index).get_value()
    }

    pub fn get_trader_balance(&self, trader: &Pubkey) -> (BaseAtoms, QuoteAtoms) {
        let DynamicAccount { fixed, dynamic } = self.borrow_market();

        let claimed_seats_tree: ClaimedSeatTreeReadOnly =
            ClaimedSeatTreeReadOnly::new(dynamic, fixed.claimed_seats_root_index, NIL);
        let trader_index: DataIndex =
            claimed_seats_tree.lookup_index(&ClaimedSeat::new_empty(*trader));
        let claimed_seat: &ClaimedSeat = get_helper_seat(dynamic, trader_index).get_value();
        (
            claimed_seat.base_withdrawable_balance,
            claimed_seat.quote_withdrawable_balance,
        )
    }

    pub fn get_trader_key_by_index(&self, index: DataIndex) -> &Pubkey {
        let DynamicAccount { dynamic, .. } = self.borrow_market();

        &get_helper_seat(dynamic, index).get_value().trader
    }

    pub fn get_trader_voume(&self, trader: &Pubkey) -> QuoteAtoms {
        let DynamicAccount { fixed, dynamic } = self.borrow_market();

        let claimed_seats_tree: ClaimedSeatTreeReadOnly =
            ClaimedSeatTreeReadOnly::new(dynamic, fixed.claimed_seats_root_index, NIL);
        let trader_index: DataIndex =
            claimed_seats_tree.lookup_index(&ClaimedSeat::new_empty(*trader));
        let claimed_seat: &ClaimedSeat = get_helper_seat(dynamic, trader_index).get_value();

        claimed_seat.quote_volume
    }

    /// Get the trader's perps position: (position_size as i64, quote_cost_basis as u64)
    pub fn get_trader_position(&self, trader: &Pubkey) -> (i64, u64) {
        let DynamicAccount { fixed, dynamic } = self.borrow_market();

        let claimed_seats_tree: ClaimedSeatTreeReadOnly =
            ClaimedSeatTreeReadOnly::new(dynamic, fixed.claimed_seats_root_index, NIL);
        let trader_index: DataIndex =
            claimed_seats_tree.lookup_index(&ClaimedSeat::new_empty(*trader));
        let claimed_seat: &ClaimedSeat = get_helper_seat(dynamic, trader_index).get_value();

        (
            claimed_seat.get_position_size(),
            claimed_seat.get_quote_cost_basis(),
        )
    }

    pub fn get_bids(&self) -> BooksideReadOnly {
        let DynamicAccount { dynamic, fixed } = self.borrow_market();
        BooksideReadOnly::new(
            dynamic,
            fixed.get_bids_root_index(),
            fixed.get_bids_best_index(),
        )
    }

    pub fn get_asks(&self) -> BooksideReadOnly {
        let DynamicAccount { dynamic, fixed } = self.borrow_market();
        BooksideReadOnly::new(
            dynamic,
            fixed.get_asks_root_index(),
            fixed.get_asks_best_index(),
        )
    }

    fn is_missing_global_account(
        &self,
        resting_order: &RestingOrder,
        is_bid: bool,
        global_trade_accounts_opts: &[Option<GlobalTradeAccounts>; 2],
    ) -> bool {
        if resting_order.get_order_type() == OrderType::Global {
            // If global accounts are needed but not present, then this will
            // crash. This is an intentional product decision. Would be
            // valid to walk past, but we have chosen to give no fill rather
            // than worse price if the taker takes the shortcut of not
            // including global account.
            let global_trade_accounts_opt: &Option<GlobalTradeAccounts> = if is_bid {
                &global_trade_accounts_opts[0]
            } else {
                &global_trade_accounts_opts[1]
            };
            if global_trade_accounts_opt.is_none() {
                return true;
            }
        }
        return false;
    }

    fn is_unbacked_global_order(
        &self,
        resting_order: &RestingOrder,
        is_bid: bool,
        global_trade_accounts_opts: &[Option<GlobalTradeAccounts>; 2],
        matched_base_atoms: BaseAtoms,
        matched_quote_atoms: QuoteAtoms,
    ) -> bool {
        if resting_order.get_order_type() == OrderType::Global {
            // If global accounts are needed but not present, then this will
            // crash. This is an intentional product decision. Would be
            // valid to walk past, but we have chosen to give no fill rather
            // than worse price if the taker takes the shortcut of not
            // including global account.
            let global_trade_accounts_opt: &Option<GlobalTradeAccounts> = if is_bid {
                &global_trade_accounts_opts[0]
            } else {
                &global_trade_accounts_opts[1]
            };
            let has_enough_tokens: bool = can_back_order(
                global_trade_accounts_opt,
                self.get_trader_key_by_index(resting_order.get_trader_index()),
                GlobalAtoms::new(if is_bid {
                    matched_base_atoms.as_u64()
                } else {
                    matched_quote_atoms.as_u64()
                }),
            );
            if !has_enough_tokens {
                return true;
            }
        }
        return false;
    }

    pub fn get_trader_index(&self, trader: &Pubkey) -> DataIndex {
        let DynamicAccount { fixed, dynamic } = self.borrow_market();

        let claimed_seats_tree: ClaimedSeatTreeReadOnly =
            ClaimedSeatTreeReadOnly::new(dynamic, fixed.claimed_seats_root_index, NIL);
        let trader_index: DataIndex =
            claimed_seats_tree.lookup_index(&ClaimedSeat::new_empty(*trader));
        trader_index
    }
}

// This generic impl covers MarketRef, MarketRefMut and other
// DynamicAccount variants that allow write access.
impl<
        Fixed: DerefOrBorrowMut<MarketFixed> + DerefOrBorrow<MarketFixed>,
        Dynamic: DerefOrBorrowMut<[u8]> + DerefOrBorrow<[u8]>,
    > DynamicAccount<Fixed, Dynamic>
{
    fn borrow_mut(&mut self) -> MarketRefMut {
        MarketRefMut {
            fixed: self.fixed.deref_or_borrow_mut(),
            dynamic: self.dynamic.deref_or_borrow_mut(),
        }
    }

    pub fn market_expand(&mut self) -> ProgramResult {
        let DynamicAccount { fixed, dynamic } = self.borrow_mut();
        let mut free_list: FreeList<MarketUnusedFreeListPadding> =
            FreeList::new(dynamic, fixed.free_list_head_index);

        free_list.add(fixed.num_bytes_allocated);
        fixed.num_bytes_allocated += MARKET_BLOCK_SIZE as u32;
        fixed.free_list_head_index = free_list.get_head();
        Ok(())
    }

    pub fn market_expand_n(&mut self, mut n: u32) -> ProgramResult {
        let DynamicAccount { fixed, dynamic } = self.borrow_mut();
        let mut free_list: FreeList<MarketUnusedFreeListPadding> =
            FreeList::new(dynamic, fixed.free_list_head_index);
        while n > 0 {
            free_list.add(fixed.num_bytes_allocated);
            fixed.num_bytes_allocated += MARKET_BLOCK_SIZE as u32;
            fixed.free_list_head_index = free_list.get_head();
            n -= 1;
        }
        Ok(())
    }

    pub fn claim_seat(&mut self, trader: &Pubkey) -> ProgramResult {
        let DynamicAccount { fixed, dynamic } = self.borrow_mut();
        let free_address: DataIndex = get_free_address_on_market_fixed_for_seat(fixed, dynamic);

        let mut claimed_seats_tree: ClaimedSeatTree =
            ClaimedSeatTree::new(dynamic, fixed.claimed_seats_root_index, NIL);

        let claimed_seat: ClaimedSeat = ClaimedSeat::new_empty(*trader);
        require!(
            claimed_seats_tree.lookup_index(&claimed_seat) == NIL,
            ManifestError::AlreadyClaimedSeat,
            "Already claimed seat",
        )?;
        claimed_seats_tree.insert(free_address, claimed_seat);
        // theoretically we need to update the total withdrawable amount, but it's always 0 here.
        // but let's check here that the assumption holds.
        #[cfg(feature = "certora")]
        {
            cvt::cvt_assert!(claimed_seat.base_withdrawable_balance == BaseAtoms::new(0));
            cvt::cvt_assert!(claimed_seat.quote_withdrawable_balance == QuoteAtoms::new(0));
        }
        fixed.claimed_seats_root_index = claimed_seats_tree.get_root_index();

        get_mut_helper::<RBNode<ClaimedSeat>>(dynamic, free_address)
            .set_payload_type(MarketDataTreeNodeType::ClaimedSeat as u8);
        Ok(())
    }

    // Only used when temporarily claiming for swap and we dont have the system
    // program to expand. Otherwise, there is no reason to ever give up your
    // seat.
    pub fn release_seat(&mut self, trader: &Pubkey) -> ProgramResult {
        let trader_seat_index: DataIndex = self.get_trader_index(trader);
        let DynamicAccount { fixed, dynamic } = self.borrow_mut();

        let mut claimed_seats_tree: ClaimedSeatTree =
            ClaimedSeatTree::new(dynamic, fixed.claimed_seats_root_index, NIL);
        claimed_seats_tree.remove_by_index(trader_seat_index);
        fixed.claimed_seats_root_index = claimed_seats_tree.get_root_index();

        // Put back seat on free list.
        release_address_on_market_fixed_for_seat(fixed, dynamic, trader_seat_index);
        Ok(())
    }

    pub fn deposit(
        &mut self,
        trader_index: DataIndex,
        amount_atoms: u64,
        is_base: bool,
    ) -> ProgramResult {
        require!(
            is_not_nil!(trader_index),
            ManifestError::InvalidDepositAccounts,
            "No seat initialized",
        )?;
        let DynamicAccount { fixed, dynamic } = self.borrow_mut();
        update_balance(fixed, dynamic, trader_index, is_base, true, amount_atoms)?;
        Ok(())
    }

    pub fn withdraw(
        &mut self,
        trader_index: DataIndex,
        amount_atoms: u64,
        is_base: bool,
    ) -> ProgramResult {
        let DynamicAccount { fixed, dynamic } = self.borrow_mut();
        update_balance(fixed, dynamic, trader_index, is_base, false, amount_atoms)?;
        Ok(())
    }

    /// Funding rate scaling factor (1e9). Shared with crank_funding.
    const FUNDING_SCALE: i64 = 1_000_000_000;

    /// Settle accumulated funding for a trader using lazy cumulative approach.
    ///
    /// Reads the trader's `last_cumulative_funding` (stored in base_withdrawable_balance),
    /// computes the funding owed based on position_size and the delta from the global
    /// cumulative rate, adjusts quote margin, and zeroes base_withdrawable_balance
    /// (preparing it for the matching engine's virtual base operations).
    ///
    /// Must be called at the START of any user interaction (swap, batch_update,
    /// liquidate, deposit, withdraw) before any base balance operations.
    pub fn settle_funding_for_trader(&mut self, trader_index: DataIndex) -> ProgramResult {
        let DynamicAccount { fixed, dynamic } = self.borrow_mut();
        let claimed_seat: &mut ClaimedSeat =
            get_mut_helper_seat(dynamic, trader_index).get_mut_value();

        let position_size: i64 = claimed_seat.get_position_size();
        if position_size != 0 {
            let global_cumulative: i64 = fixed.get_cumulative_funding();
            let last_cumulative: i64 = claimed_seat.get_last_cumulative_funding();
            let delta: i64 = global_cumulative.wrapping_sub(last_cumulative);

            if delta != 0 {
                // funding_owed = position_size * delta / FUNDING_SCALE
                // Longs pay positive funding, shorts receive positive funding
                let funding_owed: i64 = ((position_size as i128 * delta as i128)
                    / Self::FUNDING_SCALE as i128) as i64;

                let current_margin: u64 = claimed_seat.quote_withdrawable_balance.as_u64();
                let new_margin: u64 = if funding_owed >= 0 {
                    let owed: u64 = funding_owed as u64;
                    if owed <= current_margin {
                        current_margin - owed
                    } else {
                        // Funding exceeds margin — draw deficit from insurance fund.
                        // This prevents silent vault insolvency.
                        let deficit: u64 = owed - current_margin;
                        fixed.draw_from_insurance_fund(deficit);
                        0
                    }
                } else {
                    current_margin.saturating_add(funding_owed.unsigned_abs())
                };
                claimed_seat.quote_withdrawable_balance = QuoteAtoms::new(new_margin);
            }
        }

        // Zero base_withdrawable_balance for matching engine (base is virtual in perps).
        // The cumulative funding value was consumed above.
        claimed_seat.base_withdrawable_balance = BaseAtoms::new(0);
        Ok(())
    }

    /// Store the current global cumulative funding rate into the trader's seat.
    ///
    /// Must be called at the END of any user interaction, after all matching
    /// engine operations are complete, to persist the funding checkpoint.
    pub fn store_cumulative_for_trader(&mut self, trader_index: DataIndex) {
        let DynamicAccount { fixed, dynamic } = self.borrow_mut();
        let global_cumulative: i64 = fixed.get_cumulative_funding();
        let claimed_seat: &mut ClaimedSeat =
            get_mut_helper_seat(dynamic, trader_index).get_mut_value();
        claimed_seat.set_last_cumulative_funding(global_cumulative);
    }

    pub fn place_order_(
        &mut self,
        args: AddOrderToMarketArgs,
    ) -> Result<AddOrderToMarketResult, ProgramError> {
        market_helpers::place_order_helper(self, args)
    }

    /// Place an order and update the market
    ///
    /// 1. Check the order against the opposite bookside
    /// 2. Rest any amount of the order leftover on the book
    pub fn place_order(
        &mut self,
        args: AddOrderToMarketArgs,
    ) -> Result<AddOrderToMarketResult, ProgramError> {
        let AddOrderToMarketArgs {
            market,
            trader_index,
            num_base_atoms,
            price,
            is_bid,
            last_valid_slot,
            order_type,
            global_trade_accounts_opts,
            current_slot,
        } = args;
        assert_already_has_seat(trader_index)?;
        let now_slot: u32 = current_slot.unwrap_or_else(|| get_now_slot());

        // Reverse orders will have their last valid slot overriden to no expiration.
        if !order_type.is_reversible() {
            assert_not_already_expired(last_valid_slot, now_slot)?;
        }

        let DynamicAccount { fixed, dynamic } = self.borrow_mut();

        let mut current_maker_order_index: DataIndex = if is_bid {
            fixed.asks_best_index
        } else {
            fixed.bids_best_index
        };

        let mut total_base_atoms_traded: BaseAtoms = BaseAtoms::ZERO;
        let mut total_quote_atoms_traded: QuoteAtoms = QuoteAtoms::ZERO;

        let mut remaining_base_atoms: BaseAtoms = num_base_atoms;
        while remaining_base_atoms > BaseAtoms::ZERO && is_not_nil!(current_maker_order_index) {
            let maker_order: &RestingOrder =
                get_helper::<RBNode<RestingOrder>>(dynamic, current_maker_order_index).get_value();

            // Remove the resting order if expired or somehow a zero order got on the book.
            if maker_order.is_expired(now_slot) || maker_order.get_num_base_atoms().as_u64() == 0 {
                let next_maker_order_index: DataIndex = get_next_candidate_match_index(
                    fixed,
                    dynamic,
                    current_maker_order_index,
                    is_bid,
                );
                remove_and_update_balances(
                    fixed,
                    dynamic,
                    current_maker_order_index,
                    global_trade_accounts_opts,
                )?;
                current_maker_order_index = next_maker_order_index;
                continue;
            }

            // Stop trying to match if price no longer satisfies limit.
            if (is_bid && maker_order.get_price() > price)
                || (!is_bid && maker_order.get_price() < price)
            {
                break;
            }

            // Got a match. First make sure we are allowed to match. We check
            // inside the matching rather than skipping the matching altogether
            // because post only orders should fail, not produce a crossed book.
            assert_can_take(order_type)?;

            let maker_sequence_number = maker_order.get_sequence_number();
            let maker_trader_index: DataIndex = maker_order.get_trader_index();
            let did_fully_match_resting_order: bool =
                remaining_base_atoms >= maker_order.get_num_base_atoms();
            let base_atoms_traded: BaseAtoms = if did_fully_match_resting_order {
                maker_order.get_num_base_atoms()
            } else {
                remaining_base_atoms
            };

            let matched_price: QuoteAtomsPerBaseAtom = maker_order.get_price();
            let maker_order_type: OrderType = maker_order.get_order_type();
            let maker_price_reverse: Result<QuoteAtomsPerBaseAtom, _> = maker_order.reverse_price();

            // on full fill: round in favor of the taker
            // on partial fill: round in favor of the maker
            let quote_atoms_traded: QuoteAtoms = matched_price.checked_quote_for_base(
                base_atoms_traded,
                is_bid != did_fully_match_resting_order,
            )?;

            // If it is a global order, just in time bring the funds over, or
            // remove from the tree and continue on to the next order.
            let maker: Pubkey = get_helper_seat(dynamic, maker_trader_index)
                .get_value()
                .trader;
            let taker: Pubkey = get_helper_seat(dynamic, trader_index).get_value().trader;
            let is_global: bool = maker_order.is_global();
            let is_maker_reverse: bool = maker_order.is_reversible();

            let maker_reverse_spread: u16 = maker_order.get_reverse_spread();

            if is_global {
                let global_trade_accounts_opt: &Option<GlobalTradeAccounts> = if is_bid {
                    &global_trade_accounts_opts[0]
                } else {
                    &global_trade_accounts_opts[1]
                };
                // When the global account is not included, a taker order can
                // halt here, but a possible maker order will need to crash
                // since that would result in a crossed book.
                if global_trade_accounts_opt.is_none() {
                    if order_type_can_rest(order_type) {
                        return Err(ManifestError::MissingGlobal.into());
                    } else {
                        break;
                    }
                }
                // When is_bid, the taker is supplying quote, so the global
                // maker needs to supply base.
                let has_enough_tokens: bool = try_to_move_global_tokens(
                    global_trade_accounts_opt,
                    &maker,
                    GlobalAtoms::new(if is_bid {
                        base_atoms_traded.as_u64()
                    } else {
                        quote_atoms_traded.as_u64()
                    }),
                )?;

                if !has_enough_tokens {
                    let next_maker_order_index: DataIndex = get_next_candidate_match_index(
                        fixed,
                        dynamic,
                        current_maker_order_index,
                        is_bid,
                    );
                    remove_and_update_balances(
                        fixed,
                        dynamic,
                        current_maker_order_index,
                        global_trade_accounts_opts,
                    )?;
                    current_maker_order_index = next_maker_order_index;
                    continue;
                }
            }

            total_base_atoms_traded = total_base_atoms_traded.checked_add(base_atoms_traded)?;
            total_quote_atoms_traded = total_quote_atoms_traded.checked_add(quote_atoms_traded)?;

            // Possibly increase bonus atom maker gets from the rounding the
            // quote in their favor. They will get one less than expected when
            // cancelling because of rounding, this counters that. This ensures
            // that the amount of quote that the maker has credit for when they
            // cancel/expire is always the maximum amount that could have been
            // used in matching that order.
            // Example:
            // Maker deposits 11            | Balance: 0 base 11 quote | Orders: []
            // Maker bid for 10@1.15        | Balance: 0 base 0 quote  | Orders: [bid 10@1.15]
            // Swap    5 base <--> 5 quote  | Balance: 5 base 0 quote  | Orders: [bid 5@1.15]
            //     <this code block>        | Balance: 5 base 1 quote  | Orders: [bid 5@1.15]
            // Maker cancel                 | Balance: 5 base 6 quote  | Orders: []
            //
            // The swapper deposited 5 base and withdrew 5 quote. The maker deposited 11 quote.
            // If we didnt do this adjustment, there would be an unaccounted for
            // quote atom.
            // Note that we do not have to do this on the other direction
            // because the amount of atoms that a maker needs to support an ask
            // is exact. The rounding is always on quote.
            //
            // Do not credit the bonus atom on global orders. This is because
            // only the number of atoms required for the trade were brought
            // over.  The extra one that is no longer needed for taker rounding
            // is not brought over, so dont credit the maker for it.
            if !is_bid && !is_global {
                // These are only used when is_bid.
                let previous_maker_quote_atoms_allocated: QuoteAtoms =
                    matched_price.checked_quote_for_base(maker_order.get_num_base_atoms(), true)?;
                let new_maker_quote_atoms_allocated: QuoteAtoms = matched_price
                    .checked_quote_for_base(
                        maker_order
                            .get_num_base_atoms()
                            .checked_sub(base_atoms_traded)?,
                        true,
                    )?;
                let bonus_atom_or_zero: QuoteAtoms = previous_maker_quote_atoms_allocated
                    .checked_sub(new_maker_quote_atoms_allocated)?
                    .checked_sub(quote_atoms_traded)?;

                // The bonus atom isnt actually traded, it is recouped to the
                // maker though from the tokens that they had been using to back
                // the order since it is no longer needed. So we do not need to
                // update the fill logs or amounts.
                update_balance(
                    fixed,
                    dynamic,
                    maker_trader_index,
                    is_bid,
                    true,
                    bonus_atom_or_zero.as_u64(),
                )?;
            }

            // Increase maker from the matched amount in the trade.
            // In perps, only credit quote to maker. Skip base credits since base
            // is virtual and base_withdrawable_balance stores cumulative funding.
            // Position tracking is handled by update_perps_position below.
            if is_bid {
                update_balance(
                    fixed,
                    dynamic,
                    maker_trader_index,
                    false, // quote side
                    true,
                    quote_atoms_traded.into(),
                )?;
            }
            // Decrease taker
            update_balance(
                fixed,
                dynamic,
                trader_index,
                !is_bid,
                false,
                if is_bid {
                    quote_atoms_traded.into()
                } else {
                    base_atoms_traded.into()
                },
            )?;
            // Increase taker
            update_balance(
                fixed,
                dynamic,
                trader_index,
                is_bid,
                true,
                if is_bid {
                    base_atoms_traded.into()
                } else {
                    quote_atoms_traded.into()
                },
            )?;

            // Update perps position tracking for maker and taker
            update_perps_position(
                fixed,
                dynamic,
                maker_trader_index,
                base_atoms_traded.as_u64(),
                quote_atoms_traded.as_u64(),
                !is_bid,
            )?;
            update_perps_position(
                fixed,
                dynamic,
                trader_index,
                base_atoms_traded.as_u64(),
                quote_atoms_traded.as_u64(),
                is_bid,
            )?;

            emit_stack(FillLog {
                market,
                maker,
                taker,
                base_mint: Pubkey::default(),
                quote_mint: fixed.quote_mint,
                base_atoms: base_atoms_traded,
                quote_atoms: quote_atoms_traded,
                price: matched_price,
                maker_sequence_number,
                taker_sequence_number: fixed.order_sequence_number,
                taker_is_buy: PodBool::from(is_bid),
                is_maker_global: PodBool::from(is_global),
                _padding: [0; 14],
            })?;

            if did_fully_match_resting_order {
                // Get paid for removing a global order.
                if get_helper::<RBNode<RestingOrder>>(dynamic, current_maker_order_index)
                    .get_value()
                    .is_global()
                {
                    if is_bid {
                        remove_from_global(&global_trade_accounts_opts[0])?;
                    } else {
                        remove_from_global(&global_trade_accounts_opts[1])?;
                    }
                }

                let next_maker_order_index: DataIndex = get_next_candidate_match_index(
                    fixed,
                    dynamic,
                    current_maker_order_index,
                    is_bid,
                );
                remove_order_from_tree_and_free(
                    fixed,
                    dynamic,
                    current_maker_order_index,
                    !is_bid,
                )?;
                remaining_base_atoms = remaining_base_atoms.checked_sub(base_atoms_traded)?;
                current_maker_order_index = next_maker_order_index;
            } else {
                #[cfg(feature = "certora")]
                remove_from_orderbook_balance(fixed, dynamic, current_maker_order_index);
                let maker_order: &mut RestingOrder =
                    get_mut_helper::<RBNode<RestingOrder>>(dynamic, current_maker_order_index)
                        .get_mut_value();
                maker_order.reduce(base_atoms_traded)?;
                #[cfg(feature = "certora")]
                add_to_orderbook_balance(fixed, dynamic, current_maker_order_index);
                remaining_base_atoms = BaseAtoms::ZERO;
            }

            // Place the reverse order if the maker was a reverse order type.
            // This is non-trivial because in order to prevent tons of orders
            // filling the books on partial fills, we coalesce on top of book.
            if is_maker_reverse {
                if let Ok(price_reverse) = maker_price_reverse {
                    let num_base_atoms_reverse: BaseAtoms = if is_bid {
                        // Maker is now buying with the exact number of quote atoms.
                        // Do not round_up because there might not be enough atoms
                        // for that.
                        price_reverse.checked_base_for_quote(quote_atoms_traded, false)?
                    } else {
                        base_atoms_traded
                    };

                    let mut coalesced: bool = false;
                    {
                        let other_tree: Bookside = if is_bid {
                            Bookside::new(dynamic, fixed.bids_root_index, fixed.bids_best_index)
                        } else {
                            Bookside::new(dynamic, fixed.asks_root_index, fixed.asks_best_index)
                        };
                        let lookup_resting_order: RestingOrder = RestingOrder::new(
                            maker_trader_index,
                            BaseAtoms::ZERO, // Size does not matter, just price.
                            price_reverse,
                            0, // Sequence number does not matter, just price
                            NO_EXPIRATION_LAST_VALID_SLOT,
                            is_bid,
                            maker_order_type,
                        )?;

                        // Because there is a slight relaxation in matching reverse
                        // orders, do not need to worry about off by one errors
                        // causing fragmented liqudity.
                        let lookup_index: DataIndex =
                            other_tree.lookup_index(&lookup_resting_order);
                        if lookup_index != NIL {
                            let order_to_coalesce_into: &mut RestingOrder =
                                get_mut_helper::<RBNode<RestingOrder>>(dynamic, lookup_index)
                                    .get_mut_value();
                            order_to_coalesce_into.increase(num_base_atoms_reverse)?;
                            coalesced = true;
                        }
                    }

                    // If there was 1 atom and because taker rounding is in effect,
                    // then this would result in an empty order.
                    if !coalesced && num_base_atoms_reverse.as_u64() > 0 {
                        // This code is similar to rest_remaining except it doesnt
                        // require borrowing data.  Non-trivial to combine the code
                        // because the certora formal verification inserted itself
                        // there.
                        let reverse_order_sequence_number: u64 = fixed.order_sequence_number;
                        fixed.order_sequence_number = reverse_order_sequence_number.wrapping_add(1);

                        // Put the remaining in an order on the other bookside.
                        // There are 2 cases, either the maker was fully exhausted and
                        // we know that we will be able to use their address, or they
                        // were not fully exhausted and we know the order will not rest.
                        // In the second case, that uses the free block that was
                        // speculatively there for the current trader to rest.
                        let free_address: DataIndex = if is_bid {
                            get_free_address_on_market_fixed_for_bid_order(fixed, dynamic)
                        } else {
                            get_free_address_on_market_fixed_for_ask_order(fixed, dynamic)
                        };

                        let mut new_reverse_resting_order: RestingOrder = RestingOrder::new(
                            maker_trader_index,
                            num_base_atoms_reverse,
                            price_reverse,
                            reverse_order_sequence_number,
                            // Does not expire.
                            NO_EXPIRATION_LAST_VALID_SLOT,
                            is_bid,
                            maker_order_type,
                        )?;
                        new_reverse_resting_order.set_reverse_spread(maker_reverse_spread);
                        insert_order_into_tree(
                            is_bid,
                            fixed,
                            dynamic,
                            free_address,
                            &new_reverse_resting_order,
                        );
                        set_payload_order(dynamic, free_address);
                    }

                    // Reserve tokens for the reverse order.
                    // In perps, skip base deductions (base is virtual and
                    // base_withdrawable_balance stores cumulative funding).
                    if is_bid {
                        update_balance(
                            fixed,
                            dynamic,
                            maker_trader_index,
                            false, // quote side (is_bid=true → !is_bid=false)
                            false,
                            num_base_atoms_reverse
                                .checked_mul(price_reverse, true)?
                                .into(),
                        )?;
                    }
                }
            }

            // Stop if the last resting order did not fully match since that
            // means the taker was exhausted.
            if !did_fully_match_resting_order {
                break;
            }
        }

        // Record volume on market
        fixed.quote_volume = fixed.quote_volume.wrapping_add(total_quote_atoms_traded);

        // Bump the order sequence number even for orders which do not end up
        // resting.
        let order_sequence_number: u64 = fixed.order_sequence_number;
        fixed.order_sequence_number = order_sequence_number.wrapping_add(1);

        // If there is nothing left to rest, then return before resting.
        if !order_type_can_rest(order_type)
            || remaining_base_atoms == BaseAtoms::ZERO
            || price == QuoteAtomsPerBaseAtom::ZERO
        {
            return Ok(AddOrderToMarketResult {
                order_sequence_number,
                order_index: NIL,
                base_atoms_traded: total_base_atoms_traded,
                quote_atoms_traded: total_quote_atoms_traded,
            });
        }

        self.rest_remaining(
            args,
            remaining_base_atoms,
            order_sequence_number,
            total_base_atoms_traded,
            total_quote_atoms_traded,
        )
    }

    /// Rest the remaining order onto the market in a RestingOrder.
    #[cfg(feature = "certora")]
    pub fn certora_rest_remaining(
        &mut self,
        args: AddOrderToMarketArgs,
        remaining_base_atoms: BaseAtoms,
        order_sequence_number: u64,
        total_base_atoms_traded: BaseAtoms,
        total_quote_atoms_traded: QuoteAtoms,
    ) -> Result<AddOrderToMarketResult, ProgramError> {
        self.rest_remaining(
            args,
            remaining_base_atoms,
            order_sequence_number,
            total_base_atoms_traded,
            total_quote_atoms_traded,
        )
    }

    fn rest_remaining(
        &mut self,
        args: AddOrderToMarketArgs,
        remaining_base_atoms: BaseAtoms,
        order_sequence_number: u64,
        total_base_atoms_traded: BaseAtoms,
        total_quote_atoms_traded: QuoteAtoms,
    ) -> Result<AddOrderToMarketResult, ProgramError> {
        let AddOrderToMarketArgs {
            trader_index,
            price,
            is_bid,
            last_valid_slot,
            order_type,
            global_trade_accounts_opts,
            ..
        } = args;
        let DynamicAccount { fixed, dynamic } = self.borrow_mut();

        // Put the remaining in an order on the other bookside.
        let free_address: DataIndex = if is_bid {
            get_free_address_on_market_fixed_for_bid_order(fixed, dynamic)
        } else {
            get_free_address_on_market_fixed_for_ask_order(fixed, dynamic)
        };

        let mut resting_order: RestingOrder = RestingOrder::new(
            trader_index,
            remaining_base_atoms,
            price,
            order_sequence_number,
            if !order_type.is_reversible() {
                last_valid_slot
            } else {
                NO_EXPIRATION_LAST_VALID_SLOT
            },
            is_bid,
            order_type,
        )?;

        if order_type.is_reversible() {
            resting_order.set_reverse_spread(last_valid_slot as u16);
        }

        if resting_order.is_global() {
            let global_trade_account_opt = &global_trade_accounts_opts[if is_bid { 1 } else { 0 }];
            require!(
                global_trade_account_opt.is_some(),
                ManifestError::MissingGlobal,
                "Missing global accounts when adding a global",
            )?;
            try_to_add_to_global(&global_trade_account_opt.as_ref().unwrap(), &resting_order)?;
        } else {
            // Place the remaining.
            // Rounds up quote atoms so price can be rounded in favor of taker
            update_balance(
                fixed,
                dynamic,
                trader_index,
                !is_bid,
                false,
                if is_bid {
                    remaining_base_atoms.checked_mul(price, true)?.into()
                } else {
                    remaining_base_atoms.into()
                },
            )?;
        }
        insert_order_into_tree(is_bid, fixed, dynamic, free_address, &resting_order);

        set_payload_order(dynamic, free_address);

        Ok(AddOrderToMarketResult {
            order_sequence_number,
            order_index: free_address,
            base_atoms_traded: total_base_atoms_traded,
            quote_atoms_traded: total_quote_atoms_traded,
        })
    }

    // Does a linear scan over the orderbook to find the index to cancel.
    pub fn cancel_order(
        &mut self,
        trader_index: DataIndex,
        order_sequence_number: u64,
        global_trade_accounts_opts: &[Option<GlobalTradeAccounts>; 2],
    ) -> ProgramResult {
        let DynamicAccount { fixed, dynamic } = self.borrow_mut();

        let mut index_to_remove: DataIndex = NIL;

        // One iteration to find the index to cancel in the ask side.
        let tree: BooksideReadOnly =
            BooksideReadOnly::new(dynamic, fixed.asks_root_index, fixed.asks_best_index);
        for (index, resting_order) in tree.iter::<RestingOrder>() {
            if resting_order.get_sequence_number() == order_sequence_number {
                require!(
                    resting_order.get_trader_index() == trader_index,
                    ManifestError::InvalidCancel,
                    "Cannot cancel for another trader",
                )?;
                require!(
                    index_to_remove == NIL,
                    ManifestError::InvalidCancel,
                    "Book is broken, matched multiple orders",
                )?;
                index_to_remove = index;
            }
        }

        // Second iteration to find the index to cancel in the bid side.
        let tree: BooksideReadOnly =
            BooksideReadOnly::new(dynamic, fixed.bids_root_index, fixed.bids_best_index);
        for (index, resting_order) in tree.iter::<RestingOrder>() {
            if resting_order.get_sequence_number() == order_sequence_number {
                require!(
                    resting_order.get_trader_index() == trader_index,
                    ManifestError::InvalidCancel,
                    "Cannot cancel for another trader",
                )?;
                require!(
                    index_to_remove == NIL,
                    ManifestError::InvalidCancel,
                    "Book is broken, matched multiple orders",
                )?;
                index_to_remove = index;
            }
        }

        if is_not_nil!(index_to_remove) {
            // Cancel order by index will update balances.
            self.cancel_order_by_index(index_to_remove, global_trade_accounts_opts)?;
            return Ok(());
        }

        // Do not fail silently.
        require!(
            false,
            ManifestError::InvalidCancel,
            "Unable to find the order for cancel. Sequence number: {}",
            order_sequence_number
        )?;
        Ok(())
    }

    #[cfg_attr(feature = "certora", cvt_hook_end(cancel_order_by_index_was_called()))]
    pub fn cancel_order_by_index(
        &mut self,
        order_index: DataIndex,
        global_trade_accounts_opts: &[Option<GlobalTradeAccounts>; 2],
    ) -> ProgramResult {
        let DynamicAccount { fixed, dynamic } = self.borrow_mut();
        // TODO: Undo expansion here when it was just
        // remove_and_update_balances(fixed, dynamic, order_index, global_trade_accounts_opts)?;
        // because the tracking for

        let resting_order: &RestingOrder = get_helper_order(dynamic, order_index).get_value();
        let is_bid: bool = resting_order.get_is_bid();

        // Important to round up because there was an extra atom taken for full
        // taker rounding when the order was placed.
        let amount_atoms: u64 = if is_bid {
            (resting_order
                .get_price()
                .checked_quote_for_base(resting_order.get_num_base_atoms(), true)
                .unwrap())
            .into()
        } else {
            resting_order.get_num_base_atoms().into()
        };

        // Update the accounting for the order that was just canceled.
        if resting_order.is_global() {
            if is_bid {
                remove_from_global(&global_trade_accounts_opts[1])?;
            } else {
                remove_from_global(&global_trade_accounts_opts[0])?;
            }

            // Certora version was equivalent except it returned early here.
            // Thats a bug, but keeping the certora code as close to what they
            // wrote as possible. Formal verification does not cover global, so
            // that would explain why introducing this bug for formal
            // verification does not cause failures.
            #[cfg(feature = "certora")]
            return Ok(());
        } else if is_bid {
            // For bids: return quote atoms to the trader's balance.
            update_balance(
                fixed,
                dynamic,
                resting_order.get_trader_index(),
                !is_bid,
                true,
                amount_atoms,
            )?;
        }
        // For asks: do NOT return base atoms. Base is virtual in perps,
        // and base_withdrawable_balance is repurposed for cumulative funding tracking.
        // The virtual base atoms were meaningless accounting artifacts.
        remove_order_from_tree_and_free(fixed, dynamic, order_index, is_bid)?;

        Ok(())
    }
}

fn set_payload_order(dynamic: &mut [u8], free_address: DataIndex) {
    get_mut_helper_order(dynamic, free_address)
        .set_payload_type(MarketDataTreeNodeType::RestingOrder as u8);
}

#[cfg(feature = "certora")]
fn add_to_orderbook_balance(fixed: &mut MarketFixed, dynamic: &mut [u8], order_index: DataIndex) {
    // Update the sum of orderbook balance.
    let resting_order = get_helper_order(dynamic, order_index).get_value();
    let (base_amount, quote_amount) = match resting_order.get_orderbook_atoms() {
        Ok(result) => result,
        // make errors nondet(), so we can see the violation
        Err(_) => (nondet(), nondet()),
    };
    fixed.orderbook_base_atoms = fixed.orderbook_base_atoms.saturating_add(base_amount);
    fixed.orderbook_quote_atoms = fixed.orderbook_quote_atoms.saturating_add(quote_amount);
}

#[cfg(feature = "certora")]
fn remove_from_orderbook_balance(
    fixed: &mut MarketFixed,
    dynamic: &mut [u8],
    order_index: DataIndex,
) {
    // Update the sum of orderbook balance.
    let resting_order = get_helper_order(dynamic, order_index).get_value();
    let (base_amount, quote_amount) = match resting_order.get_orderbook_atoms() {
        Ok(result) => result,
        // make errors nondet(), so we can see the violation
        Err(_) => (nondet(), nondet()),
    };
    fixed.orderbook_base_atoms = fixed.orderbook_base_atoms.saturating_sub(base_amount);
    fixed.orderbook_quote_atoms = fixed.orderbook_quote_atoms.saturating_sub(quote_amount);
}

fn remove_order_from_tree(
    fixed: &mut MarketFixed,
    dynamic: &mut [u8],
    order_index: DataIndex,
    is_bids: bool,
) -> ProgramResult {
    #[cfg(feature = "certora")]
    remove_from_orderbook_balance(fixed, dynamic, order_index);
    let mut tree: Bookside = if is_bids {
        Bookside::new(dynamic, fixed.bids_root_index, fixed.bids_best_index)
    } else {
        Bookside::new(dynamic, fixed.asks_root_index, fixed.asks_best_index)
    };
    tree.remove_by_index(order_index);

    // Possibly changes the root and/or best.
    if is_bids {
        trace!(
            "remove order bid root:{}->{} max:{}->{}",
            fixed.bids_root_index,
            tree.get_root_index(),
            fixed.bids_best_index,
            tree.get_max_index()
        );
        fixed.bids_root_index = tree.get_root_index();
        fixed.bids_best_index = tree.get_max_index();
    } else {
        trace!(
            "remove order ask root:{}->{} max:{}->{}",
            fixed.asks_root_index,
            tree.get_root_index(),
            fixed.asks_best_index,
            tree.get_max_index()
        );
        fixed.asks_root_index = tree.get_root_index();
        fixed.asks_best_index = tree.get_max_index();
    }
    Ok(())
}

// Remove order from the tree, free the block.
#[cfg_attr(
    feature = "certora",
    cvt_hook_end(remove_order_from_tree_and_free_was_called())
)]
fn remove_order_from_tree_and_free(
    fixed: &mut MarketFixed,
    dynamic: &mut [u8],
    order_index: DataIndex,
    is_bids: bool,
) -> ProgramResult {
    remove_order_from_tree(fixed, dynamic, order_index, is_bids)?;
    // Separate release functions because certora needs that.
    if is_bids {
        release_address_on_market_fixed_for_bid_order(fixed, dynamic, order_index);
    } else {
        release_address_on_market_fixed_for_ask_order(fixed, dynamic, order_index);
    }
    Ok(())
}

#[allow(unused_variables)]
pub fn update_balance(
    fixed: &mut MarketFixed,
    dynamic: &mut [u8],
    trader_index: DataIndex,
    is_base: bool,
    is_increase: bool,
    amount_atoms: u64,
) -> ProgramResult {
    let claimed_seat: &mut ClaimedSeat = get_mut_helper_seat(dynamic, trader_index).get_mut_value();

    trace!("update_balance_by_trader_index idx:{trader_index} base:{is_base} inc:{is_increase} amount:{amount_atoms}");

    // Update the sum of withdrawable balance.
    // Subtract the current value from the sum here and at the end of the function, add
    // the new value to the sum.
    #[cfg(feature = "certora")]
    {
        fixed.withdrawable_base_atoms = fixed
            .withdrawable_base_atoms
            .saturating_sub(claimed_seat.base_withdrawable_balance);
        fixed.withdrawable_quote_atoms = fixed
            .withdrawable_quote_atoms
            .saturating_sub(claimed_seat.quote_withdrawable_balance);
    }

    if is_base {
        if is_increase {
            claimed_seat.base_withdrawable_balance = claimed_seat
                .base_withdrawable_balance
                .checked_add(BaseAtoms::new(amount_atoms))?;
        } else {
            require!(
                claimed_seat.base_withdrawable_balance >= BaseAtoms::new(amount_atoms),
                ProgramError::InsufficientFunds,
                "Not enough base atoms. Has {}, needs {}",
                claimed_seat.base_withdrawable_balance,
                amount_atoms
            )?;
            claimed_seat.base_withdrawable_balance = claimed_seat
                .base_withdrawable_balance
                .checked_sub(BaseAtoms::new(amount_atoms))?;
        }
    } else if is_increase {
        claimed_seat.quote_withdrawable_balance = claimed_seat
            .quote_withdrawable_balance
            .checked_add(QuoteAtoms::new(amount_atoms))?;
    } else {
        require!(
            claimed_seat.quote_withdrawable_balance >= QuoteAtoms::new(amount_atoms),
            ProgramError::InsufficientFunds,
            "Not enough quote atoms. Has {}, needs {}",
            claimed_seat.quote_withdrawable_balance,
            amount_atoms
        )?;
        claimed_seat.quote_withdrawable_balance = claimed_seat
            .quote_withdrawable_balance
            .checked_sub(QuoteAtoms::new(amount_atoms))?;
    }

    // Update the sum of withdrawable balance, second part.
    #[cfg(feature = "certora")]
    {
        fixed.withdrawable_base_atoms = fixed
            .withdrawable_base_atoms
            .saturating_add(claimed_seat.base_withdrawable_balance);
        fixed.withdrawable_quote_atoms = fixed
            .withdrawable_quote_atoms
            .saturating_add(claimed_seat.quote_withdrawable_balance);
    }

    Ok(())
}

/// Update perps position tracking for a trader after a fill.
/// `is_bid` means the trader is buying (going long).
pub fn update_perps_position(
    fixed: &mut MarketFixed,
    dynamic: &mut [u8],
    trader_index: DataIndex,
    base_atoms_traded: u64,
    quote_atoms_traded: u64,
    is_bid: bool,
) -> ProgramResult {
    let claimed_seat: &mut ClaimedSeat =
        get_mut_helper_seat(dynamic, trader_index).get_mut_value();

    let old_position: i64 = claimed_seat.get_position_size();
    let old_cost_basis: u64 = claimed_seat.get_quote_cost_basis();

    let new_position: i64 = if is_bid {
        old_position
            .checked_add(base_atoms_traded as i64)
            .ok_or(ProgramError::ArithmeticOverflow)?
    } else {
        old_position
            .checked_sub(base_atoms_traded as i64)
            .ok_or(ProgramError::ArithmeticOverflow)?
    };

    // Update cost basis
    let new_cost_basis: u64 = if old_position == 0 {
        // Opening fresh position
        quote_atoms_traded
    } else if (old_position > 0 && is_bid) || (old_position < 0 && !is_bid) {
        // Increasing position in same direction
        old_cost_basis.saturating_add(quote_atoms_traded)
    } else {
        // Reducing or flipping position
        let old_abs: u64 = old_position.unsigned_abs();
        if base_atoms_traded <= old_abs {
            // Partial close: proportionally reduce cost basis
            let closed_cost: u64 = ((old_cost_basis as u128)
                .checked_mul(base_atoms_traded as u128)
                .unwrap_or(u128::MAX)
                / (old_abs as u128)) as u64;
            old_cost_basis.saturating_sub(closed_cost)
        } else {
            // Full close + open in opposite direction
            let remaining: u64 = base_atoms_traded - old_abs;
            ((quote_atoms_traded as u128)
                .checked_mul(remaining as u128)
                .unwrap_or(u128::MAX)
                / (base_atoms_traded as u128)) as u64
        }
    };

    claimed_seat.set_position_size(new_position);
    claimed_seat.set_quote_cost_basis(new_cost_basis);

    // Update global position tracking
    #[cfg(not(feature = "certora"))]
    {
        let old_long: u64 = if old_position > 0 {
            old_position as u64
        } else {
            0
        };
        let old_short: u64 = if old_position < 0 {
            old_position.unsigned_abs()
        } else {
            0
        };
        let new_long: u64 = if new_position > 0 {
            new_position as u64
        } else {
            0
        };
        let new_short: u64 = if new_position < 0 {
            new_position.unsigned_abs()
        } else {
            0
        };

        if new_long != old_long {
            let current: u64 = fixed.get_total_long_base_atoms();
            if new_long > old_long {
                fixed.set_total_long_base_atoms(current.saturating_add(new_long - old_long));
            } else {
                fixed.set_total_long_base_atoms(current.saturating_sub(old_long - new_long));
            }
        }
        if new_short != old_short {
            let current: u64 = fixed.get_total_short_base_atoms();
            if new_short > old_short {
                fixed.set_total_short_base_atoms(current.saturating_add(new_short - old_short));
            } else {
                fixed.set_total_short_base_atoms(current.saturating_sub(old_short - new_short));
            }
        }
    }

    Ok(())
}

#[inline(always)]
fn insert_order_into_tree(
    is_bid: bool,
    fixed: &mut MarketFixed,
    dynamic: &mut [u8],
    free_address: DataIndex,
    resting_order: &RestingOrder,
) {
    let mut tree: Bookside = if is_bid {
        Bookside::new(dynamic, fixed.bids_root_index, fixed.bids_best_index)
    } else {
        Bookside::new(dynamic, fixed.asks_root_index, fixed.asks_best_index)
    };
    tree.insert(free_address, *resting_order);

    if is_bid {
        trace!(
            "insert order bid {resting_order:?} root:{}->{} max:{}->{}->{}",
            fixed.bids_root_index,
            tree.get_root_index(),
            fixed.bids_best_index,
            tree.get_max_index(),
            tree.get_next_lower_index::<RestingOrder>(tree.get_max_index()),
        );
        fixed.bids_root_index = tree.get_root_index();
        fixed.bids_best_index = tree.get_max_index();
    } else {
        trace!(
            "insert order ask {resting_order:?} root:{}->{} max:{}->{}->{}",
            fixed.asks_root_index,
            tree.get_root_index(),
            fixed.asks_best_index,
            tree.get_max_index(),
            tree.get_next_lower_index::<RestingOrder>(tree.get_max_index()),
        );
        fixed.asks_root_index = tree.get_root_index();
        fixed.asks_best_index = tree.get_max_index();
    }
    #[cfg(feature = "certora")]
    add_to_orderbook_balance(fixed, dynamic, free_address);
}

fn get_next_candidate_match_index(
    fixed: &MarketFixed,
    dynamic: &[u8],
    current_maker_order_index: DataIndex,
    is_bid: bool,
) -> DataIndex {
    if is_bid {
        let tree: BooksideReadOnly =
            BooksideReadOnly::new(dynamic, fixed.asks_root_index, fixed.asks_best_index);
        let next_order_index: DataIndex =
            tree.get_next_lower_index::<RestingOrder>(current_maker_order_index);
        next_order_index
    } else {
        let tree: BooksideReadOnly =
            BooksideReadOnly::new(dynamic, fixed.bids_root_index, fixed.bids_best_index);
        let next_order_index: DataIndex =
            tree.get_next_lower_index::<RestingOrder>(current_maker_order_index);
        next_order_index
    }
}

fn remove_and_update_balances(
    fixed: &mut MarketFixed,
    dynamic: &mut [u8],
    order_to_remove_index: DataIndex,
    global_trade_accounts_opts: &[Option<GlobalTradeAccounts>; 2],
) -> ProgramResult {
    let resting_order_to_remove: &RestingOrder =
        get_helper_order(dynamic, order_to_remove_index).get_value();
    let order_to_remove_is_bid: bool = resting_order_to_remove.get_is_bid();

    // Global order balances are accounted for on the global accounts, not on the market.
    if resting_order_to_remove.is_global() {
        if order_to_remove_is_bid {
            remove_from_global(&global_trade_accounts_opts[1])?;
        } else {
            remove_from_global(&global_trade_accounts_opts[0])?;
        }
    } else if order_to_remove_is_bid {
        // For bids: return quote atoms to the trader's balance.
        let amount_atoms_to_return: u64 = resting_order_to_remove
            .get_price()
            .checked_quote_for_base(resting_order_to_remove.get_num_base_atoms(), true)?
            .as_u64();
        update_balance(
            fixed,
            dynamic,
            resting_order_to_remove.get_trader_index(),
            !order_to_remove_is_bid,
            true,
            amount_atoms_to_return,
        )?;
    }
    // For asks: do NOT return base atoms. Base is virtual in perps,
    // and base_withdrawable_balance is repurposed for cumulative funding tracking.
    remove_order_from_tree_and_free(
        fixed,
        dynamic,
        order_to_remove_index,
        order_to_remove_is_bid,
    )?;
    Ok(())
}

#[cfg(any(test, feature = "no-clock"))]
pub fn create_empty_market(
    base_mint: &str,
    quote_mint: &str,
    base_decimals: u8,
    quote_decimals: u8,
    mint_authority: &Pubkey,
    market_key: &Pubkey,
) -> MarketFixed {
    // Values on the mints are not important.
    use solana_program::account_info::AccountInfo;
    use spl_token_2022::state::Mint;
    use std::{cell::RefCell, rc::Rc, str::FromStr};
    let mut lamports: u64 = 0;
    let base_mint: MintAccountInfo = MintAccountInfo {
        mint: Mint {
            mint_authority: Some(*mint_authority).into(),
            supply: 0,
            decimals: base_decimals,
            is_initialized: true,
            freeze_authority: None.into(),
        },
        info: &AccountInfo {
            key: &Pubkey::from_str(base_mint).expect("Valid base mint"),
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
            mint_authority: Some(*mint_authority).into(),
            supply: 0,
            decimals: quote_decimals,
            is_initialized: true,
            freeze_authority: None.into(),
        },
        info: &AccountInfo {
            key: &Pubkey::from_str(quote_mint).expect("Valid quote mint"),
            lamports: Rc::new(RefCell::new(&mut lamports)),
            data: Rc::new(RefCell::new(&mut [])),
            owner: &Pubkey::new_unique(),
            rent_epoch: 0,
            is_signer: false,
            is_writable: false,
            executable: false,
        },
    };
    let market_fixed: MarketFixed = MarketFixed::new_empty(0, base_mint.mint.decimals, &quote_mint);
    market_fixed
}
