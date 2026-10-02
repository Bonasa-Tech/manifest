//! Bounded, resumable compaction. Relink nodes without changing tree shape:
//! removing and reinserting equal-price orders through the normal insertion
//! path would change their FIFO priority. Order identities, balances and global
//! gas deposits stay put.
//!
//! A run relocates at most a caller supplied number of nodes and leaves the
//! market fully consistent and no larger than it started, so a market too big
//! to compact in one transaction is compacted by repeating the call. Repeated
//! bounded runs converge on the layout one unbounded run would produce.
use super::*;

/// Byte offsets inside an `RBNode<V>` header. Identical for every payload.
const NODE_LEFT: usize = 0;
const NODE_RIGHT: usize = 4;
const NODE_PARENT: usize = 8;
const NODE_TYPE: usize = 13;
const NODE_VALUE: usize = 16;
/// `RestingOrder::trader_index` sits 32 bytes into the payload, after the
/// 16 byte price, 8 byte size and 8 byte sequence number. Pinned by
/// `node_layout_offsets_match_accessors`.
const ORDER_TRADER: usize = NODE_VALUE + 32;

const_assert_eq!(size_of::<RBNode<ClaimedSeat>>(), MARKET_BLOCK_SIZE);
const_assert_eq!(size_of::<RBNode<RestingOrder>>(), MARKET_BLOCK_SIZE);

/// Largest market this will compact. The bitmap is one bit per block, so this
/// caps that allocation at 8 KiB of a 32 KiB heap. This does not bound the
/// moved-seat buffer or the cost of rebuilding the surviving seat tree.
pub const MAX_DEFRAG_BLOCKS: usize = 65_536;

fn bit(words: &[u64], block: usize) -> bool {
    words[block / 64] & (1u64 << (block % 64)) != 0
}
fn set_bit(words: &mut [u64], block: usize) {
    words[block / 64] |= 1u64 << (block % 64);
}
fn clear_bit(words: &mut [u64], block: usize) {
    words[block / 64] &= !(1u64 << (block % 64));
}

/// Ascending block indices whose bit is set, stepping over runs of zeroes a
/// word at a time. A market is mostly reclaimable seats and free space, so
/// walking every block to find the survivors costs far more than the work.
struct SetBlocks<'a> {
    words: &'a [u64],
    word: u64,
    at: usize,
}
impl<'a> SetBlocks<'a> {
    fn new(words: &'a [u64]) -> Self {
        SetBlocks {
            words,
            word: words.first().copied().unwrap_or(0),
            at: 0,
        }
    }
}
impl Iterator for SetBlocks<'_> {
    type Item = usize;
    fn next(&mut self) -> Option<usize> {
        loop {
            if self.word != 0 {
                let block: usize = self.at * 64 + self.word.trailing_zeros() as usize;
                self.word &= self.word - 1;
                return Some(block);
            }
            self.at += 1;
            self.word = *self.words.get(self.at)?;
        }
    }
}

