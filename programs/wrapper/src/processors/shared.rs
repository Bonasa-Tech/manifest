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
    convert_red_black_tree_to_linked_list, get_helper, get_mut_helper, trace, DataIndex, FreeList,
    FreeListNode, HyperTreeReadOperations, HyperTreeWriteOperations, LinkedList,
    LinkedListReadOnly, RBNode, RedBlackTree, RedBlackTreeReadOnly, NIL,
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

// Layout note on the wrapper's use of hypertree.
//
// The market infos are a red-black tree keyed by market, looked up once per
// instruction (`get_market_info_index_for_market`). A wrapper holds a handful
// of markets, so that lookup is a few hundred CU and not worth a layout change.
//
// The open orders of one market used to be a red-black tree keyed by client
// order id as well, which was the wrong tool: a wrapper holds one trader's
// orders on a market, typically ten to a few dozen, every batch update walks
// all of them (`sync_fast`) and inserts or removes a few, and nothing looks an
// order up by id (cancels are matched during the walk). Measured on SBPF v2
// with 20 open orders, a tree insert was about 500 CU, a remove about 440 CU
// and a walk about 100 CU per order, all pointer chasing through 96 byte nodes
// plus rebalancing. They are now a `LinkedList` over the same blocks: an
// insert or remove is a couple of link writes and a walk step is one link
// read, a few dozen CU each. The list carries no key order: new orders go on
// at the head, and a market converted from a tree keeps the order the tree
// walk produced.
//
// Existing wrappers are converted lazily, one market at a time. The market
// info node's `payload_type` says which layout its orders are in:
// `ORDERS_LAYOUT_TREE` for market infos written before the list existed (the
// tree never set the field, so it is zero) and `ORDERS_LAYOUT_LIST` after.
// `ensure_orders_list` converts a tree in place, from `sync_fast`, which every
// instruction that reads or writes orders goes through first. Nodes keep their
// addresses and payloads, so the free list does not change.
//
// Converting under a deployed client is safe because of how the list is laid
// out rather than because clients were updated. `hypertree::LinkedList` keeps
// the previous node in `parent` and leaves `left` at NIL, so a list is a
// right-leaning tree spine: every node the right child of the one before it,
// with parent links that agree. The client's tree parser checks exactly that,
// plus offsets and cycles, and nothing about key ordering or balance, and the
// in-order walk of a right spine is the spine itself. So it reads a converted
// market and returns the same orders in the same sequence, and needs no
// knowledge of any of this. The client-side test `wrapperLayout.ts` pins that,
// and `lib/src/linked_list.rs` pins the shape from this side.
//
// The compatibility is with that specific parser, not with red-black tree
// readers in general: `hypertree::validate_red_black_tree` checks the ordering
// and black-height invariants, which a spine does not satisfy.
pub type MarketInfosTree<'a> = RedBlackTree<'a, MarketInfo>;
pub type MarketInfosTreeReadOnly<'a> = RedBlackTreeReadOnly<'a, MarketInfo>;
pub type OpenOrdersList<'a> = LinkedList<'a, WrapperOpenOrder>;
pub type OpenOrdersListReadOnly<'a> = LinkedListReadOnly<'a, WrapperOpenOrder>;

/// `payload_type` of a market info node whose open orders are a red-black
/// tree, the layout before the list. Zero because the tree never set it.
pub const ORDERS_LAYOUT_TREE: u8 = 0;
/// `payload_type` of a market info node whose open orders are an
/// [`OpenOrdersList`].
pub const ORDERS_LAYOUT_LIST: u8 = 1;

pub const WRAPPER_BLOCK_PAYLOAD_SIZE: usize = 80;
pub const BLOCK_HEADER_SIZE: usize = 16;
pub const WRAPPER_BLOCK_SIZE: usize = WRAPPER_BLOCK_PAYLOAD_SIZE + BLOCK_HEADER_SIZE;

// Node reads land on block boundaries and need them aligned; see
// `hypertree::get_helper`.
const_assert_eq!(WRAPPER_BLOCK_SIZE % 8, 0);

// This is the maximum number of order ids/cancels assembled for one core CPI;
// it is not a market traversal budget. Bounded traversals have their own
// explicit step quotas and persistent progress state.
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

