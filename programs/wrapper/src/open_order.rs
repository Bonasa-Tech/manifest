use bytemuck::{Pod, Zeroable};
use hypertree::{DataIndex, PodBool};
use manifest::{
    quantities::{BaseAtoms, QuoteAtomsPerBaseAtom},
    state::OrderType,
};
use shank::ShankType;
use static_assertions::const_assert_eq;
use std::{cmp::Ordering, mem::size_of};

use crate::processors::shared::WRAPPER_BLOCK_PAYLOAD_SIZE;

#[repr(C)]
#[derive(Default, Debug, Copy, Clone, Zeroable, Pod, ShankType)]
pub struct WrapperOpenOrder {
    price: QuoteAtomsPerBaseAtom,
    client_order_id: u64,
    order_sequence_number: u64,
    num_base_atoms: BaseAtoms,
    market_data_index: DataIndex,
    last_valid_slot: u32,
    is_bid: PodBool,
    order_type: OrderType,

    _padding: [u8; 30],
}

// Blocks on wrapper are bigger than blocks on the market because there is more
// data to store here.

// 16 + // price
// 8 + // client_order_id
// 8 + // order_sequence_number
// 8 + // num_base_atoms
// 4 + // market_data_index
// 4 + // last_valid_slot
// 1 + // is_bid
// 1 + // order_type
// 30  // padding
// = 80
const_assert_eq!(size_of::<WrapperOpenOrder>(), WRAPPER_BLOCK_PAYLOAD_SIZE);
const_assert_eq!(size_of::<WrapperOpenOrder>() % 16, 0);

impl WrapperOpenOrder {
    /// Create a new WrapperOpenOrder.
    pub fn new(
        client_order_id: u64,
        order_sequence_number: u64,
        price: QuoteAtomsPerBaseAtom,
        num_base_atoms: BaseAtoms,
        last_valid_slot: u32,
        market_data_index: DataIndex,
        is_bid: bool,
        order_type: OrderType,
    ) -> Self {
        WrapperOpenOrder {
            client_order_id,
            order_sequence_number,
            price,
            num_base_atoms,
            last_valid_slot,
            order_type,
            market_data_index,
            is_bid: PodBool::from_bool(is_bid),
            _padding: [0; 30],
        }
    }

    /// is_bid as a boolean.
    pub fn get_is_bid(&self) -> bool {
        self.is_bid.0 == 1
    }

    /// get the order sequence number from the market.
    pub fn get_order_sequence_number(&self) -> u64 {
        self.order_sequence_number
    }

    /// Get the DataIndex for the order in the core program.
    pub fn get_market_data_index(&self) -> DataIndex {
        self.market_data_index
    }

    /// Get the remaining number of base atoms in the order.
    pub fn get_num_base_atoms(&self) -> BaseAtoms {
        self.num_base_atoms
    }

    /// Get client defined order id for the order.
    pub fn get_client_order_id(&self) -> u64 {
        self.client_order_id
    }

    /// Get price on the order.
    pub fn get_price(&self) -> QuoteAtomsPerBaseAtom {
        self.price
    }

    /// Set the price on an order.
    pub fn set_price(&mut self, price: QuoteAtomsPerBaseAtom) {
        self.price = price;
    }

    /// Update the number of remaining base atoms.
    pub fn update_remaining(&mut self, num_base_atoms: BaseAtoms) {
        self.num_base_atoms = num_base_atoms;
    }
}

impl Ord for WrapperOpenOrder {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.client_order_id).cmp(&(other.client_order_id))
    }
}

impl PartialOrd for WrapperOpenOrder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for WrapperOpenOrder {
    fn eq(&self, other: &Self) -> bool {
        (self.client_order_id) == (other.client_order_id)
    }
}

impl Eq for WrapperOpenOrder {}

impl std::fmt::Display for WrapperOpenOrder {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.client_order_id)
    }
}

#[test]
fn test_display() {
    let open_order: WrapperOpenOrder = WrapperOpenOrder::new(
        0,
        0,
        1.0.try_into().unwrap(),
        BaseAtoms::ZERO,
        0,
        0,
        false,
        OrderType::Limit,
    );
    let _ = format!("{}", open_order);
}

#[test]
fn test_cmp() {
    let open_order: WrapperOpenOrder = WrapperOpenOrder::new(
        0,
        0,
        1.0.try_into().unwrap(),
        BaseAtoms::ZERO,
        0,
        0,
        false,
        OrderType::Limit,
    );
    let open_order2: WrapperOpenOrder = WrapperOpenOrder::new(
        1,
        0,
        1.0.try_into().unwrap(),
        BaseAtoms::ZERO,
        0,
        0,
        false,
        OrderType::Limit,
    );
    assert!(open_order2 > open_order);
    assert!(open_order2 != open_order);
}
