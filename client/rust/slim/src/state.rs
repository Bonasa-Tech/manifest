//! Market state parsing for Manifest.

use crate::constants::{
    OrderType, CLAIMED_SEAT_SIZE, MARKET_FIXED_DISCRIMINANT, MARKET_FIXED_SIZE,
    NO_EXPIRATION_LAST_VALID_SLOT, RESTING_ORDER_SIZE,
};
use hypertree::{DataIndex, NIL, RBTREE_OVERHEAD_BYTES};
use solana_pubkey::Pubkey;
use std::{cmp::Ordering, collections::HashSet};

/// The fixed header of a market account.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct MarketFixed {
    /// Discriminant for identifying this type of account.
    pub discriminant: u64,

    /// Version
    pub version: u8,
    pub base_mint_decimals: u8,
    pub quote_mint_decimals: u8,
    pub base_vault_bump: u8,
    pub quote_vault_bump: u8,
    pub _padding1: [u8; 3],

    /// Base mint
    pub base_mint: [u8; 32],
    /// Quote mint
    pub quote_mint: [u8; 32],

    /// Base vault
    pub base_vault: [u8; 32],
    /// Quote vault
    pub quote_vault: [u8; 32],

    /// The sequence number of the next order.
    pub order_sequence_number: u64,

    /// Num bytes allocated as RestingOrder or ClaimedSeat or FreeList.
    pub num_bytes_allocated: u32,

    /// Red-black tree root representing the bids in the order book.
    pub bids_root_index: DataIndex,
    pub bids_best_index: DataIndex,

    /// Red-black tree root representing the asks in the order book.
    pub asks_root_index: DataIndex,
    pub asks_best_index: DataIndex,

    /// Red-black tree root representing the seats
    pub claimed_seats_root_index: DataIndex,

    /// Cached best claimed seat. This is presently unused by the client, but
    /// retained so the following fields match the on-chain account layout.
    pub claimed_seats_best_index: DataIndex,

    /// LinkedList representing all free blocks
    pub free_list_head_index: DataIndex,

    pub _padding2: [u64; 1],

    /// Quote volume traded over lifetime, can overflow.
    pub quote_volume: u64,

    pub _padding3: [u64; 7],
}

impl MarketFixed {
    /// Parse a MarketFixed from bytes.
    pub fn try_from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < std::mem::size_of::<MarketFixed>() {
            return None;
        }

        // All fields accept every bit pattern; read_unaligned avoids requiring
        // RPC-provided byte buffers to satisfy MarketFixed's alignment.
        let fixed = unsafe { data.as_ptr().cast::<MarketFixed>().read_unaligned() };

        if fixed.discriminant != MARKET_FIXED_DISCRIMINANT {
            return None;
        }

        Some(fixed)
    }

    /// Get the base mint as a Pubkey.
    pub fn get_base_mint(&self) -> Pubkey {
        Pubkey::from(self.base_mint)
    }

    /// Get the quote mint as a Pubkey.
    pub fn get_quote_mint(&self) -> Pubkey {
        Pubkey::from(self.quote_mint)
    }

    /// Get the base vault as a Pubkey.
    pub fn get_base_vault(&self) -> Pubkey {
        Pubkey::from(self.base_vault)
    }

    /// Get the quote vault as a Pubkey.
    pub fn get_quote_vault(&self) -> Pubkey {
        Pubkey::from(self.quote_vault)
    }

    /// Check if there's a free block available.
    pub fn has_free_block(&self) -> bool {
        self.free_list_head_index != NIL
    }
}

/// A resting order on the book.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct RestingOrder {
    /// Price encoded as mantissa * 10^(exponent + 18)
    pub price: [u64; 2],
    /// Number of base atoms in the order
    pub num_base_atoms: u64,
    /// Unique sequence number for the order
    pub sequence_number: u64,
    /// Index of the trader in the claimed seats tree
    pub trader_index: DataIndex,
    /// Last valid slot (0 = no expiration)
    pub last_valid_slot: u32,
    /// Whether this is a bid (1) or ask (0)
    pub is_bid: u8,
    /// Order type
    pub order_type: u8,
    /// Spread for reverse orders
    pub reverse_spread: u16,
    pub _padding: [u8; 20],
}

