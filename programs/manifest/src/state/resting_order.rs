use std::mem::size_of;

use crate::quantities::{
    u64_slice_to_u128, BaseAtoms, PriceConversionError, QuoteAtomsPerBaseAtom,
};
#[cfg(feature = "certora")]
use crate::quantities::{QuoteAtoms, WrapperU64};
use borsh::{BorshDeserialize, BorshSerialize};
use bytemuck::{Pod, Zeroable};
use hypertree::{DataIndex, PodBool};
use num_enum::{IntoPrimitive, TryFromPrimitive};
use shank::ShankType;
use solana_program::{entrypoint::ProgramResult, program_error::ProgramError};
use static_assertions::const_assert_eq;
use std::cmp::Ordering;

use super::{constants::NO_EXPIRATION_LAST_VALID_SLOT, RESTING_ORDER_SIZE};

#[derive(
    Debug,
    BorshDeserialize,
    BorshSerialize,
    PartialEq,
    Clone,
    Copy,
    ShankType,
    IntoPrimitive,
    TryFromPrimitive,
)]
#[repr(u8)]
pub enum OrderType {
    // Normal limit order.
    Limit = 0,

    // Does not rest. Take only.
    ImmediateOrCancel = 1,

    // Fails if would cross the orderbook.
    PostOnly = 2,

    // Global orders are post only but use funds from the global account.
    Global = 3,

    // Reverse orders behave like an AMM. When filled, they place an order on
    // the other side of the book with a small fee (spread).
    // Note: reverse orders can take but don't reverse when taking.
    Reverse = 4,

    // Same as a reverse order except that it much tighter, allowing for stables
    // to have even smaller spreads.
    ReverseTight = 5,
}
unsafe impl bytemuck::Zeroable for OrderType {}
unsafe impl bytemuck::Pod for OrderType {}
impl Default for OrderType {
    fn default() -> Self {
        OrderType::Limit
    }
}
impl OrderType {
    pub fn is_reversible(self) -> bool {
        self == OrderType::Reverse || self == OrderType::ReverseTight
    }

    pub fn max_exponent(self) -> i8 {
        match self {
            OrderType::Reverse => QuoteAtomsPerBaseAtom::MAX_EXP - 5,
            OrderType::ReverseTight => QuoteAtomsPerBaseAtom::MAX_EXP - 8,
            _ => QuoteAtomsPerBaseAtom::MAX_EXP,
        }
    }
}

pub fn order_type_can_rest(order_type: OrderType) -> bool {
    order_type != OrderType::ImmediateOrCancel
}

pub fn order_type_can_take(order_type: OrderType) -> bool {
    order_type != OrderType::PostOnly && order_type != OrderType::Global
}

#[repr(C)]
#[derive(Default, Debug, Copy, Clone, Zeroable, Pod, ShankType)]
pub struct RestingOrder {
    price: QuoteAtomsPerBaseAtom,
    num_base_atoms: BaseAtoms,
    sequence_number: u64,
    trader_index: DataIndex,
    last_valid_slot: u32,
    is_bid: PodBool,
    order_type: OrderType,
    // Spread for reverse orders. Defaults to zero.
    reverse_spread: u16,
    _padding: [u8; 20],
}

// 16 +  // price
//  8 +  // num_base_atoms
//  8 +  // sequence_number
//  4 +  // trader_index
//  4 +  // last_valid_slot
//  1 +  // is_bid
//  1 +  // order_type
//  2 +  // spread
// 20    // padding 2
// = 64
const_assert_eq!(size_of::<RestingOrder>(), RESTING_ORDER_SIZE);
const_assert_eq!(size_of::<RestingOrder>() % 8, 0);

