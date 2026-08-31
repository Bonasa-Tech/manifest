use std::{
    cell::{Ref, RefMut},
    mem::size_of,
};

use crate::{
    loader::WrapperStateAccountInfo, market_info::MarketInfo, open_order::WrapperOpenOrder,
    processors::batch_upate::WrapperCancelOrderParams, wrapper_state::ManifestWrapperStateFixed,
};
use bytemuck::{Pod, Zeroable};
use hypertree::{
    get_helper, get_mut_helper, trace, DataIndex, FreeList, FreeListNode, HyperTreeReadOperations,
    HyperTreeValueIteratorTrait, HyperTreeWriteOperations, RBNode, RedBlackTree,
    RedBlackTreeReadOnly, NIL,
};
use manifest::{
    program::{batch_update::CancelOrderParams, get_dynamic_account, invoke},
    quantities::{BaseAtoms, QuoteAtoms},
    state::{
        claimed_seat::ClaimedSeat, get_helper_seat, utils::get_now_slot, MarketFixed, OrderType,
        RestingOrder,
    },
    validation::{ManifestAccountInfo, Program, Signer},
};
use solana_program::{
    account_info::AccountInfo,
    entrypoint::ProgramResult,
    program_error::ProgramError,
    pubkey::Pubkey,
    system_instruction,
    sysvar::{rent::Rent, Sysvar},
};
use static_assertions::const_assert_eq;

// CU note on the wrapper's use of hypertree.
//
// The wrapper keeps its per-market open orders (keyed by client order id) and
// its market infos in red-black trees, the same structure the core uses for a
// book that can hold thousands of orders. A wrapper holds one trader's orders,
// typically ten to a few dozen, and at that size the tree is the wrong tool:
// measured on SBPF v2 with a maker holding 20 open orders, one
// `OpenOrdersTree::insert` is about 500 CU, one `remove_by_index` about 440
// CU, and walking the tree costs about 100 CU per order, all of it pointer
// chasing through 96 byte nodes plus rebalancing. Every batch update walks
// the open orders, so those costs are paid per open order per transaction,
// not just per placed order.
//
// A structure without ordering (the orders are only ever walked in full, and
// cancels are matched during that walk) would make an insert and a remove a
// couple of link writes and a walk step a single read, which at these sizes
// is several times cheaper and only loses to the tree at hundreds of open
// orders per market, more than a wrapper account is meant to hold. That is a
// wrapper state layout change, so it is left for its own change; it is the
// largest remaining wrapper-side saving. The same reasoning does not apply to
// `MarketInfosTree`, which is looked up by key once per instruction and holds
// a handful of markets.
pub type MarketInfosTree<'a> = RedBlackTree<'a, MarketInfo>;
pub type MarketInfosTreeReadOnly<'a> = RedBlackTreeReadOnly<'a, MarketInfo>;
pub type OpenOrdersTree<'a> = RedBlackTree<'a, WrapperOpenOrder>;
pub type OpenOrdersTreeReadOnly<'a> = RedBlackTreeReadOnly<'a, WrapperOpenOrder>;

pub const WRAPPER_BLOCK_PAYLOAD_SIZE: usize = 80;
pub const BLOCK_HEADER_SIZE: usize = 16;
pub const WRAPPER_BLOCK_SIZE: usize = WRAPPER_BLOCK_PAYLOAD_SIZE + BLOCK_HEADER_SIZE;

pub const EXPECTED_ORDER_BATCH_SIZE: usize = 16;

/// Blocks added per wrapper expansion. Growing costs a system transfer CPI
/// plus a realloc (about 2,000 CU), so a maker whose open order count is
/// climbing pays it once per this many new orders instead of once per order.
pub const WRAPPER_EXPAND_BLOCKS: usize = 4;

#[repr(C, packed)]
#[derive(Default, Copy, Clone, Pod, Zeroable)]
pub struct UnusedWrapperFreeListPadding {
    _padding: [u64; 9],
    _padding2: [u32; 5],
}
pub const FREE_LIST_HEADER_SIZE: usize = 4;
// Assert that the free list blocks take up the same size as regular blocks.
const_assert_eq!(
    size_of::<UnusedWrapperFreeListPadding>(),
    WRAPPER_BLOCK_SIZE - FREE_LIST_HEADER_SIZE
);
// Does not align to 8 bytes but not necessary
// const_assert_eq!(size_of::<UnusedWrapperFreeListPadding>() % 8, 0);