impl RestingOrder {
    /// Check if this is a bid order.
    pub fn is_bid(&self) -> bool {
        self.is_bid == 1
    }

    /// Get the order type.
    pub fn get_order_type(&self) -> OrderType {
        OrderType::from_u8(self.order_type).unwrap_or_default()
    }

    /// Check if the order is a global order.
    pub fn is_global(&self) -> bool {
        self.get_order_type() == OrderType::Global
    }

    /// Check if the order is expired.
    pub fn is_expired(&self, current_slot: u32) -> bool {
        self.last_valid_slot != NO_EXPIRATION_LAST_VALID_SLOT && self.last_valid_slot < current_slot
    }

    /// Get the price as a u128.
    pub fn get_price_raw(&self) -> u128 {
        u128::from(self.price[0]) | (u128::from(self.price[1]) << 64)
    }

    /// Get the price as a float (approximate).
    pub fn get_price_float(&self) -> f64 {
        let raw = self.get_price_raw();
        (raw as f64) / 1e18
    }
}

/// A claimed seat (trader record) on the market.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ClaimedSeat {
    /// The trader's public key
    pub trader: [u8; 32],
    /// Withdrawable base balance
    pub base_withdrawable_balance: u64,
    /// Withdrawable quote balance
    pub quote_withdrawable_balance: u64,
    /// Quote volume traded by this trader
    pub quote_volume: u64,
    pub _padding: [u8; 8],
}

impl ClaimedSeat {
    /// Get the trader's pubkey.
    pub fn get_trader(&self) -> Pubkey {
        Pubkey::from(self.trader)
    }
}

/// Red-black tree node header (comes before the payload).
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct RBNodeHeader {
    pub left: DataIndex,
    pub right: DataIndex,
    pub parent: DataIndex,
    pub color: u8, // 0 = black, 1 = red
    pub payload_type: u8,
    pub _padding: u16,
}

/// Full market state including dynamic data.
pub struct Market<'a> {
    /// The fixed header.
    pub fixed: MarketFixed,
    /// The dynamic data (orders, seats, free list).
    pub dynamic: &'a [u8],
}

impl<'a> Market<'a> {
    /// Parse a market from raw account data.
    pub fn try_from_bytes(data: &'a [u8]) -> Option<Self> {
        let fixed = MarketFixed::try_from_bytes(data)?;
        let dynamic = &data[MARKET_FIXED_SIZE..];
        let market = Self { fixed, dynamic };
        let bids =
            market.validate_tree(market.fixed.bids_root_index, RESTING_ORDER_SIZE, Some(true))?;
        let asks = market.validate_tree(
            market.fixed.asks_root_index,
            RESTING_ORDER_SIZE,
            Some(false),
        )?;
        let seats = market.validate_tree(
            market.fixed.claimed_seats_root_index,
            CLAIMED_SEAT_SIZE,
            None,
        )?;
        market.validate_claimed_seat_ordering(market.fixed.claimed_seats_root_index)?;
        if !bids.is_disjoint(&asks) || !bids.is_disjoint(&seats) || !asks.is_disjoint(&seats) {
            return None;
        }
        market.validate_best_index(market.fixed.bids_root_index, market.fixed.bids_best_index)?;
        market.validate_best_index(market.fixed.asks_root_index, market.fixed.asks_best_index)?;
        Some(market)
    }

    /// Get the base mint.
    pub fn get_base_mint(&self) -> Pubkey {
        self.fixed.get_base_mint()
    }

    /// Get the quote mint.
    pub fn get_quote_mint(&self) -> Pubkey {
        self.fixed.get_quote_mint()
    }

    /// Get a resting order at the given index.
    pub fn get_order(&self, index: DataIndex) -> Option<RestingOrder> {
        self.read_payload(index, RESTING_ORDER_SIZE)
    }

    /// Get a claimed seat at the given index.
    pub fn get_seat(&self, index: DataIndex) -> Option<ClaimedSeat> {
        self.read_payload(index, CLAIMED_SEAT_SIZE)
    }

    /// Get the best bid price as a float (or None if no bids).
    pub fn get_best_bid(&self) -> Option<f64> {
        let order = self.get_order(self.fixed.bids_best_index)?;
        Some(order.get_price_float())
    }