impl RestingOrder {
    pub fn new(
        trader_index: DataIndex,
        num_base_atoms: BaseAtoms,
        price: QuoteAtomsPerBaseAtom,
        sequence_number: u64,
        last_valid_slot: u32,
        is_bid: bool,
        order_type: OrderType,
    ) -> Result<Self, ProgramError> {
        // Reverse orders cannot have expiration. The purpose of those orders is to
        // be a permanent liquidity on the book.
        assert!(!(order_type.is_reversible() && last_valid_slot != NO_EXPIRATION_LAST_VALID_SLOT));

        Ok(RestingOrder {
            trader_index,
            num_base_atoms,
            last_valid_slot,
            price,
            sequence_number,
            is_bid: PodBool::from_bool(is_bid),
            order_type,
            reverse_spread: 0,
            _padding: Default::default(),
        })
    }

    pub fn get_trader_index(&self) -> DataIndex {
        self.trader_index
    }

    pub fn get_num_base_atoms(&self) -> BaseAtoms {
        self.num_base_atoms
    }

    pub fn get_price(&self) -> QuoteAtomsPerBaseAtom {
        self.price
    }

    #[cfg(any(test, feature = "no-clock"))]
    pub fn set_sequence_number(&mut self, sequence_number: u64) {
        self.sequence_number = sequence_number;
    }
    #[cfg(any(test, feature = "no-clock"))]
    pub fn set_last_valid_slot(&mut self, last_valid_slot: u32) {
        self.last_valid_slot = last_valid_slot;
    }

    pub fn get_order_type(&self) -> OrderType {
        self.order_type
    }

    #[cfg(feature = "certora")]
    pub fn is_global(&self) -> bool {
        false
    }

    #[cfg(not(feature = "certora"))]
    pub fn is_global(&self) -> bool {
        self.order_type == OrderType::Global
    }

    pub fn is_reversible(&self) -> bool {
        self.order_type.is_reversible()
    }

    pub fn reverse_price(&self) -> Result<QuoteAtomsPerBaseAtom, PriceConversionError> {
        let base = match self.order_type {
            OrderType::Reverse => 100_000_u32,
            OrderType::ReverseTight => 100_000_000_u32,
            _ => return Ok(self.price),
        };

        if self.get_is_bid() {
            // Bid @P * (1 - spread) --> Ask @P
            // equivalent to
            // Bid @P --> Ask @P / (1 - spread)
            self.price
                .checked_multiply_rational(base, base - self.reverse_spread as u32, false)
        } else {
            // Ask @P --> Bid @P * (1 - spread)
            self.price
                .checked_multiply_rational(base - self.reverse_spread as u32, base, true)
        }
    }

    pub fn get_reverse_spread(self) -> u16 {
        self.reverse_spread
    }

    pub fn set_reverse_spread(&mut self, spread: u16) {
        self.reverse_spread = spread;
    }

    pub fn get_sequence_number(&self) -> u64 {
        self.sequence_number
    }

    pub fn is_expired(&self, current_slot: u32) -> bool {
        self.last_valid_slot != NO_EXPIRATION_LAST_VALID_SLOT && self.last_valid_slot < current_slot
    }

    pub fn get_is_bid(&self) -> bool {
        self.is_bid.0 == 1
    }

    // compute the "value" of an order, i.e. the tokens that are reserved for the trade and
    // that will be returned when it is cancelled.
    #[cfg(feature = "certora")]
    pub fn get_orderbook_atoms(&self) -> Result<(BaseAtoms, QuoteAtoms), ProgramError> {
        if self.is_global() {
            return Ok((BaseAtoms::new(0), QuoteAtoms::new(0)));
        } else if self.get_is_bid() {
            let quote_amount = self.num_base_atoms.checked_mul(self.price, true)?;
            return Ok((BaseAtoms::new(0), quote_amount));
        } else {
            return Ok((self.num_base_atoms, QuoteAtoms::new(0)));
        }
    }

    pub fn reduce(&mut self, size: BaseAtoms) -> ProgramResult {
        self.num_base_atoms = self.num_base_atoms.checked_sub(size)?;
        Ok(())
    }