/// Makes sure the wrapper has a free slot, expanding by
/// [`WRAPPER_EXPAND_BLOCKS`] when it has none.
pub(crate) fn expand_wrapper_if_needed<'a, 'info>(
    wrapper_state_account_info: &WrapperStateAccountInfo<'a, 'info>,
    payer: &Signer<'a, 'info>,
    system_program: &Program<'a, 'info>,
) -> ProgramResult {
    ensure_free_slots(wrapper_state_account_info, payer, system_program, 1)
}

/// Makes sure at least `needed` free slots exist, growing the wrapper once
/// (one system transfer CPI and one realloc) by a multiple of
/// [`WRAPPER_EXPAND_BLOCKS`] if not.
pub(crate) fn ensure_free_slots<'a, 'info>(
    wrapper_state_account_info: &WrapperStateAccountInfo<'a, 'info>,
    payer: &Signer<'a, 'info>,
    system_program: &Program<'a, 'info>,
    needed: usize,
) -> ProgramResult {
    let free: usize = count_free_slots(wrapper_state_account_info, needed);
    if free >= needed {
        return Ok(());
    }
    let missing: usize = needed - free;
    let blocks: usize = missing.div_ceil(WRAPPER_EXPAND_BLOCKS) * WRAPPER_EXPAND_BLOCKS;

    {
        let wrapper_state: &AccountInfo = wrapper_state_account_info.info;

        let wrapper_data: Ref<&mut [u8]> = wrapper_state.try_borrow_data()?;
        let old_size: usize = wrapper_data.len();
        let new_size: usize = old_size + WRAPPER_BLOCK_SIZE * blocks;
        drop(wrapper_data);
        let rent: Rent = Rent::get()?;
        let new_minimum_balance: u64 = rent.minimum_balance(new_size);
        let old_minimum_balance: u64 = rent.minimum_balance(old_size);
        let lamports_diff: u64 = new_minimum_balance.saturating_sub(old_minimum_balance);
        invoke(
            &system_instruction::transfer(payer.key, wrapper_state.key, lamports_diff),
            &[
                payer.info.clone(),
                wrapper_state.clone(),
                system_program.info.clone(),
            ],
        )?;
        trace!(
            "expand_if_needed -> realloc {} {:?}",
            new_size,
            wrapper_state.key
        );

        #[cfg(feature = "fuzz")]
        {
            solana_program::program::invoke(
                &system_instruction::allocate(wrapper_state.key, new_size as u64),
                &[wrapper_state.clone(), system_program.info.clone()],
            )?;
        }
        #[cfg(not(feature = "fuzz"))]
        {
            #[allow(deprecated)]
            wrapper_state.realloc(new_size, false)?;
        }
    }

    let wrapper_state_info: &AccountInfo = wrapper_state_account_info.info;
    let wrapper_data: &mut [u8] = &mut wrapper_state_info.try_borrow_mut_data().unwrap();
    for _ in 0..blocks {
        expand_wrapper(wrapper_data);
    }
    Ok(())
}

/// Number of free slots, counting at most `limit` of them.
fn count_free_slots(wrapper_state: &WrapperStateAccountInfo, limit: usize) -> usize {
    let wrapper_data: Ref<&mut [u8]> = wrapper_state.info.try_borrow_data().unwrap();
    let (fixed_data, dynamic_data) = wrapper_data.split_at(size_of::<ManifestWrapperStateFixed>());
    let wrapper_fixed: &ManifestWrapperStateFixed = get_helper(fixed_data, 0);
    let mut index: DataIndex = wrapper_fixed.free_list_head_index;
    let mut count: usize = 0;
    while index != NIL && count < limit {
        count += 1;
        index = get_helper::<FreeListNode<UnusedWrapperFreeListPadding>>(dynamic_data, index)
            .get_next_index();
    }
    count
}

pub fn expand_wrapper(wrapper_data: &mut [u8]) {
    let (fixed_data, dynamic_data) =
        wrapper_data.split_at_mut(size_of::<ManifestWrapperStateFixed>());

    let wrapper_fixed: &mut ManifestWrapperStateFixed = get_mut_helper(fixed_data, 0);
    let mut free_list: FreeList<UnusedWrapperFreeListPadding> =
        FreeList::new(dynamic_data, wrapper_fixed.free_list_head_index);

    free_list.add(wrapper_fixed.num_bytes_allocated);
    wrapper_fixed.num_bytes_allocated += WRAPPER_BLOCK_SIZE as u32;
    wrapper_fixed.free_list_head_index = free_list.get_head();
}

