use std::mem::size_of;

use crate::quantities::{BaseAtoms, PriceConversionError, QuoteAtomsPerBaseAtom};
#[cfg(feature = "certora")]
use crate::quantities::{QuoteAtoms, WrapperU64};
use borsh::{BorshDeserialize, BorshSerialize};
use bytemuck::{Pod, Zeroable};
use hypertree::{DataIndex, PodBool};
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
    Eq,
    PartialOrd,
    Ord,
    Clone,
    Copy,
    ShankType,
    Zeroable,
    Pod,
)]
#[repr(transparent)]
pub struct OrderType(u8);

impl Default for OrderType {
    fn default() -> Self {
        OrderType::Limit
    }
}
impl OrderType {
    #[allow(non_upper_case_globals)]
    pub const Limit: Self = Self(0);
    #[allow(non_upper_case_globals)]
    pub const ImmediateOrCancel: Self = Self(1);
    #[allow(non_upper_case_globals)]
    pub const PostOnly: Self = Self(2);
    #[allow(non_upper_case_globals)]
    pub const Global: Self = Self(3);
    #[allow(non_upper_case_globals)]
    pub const Reverse: Self = Self(4);
    #[allow(non_upper_case_globals)]
    pub const ReverseTight: Self = Self(5);

    pub const fn as_u8(self) -> u8 {
        self.0
    }

    pub const fn is_valid(self) -> bool {
        self.0 <= Self::ReverseTight.0
    }

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

impl TryFrom<u8> for OrderType {
    type Error = ProgramError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        let order_type: OrderType = Self(value);
        if order_type.is_valid() {
            Ok(order_type)
        } else {
            Err(ProgramError::InvalidInstructionData)
        }
    }
}

impl From<OrderType> for u8 {
    fn from(value: OrderType) -> Self {
        value.0
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
        if !order_type.is_valid() {
            return Err(ProgramError::InvalidInstructionData);
        }
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
        debug_assert!(self.order_type.is_valid());
        self.order_type
    }

    pub fn has_valid_order_type(&self) -> bool {
        self.order_type.is_valid()
    }

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

    /// Build one of the alternate keys used only by reverse-order coalescing.
    ///
    /// Reverse orders may coalesce when their encoded prices differ by one
    /// unit. That tolerance must not be implemented in `PartialEq`: doing so
    /// would make equality disagree with the tree ordering and could force the
    /// tree to scan an entire same-price bucket. Instead, the caller performs
    /// at most three normal lookups using the exact price and the two adjacent
    /// prices. Each lookup still uses the total `(price, trader, order type)`
    /// key and therefore remains logarithmic.
    pub(crate) fn with_price_offset(mut self, offset: i8) -> Option<Self> {
        self.price.inner = match offset {
            -1 => {
                if self.price.inner == [0, 0] {
                    return None;
                }
                let (low, borrow) = self.price.inner[0].overflowing_sub(1);
                [low, self.price.inner[1] - u64::from(borrow)]
            }
            0 => self.price.inner,
            1 => {
                let (low, carry) = self.price.inner[0].overflowing_add(1);
                let high = self.price.inner[1].checked_add(u64::from(carry))?;
                [low, high]
            }
            _ => return None,
        };
        Some(self)
    }

    pub(crate) fn has_same_coalescing_key(&self, other: &Self) -> bool {
        self.price == other.price
            && self.trader_index == other.trader_index
            && self.order_type == other.order_type
    }
}

impl Ord for RestingOrder {
    fn cmp(&self, other: &Self) -> Ordering {
        // Bids and asks live in separate trees. Price remains the primary key,
        // including the existing reversed price order for asks, so this does
        // not change price priority.
        debug_assert!(self.get_is_bid() == other.get_is_bid());

        let price_ordering = if self.get_is_bid() {
            (self.price).cmp(&other.price)
        } else {
            (other.price).cmp(&(self.price))
        };

        // The generic red-black-tree lookup has to search both subtrees when
        // `cmp` returns Equal but `eq` returns false. Price alone is therefore
        // not a sufficient key: many unrelated traders can place orders at the
        // same price, and a reverse-order lookup for one trader could scan that
        // attacker-controlled bucket until it exhausts the compute budget.
        //
        // Earlier sequence numbers have priority within a price level. The
        // tree's maximum is the next order to match, so reverse the sequence
        // comparison: a lower (earlier) sequence sorts higher. Trader and
        // order type remain deterministic final tie breakers for defensive
        // handling of malformed duplicate sequences.
        price_ordering
            .then_with(|| other.sequence_number.cmp(&self.sequence_number))
            .then_with(|| self.trader_index.cmp(&other.trader_index))
            .then_with(|| self.order_type.cmp(&other.order_type))
    }
}

impl PartialOrd for RestingOrder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for RestingOrder {
    fn eq(&self, other: &Self) -> bool {
        self.sequence_number == other.sequence_number
            && self.trader_index == other.trader_index
            && self.order_type == other.order_type
            && self.price == other.price
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
    fn same_price_orders_have_total_identity_keys() {
        let first = RestingOrder::new(
            1,
            BaseAtoms::ONE,
            1.0.try_into().unwrap(),
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            true,
            OrderType::Reverse,
        )
        .unwrap();
        let unrelated_same_price = RestingOrder::new(
            2,
            BaseAtoms::ONE,
            first.get_price(),
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            true,
            OrderType::Reverse,
        )
        .unwrap();
        let unrelated_same_trader = RestingOrder::new(
            1,
            BaseAtoms::ONE,
            first.get_price(),
            0,
            NO_EXPIRATION_LAST_VALID_SLOT,
            true,
            OrderType::ReverseTight,
        )
        .unwrap();

        for unrelated in [unrelated_same_price, unrelated_same_trader] {
            assert_ne!(first, unrelated);
            assert_ne!(
                first.cmp(&unrelated),
                Ordering::Equal,
                "same-price attacker orders must not enter the tree's equal-key subtree scan"
            );
        }
    }

    #[test]
    fn same_price_orders_are_fifo_on_both_sides() {
        for is_bid in [true, false] {
            let is_bid: bool = is_bid;
            let earlier: RestingOrder = RestingOrder::new(
                9,
                BaseAtoms::ONE,
                1.0.try_into().unwrap(),
                41,
                NO_EXPIRATION_LAST_VALID_SLOT,
                is_bid,
                OrderType::Limit,
            )
            .unwrap();
            let later: RestingOrder = RestingOrder::new(
                1,
                BaseAtoms::ONE,
                earlier.get_price(),
                42,
                NO_EXPIRATION_LAST_VALID_SLOT,
                is_bid,
                OrderType::Limit,
            )
            .unwrap();
            assert!(earlier > later, "earlier order must be tree maximum");
        }
    }

    #[test]
    fn invalid_order_type_byte_is_safe_and_detectable() {
        let mut bytes: [u8; size_of::<RestingOrder>()] = [0_u8; size_of::<RestingOrder>()];
        // order_type is byte 41 in the stable account layout.
        bytes[41] = u8::MAX;
        let order: RestingOrder = bytemuck::pod_read_unaligned::<RestingOrder>(&bytes);
        assert!(!order.has_valid_order_type());
        assert!(OrderType::try_from(u8::MAX).is_err());
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