    // Only needed for combining orders. There is no edit_order function.
    pub fn increase(&mut self, size: BaseAtoms) -> ProgramResult {
        self.num_base_atoms = self.num_base_atoms.checked_add(size)?;
        Ok(())
    }
}

impl Ord for RestingOrder {
    fn cmp(&self, other: &Self) -> Ordering {
        // We only compare bids with bids or asks with asks. If you want to
        // check if orders match, directly access their prices.
        debug_assert!(self.get_is_bid() == other.get_is_bid());

        if self.get_is_bid() {
            (self.price).cmp(&other.price)
        } else {
            (other.price).cmp(&(self.price))
        }
    }
}

impl PartialOrd for RestingOrder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for RestingOrder {
    fn eq(&self, other: &Self) -> bool {
        if self.trader_index != other.trader_index || self.order_type != other.order_type {
            return false;
        }
        if self.order_type.is_reversible() {
            // Allow off by 1 for reverse orders to enable coalescing. Otherwise there is a back and forth that fragments into many orders.
            self.price == other.price
                || u64_slice_to_u128(self.price.inner) + 1 == u64_slice_to_u128(other.price.inner)
                || u64_slice_to_u128(self.price.inner) - 1 == u64_slice_to_u128(other.price.inner)
        } else {
            // Only used in equality check of lookups, so we can ignore size, seqnum, ...
            self.price == other.price
        }
    }
}

impl Eq for RestingOrder {}

impl std::fmt::Display for RestingOrder {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}@{}", self.num_base_atoms, self.price)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::quantities::WrapperU64;

    #[test]
    fn test_default() {
        assert_eq!(OrderType::default(), OrderType::Limit);
    }

    #[test]
    fn test_display() {
        let resting_order: RestingOrder = RestingOrder::new(
            0,
            BaseAtoms::ZERO,
            QuoteAtomsPerBaseAtom::ZERO,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            true,
            OrderType::Limit,
        )
        .unwrap();
        let _ = format!("{}", resting_order);
    }

    #[test]
    fn test_cmp() {
        let resting_order_1: RestingOrder = RestingOrder::new(
            0,
            BaseAtoms::new(1),
            QuoteAtomsPerBaseAtom::try_from(1.00000000000001).unwrap(),
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            true,
            OrderType::Limit,
        )
        .unwrap();
        let resting_order_2: RestingOrder = RestingOrder::new(
            0,
            BaseAtoms::new(1_000_000_000),
            QuoteAtomsPerBaseAtom::try_from(1.01).unwrap(),
            1,
            NO_EXPIRATION_LAST_VALID_SLOT,
            true,
            OrderType::Limit,
        )
        .unwrap();
        assert!(resting_order_1 < resting_order_2);
        assert!(resting_order_1 != resting_order_2);

        let resting_order_1: RestingOrder = RestingOrder::new(
            0,
            BaseAtoms::new(1),
            QuoteAtomsPerBaseAtom::try_from(1.00000000000001).unwrap(),
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            false,
            OrderType::Limit,
        )
        .unwrap();
        let resting_order_2: RestingOrder = RestingOrder::new(
            0,
            BaseAtoms::new(1_000_000_000),
            QuoteAtomsPerBaseAtom::try_from(1.01).unwrap(),
            1,
            NO_EXPIRATION_LAST_VALID_SLOT,
            false,
            OrderType::Limit,
        )
        .unwrap();
        assert!(resting_order_1 > resting_order_2);
        assert!(resting_order_1 != resting_order_2);
    }

    #[test]
    fn test_setters() {
        let mut resting_order: RestingOrder = RestingOrder::new(
            0,
            BaseAtoms::ZERO,
            QuoteAtomsPerBaseAtom::ZERO,
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            true,
            OrderType::Limit,
        )
        .unwrap();
        resting_order.set_last_valid_slot(1);
        resting_order.set_sequence_number(1);
    }
}