pub(crate) fn sync(
    wrapper_state: &WrapperStateAccountInfo,
    market: &ManifestAccountInfo<MarketFixed>,
) -> ProgramResult {
    let market_info_index: DataIndex =
        get_market_info_index_for_market(wrapper_state, market.info.key);
    sync_fast(
        wrapper_state,
        market,
        market_info_index,
        get_now_slot(),
        false,
        None,
    )
}

/// Cancels to match against the open orders while `sync_fast` walks them, so
/// the walk happens once. Collects the wrapper indices, the core cancels with
/// index hints, and the funds those cancels free up (only priced when a new
/// order needs the balance check, since pricing a bid is u128 math).
pub(crate) struct CancelMatcher<'a> {
    pub cancels: &'a [WrapperCancelOrderParams],
    pub cancel_all: bool,
    pub needs_base: bool,
    pub needs_quote: bool,
    pub wrapper_indices: Vec<DataIndex>,
    pub core_cancels: Vec<CancelOrderParams>,
    pub freed_base_atoms: BaseAtoms,
    pub freed_quote_atoms: QuoteAtoms,
}

impl<'a> CancelMatcher<'a> {
    pub fn new(
        cancels: &'a [WrapperCancelOrderParams],
        cancel_all: bool,
        needs_base: bool,
        needs_quote: bool,
    ) -> Self {
        CancelMatcher {
            cancels,
            cancel_all,
            needs_base,
            needs_quote,
            wrapper_indices: Vec::with_capacity(EXPECTED_ORDER_BATCH_SIZE),
            core_cancels: Vec::with_capacity(EXPECTED_ORDER_BATCH_SIZE),
            freed_base_atoms: BaseAtoms::ZERO,
            freed_quote_atoms: QuoteAtoms::ZERO,
        }
    }

    fn is_empty(&self) -> bool {
        !self.cancel_all && self.cancels.is_empty()
    }

    /// Whether this open order is one the batch asked to cancel.
    ///
    /// A batch is a handful of ids, so comparing each open order against all
    /// of them costs a few CU per order, where hashing cost hundreds.
    /// cancel_all is bounded to EXPECTED_ORDER_BATCH_SIZE cancels per
    /// transaction so its cost stays bounded, the rest is left for a retry.
    fn matches(&self, order: &WrapperOpenOrder) -> bool {
        if self.cancel_all {
            self.core_cancels.len() < EXPECTED_ORDER_BATCH_SIZE
        } else {
            self.cancels
                .iter()
                .any(|cancel: &WrapperCancelOrderParams| {
                    cancel.client_order_id() == order.get_client_order_id()
                })
        }
    }

    /// Records an open order as cancelled. Only call after [`Self::matches`]
    /// and after the order has been checked against the core, because the
    /// hint recorded here has to be right: the core validates every cancel in
    /// a batch before it processes any placement, so one stale hint fails the
    /// whole instruction.
    fn record(&mut self, wrapper_index: DataIndex, order: &WrapperOpenOrder) {
        self.wrapper_indices.push(wrapper_index);
        self.core_cancels.push(CancelOrderParams::new_with_hint(
            order.get_order_sequence_number(),
            Some(order.get_market_data_index()),
        ));
        if order.get_is_bid() {
            if self.needs_quote {
                self.freed_quote_atoms += order
                    .get_price()
                    .checked_quote_for_base(order.get_num_base_atoms(), true)
                    .unwrap();
            }
        } else if self.needs_base {
            self.freed_base_atoms += order.get_num_base_atoms();
        }
    }
}