    /// Get the best ask price as a float (or None if no asks).
    pub fn get_best_ask(&self) -> Option<f64> {
        let order = self.get_order(self.fixed.asks_best_index)?;
        Some(order.get_price_float())
    }

    /// Iterate over all bids (from best to worst).
    pub fn iter_bids(&'a self) -> OrderIterator<'a> {
        OrderIterator::new_bids(self)
    }

    /// Iterate over all asks (from best to worst).
    pub fn iter_asks(&'a self) -> OrderIterator<'a> {
        OrderIterator::new_asks(self)
    }

    /// Find a trader's seat by their pubkey.
    pub fn find_trader_seat(&self, trader: &Pubkey) -> Option<(DataIndex, ClaimedSeat)> {
        // Walk the claimed seats tree to find the trader
        self.walk_tree_for_trader(self.fixed.claimed_seats_root_index, trader)
    }

    fn walk_tree_for_trader(
        &self,
        mut index: DataIndex,
        trader: &Pubkey,
    ) -> Option<(DataIndex, ClaimedSeat)> {
        while index != NIL {
            let seat = self.get_seat(index)?;
            let seat_trader = seat.get_trader();
            if &seat_trader == trader {
                return Some((index, seat));
            }
            let header = self.get_header(index)?;
            index = if trader.to_bytes() < seat_trader.to_bytes() {
                header.left
            } else {
                header.right
            };
        }
        None
    }

    fn get_header(&self, index: DataIndex) -> Option<RBNodeHeader> {
        self.read_payload_at(index as usize, RBTREE_OVERHEAD_BYTES)
    }

    fn read_payload<T: Copy>(&self, index: DataIndex, size: usize) -> Option<T> {
        if index == NIL {
            return None;
        }
        self.read_payload_at((index as usize).checked_add(RBTREE_OVERHEAD_BYTES)?, size)
    }

    fn read_payload_at<T: Copy>(&self, offset: usize, size: usize) -> Option<T> {
        if size != std::mem::size_of::<T>() || offset.checked_add(size)? > self.dynamic.len() {
            return None;
        }
        Some(unsafe {
            self.dynamic
                .as_ptr()
                .add(offset)
                .cast::<T>()
                .read_unaligned()
        })
    }

    fn validate_tree(
        &self,
        root: DataIndex,
        payload_size: usize,
        expected_is_bid: Option<bool>,
    ) -> Option<HashSet<DataIndex>> {
        if root == NIL {
            return Some(HashSet::new());
        }
        if self.get_header(root)?.color != 0 {
            return None;
        }
        let mut stack = vec![(root, NIL, 0_u32, None, None)];
        let mut seen = HashSet::new();
        let mut expected_black_height: Option<u32> = None;
        while let Some((index, expected_parent, black_height, lower, upper)) = stack.pop() {
            if index == NIL {
                let leaf_black_height = black_height + 1;
                if expected_black_height
                    .replace(leaf_black_height)
                    .is_some_and(|height| height != leaf_black_height)
                {
                    return None;
                }
                continue;
            }
            if index as usize % 8 != 0 || !seen.insert(index) {
                return None;
            }
            let end = (index as usize)
                .checked_add(RBTREE_OVERHEAD_BYTES)?
                .checked_add(payload_size)?;
            if end > self.dynamic.len() {
                return None;
            }
            let header = self.get_header(index)?;
            if header.color > 1 || header.parent != expected_parent {
                return None;
            }
            if header.color == 1 {
                if self
                    .get_header(header.left)
                    .is_some_and(|child| child.color == 1)
                    || self
                        .get_header(header.right)
                        .is_some_and(|child| child.color == 1)
                {
                    return None;
                }
            }
            if let Some(is_bid) = expected_is_bid {
                let order = self.get_order(index)?;
                if order.is_bid != u8::from(is_bid)
                    || OrderType::from_u8(order.order_type).is_none()
                    || lower.is_some_and(|minimum| {
                        Self::compare_orders(&order, &minimum) != Ordering::Greater
                    })
                    || upper.is_some_and(|maximum| {
                        Self::compare_orders(&order, &maximum) != Ordering::Less
                    })
                {
                    return None;
                }
                let next_black_height = black_height + u32::from(header.color == 0);
                stack.push((header.left, index, next_black_height, lower, Some(order)));
                stack.push((header.right, index, next_black_height, Some(order), upper));
            } else {
                let next_black_height = black_height + u32::from(header.color == 0);
                stack.push((header.left, index, next_black_height, None, None));
                stack.push((header.right, index, next_black_height, None, None));
            }
        }
        Some(seen)
    }

    fn compare_orders(left: &RestingOrder, right: &RestingOrder) -> Ordering {
        let price_ordering = if left.is_bid() {
            left.get_price_raw().cmp(&right.get_price_raw())
        } else {
            right.get_price_raw().cmp(&left.get_price_raw())
        };
        price_ordering
            // RestingOrder::cmp uses reversed sequence order so the earliest
            // order has priority at a price level. This validator must match
            // the on-chain ordering exactly or it can reject authentic trees.
            .then_with(|| right.sequence_number.cmp(&left.sequence_number))
            .then_with(|| left.trader_index.cmp(&right.trader_index))
            .then_with(|| left.order_type.cmp(&right.order_type))
    }

    fn validate_claimed_seat_ordering(&self, root: DataIndex) -> Option<()> {
        let mut stack = vec![(root, None::<[u8; 32]>, None::<[u8; 32]>)];
        while let Some((index, lower, upper)) = stack.pop() {
            if index == NIL {
                continue;
            }
            let header = self.get_header(index)?;
            let trader = self.get_seat(index)?.trader;
            if lower.is_some_and(|minimum| trader <= minimum)
                || upper.is_some_and(|maximum| trader >= maximum)
            {
                return None;
            }
            stack.push((header.left, lower, Some(trader)));
            stack.push((header.right, Some(trader), upper));
        }
        Some(())
    }

    fn validate_best_index(&self, root: DataIndex, best: DataIndex) -> Option<()> {
        if root == NIL {
            return (best == NIL).then_some(());
        }
        if best == NIL {
            return None;
        }
        let mut index = root;
        loop {
            let header = self.get_header(index)?;
            if header.right == NIL {
                return (index == best).then_some(());
            }
            index = header.right;
        }
    }
}

/// Iterator over orders in the book.
pub struct OrderIterator<'a> {
    market: &'a Market<'a>,
    current_index: DataIndex,
    #[allow(dead_code)]
    is_bids: bool,
}

impl<'a> OrderIterator<'a> {
    fn new_bids(market: &'a Market<'a>) -> Self {
        Self {
            market,
            current_index: market.fixed.bids_best_index,
            is_bids: true,
        }
    }

