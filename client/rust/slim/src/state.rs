//! Market state parsing for Manifest.

use crate::constants::CLAIMED_SEAT_SIZE;
use crate::constants::MARKET_FIXED_DISCRIMINANT;
use crate::constants::MARKET_FIXED_SIZE;
use crate::constants::NO_EXPIRATION_LAST_VALID_SLOT;
use crate::constants::OrderType;
use crate::constants::RESTING_ORDER_SIZE;
use hypertree::DataIndex;
use hypertree::NIL;
use hypertree::RBTREE_OVERHEAD_BYTES;
use solana_pubkey::Pubkey;

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

    /// LinkedList representing all free blocks
    pub free_list_head_index: DataIndex,

    pub _padding2: [u32; 1],

    /// Quote volume traded over lifetime, can overflow.
    pub quote_volume: u64,

    pub _padding3: [u64; 8],
}

impl MarketFixed {
    /// Parse a MarketFixed from bytes.
    pub fn try_from_bytes(data: &[u8]) -> Option<&Self> {
        if data.len() < MARKET_FIXED_SIZE {
            return None;
        }

        // Safety: We've verified the length is sufficient
        let fixed = unsafe { &*(data.as_ptr() as *const MarketFixed) };

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
        self.last_valid_slot != NO_EXPIRATION_LAST_VALID_SLOT
            && self.last_valid_slot < current_slot
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
    pub color: u32, // 0 = black, 1 = red
}

/// Full market state including dynamic data.
pub struct Market<'a> {
    /// The fixed header.
    pub fixed: &'a MarketFixed,
    /// The dynamic data (orders, seats, free list).
    pub dynamic: &'a [u8],
}

impl<'a> Market<'a> {
    /// Parse a market from raw account data.
    pub fn try_from_bytes(data: &'a [u8]) -> Option<Self> {
        let fixed = MarketFixed::try_from_bytes(data)?;
        let dynamic = &data[MARKET_FIXED_SIZE..];
        Some(Self { fixed, dynamic })
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
    pub fn get_order(&self, index: DataIndex) -> Option<&RestingOrder> {
        if index == NIL {
            return None;
        }
        let offset = index as usize;
        if offset + RBTREE_OVERHEAD_BYTES + RESTING_ORDER_SIZE > self.dynamic.len() {
            return None;
        }
        // Skip the RBNode header
        let order_ptr = self.dynamic.as_ptr().wrapping_add(offset + RBTREE_OVERHEAD_BYTES);
        Some(unsafe { &*(order_ptr as *const RestingOrder) })
    }

    /// Get a claimed seat at the given index.
    pub fn get_seat(&self, index: DataIndex) -> Option<&ClaimedSeat> {
        if index == NIL {
            return None;
        }
        let offset = index as usize;
        if offset + RBTREE_OVERHEAD_BYTES + CLAIMED_SEAT_SIZE > self.dynamic.len() {
            return None;
        }
        // Skip the RBNode header
        let seat_ptr = self.dynamic.as_ptr().wrapping_add(offset + RBTREE_OVERHEAD_BYTES);
        Some(unsafe { &*(seat_ptr as *const ClaimedSeat) })
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
    pub fn find_trader_seat(&self, trader: &Pubkey) -> Option<(DataIndex, &ClaimedSeat)> {
        // Walk the claimed seats tree to find the trader
        self.walk_tree_for_trader(self.fixed.claimed_seats_root_index, trader)
    }

    fn walk_tree_for_trader(
        &self,
        index: DataIndex,
        trader: &Pubkey,
    ) -> Option<(DataIndex, &ClaimedSeat)> {
        if index == NIL {
            return None;
        }

        let seat = self.get_seat(index)?;
        let seat_trader = seat.get_trader();

        if &seat_trader == trader {
            return Some((index, seat));
        }

        // Get the node header to traverse the tree
        let offset = index as usize;
        if offset + RBTREE_OVERHEAD_BYTES > self.dynamic.len() {
            return None;
        }
        let header_ptr = self.dynamic.as_ptr().wrapping_add(offset);
        let header = unsafe { &*(header_ptr as *const RBNodeHeader) };

        // Binary search based on trader pubkey comparison
        if trader.to_bytes() < seat_trader.to_bytes() {
            self.walk_tree_for_trader(header.left, trader)
        } else {
            self.walk_tree_for_trader(header.right, trader)
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

impl<'a> Iterator for OrderIterator<'a> {
    type Item = (DataIndex, RestingOrder);

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_index == NIL {
            return None;
        }

        let index = self.current_index;
        let order = *self.market.get_order(index)?;

        // Get the next index by traversing the tree
        let offset = index as usize;
        if offset + RBTREE_OVERHEAD_BYTES > self.market.dynamic.len() {
            self.current_index = NIL;
            return Some((index, order));
        }

        let header_ptr = self.market.dynamic.as_ptr().wrapping_add(offset);
        let header = unsafe { &*(header_ptr as *const RBNodeHeader) };

        // Get next lower index in the tree
        self.current_index = self.get_next_lower_index(index, header);

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
                let h = unsafe {
                    &*(self.market.dynamic.as_ptr().wrapping_add(offset) as *const RBNodeHeader)
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
            let parent_header = unsafe {
                &*(self.market.dynamic.as_ptr().wrapping_add(offset) as *const RBNodeHeader)
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