/// Refreshes the wrapper's view of one market: the balances always, and the
/// open orders unless they are known to be exact.
///
/// The open orders are walked against the core when needed: orders that are
/// gone or empty on the core are dropped, the rest get their remaining size
/// and price updated. They are known to be exact, so the walk (about 180 CU
/// per open order) is skipped, when either:
///
/// * `orders_exact` says so: the caller placed nothing this batch, so the
///   core ran no matching and cannot have touched an order this instruction
///   did not touch itself, or
/// * nothing that can touch this trader's orders happened on the market since
///   the last exact sync. Every fill, expiry pruning and reverse order comes
///   with an order placement, which bumps the market's order sequence number;
///   cancels and withdrawals that did not go through this wrapper change the
///   seat's withdrawable balances; and global clean or evict can only remove
///   global orders, so those are counted and force a walk while any is open.
///   The one gap is a cancel made directly on the core followed by a
///   withdrawal of exactly the freed amount, which moves neither of the two
///   things this looks at and so leaves an entry here for an order the core
///   no longer has.
///
/// That entry costs nothing and does not last. Cancelling it succeeds,
/// because a cancel candidate is read from the core below whether or not the
/// walk is being skipped, and an entry found gone is dropped rather than
/// forwarded; that check is not optional, since the core validates every
/// cancel in a batch before processing any placement, so forwarding a stale
/// hint would fail the whole instruction. `cancel_all` reaches it the same
/// way. Any batch that places an order clears it too, because the sync after
/// that CPI re-reads all of them. Until one of those happens it holds a
/// wrapper slot and nothing else.
///
/// When `matcher` is given the walk also matches the cancels, so the orders
/// are only walked once per batch update; on a skipped walk only the wrapper
/// nodes and the few orders being cancelled are read.
pub(crate) fn sync_fast(
    wrapper_state: &WrapperStateAccountInfo,
    market: &ManifestAccountInfo<MarketFixed>,
    market_info_index: DataIndex,
    now_slot: u32,
    orders_exact: bool,
    mut matcher: Option<&mut CancelMatcher>,
) -> ProgramResult {
    let market_data: Ref<'_, &mut [u8]> = market.try_borrow_data()?;
    let market_ref = get_dynamic_account::<MarketFixed>(&market_data);
    let market_sequence_number: u64 = market_ref.fixed.get_order_sequence_number();

    let mut wrapper_data: RefMut<&mut [u8]> = wrapper_state.info.try_borrow_mut_data()?;
    let (fixed_data, wrapper_dynamic_data) =
        wrapper_data.split_at_mut(size_of::<ManifestWrapperStateFixed>());

    let market_info: &mut MarketInfo =
        get_mut_helper::<RBNode<MarketInfo>>(wrapper_dynamic_data, market_info_index)
            .get_mut_value();
    let claimed_seat: &ClaimedSeat =
        get_helper_seat(market_ref.dynamic, market_info.trader_index).get_value();
    let quiet: bool = market_info.last_synced_order_sequence_number == market_sequence_number
        && market_info.num_open_global_orders == 0
        && claimed_seat.base_withdrawable_balance == market_info.base_balance
        && claimed_seat.quote_withdrawable_balance == market_info.quote_balance;
    let read_core: bool = !orders_exact && !quiet;
    let mut orders_root_index: DataIndex = market_info.orders_root_index;
    let match_cancels: bool = matcher.as_ref().is_some_and(|m| !m.is_empty());

    if orders_root_index != NIL && (read_core || match_cancels) {
        let orders_tree: OpenOrdersTreeReadOnly =
            OpenOrdersTreeReadOnly::new(wrapper_dynamic_data, orders_root_index, NIL);

        // Walk the tree once to collect where each open order lives on the
        // core (the iterator borrows the tree, so nodes cannot be updated
        // while walking), then handle each order in place.
        let mut to_remove_indices: Vec<DataIndex> = Vec::with_capacity(EXPECTED_ORDER_BATCH_SIZE);
        let mut to_update_and_core_indices: Vec<(DataIndex, DataIndex)> =
            Vec::with_capacity(EXPECTED_ORDER_BATCH_SIZE);
        for (order_index, order) in orders_tree.iter::<WrapperOpenOrder>() {
            to_update_and_core_indices.push((order_index, order.get_market_data_index()));
        }
        let mut num_open_global_orders: u32 = 0;
        for (order_index, core_data_index) in to_update_and_core_indices.iter() {
            let node: &mut WrapperOpenOrder =
                get_mut_helper::<RBNode<WrapperOpenOrder>>(wrapper_dynamic_data, *order_index)
                    .get_mut_value();
            // An order this batch is about to cancel is read from the core
            // even when the rest of the walk is being skipped. The cancel
            // carries this entry's index as a hint, the core validates every
            // cancel in a batch before processing any placement, and one hint
            // pointing at an order that is no longer there fails the whole
            // instruction. That is also the only way an entry left behind by
            // the gap described above gets cleared, since the batch that
            // would clear it cannot run if it aborts first.
            let is_cancel_candidate: bool = matcher.as_ref().is_some_and(|m| m.matches(node));
            if read_core || is_cancel_candidate {
                let core_resting_order: &RestingOrder =
                    get_helper::<RBNode<RestingOrder>>(market_ref.dynamic, *core_data_index)
                        .get_value();
                // Verifies that it is not just zeroed and happens to match
                // seq num, also check that there are base atoms left.
                if core_resting_order.get_sequence_number() != node.get_order_sequence_number()
                    || core_resting_order.get_num_base_atoms() == BaseAtoms::ZERO
                {
                    to_remove_indices.push(*order_index);
                    continue;
                }
                if read_core {
                    node.update_remaining(core_resting_order.get_num_base_atoms());
                    node.set_price(core_resting_order.get_price());
                    if node.get_order_type() == OrderType::Global {
                        num_open_global_orders += 1;
                    }
                }
            }
            if is_cancel_candidate {
                if let Some(matcher) = matcher.as_deref_mut() {
                    matcher.record(*order_index, node);
                }
            }
        }

        // Entries can be dropped on either path: the full walk drops
        // everything the core no longer has, and the cheap walk drops a
        // cancel candidate it found stale.
        if !to_remove_indices.is_empty() {
            let mut orders_tree: RedBlackTree<WrapperOpenOrder> =
                RedBlackTree::<WrapperOpenOrder>::new(wrapper_dynamic_data, orders_root_index, NIL);
            for to_remove_index in to_remove_indices.iter() {
                orders_tree.remove_by_index(*to_remove_index);
            }
            orders_root_index = orders_tree.get_root_index();

            let wrapper_fixed: &mut ManifestWrapperStateFixed = get_mut_helper(fixed_data, 0);
            let mut free_list: FreeList<UnusedWrapperFreeListPadding> =
                FreeList::new(wrapper_dynamic_data, wrapper_fixed.free_list_head_index);
            for open_order_index in to_remove_indices.iter() {
                free_list.add(*open_order_index);
            }
            wrapper_fixed.free_list_head_index = free_list.get_head();
        }

        // Only the full walk counts every order, and it is the only path that
        // can reach a market with global orders open: a market with any is
        // never quiet.
        if read_core {
            let market_info: &mut MarketInfo =
                get_mut_helper::<RBNode<MarketInfo>>(wrapper_dynamic_data, market_info_index)
                    .get_mut_value();
            market_info.num_open_global_orders = num_open_global_orders;
        }
    }

    let market_info: &mut MarketInfo =
        get_mut_helper::<RBNode<MarketInfo>>(wrapper_dynamic_data, market_info_index)
            .get_mut_value();
    market_info.orders_root_index = orders_root_index;
    // The view is exact now, whichever way it got there.
    market_info.last_synced_order_sequence_number = market_sequence_number;
    market_info.base_balance = claimed_seat.base_withdrawable_balance;
    market_info.quote_balance = claimed_seat.quote_withdrawable_balance;
    market_info.quote_volume = claimed_seat.quote_volume;
    market_info.last_updated_slot = now_slot;
    Ok(())
}