    fn new_asks(market: &'a Market<'a>) -> Self {
        Self {
            market,
            current_index: market.fixed.asks_best_index,
            is_bids: false,
        }
    }
}

#[cfg(test)]
mod parsing_tests {
    use super::*;

    fn resting_order(sequence_number: u64, trader_index: DataIndex) -> RestingOrder {
        RestingOrder {
            price: [100_u64, 0_u64],
            num_base_atoms: 1_u64,
            sequence_number,
            trader_index,
            last_valid_slot: NO_EXPIRATION_LAST_VALID_SLOT,
            is_bid: 1_u8,
            order_type: OrderType::Limit as u8,
            reverse_spread: 0_u16,
            _padding: [0_u8; 20],
        }
    }

    #[test]
    fn same_price_orders_use_reversed_sequence_priority_before_trader() {
        let earlier: RestingOrder = resting_order(1_u64, 9_u32);
        let later: RestingOrder = resting_order(2_u64, 1_u32);

        assert_eq!(Market::compare_orders(&earlier, &later), Ordering::Greater);
        assert_eq!(Market::compare_orders(&later, &earlier), Ordering::Less);
    }

    fn market_bytes(root: DataIndex) -> Vec<u8> {
        let mut data = vec![0_u8; MARKET_FIXED_SIZE + RBTREE_OVERHEAD_BYTES + RESTING_ORDER_SIZE];
        data[..8].copy_from_slice(&MARKET_FIXED_DISCRIMINANT.to_le_bytes());
        let root_offset = std::mem::offset_of!(MarketFixed, bids_root_index);
        data[root_offset..root_offset + 4].copy_from_slice(&root.to_le_bytes());
        for field in [
            std::mem::offset_of!(MarketFixed, bids_best_index),
            std::mem::offset_of!(MarketFixed, asks_root_index),
            std::mem::offset_of!(MarketFixed, asks_best_index),
            std::mem::offset_of!(MarketFixed, claimed_seats_root_index),
            std::mem::offset_of!(MarketFixed, free_list_head_index),
        ] {
            data[field..field + 4].copy_from_slice(&NIL.to_le_bytes());
        }
        data
    }