/// Converts the market's open orders from the tree layout to the list in
/// place if that has not happened yet. Costs one tree walk and two link writes
/// per open order, once per market: 93 CU per order measured on SBPF v2, so
/// about 15,000 orders inside a transaction raised to the 1.4M CU limit and
/// about 2,100 inside the 200,000 an instruction gets by default.
///
/// Not a size anyone reaches. Nothing caps a market's open order count, but
/// each open order is a 96 byte wrapper block and an 80 byte block on the
/// core, so 15,000 of them is about 10 SOL of rent on the wrapper and another
/// 8 on the market. Cancelling does not give that back: freed blocks go on a
/// free list to be reused, neither account ever shrinks, and `collect` leaves
/// the balance the current size needs. The rent is spent for the life of the
/// accounts. Well before that it stops working for ordinary reasons: placing
/// an order leaves the market non-quiet, so the next instruction walks every
/// open order on it at a comparable cost per order, and a wrapper that large
/// could not place or cancel anything either. A wrapper stuck there is not
/// stuck holding funds, the balances and resting orders are on the core and
/// can be cancelled and withdrawn against directly. See
/// `migrate_a_large_legacy_tree_test`.
pub(crate) fn ensure_orders_list(wrapper_dynamic_data: &mut [u8], market_info_index: DataIndex) {
    let market_info_node: &mut RBNode<MarketInfo> =
        get_mut_helper::<RBNode<MarketInfo>>(wrapper_dynamic_data, market_info_index);
    if market_info_node.get_payload_type() == ORDERS_LAYOUT_LIST {
        return;
    }
    let orders_root_index: DataIndex = market_info_node.get_value().orders_root_index;
    let orders_head_index: DataIndex = convert_red_black_tree_to_linked_list::<WrapperOpenOrder>(
        wrapper_dynamic_data,
        orders_root_index,
    );
    let market_info_node: &mut RBNode<MarketInfo> =
        get_mut_helper::<RBNode<MarketInfo>>(wrapper_dynamic_data, market_info_index);
    market_info_node.get_mut_value().orders_root_index = orders_head_index;
    market_info_node.set_payload_type(ORDERS_LAYOUT_LIST);
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
    /// cancel_all is bounded to EXPECTED_ORDER_BATCH_SIZE wrapper-tracked
    /// cancels per transaction so its CPI work stays bounded; shared market
    /// size cannot affect this path. Orders placed directly through the core
    /// are intentionally excluded and remain cancellable by sequence
    /// number/index. The rest of the tracked orders are left for a retry.
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
///
/// Also where a market's orders get converted from the tree layout to the
/// list, since every instruction that touches orders comes through here
/// first.
pub(crate) fn sync_fast(
    wrapper_state: &WrapperStateAccountInfo,
    market: &ManifestAccountInfo<MarketFixed>,
    market_info_index: DataIndex,
    _now_slot: u32,
    orders_exact: bool,
    mut matcher: Option<&mut CancelMatcher>,
) -> ProgramResult {
    let market_data: Ref<'_, &mut [u8]> = market.try_borrow_data()?;
    let market_ref = get_dynamic_account::<MarketFixed>(&market_data);
    let market_sequence_number: u64 = market_ref.fixed.get_order_sequence_number();

    let mut wrapper_data: RefMut<&mut [u8]> = wrapper_state.info.try_borrow_mut_data()?;
    let (fixed_data, wrapper_dynamic_data) =
        wrapper_data.split_at_mut(size_of::<ManifestWrapperStateFixed>());
    ensure_orders_list(wrapper_dynamic_data, market_info_index);

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
        // One pass over the list, updating orders in place, unlinking the
        // ones the core no longer has and matching the cancels. Only the
        // unlinked indices are kept, for the free list.
        let mut orders: OpenOrdersList =
            OpenOrdersList::new(wrapper_dynamic_data, orders_root_index);
        let mut to_free_indices: Vec<DataIndex> = Vec::new();
        let mut num_open_global_orders: u32 = 0;
        let mut order_index: DataIndex = orders_root_index;
        while order_index != NIL {
            let next_index: DataIndex = orders.get_next_index(order_index);
            // An order this batch is about to cancel is read from the core
            // even when the rest of the walk is being skipped. The cancel
            // carries this entry's index as a hint, the core validates every
            // cancel in a batch before processing any placement, and one hint
            // pointing at an order that is no longer there fails the whole
            // instruction. That is also the only way an entry left behind by
            // the gap described above gets cleared, since the batch that
            // would clear it cannot run if it aborts first.
            let is_cancel_candidate: bool = matcher
                .as_ref()
                .is_some_and(|m| m.matches(orders.get_mut_value(order_index)));
            let gone: bool = (read_core || is_cancel_candidate) && {
                let order: &mut WrapperOpenOrder = orders.get_mut_value(order_index);
                let core_resting_order: &RestingOrder = get_helper::<RBNode<RestingOrder>>(
                    market_ref.dynamic,
                    order.get_market_data_index(),
                )
                .get_value();
                // Verifies that it is not just zeroed and happens to match
                // seq num, also check that there are base atoms left.
                if core_resting_order.get_sequence_number() != order.get_order_sequence_number()
                    || core_resting_order.get_num_base_atoms() == BaseAtoms::ZERO
                {
                    true
                } else {
                    if read_core {
                        order.update_remaining(core_resting_order.get_num_base_atoms());
                        order.set_price(core_resting_order.get_price());
                        if order.get_order_type() == OrderType::Global {
                            num_open_global_orders += 1;
                        }
                    }
                    false
                }
            };
            if gone {
                orders.remove_by_index(order_index);
                to_free_indices.push(order_index);
            } else if is_cancel_candidate {
                if let Some(matcher) = matcher.as_deref_mut() {
                    matcher.record(order_index, orders.get_mut_value(order_index));
                }
            }
            order_index = next_index;
        }
        orders_root_index = orders.get_root_index();

        // Entries can be dropped on either path: the full walk drops
        // everything the core no longer has, and the cheap walk drops a
        // cancel candidate it found stale.
        if !to_free_indices.is_empty() {
            let wrapper_fixed: &mut ManifestWrapperStateFixed = get_mut_helper(fixed_data, 0);
            let mut free_list: FreeList<UnusedWrapperFreeListPadding> =
                FreeList::new(wrapper_dynamic_data, wrapper_fixed.free_list_head_index);
            for open_order_index in to_free_indices.iter() {
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
    // The cancel-all cursor is independent of sync freshness and must not be
    // overwritten here.
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