pub(crate) fn get_market_info_index_for_market(
    wrapper_state: &WrapperStateAccountInfo,
    market: &Pubkey,
) -> DataIndex {
    let mut wrapper_data: RefMut<&mut [u8]> = wrapper_state.info.try_borrow_mut_data().unwrap();
    let (fixed_data, wrapper_dynamic_data) =
        wrapper_data.split_at_mut(size_of::<ManifestWrapperStateFixed>());

    let wrapper_fixed: &ManifestWrapperStateFixed = get_helper(fixed_data, 0);
    let market_infos_tree: MarketInfosTree = MarketInfosTree::new(
        wrapper_dynamic_data,
        wrapper_fixed.market_infos_root_index,
        NIL,
    );

    // Just need to lookup by market key so the rest doesnt matter.
    let market_info_index: DataIndex =
        market_infos_tree.lookup_index(&MarketInfo::new_empty(*market, NIL));
    market_info_index
}

pub(crate) fn get_trader_index_hint_for_market(
    wrapper_state: &WrapperStateAccountInfo,
    market_key: &Pubkey,
) -> Result<Option<DataIndex>, ProgramError> {
    let market_info_index: DataIndex = get_market_info_index_for_market(wrapper_state, market_key);

    let wrapper_data: Ref<&mut [u8]> = wrapper_state.info.try_borrow_data()?;
    let (_fixed_data, wrapper_dynamic_data) =
        wrapper_data.split_at(size_of::<ManifestWrapperStateFixed>());
    let market_info: MarketInfo =
        *get_helper::<RBNode<MarketInfo>>(wrapper_dynamic_data, market_info_index).get_value();
    let trader_index_hint: Option<DataIndex> = Some(market_info.trader_index);
    Ok(trader_index_hint)
}