    #[test]
    fn rejects_truncated_and_out_of_bounds_market_data() {
        assert_eq!(std::mem::size_of::<MarketFixed>(), MARKET_FIXED_SIZE);
        assert!(Market::try_from_bytes(&[]).is_none());
        assert!(Market::try_from_bytes(&market_bytes(8)).is_none());
    }

    #[test]
    fn rejects_cyclic_tree() {
        let mut data = market_bytes(0);
        let dynamic = &mut data[MARKET_FIXED_SIZE..];
        dynamic[0..4].copy_from_slice(&0_u32.to_le_bytes());
        dynamic[4..8].copy_from_slice(&NIL.to_le_bytes());
        dynamic[8..12].copy_from_slice(&NIL.to_le_bytes());
        assert!(Market::try_from_bytes(&data).is_none());
    }

    #[test]
    fn rejects_invalid_node_color() {
        let mut data = market_bytes(0);
        let dynamic = &mut data[MARKET_FIXED_SIZE..];
        dynamic[0..12].copy_from_slice(&[0xff; 12]);
        dynamic[12] = 2;
        assert!(Market::try_from_bytes(&data).is_none());
    }

    fn set_order(data: &mut [u8], index: DataIndex, price: u64) {
        let offset = MARKET_FIXED_SIZE
            + index as usize
            + RBTREE_OVERHEAD_BYTES
            + std::mem::offset_of!(RestingOrder, price);
        data[offset..offset + 8].copy_from_slice(&price.to_le_bytes());
        let is_bid_offset = MARKET_FIXED_SIZE
            + index as usize
            + RBTREE_OVERHEAD_BYTES
            + std::mem::offset_of!(RestingOrder, is_bid);
        data[is_bid_offset] = 1;
    }

    #[test]
    fn rejects_non_extremal_best_index_and_invalid_ordering() {
        let child = (RBTREE_OVERHEAD_BYTES + RESTING_ORDER_SIZE) as DataIndex;
        let mut data =
            vec![
                0_u8;
                MARKET_FIXED_SIZE + child as usize + RBTREE_OVERHEAD_BYTES + RESTING_ORDER_SIZE
            ];
        data[..8].copy_from_slice(&MARKET_FIXED_DISCRIMINANT.to_le_bytes());
        for (field, value) in [
            (std::mem::offset_of!(MarketFixed, bids_root_index), 0),
            (std::mem::offset_of!(MarketFixed, bids_best_index), 0),
            (std::mem::offset_of!(MarketFixed, asks_root_index), NIL),
            (std::mem::offset_of!(MarketFixed, asks_best_index), NIL),
            (
                std::mem::offset_of!(MarketFixed, claimed_seats_root_index),
                NIL,
            ),
            (std::mem::offset_of!(MarketFixed, free_list_head_index), NIL),
        ] {
            data[field..field + 4].copy_from_slice(&value.to_le_bytes());
        }
        let dynamic = &mut data[MARKET_FIXED_SIZE..];
        dynamic[4..8].copy_from_slice(&child.to_le_bytes());
        dynamic[child as usize + 8..child as usize + 12].copy_from_slice(&0_u32.to_le_bytes());
        dynamic[child as usize + 12] = 1;
        set_order(&mut data, 0, 1);
        set_order(&mut data, child, 2);

        // The right child is the best bid, not the reachable root.
        assert!(Market::try_from_bytes(&data).is_none());

        data[std::mem::offset_of!(MarketFixed, bids_best_index)
            ..std::mem::offset_of!(MarketFixed, bids_best_index) + 4]
            .copy_from_slice(&child.to_le_bytes());
        set_order(&mut data, child, 0);
        assert!(Market::try_from_bytes(&data).is_none());
    }

