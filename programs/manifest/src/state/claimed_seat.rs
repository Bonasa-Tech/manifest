use std::mem::size_of;

#[cfg(feature = "certora")]
use crate::quantities::WrapperU64;
use crate::quantities::{BaseAtoms, QuoteAtoms};
use bytemuck::{Pod, Zeroable};
use shank::ShankType;
use solana_program::pubkey::Pubkey;
use static_assertions::const_assert_eq;
use std::cmp::Ordering;

use super::constants::CLAIMED_SEAT_SIZE;

#[repr(C)]
#[derive(Default, Debug, Copy, Clone, Zeroable, Pod, ShankType)]
pub struct ClaimedSeat {
    pub trader: Pubkey,
    // Balances are withdrawable on the exchange. They do not include funds in
    // open orders. When moving funds over to open orders, use the worst case
    // rounding.
    pub base_withdrawable_balance: BaseAtoms,
    pub quote_withdrawable_balance: QuoteAtoms,
    /// Quote volume traded over lifetime, can overflow. Double counts self
    /// trades. This is for informational and monitoring purposes only. This is
    /// not guaranteed to be maintained. It does not secure any value in
    /// manifest. Use at your own risk.
    pub quote_volume: QuoteAtoms,
    _padding: [u8; 8],
}
// 32 + // trader
//  8 + // base_balance
//  8 + // quote_balance
//  8 + // quote_volume
//  8   // padding
// = 64
const_assert_eq!(size_of::<ClaimedSeat>(), CLAIMED_SEAT_SIZE);
const_assert_eq!(size_of::<ClaimedSeat>() % 8, 0);

impl ClaimedSeat {
    pub fn new_empty(trader: Pubkey) -> Self {
        ClaimedSeat {
            trader,
            ..Default::default()
        }
    }
}

#[cfg(feature = "certora")]
impl nondet::Nondet for ClaimedSeat {
    fn nondet() -> Self {
        ClaimedSeat {
            trader: nondet::nondet(),
            base_withdrawable_balance: BaseAtoms::new(nondet::nondet()),
            quote_withdrawable_balance: QuoteAtoms::new(nondet::nondet()),
            quote_volume: QuoteAtoms::new(nondet::nondet()),
            _padding: [0; 8],
        }
    }
}

impl Ord for ClaimedSeat {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.trader).cmp(&(other.trader))
    }
}

impl PartialOrd for ClaimedSeat {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for ClaimedSeat {
    fn eq(&self, other: &Self) -> bool {
        (self.trader) == (other.trader)
    }
}

impl Eq for ClaimedSeat {}

impl std::fmt::Display for ClaimedSeat {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.trader)
    }
}

#[test]
fn test_display() {
    let claimed_seat: ClaimedSeat = ClaimedSeat::new_empty(Pubkey::default());
    let _ = format!("{}", claimed_seat);
}
