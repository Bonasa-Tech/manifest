//! Atomic compaction. Relink nodes without changing tree shape: removing and
//! reinserting equal-price orders through the normal insertion path would change
//! their FIFO priority. Order identities, balances and global gas deposits stay put.
use super::*;

fn bit(words: &[u64], index: DataIndex) -> bool {
    let n = index as usize / MARKET_BLOCK_SIZE;
    words[n / 64] & (1u64 << (n % 64)) != 0
}
fn mark(words: &mut [u64], index: DataIndex) {
    let n = index as usize / MARKET_BLOCK_SIZE;
    words[n / 64] |= 1u64 << (n % 64);
}
fn relocated(words: &[u64], prefix: &[u32], index: DataIndex) -> DataIndex {
    if index == NIL {
        return NIL;
    }
    let n = index as usize / MARKET_BLOCK_SIZE;
    let below = words[n / 64] & ((1u64 << (n % 64)) - 1);
    (prefix[n / 64] + below.count_ones()) * MARKET_BLOCK_SIZE as u32
}

impl<
        Fixed: DerefOrBorrowMut<MarketFixed> + DerefOrBorrow<MarketFixed>,
        Dynamic: DerefOrBorrowMut<[u8]> + DerefOrBorrow<[u8]>,
    > DynamicAccount<Fixed, Dynamic>
{
    /// Caller ensures at least two free blocks exist and truncates to the
    /// returned size after dropping all account-data borrows.
    pub fn defragment(&mut self) -> Result<usize, ProgramError> {
        let DynamicAccount { fixed, dynamic } = self.borrow_mut();
        let allocated = fixed.num_bytes_allocated as usize;
        require!(
            allocated <= dynamic.len() && allocated % MARKET_BLOCK_SIZE == 0,
            ProgramError::InvalidAccountData,
            "Invalid allocation"
        )?;
        // At most 16 KiB of bits plus 8 KiB of prefix counts for a 10 MiB account.
        // Reuse the bitmap: first referenced seats, then all retained nodes.
        let mut live = vec![0u64; (allocated / MARKET_BLOCK_SIZE).div_ceil(64)];
        // FreeList::add zeroes the payload (including the node type). Scan
        // allocated blocks once; this avoids repeated tree traversal and marks
        // every order together with its owner, including zero-balance owners.
        for index in (0..allocated).step_by(MARKET_BLOCK_SIZE) {
            match dynamic[index + 13] {
                kind if kind == MarketDataTreeNodeType::RestingOrder as u8 => {
                    let order =
                        get_helper::<RBNode<RestingOrder>>(dynamic, index as u32).get_value();
                    mark(&mut live, index as u32);
                    mark(&mut live, order.get_trader_index());
                }
                kind if kind == MarketDataTreeNodeType::ClaimedSeat as u8 => {
                    let seat = get_helper::<RBNode<ClaimedSeat>>(dynamic, index as u32).get_value();
                    if seat.base_withdrawable_balance != BaseAtoms::ZERO
                        || seat.quote_withdrawable_balance != QuoteAtoms::ZERO
                    {
                        mark(&mut live, index as u32);
                    }
                }
                _ => {}
            }
        }
        let mut prefix = Vec::with_capacity(live.len());
        let mut count = 0;
        for word in &live {
            prefix.push(count);
            count += word.count_ones();
        }
        let used = count as usize * MARKET_BLOCK_SIZE;
        require!(
            used + 2 * MARKET_BLOCK_SIZE <= dynamic.len(),
            ProgramError::AccountDataTooSmall,
            "Defrag needs two spare nodes"
        )?;

        // Destination never exceeds source. Map every reference using the
        // original bitmap before overwriting each source block.
        for old in (0..allocated).step_by(MARKET_BLOCK_SIZE) {
            if !bit(&live, old as u32) {
                continue;
            }
            let new = relocated(&live, &prefix, old as u32) as usize;
            let mut node = [0u8; MARKET_BLOCK_SIZE];
            node.copy_from_slice(&dynamic[old..old + MARKET_BLOCK_SIZE]);
            if node[13] == MarketDataTreeNodeType::RestingOrder as u8 {
                for offset in [0, 4, 8] {
                    let index = u32::from_le_bytes(node[offset..offset + 4].try_into().unwrap());
                    node[offset..offset + 4]
                        .copy_from_slice(&relocated(&live, &prefix, index).to_le_bytes());
                }
                let trader = u32::from_le_bytes(node[48..52].try_into().unwrap());
                node[48..52].copy_from_slice(&relocated(&live, &prefix, trader).to_le_bytes());
            }
            dynamic[new..new + MARKET_BLOCK_SIZE].copy_from_slice(&node);
        }
        fixed.bids_root_index = relocated(&live, &prefix, fixed.bids_root_index);
        fixed.bids_best_index = relocated(&live, &prefix, fixed.bids_best_index);
        fixed.asks_root_index = relocated(&live, &prefix, fixed.asks_root_index);
        fixed.asks_best_index = relocated(&live, &prefix, fixed.asks_best_index);
        fixed.claimed_seats_root_index = NIL;
        for index in (0..used).step_by(MARKET_BLOCK_SIZE) {
            if dynamic[index + 13] == MarketDataTreeNodeType::ClaimedSeat as u8 {
                let seat = *get_helper::<RBNode<ClaimedSeat>>(dynamic, index as u32).get_value();
                let mut tree =
                    RedBlackTree::<ClaimedSeat>::new(dynamic, fixed.claimed_seats_root_index, NIL);
                tree.insert(index as u32, seat);
                fixed.claimed_seats_root_index = tree.get_root_index();
                get_mut_helper::<RBNode<ClaimedSeat>>(dynamic, index as u32)
                    .set_payload_type(MarketDataTreeNodeType::ClaimedSeat as u8);
            }
        }
        dynamic[used..used + 2 * MARKET_BLOCK_SIZE].fill(0);
        dynamic[used..used + 4].copy_from_slice(&((used + MARKET_BLOCK_SIZE) as u32).to_le_bytes());
        dynamic[used + MARKET_BLOCK_SIZE..used + MARKET_BLOCK_SIZE + 4]
            .copy_from_slice(&NIL.to_le_bytes());
        fixed.free_list_head_index = used as u32;
        fixed.num_bytes_allocated = (used + 2 * MARKET_BLOCK_SIZE) as u32;
        fixed.free_blocks_plus_one = 3;
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
        let size = m.defragment().unwrap();
        m.dynamic.truncate(size - MARKET_FIXED_SIZE);
        assert_eq!(size, MARKET_FIXED_SIZE + 80 * (2 + 12 + 2));
        assert_eq!(m.get_trader_index(&empty), NIL);
        assert_eq!(m.get_trader_balance(&funded).0.as_u64(), 1);
        let new_maker = m.resolve_trader_index(old_maker, &maker);
        assert_ne!(old_maker, new_maker);
        assert_eq!(
            m.get_bids()
                .iter::<RestingOrder>()
                .map(|(_, o)| o.get_sequence_number())
                .collect::<Vec<_>>(),
            before
        );
        for (old, seq) in order_indices {
            let new = m.resolve_order_index(old, seq, new_maker);
            assert_ne!(new, NIL);
            assert_eq!(
                get_helper_order(&m.dynamic, new)
                    .get_value()
                    .get_trader_index(),
                new_maker
            );
        }
        assert_eq!(free_count(&m), 2);
        assert_eq!(m.fixed.cached_free_blocks(), Some(2));
        let again = m.defragment().unwrap();
        assert_eq!(size, again);
    }
    #[test]
    fn free_count_migrates_legacy_and_tracks_allocation_release_and_growth() {
        let mut m = market();
        m.market_expand_n(12).unwrap();
        m.fixed.free_blocks_plus_one = 0;
        m.initialize_free_block_count().unwrap();
        assert_eq!(m.fixed.cached_free_blocks(), Some(12));
        let trader = Pubkey::new_unique();
        m.claim_seat(&trader).unwrap();
        assert_eq!(m.fixed.cached_free_blocks(), Some(11));
        m.release_seat(&trader).unwrap();
        m.market_expand().unwrap();
        assert_eq!(m.fixed.cached_free_blocks(), Some(free_count(&m)));
        assert_eq!(m.fixed.cached_free_blocks(), Some(13));
    }
}