    #[test]
    fn rejects_unordered_claimed_seats() {
        let child = (RBTREE_OVERHEAD_BYTES + CLAIMED_SEAT_SIZE) as DataIndex;
        let mut data =
            vec![
                0_u8;
                MARKET_FIXED_SIZE + child as usize + RBTREE_OVERHEAD_BYTES + CLAIMED_SEAT_SIZE
            ];
        data[..8].copy_from_slice(&MARKET_FIXED_DISCRIMINANT.to_le_bytes());
        for (field, value) in [
            (std::mem::offset_of!(MarketFixed, bids_root_index), NIL),
            (std::mem::offset_of!(MarketFixed, bids_best_index), NIL),
            (std::mem::offset_of!(MarketFixed, asks_root_index), NIL),
            (std::mem::offset_of!(MarketFixed, asks_best_index), NIL),
            (
                std::mem::offset_of!(MarketFixed, claimed_seats_root_index),
                0,
            ),
            (std::mem::offset_of!(MarketFixed, free_list_head_index), NIL),
        ] {
            data[field..field + 4].copy_from_slice(&value.to_le_bytes());
        }
        let dynamic = &mut data[MARKET_FIXED_SIZE..];
        dynamic[0..4].copy_from_slice(&NIL.to_le_bytes());
        dynamic[4..8].copy_from_slice(&child.to_le_bytes());
        dynamic[8..12].copy_from_slice(&NIL.to_le_bytes());
        dynamic[child as usize..child as usize + 4].copy_from_slice(&NIL.to_le_bytes());
        dynamic[child as usize + 4..child as usize + 8].copy_from_slice(&NIL.to_le_bytes());
        dynamic[child as usize + 8..child as usize + 12].copy_from_slice(&0_u32.to_le_bytes());
        dynamic[child as usize + 12] = 1;

        let root_trader_offset = RBTREE_OVERHEAD_BYTES;
        dynamic[root_trader_offset..root_trader_offset + 32].fill(2);
        let child_trader_offset = child as usize + RBTREE_OVERHEAD_BYTES;
        // A right child must be greater than its parent, but this one is lower.
        dynamic[child_trader_offset..child_trader_offset + 32].fill(1);
        assert!(Market::try_from_bytes(&data).is_none());
    }
}

impl<'a> Iterator for OrderIterator<'a> {
    type Item = (DataIndex, RestingOrder);

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_index == NIL {
            return None;
        }

        let index = self.current_index;
        let order = self.market.get_order(index)?;

        // Get the next index by traversing the tree
        let offset = index as usize;
        if offset + RBTREE_OVERHEAD_BYTES > self.market.dynamic.len() {
            self.current_index = NIL;
            return Some((index, order));
        }

        let header = self.market.get_header(index)?;

        // Get next lower index in the tree
        self.current_index = self.get_next_lower_index(index, &header);

        Some((index, order))
    }
}

impl<'a> OrderIterator<'a> {
    fn get_next_lower_index(&self, current: DataIndex, header: &RBNodeHeader) -> DataIndex {
        // If there's a left child, go left then all the way right
        if header.left != NIL {
            let mut index = header.left;
            loop {
                let offset = index as usize;
                if offset + RBTREE_OVERHEAD_BYTES > self.market.dynamic.len() {
                    break;
                }
                let Some(h) = self.market.get_header(index) else {
                    return NIL;
                };
                if h.right == NIL {
                    return index;
                }
                index = h.right;
            }
            return index;
        }

        // Otherwise, go up until we come from a right child
        let mut child = current;
        let mut parent_idx = header.parent;

        while parent_idx != NIL {
            let offset = parent_idx as usize;
            if offset + RBTREE_OVERHEAD_BYTES > self.market.dynamic.len() {
                return NIL;
            }
            let Some(parent_header) = self.market.get_header(parent_idx) else {
                return NIL;
            };

            if parent_header.right == child {
                return parent_idx;
            }

            child = parent_idx;
            parent_idx = parent_header.parent;
        }

        NIL
    }
}
