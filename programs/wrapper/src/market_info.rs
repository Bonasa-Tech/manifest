use bytemuck::{Pod, Zeroable};
use hypertree::{DataIndex, NIL};
use manifest::quantities::{BaseAtoms, QuoteAtoms};
use shank::ShankType;
use solana_program::pubkey::Pubkey;
use static_assertions::const_assert_eq;
use std::{cmp::Ordering, mem::size_of};

use crate::processors::shared::WRAPPER_BLOCK_PAYLOAD_SIZE;

#[repr(C)]
#[derive(Default, Debug, Copy, Clone, Zeroable, Pod, ShankType)]
pub struct MarketInfo {
    /// Pubkey for the market
    pub market: Pubkey,

    /// Root in the wrapper account.
    pub orders_root_index: DataIndex,

    /// Trader index in the market.
    pub trader_index: DataIndex,

    /// Withdrawable base balance on the market.
    pub base_balance: BaseAtoms,
    /// Withdrawable quote balance on the market.
    pub quote_balance: QuoteAtoms,

    /// Quote volume traded over lifetime, can overflow.
    pub quote_volume: QuoteAtoms,

    /// Reserved for byte-layout compatibility. A short-lived client release
    /// exposed this as a cancel-all scan cursor, but cancel-all no longer scans
    /// untracked core orders. Do not interpret or update this value.
    pub cancel_all_scan_cursor: u32,
    /// Open orders of type Global on this market that the wrapper tracks.
    /// Those can be removed by global clean and evict without any order being
    /// placed, so while it is non zero the opening sync always walks the
    /// orders (see `sync_fast`). Recounted on every full walk. Was padding.
    pub num_open_global_orders: u32,
    /// The market's order sequence number the last time the wrapper's view of
    /// its open orders was exact, zero if never. Was padding.
    pub last_synced_order_sequence_number: u64,
}

// Blocks on wrapper are bigger than blocks on the market because there is more
// data to store here.

// 32 + // market
// 4 +  // orders_root
// 4 +  // trader_index
// 8 +  // base_balance
// 8 +  // quote_balance
// 8 +  // quote_volume
// 4 +  // cancel_all_scan_cursor
// 4 +  // num_open_global_orders
// 8    // last_synced_order_sequence_number
// = 80
const_assert_eq!(size_of::<MarketInfo>(), WRAPPER_BLOCK_PAYLOAD_SIZE);
const_assert_eq!(size_of::<MarketInfo>() % 16, 0);

impl MarketInfo {
    // Create a new empty market info.
    pub fn new_empty(market: Pubkey, trader_index: DataIndex) -> Self {
        MarketInfo {
            market,
            orders_root_index: NIL,
            trader_index,
            ..Default::default()
        }
    }
}

impl Ord for MarketInfo {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.market).cmp(&(other.market))
    }
}

impl PartialOrd for MarketInfo {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for MarketInfo {
    fn eq(&self, other: &Self) -> bool {
        (self.market) == (other.market)
    }
}

impl Eq for MarketInfo {}

impl std::fmt::Display for MarketInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.market)
    }
}

#[test]
fn test_display() {
    let market_info: MarketInfo = MarketInfo::new_empty(Pubkey::default(), 0);
    let _ = format!("{}", market_info);
}

#[test]
fn test_cmp() {
    let market_info: MarketInfo = MarketInfo::new_empty(Pubkey::new_unique(), 0);
    let market_info2: MarketInfo = MarketInfo::new_empty(Pubkey::new_unique(), 1);
    assert!(market_info > market_info2 || market_info < market_info2);
}
