use hypertree::RBTREE_OVERHEAD_BYTES;
use static_assertions::const_assert_eq;

pub const MARKET_FIXED_SIZE: usize = 256;
pub const GLOBAL_FIXED_SIZE: usize = 96;

// Red black tree overhead is 16 bytes. If each block is 80 bytes, then we get
// 64 bytes for a RestingOrder or ClaimedSeat.
pub const GLOBAL_BLOCK_SIZE: usize = 64;
pub const MARKET_BLOCK_SIZE: usize = 80;
const MARKET_BLOCK_PAYLOAD_SIZE: usize = MARKET_BLOCK_SIZE - RBTREE_OVERHEAD_BYTES;
pub const RESTING_ORDER_SIZE: usize = MARKET_BLOCK_PAYLOAD_SIZE;
pub const CLAIMED_SEAT_SIZE: usize = MARKET_BLOCK_PAYLOAD_SIZE;
const GLOBAL_BLOCK_PAYLOAD_SIZE: usize = GLOBAL_BLOCK_SIZE - RBTREE_OVERHEAD_BYTES;
pub const GLOBAL_TRADER_SIZE: usize = GLOBAL_BLOCK_PAYLOAD_SIZE;
pub const GLOBAL_DEPOSIT_SIZE: usize = GLOBAL_BLOCK_PAYLOAD_SIZE;
const FREE_LIST_OVERHEAD: usize = 4;
pub const MARKET_FREE_LIST_BLOCK_SIZE: usize = MARKET_BLOCK_SIZE - FREE_LIST_OVERHEAD;
pub const GLOBAL_FREE_LIST_BLOCK_SIZE: usize = GLOBAL_BLOCK_SIZE - FREE_LIST_OVERHEAD;

pub const NO_EXPIRATION_LAST_VALID_SLOT: u32 = 0;

pub const MARKET_FIXED_DISCRIMINANT: u64 = 4859840929024028656;
pub const GLOBAL_FIXED_DISCRIMINANT: u64 = 10787423733276977665;

// Amount of gas deposited for every global order. This is done to as an
// economic disincentive to spam.
//
// - Every time you place a global order, you deposit 5000 lamports into the
// global account. This is an overestimate for the gas burden on whoever will
// remove it from orderbook.
// - When you remove an order because you fill it, you cancel it yourself, you try
// to match and the funds for it dont exist, or you remove it because it is
// expired, you get the 5000 lamports.
//
// Note that if your seat gets evicted, then all your orders are unbacked and
// now are free to have their deposits claimed. So there is an incentive to keep
// capital on the exchange to prevent that.
pub const GAS_DEPOSIT_LAMPORTS: u64 = 5_000;

/// Limit on the number of global seats available. Set so that this is hit
/// before the global account starts running into account size limits, but is
/// generous enough that it really should only matter in deterring spam.  Sized
/// to fit in 4 pages. This is sufficiently big such that it is not possible to
/// fully evict all seats in one flash loan transaction due to the withdraw
/// accounts limit.
#[cfg(feature = "test")]
pub const MAX_GLOBAL_SEATS: u16 = 4;
#[cfg(not(feature = "test"))]
pub const MAX_GLOBAL_SEATS: u16 = 999;

// Given Solana's eight-byte-aligned account-data base, these block sizes
// preserve alignment at every node boundary. Safe byte-slice helpers still
// validate their effective address because arbitrary slices need not share
// that base alignment.
const_assert_eq!(MARKET_BLOCK_SIZE % 8, 0);
const_assert_eq!(GLOBAL_BLOCK_SIZE % 8, 0);