fn read_index(data: &[u8], at: usize) -> DataIndex {
    DataIndex::from_le_bytes(data[at..at + 4].try_into().unwrap())
}
fn write_index(data: &mut [u8], at: usize, value: DataIndex) {
    data[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

/// Slide one node to a lower block. The order trees are kept, so an order's
/// neighbours are repointed here. The seat tree is rebuilt from the survivors
/// instead, and a seat's stale links must not be followed on the way: the block
/// a link names may already have been handed out as a destination.
/// `old` and `new` are block aligned and cannot overlap because `new < old`.
fn relocate(
    fixed: &mut MarketFixed,
    dynamic: &mut [u8],
    old: usize,
    new: usize,
    moved_seats: &mut Vec<(DataIndex, DataIndex)>,
) -> Result<(), ProgramError> {
    // The vacated block needs no blanking: it is below the frontier, where the
    // free list rebuild zeroes it, or above it, where it is truncated away.
    dynamic.copy_within(old..old + MARKET_BLOCK_SIZE, new);

    let old_index: DataIndex = old as DataIndex;
    let new_index: DataIndex = new as DataIndex;
    if dynamic[new + NODE_TYPE] == MarketDataTreeNodeType::ClaimedSeat as u8 {
        // Orders name their owner by index and the rebuilt tree cannot fix
        // that. Patched in one pass afterwards, so the book is walked once per
        // run rather than once per moved seat.
        moved_seats.push((old_index, new_index));
        return Ok(());
    }
    let left: DataIndex = read_index(dynamic, new + NODE_LEFT);
    let right: DataIndex = read_index(dynamic, new + NODE_RIGHT);
    let parent: DataIndex = read_index(dynamic, new + NODE_PARENT);
    if left != NIL {
        write_index(dynamic, left as usize + NODE_PARENT, new_index);
    }
    if right != NIL {
        write_index(dynamic, right as usize + NODE_PARENT, new_index);
    }
    if parent != NIL {
        // Exactly one of the parent's two children named this node.
        let side: usize = if read_index(dynamic, parent as usize + NODE_LEFT) == old_index {
            NODE_LEFT
        } else if read_index(dynamic, parent as usize + NODE_RIGHT) == old_index {
            NODE_RIGHT
        } else {
            return Err(ProgramError::InvalidAccountData);
        };
        write_index(dynamic, parent as usize + side, new_index);
    } else if get_helper::<RBNode<RestingOrder>>(dynamic, new_index)
        .get_value()
        .get_is_bid()
    {
        fixed.bids_root_index = new_index;
    } else {
        fixed.asks_root_index = new_index;
    }
    if fixed.bids_best_index == old_index {
        fixed.bids_best_index = new_index;
    }
    if fixed.asks_best_index == old_index {
        fixed.asks_best_index = new_index;
    }
    Ok(())
}

impl<
        Fixed: DerefOrBorrowMut<MarketFixed> + DerefOrBorrow<MarketFixed>,
        Dynamic: DerefOrBorrowMut<[u8]> + DerefOrBorrow<[u8]>,
    > DynamicAccount<Fixed, Dynamic>
{
    /// Caller ensures at least two free blocks exist and truncates to the
    /// returned size after dropping all account-data borrows.
    ///
    /// `limit` caps how many nodes this run relocates; zero is unbounded.
    /// Reclaiming empty seats is not capped because it costs no tree work: the
    /// seat tree is rebuilt from the survivors whether one seat goes or all of
    /// them do. The other cost a budget cannot bound is the single pass over
    /// allocated blocks, which is what `MAX_DEFRAG_BLOCKS` exists to keep
    /// affordable. Simulate a market's first run to pick a budget that fits.
    pub fn defragment(&mut self, limit: u32) -> Result<usize, ProgramError> {
        let DynamicAccount { fixed, dynamic } = self.borrow_mut();
        let allocated: usize = fixed.num_bytes_allocated as usize;
        require!(
            allocated <= dynamic.len() && allocated % MARKET_BLOCK_SIZE == 0,
            ProgramError::InvalidAccountData,
            "Invalid allocation"
        )?;
        let blocks: usize = allocated / MARKET_BLOCK_SIZE;
        require!(
            blocks <= MAX_DEFRAG_BLOCKS,
            ProgramError::InvalidAccountData,
            "Market has {} blocks, more than the {} this can compact",
            blocks,
            MAX_DEFRAG_BLOCKS
        )?;

        // Mark what survives. `FreeList::add` zeroes the payload including the
        // node type, so free blocks read as `Empty`. Scanning allocated blocks
        // once marks every order together with its owner, including owners
        // whose balances are zero.
        let mut keep: Vec<u64> = vec![0u64; blocks.div_ceil(64)];
        for block in 0..blocks {
            let index: usize = block * MARKET_BLOCK_SIZE;
            match dynamic[index + NODE_TYPE] {
                kind if kind == MarketDataTreeNodeType::RestingOrder as u8 => {
                    set_bit(&mut keep, block);
                    let owner: DataIndex = read_index(dynamic, index + ORDER_TRADER);
                    require!(
                        owner != NIL
                            && (owner as usize) < allocated
                            && owner as usize % MARKET_BLOCK_SIZE == 0,
                        ProgramError::InvalidAccountData,
                        "Resting order at {} has invalid owner {}",
                        index,
                        owner
                    )?;
                    set_bit(&mut keep, owner as usize / MARKET_BLOCK_SIZE);
                }
                kind if kind == MarketDataTreeNodeType::ClaimedSeat as u8 => {
                    let seat: &ClaimedSeat =
                        get_helper::<RBNode<ClaimedSeat>>(dynamic, index as DataIndex).get_value();
                    if seat.base_withdrawable_balance != BaseAtoms::ZERO
                        || seat.quote_withdrawable_balance != QuoteAtoms::ZERO
                    {
                        set_bit(&mut keep, block);
                    }
                }
                _ => {}
            }
        }
        let keep_count: usize = keep.iter().map(|word| word.count_ones() as usize).sum();
        require!(
            keep_count + 2 <= blocks,
            ProgramError::AccountDataTooSmall,
            "Defrag needs two spare nodes"
        )?;

        // Everything not kept is a hole: a free block, or a seat this run
        // reclaims. Work down from the top, because only vacating a high block
        // lets the account shrink, sliding each survivor into the lowest hole
        // beneath it. `hole` only ever advances, so destinations are found in
        // one pass. Stop once the cursors meet: everything below is packed.
        let mut budget: u32 = if limit == 0 { u32::MAX } else { limit };
        // Intentionally unbounded when limit == 0: one pair per relocated seat,
        // plus Vec growth allocations the SBF bump allocator cannot reclaim.
        // MAX_DEFRAG_BLOCKS caps only the bitmap, not this allocation. Operators
        // must supply a smaller relocation budget if the default exceeds heap.
        let mut moved_seats: Vec<(DataIndex, DataIndex)> = Vec::new();
        let mut hole: usize = 0;
        let mut block: usize = blocks;
        while block > 0 && budget > 0 {
            block -= 1;
            if !bit(&keep, block) {
                continue;
            }
            while hole < block && bit(&keep, hole) {
                hole += 1;
            }
            if hole >= block {
                break;
            }
            relocate(
                fixed,
                dynamic,
                block * MARKET_BLOCK_SIZE,
                hole * MARKET_BLOCK_SIZE,
                &mut moved_seats,
            )?;
            clear_bit(&mut keep, block);
            set_bit(&mut keep, hole);
            budget -= 1;
        }

        // Every destination is below every source, so a rewritten index can
        // never collide with one still to be rewritten. The walk went down the
        // account, so `moved_seats` is already sorted descending on the old
        // index and can be searched rather than scanned per order.
        if !moved_seats.is_empty() {
            for block in SetBlocks::new(&keep) {
                let index: usize = block * MARKET_BLOCK_SIZE;
                if dynamic[index + NODE_TYPE] != MarketDataTreeNodeType::RestingOrder as u8 {
                    continue;
                }
                let owner: DataIndex = read_index(dynamic, index + ORDER_TRADER);
                if let Ok(found) = moved_seats.binary_search_by(|probe| owner.cmp(&probe.0)) {
                    write_index(dynamic, index + ORDER_TRADER, moved_seats[found].1);
                }
            }
        }

        // Keep everything up to the last survivor, and never fewer than two
        // spare nodes, which the paths that allocate twice depend on.
        let frontier: usize = keep
            .iter()
            .rposition(|word| *word != 0)
            .map_or(0, |at| at * 64 + (64 - keep[at].leading_zeros() as usize));
        let new_blocks: usize = frontier.max(keep_count + 2);

        // Rebuild the seat tree from the survivors. Reclaimed seats simply
        // never go back into it, which is why reclaiming needs no tree work,
        // and why a run that reclaims thousands costs no more than one that
        // reclaims none. The bitmap decides, not the node type: a reclaimed
        // seat that no survivor happened to displace still reads as a seat.
        fixed.claimed_seats_root_index = NIL;
        for block in SetBlocks::new(&keep) {
            let index: usize = block * MARKET_BLOCK_SIZE;
            if dynamic[index + NODE_TYPE] != MarketDataTreeNodeType::ClaimedSeat as u8 {
                continue;
            }
            let seat: ClaimedSeat =
                *get_helper::<RBNode<ClaimedSeat>>(dynamic, index as DataIndex).get_value();
            let mut tree: RedBlackTree<ClaimedSeat> =
                RedBlackTree::<ClaimedSeat>::new(dynamic, fixed.claimed_seats_root_index, NIL);
            tree.insert(index as DataIndex, seat);
            fixed.claimed_seats_root_index = tree.get_root_index();
            get_mut_helper::<RBNode<ClaimedSeat>>(dynamic, index as DataIndex)
                .set_payload_type(MarketDataTreeNodeType::ClaimedSeat as u8);
        }

        // Chain the holes below the frontier, lowest first, so later
        // allocations prefer low blocks and leave the tail reclaimable. This
        // also blanks the reclaimed seats that were never displaced.
        let mut head: DataIndex = NIL;
        for block in (0..new_blocks).rev() {
            if bit(&keep, block) {
                continue;
            }
            let index: usize = block * MARKET_BLOCK_SIZE;
            dynamic[index..index + MARKET_BLOCK_SIZE].fill(0);
            write_index(dynamic, index, head);
            head = index as DataIndex;
        }
        fixed.free_list_head_index = head;
        fixed.num_bytes_allocated = (new_blocks * MARKET_BLOCK_SIZE) as u32;
        // Invalidate wrappers' quiet-sync shortcut even when no order traded.
        fixed.order_sequence_number = fixed
            .order_sequence_number
            .checked_add(1)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        Ok(MARKET_FIXED_SIZE + fixed.num_bytes_allocated as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn market() -> super::super::super::MarketValue {
        let key = Pubkey::new_unique();
        DynamicAccount {
            fixed: MarketFixed::new_empty_from_mint_parts(&key, 6, &key, 6, key, 0, key, 0),
            dynamic: vec![0; 80 * 200],
        }
    }
    fn free_count(market: &super::super::super::MarketValue) -> u32 {
        let mut n = 0;
        let mut index = market.fixed.free_list_head_index;
        while index != NIL {
            n += 1;
            assert!(n < 1000);
            index = u32::from_le_bytes(
                market.dynamic[index as usize..index as usize + 4]
                    .try_into()
                    .unwrap(),
            );
        }
        n
    }

    #[test]
    fn node_layout_offsets_match_accessors() {
        let mut data = vec![0u8; MARKET_BLOCK_SIZE];
        let order = RestingOrder::new(
            7 * MARKET_BLOCK_SIZE as DataIndex,
            BaseAtoms::ONE,
            1.0.try_into().unwrap(),
            0,
            0,
            true,
            OrderType::Limit,
        )
        .unwrap();
        let mut tree = RedBlackTree::<RestingOrder>::new(&mut data, NIL, NIL);
        tree.insert(0, order);
        assert_eq!(
            read_index(&data, ORDER_TRADER),
            get_helper::<RBNode<RestingOrder>>(&data, 0)
                .get_value()
                .get_trader_index()
        );
        assert_eq!(data[NODE_TYPE], 0);
        get_mut_helper::<RBNode<RestingOrder>>(&mut data, 0)
            .set_payload_type(MarketDataTreeNodeType::RestingOrder as u8);
        assert_eq!(data[NODE_TYPE], MarketDataTreeNodeType::RestingOrder as u8);
    }

    /// Builds one maker with `orders` bids at a single price plus a mix of
    /// fundable and empty seats, deliberately interleaved so compaction has to
    /// move survivors past holes.
    fn populated(orders: u64, empty_seats: usize) -> (super::super::super::MarketValue, Pubkey) {
        // Deterministic keys: seat tree shape follows the trader pubkey, so two
        // fixtures must agree byte for byte to be comparable.
        let key = |n: u8| Pubkey::new_from_array([n; 32]);
        let mut m = market();
        m.market_expand_n(150).unwrap();
        let maker = key(1);
        let funded = key(2);
        m.claim_seat(&maker).unwrap();
        for seat in 0..empty_seats {
            m.claim_seat(&key(16 + seat as u8)).unwrap();
        }
        m.claim_seat(&funded).unwrap();
        m.deposit(m.get_trader_index(&funded), 1, true).unwrap();
        let trader = m.get_trader_index(&maker);
        for sequence in 0..orders {
            let index =
                get_free_address_on_market_fixed_for_bid_order(&mut m.fixed, &mut m.dynamic);
            let order = RestingOrder::new(
                trader,
                BaseAtoms::new(1),
                1.0.try_into().unwrap(),
                sequence,
                0,
                true,
                OrderType::Limit,
            )
            .unwrap();
            let mut tree = RedBlackTree::<RestingOrder>::new(
                &mut m.dynamic,
                m.fixed.bids_root_index,
                m.fixed.bids_best_index,
            );
            tree.insert(index, order);
            m.fixed.bids_root_index = tree.get_root_index();
            m.fixed.bids_best_index = tree.get_max_index();
            get_mut_helper::<RBNode<RestingOrder>>(&mut m.dynamic, index)
                .set_payload_type(MarketDataTreeNodeType::RestingOrder as u8);
        }
        (m, maker)
    }

    fn fifo(m: &super::super::super::MarketValue) -> Vec<u64> {
        m.get_bids()
            .iter::<RestingOrder>()
            .map(|(_, o)| o.get_sequence_number())
            .collect()
    }

    #[test]
    fn defrag_rejects_an_order_missing_from_its_parent() {
        let (mut m, _) = populated(3, 1);
        let index = m
            .get_bids()
            .iter::<RestingOrder>()
            .map(|(index, _)| index)
            .find(|index| read_index(&m.dynamic, *index as usize + NODE_PARENT) != NIL)
            .unwrap();
        let parent = read_index(&m.dynamic, index as usize + NODE_PARENT) as usize;
        // A non-root order must appear in one of its parent's child links.
        if read_index(&m.dynamic, parent + NODE_LEFT) == index {
            write_index(&mut m.dynamic, parent + NODE_LEFT, NIL);
        } else {
            write_index(&mut m.dynamic, parent + NODE_RIGHT, NIL);
        }
        assert_eq!(m.defragment(0), Err(ProgramError::InvalidAccountData));
    }

    #[test]
    fn bounded_runs_converge_on_the_unbounded_layout() {
        let (mut all_at_once, maker) = populated(12, 9);
        let (mut incremental, _) = populated(12, 9);
        let before = fifo(&all_at_once);

        let target = all_at_once.defragment(0).unwrap();
        all_at_once.dynamic.truncate(target - MARKET_FIXED_SIZE);

        // One op at a time, and never more than the budget allows.
        let mut runs = 0;
        loop {
            let size = incremental.defragment(1).unwrap();
            incremental.dynamic.truncate(size - MARKET_FIXED_SIZE);
            runs += 1;
            assert!(runs < 200, "bounded defrag failed to converge");
            if size == target {
                break;
            }
        }
        assert!(runs > 1, "test did not exercise the bounded path");

        // Same bytes, not merely the same size.
        assert_eq!(incremental.fixed.num_bytes_allocated, {
            all_at_once.fixed.num_bytes_allocated
        });
        assert_eq!(incremental.dynamic, all_at_once.dynamic);
        assert_eq!(fifo(&incremental), before);
        assert_eq!(free_count(&incremental), 2);
        assert_ne!(incremental.get_trader_index(&maker), NIL);

        // Converged, so further runs are inert apart from the sequence bump.
        let again = incremental.defragment(0).unwrap();
        assert_eq!(again, target);
    }

    #[test]
    fn a_bounded_run_never_grows_the_account_and_keeps_state_valid() {
        let (mut m, maker) = populated(12, 9);
        let before = fifo(&m);
        let mut size = MARKET_FIXED_SIZE + m.fixed.num_bytes_allocated as usize;
        for _ in 0..40 {
            let next = m.defragment(2).unwrap();
            assert!(next <= size, "a bounded run grew the account");
            size = next;
            m.dynamic.truncate(size - MARKET_FIXED_SIZE);
            // Every intermediate state is a usable market.
            assert_eq!(fifo(&m), before);
            assert_ne!(m.get_trader_index(&maker), NIL);
            assert!(free_count(&m) >= 2);
            for (index, order) in m.get_bids().iter::<RestingOrder>() {
                assert_eq!(
                    order.get_trader_index(),
                    m.get_trader_index(&maker),
                    "order at {index} lost its owner"
                );
            }
        }
    }

    #[test]
    fn a_converged_market_needs_no_further_work() {
        // Nothing left to move: a budget of one still completes in one run.
        let (mut m, _) = populated(4, 0);
        let first = m.defragment(0).unwrap();
        m.dynamic.truncate(first - MARKET_FIXED_SIZE);
        let second = m.defragment(1).unwrap();
        assert_eq!(second, first);
    }

    #[test]
    fn oversized_markets_are_rejected_with_a_clear_error() {
        let mut m = market();
        m.fixed.num_bytes_allocated = ((MAX_DEFRAG_BLOCKS + 1) * MARKET_BLOCK_SIZE) as u32;
        m.dynamic
            .resize((MAX_DEFRAG_BLOCKS + 1) * MARKET_BLOCK_SIZE, 0);
        assert_eq!(
            m.defragment(0).unwrap_err(),
            ProgramError::InvalidAccountData
        );
    }

    #[test]
    fn order_resolution_searches_both_sides_by_price_and_stable_identity() {
        let mut m = market();
        m.market_expand_n(100).unwrap();
        let maker = Pubkey::new_unique();
        m.claim_seat(&maker).unwrap();
        let trader = m.get_trader_index(&maker);
        let mut expected = Vec::new();
        for sequence in 0..32 {
            let is_bid = sequence % 2 == 0;
            let price = ((sequence / 8 + 1) as f64).try_into().unwrap();
            let index =
                get_free_address_on_market_fixed_for_bid_order(&mut m.fixed, &mut m.dynamic);
            let order = RestingOrder::new(
                trader,
                BaseAtoms::ONE,
                price,
                sequence,
                0,
                is_bid,
                OrderType::Reverse,
            )
            .unwrap();
            let root = if is_bid {
                &mut m.fixed.bids_root_index
            } else {
                &mut m.fixed.asks_root_index
            };
            let mut tree = RedBlackTree::<RestingOrder>::new(&mut m.dynamic, *root, NIL);
            tree.insert(index, order);
            *root = tree.get_root_index();
            get_mut_helper::<RBNode<RestingOrder>>(&mut m.dynamic, index)
                .set_payload_type(MarketDataTreeNodeType::RestingOrder as u8);
            expected.push((index, sequence, price, is_bid));
        }
        for (index, sequence, price, is_bid) in expected {
            for hint in [index, NIL, trader, u32::MAX - 15] {
                assert_eq!(
                    m.resolve_order_index(hint, sequence, trader, price, is_bid),
                    index
                );
            }
            assert_eq!(
                m.resolve_order_index(NIL, sequence, trader, price, !is_bid),
                NIL
            );
            assert_eq!(
                m.resolve_order_index(NIL, sequence + 32, trader, price, is_bid),
                NIL
            );
            assert_eq!(
                m.resolve_order_index(index, sequence, NIL, price, is_bid),
                NIL
            );
            assert_eq!(
                m.resolve_order_index(index, sequence, trader + 80, price, is_bid),
                NIL
            );
        }
    }

    #[test]
    fn defrag_harvests_only_empty_seats_and_preserves_order_identity_and_fifo() {
        let mut m = market();
        m.market_expand_n(100).unwrap();
        let empty = Pubkey::new_unique();
        let funded = Pubkey::new_unique();
        let maker = Pubkey::new_unique();
        m.claim_seat(&empty).unwrap();
        m.claim_seat(&funded).unwrap();
        m.claim_seat(&maker).unwrap();
        let old_maker = m.get_trader_index(&maker);
        m.deposit(m.get_trader_index(&funded), 1, true).unwrap();
        let mut order_indices = Vec::new();
        for sequence in 0..12 {
            let index =
                get_free_address_on_market_fixed_for_bid_order(&mut m.fixed, &mut m.dynamic);
            let order = RestingOrder::new(
                old_maker,
                BaseAtoms::new(1),
                1.0.try_into().unwrap(),
                sequence,
                0,
                true,
                if sequence == 0 {
                    OrderType::Global
                } else {
                    OrderType::Limit
                },
            )
            .unwrap();
            let mut tree = RedBlackTree::<RestingOrder>::new(
                &mut m.dynamic,
                m.fixed.bids_root_index,
                m.fixed.bids_best_index,
            );
            tree.insert(index, order);
            m.fixed.bids_root_index = tree.get_root_index();
            m.fixed.bids_best_index = tree.get_max_index();
            get_mut_helper::<RBNode<RestingOrder>>(&mut m.dynamic, index)
                .set_payload_type(MarketDataTreeNodeType::RestingOrder as u8);
            order_indices.push((index, sequence));
        }
        let before: Vec<u64> = m
            .get_bids()
            .iter::<RestingOrder>()
            .map(|(_, o)| o.get_sequence_number())
            .collect();
        let size = m.defragment(0).unwrap();
        m.dynamic.truncate(size - MARKET_FIXED_SIZE);
        assert_eq!(size, MARKET_FIXED_SIZE + 80 * (2 + 12 + 2));
        assert_eq!(m.get_trader_index(&empty), NIL);
        assert_eq!(m.get_trader_balance(&funded).0.as_u64(), 1);
        let new_maker = m.resolve_trader_index(old_maker, &maker);
        assert_eq!(
            m.get_bids()
                .iter::<RestingOrder>()
                .map(|(_, o)| o.get_sequence_number())
                .collect::<Vec<_>>(),
            before
        );
        for (old, seq) in order_indices {
            let new = m.resolve_order_index(old, seq, new_maker, 1.0.try_into().unwrap(), true);
            assert_ne!(new, NIL);
            assert_eq!(
                get_helper_order(&m.dynamic, new)
                    .get_value()
                    .get_trader_index(),
                new_maker
            );
        }
        assert_eq!(free_count(&m), 2);
        let again = m.defragment(0).unwrap();
        assert_eq!(size, again);
    }
}
