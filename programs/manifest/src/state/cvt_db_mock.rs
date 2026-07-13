use std::marker::PhantomData;

use crate::state::{market::ClaimedSeat, RestingOrder, MARKET_BLOCK_SIZE};
use hypertree::{get_helper, get_mut_helper, DataIndex, RBNode, NIL};
use solana_program::pubkey::Pubkey;

use crate::certora::utils::alloc_havoced;
use cvt::{cvt_assert, cvt_assume};
use nondet::nondet;

const NUM_BLOCKS: usize = 10;
// -- must be big enough so that all INDEX global variables are a legal index
// -- into an array of that size
const GLOBAL_DATA_LEN: usize = NUM_BLOCKS * MARKET_BLOCK_SIZE;

const MAIN_SEAT_INDEX: u64 = 0;
const SECOND_SEAT_INDEX: u64 = MARKET_BLOCK_SIZE as u64;

static mut MAIN_SEAT_PK: *mut Pubkey = std::ptr::null_mut();
static mut IS_MAIN_SEAT_TAKEN: u64 = 0;

static mut SECOND_SEAT_PK: *mut Pubkey = std::ptr::null_mut();
static mut IS_SECOND_SEAT_TAKEN: u64 = 0;

static mut SEAT_DATA: *mut [u8; GLOBAL_DATA_LEN] = std::ptr::null_mut();

const MAIN_BID_ORDER_INDEX: u64 = 3 * MARKET_BLOCK_SIZE as u64;
static mut MAIN_BID_ORDER_TAKEN: u64 = 0;
static mut BID_ORDER_DATA: *mut [u8; GLOBAL_DATA_LEN] = std::ptr::null_mut();

const MAIN_ASK_ORDER_INDEX: u64 = 2 * MARKET_BLOCK_SIZE as u64;
static mut MAIN_ASK_ORDER_TAKEN: u64 = 0;
static mut ASK_ORDER_DATA: *mut [u8; GLOBAL_DATA_LEN] = std::ptr::null_mut();

const MAIN_SEAT_DATA_IDX: DataIndex = 0u32;
const SECOND_SEAT_DATA_IDX: DataIndex = MARKET_BLOCK_SIZE as u32;
const MAIN_BID_ORDER_DATA_IDX: DataIndex = 0u32;
const MAIN_ASK_ORDER_DATA_IDX: DataIndex = 0u32;

pub fn init_mock() {
    init_mock_traders();
    init_mock_orders();
    crate::state::init_global_mock();
}

fn init_mock_traders() {
    unsafe {
        // MAIN_SEAT_INDEX = 0;
        MAIN_SEAT_PK = alloc_havoced::<Pubkey>();
        IS_MAIN_SEAT_TAKEN = nondet();

        // SECOND_SEAT_INDEX = MARKET_BLOCK_SIZE as u64;
        SECOND_SEAT_PK = alloc_havoced::<Pubkey>();
        IS_SECOND_SEAT_TAKEN = nondet();

        SEAT_DATA = alloc_havoced::<[u8; GLOBAL_DATA_LEN]>()
    }
}

pub fn cvt_assume_main_trader_has_seat(pk: &Pubkey) {
    cvt_assume!(pk == main_trader_pk());
    cvt_assume!(unsafe { IS_MAIN_SEAT_TAKEN } == 1);
}

pub fn cvt_assume_second_trader_has_seat(pk: &Pubkey) {
    cvt_assume!(pk == second_trader_pk());
    cvt_assume!(unsafe { IS_SECOND_SEAT_TAKEN } == 1);
}

pub fn main_trader_pk() -> &'static Pubkey {
    unsafe { &*MAIN_SEAT_PK }
}

pub fn main_trader_index() -> DataIndex {
    MAIN_SEAT_INDEX as DataIndex
}

pub fn second_trader_pk() -> &'static Pubkey {
    unsafe { &*SECOND_SEAT_PK }
}

pub fn second_trader_index() -> DataIndex {
    SECOND_SEAT_INDEX as DataIndex
}

/// Read a `RBNode<ClaimedSeat>` in an array of data at a given index.
pub fn get_helper_seat(_data: &[u8], index: DataIndex) -> &'static RBNode<ClaimedSeat> {
    if index == main_trader_index() {
        get_helper::<RBNode<ClaimedSeat>>(unsafe { &*SEAT_DATA }, MAIN_SEAT_DATA_IDX)
    } else if index == second_trader_index() {
        get_helper::<RBNode<ClaimedSeat>>(unsafe { &*SEAT_DATA }, SECOND_SEAT_DATA_IDX)
    } else {
        cvt_assert!(false);
        // -- return something to make Rust happy. Protected by assert above
        get_helper::<RBNode<ClaimedSeat>>(unsafe { &*SEAT_DATA }, index)
    }
}

/// Read a `RBNode<ClaimedSeat>` in an array of data at a given index.
pub fn get_mut_helper_seat(_data: &mut [u8], index: DataIndex) -> &mut RBNode<ClaimedSeat> {
    if index == main_trader_index() {
        get_mut_helper::<RBNode<ClaimedSeat>>(unsafe { &mut *SEAT_DATA }, MAIN_SEAT_DATA_IDX)
    } else if index == second_trader_index() {
        get_mut_helper::<RBNode<ClaimedSeat>>(unsafe { &mut *SEAT_DATA }, SECOND_SEAT_DATA_IDX)
    } else {
        cvt_assert!(false);
        // -- return something to make Rust happy. Protected by assert above
        get_mut_helper::<RBNode<ClaimedSeat>>(unsafe { &mut *SEAT_DATA }, index)
    }
}

pub fn is_main_seat_taken() -> bool {
    !is_main_seat_free()
}
pub fn is_main_seat_free() -> bool {
    unsafe { IS_MAIN_SEAT_TAKEN == 0 }
}

pub fn take_main_seat() {
    cvt_assert!(is_main_seat_free());
    unsafe { IS_MAIN_SEAT_TAKEN = 1 };
}

pub fn release_main_seat() {
    cvt_assert!(is_main_seat_taken());
    unsafe { IS_MAIN_SEAT_TAKEN = 0 };
}

pub fn is_second_seat_taken() -> bool {
    !is_second_seat_free()
}
pub fn is_second_seat_free() -> bool {
    unsafe { IS_SECOND_SEAT_TAKEN == 0 }
}

pub fn take_second_seat() {
    cvt_assert!(is_second_seat_free());
    unsafe { IS_SECOND_SEAT_TAKEN = 1 };
}

pub fn release_second_seat() {
    cvt_assert!(is_second_seat_taken());
    unsafe { IS_SECOND_SEAT_TAKEN = 0 };
}

pub fn main_bid_order_index() -> DataIndex {
    MAIN_BID_ORDER_INDEX as DataIndex
}

pub fn main_ask_order_index() -> DataIndex {
    MAIN_ASK_ORDER_INDEX as DataIndex
}

fn init_mock_orders() {
    unsafe {
        BID_ORDER_DATA = alloc_havoced::<[u8; GLOBAL_DATA_LEN]>();
        MAIN_BID_ORDER_TAKEN = nondet();

        ASK_ORDER_DATA = alloc_havoced::<[u8; GLOBAL_DATA_LEN]>();
        MAIN_ASK_ORDER_TAKEN = nondet();
    }
}

pub fn get_helper_order(data: &[u8], index: DataIndex) -> &RBNode<RestingOrder> {
    if index == main_ask_order_index() {
        get_helper_ask_order(data, index)
    } else if index == main_bid_order_index() {
        get_helper_bid_order(data, index)
    } else {
        cvt_assert!(false);
        get_helper::<RBNode<RestingOrder>>(data, index)
    }
}
pub fn get_mut_helper_order(data: &mut [u8], index: DataIndex) -> &mut RBNode<RestingOrder> {
    if index == main_ask_order_index() {
        get_mut_helper_ask_order(data, index)
    } else if index == main_bid_order_index() {
        get_mut_helper_bid_order(data, index)
    } else {
        cvt_assert!(false);
        get_mut_helper::<RBNode<RestingOrder>>(data, index)
    }
}

pub fn get_helper_bid_order(_data: &[u8], index: DataIndex) -> &RBNode<RestingOrder> {
    if index == main_bid_order_index() {
        get_helper::<RBNode<RestingOrder>>(unsafe { &*BID_ORDER_DATA }, MAIN_BID_ORDER_DATA_IDX)
    } else {
        cvt_assert!(false);
        get_helper::<RBNode<RestingOrder>>(unsafe { &*BID_ORDER_DATA }, index)
    }
}
pub fn get_mut_helper_bid_order(_data: &mut [u8], index: DataIndex) -> &mut RBNode<RestingOrder> {
    if index == main_bid_order_index() {
        get_mut_helper::<RBNode<RestingOrder>>(
            unsafe { &mut *BID_ORDER_DATA },
            MAIN_BID_ORDER_DATA_IDX,
        )
    } else {
        cvt_assert!(false);
        get_mut_helper::<RBNode<RestingOrder>>(unsafe { &mut *BID_ORDER_DATA }, index)
    }
}

pub fn get_helper_ask_order(_data: &[u8], index: DataIndex) -> &RBNode<RestingOrder> {
    if index == main_ask_order_index() {
        get_helper::<RBNode<RestingOrder>>(unsafe { &*ASK_ORDER_DATA }, MAIN_ASK_ORDER_DATA_IDX)
    } else {
        cvt_assert!(false);
        get_helper::<RBNode<RestingOrder>>(unsafe { &*ASK_ORDER_DATA }, index)
    }
}
pub fn get_mut_helper_ask_order(_data: &mut [u8], index: DataIndex) -> &mut RBNode<RestingOrder> {
    if index == main_ask_order_index() {
        get_mut_helper::<RBNode<RestingOrder>>(
            unsafe { &mut *ASK_ORDER_DATA },
            MAIN_ASK_ORDER_DATA_IDX,
        )
    } else {
        cvt_assert!(false);
        get_mut_helper::<RBNode<RestingOrder>>(unsafe { &mut *ASK_ORDER_DATA }, index)
    }
}

pub fn is_bid_order_taken() -> bool {
    !is_bid_order_free()
}
pub fn is_bid_order_free() -> bool {
    unsafe { MAIN_BID_ORDER_TAKEN == 0 }
}
pub fn take_bid_order() {
    unsafe {
        cvt_assert!(is_bid_order_free());
        MAIN_BID_ORDER_TAKEN = 1
    };
}

pub fn release_bid_order() {
    unsafe {
        cvt_assert!(is_bid_order_taken());
        MAIN_BID_ORDER_TAKEN = 0;
    }
}

pub fn is_ask_order_taken() -> bool {
    !is_ask_order_free()
}
pub fn is_ask_order_free() -> bool {
    unsafe { MAIN_ASK_ORDER_TAKEN == 0 }
}

pub fn take_ask_order() {
    unsafe {
        cvt_assert!(is_ask_order_free());
        MAIN_ASK_ORDER_TAKEN = 1
    };
}

pub fn release_ask_order() {
    unsafe {
        cvt_assert!(is_ask_order_taken());
        MAIN_ASK_ORDER_TAKEN = 0;
    }
}

pub struct CvtClaimedSeatTreeReadOnly<'a> {
    _root_index: DataIndex,
    _max_index: DataIndex,
    phantom: std::marker::PhantomData<&'a [u8]>,
}

impl<'a> CvtClaimedSeatTreeReadOnly<'a> {
    pub fn new(_data: &'a [u8], _root_index: DataIndex, _max_index: DataIndex) -> Self {
        Self {
            _root_index,
            _max_index,
            phantom: PhantomData,
        }
    }

    pub fn lookup_index(&self, seat: &ClaimedSeat) -> DataIndex {
        if &seat.trader == main_trader_pk() {
            if is_main_seat_taken() {
                main_trader_index()
            } else {
                NIL
            }
        } else if &seat.trader == second_trader_pk() {
            if is_second_seat_taken() {
                second_trader_index()
            } else {
                NIL
            }
        } else {
            cvt_assert!(false);
            NIL
        }
    }
}

pub struct CvtClaimedSeatTree<'a> {
    root_index: DataIndex,
    _max_index: DataIndex,
    phantom: std::marker::PhantomData<&'a mut [u8]>,
}

impl<'a> CvtClaimedSeatTree<'a> {
    pub fn new(_data: &'a mut [u8], root_index: DataIndex, _max_index: DataIndex) -> Self {
        Self {
            root_index,
            _max_index,
            phantom: PhantomData,
        }
    }

    pub fn get_root_index(&self) -> DataIndex {
        self.root_index
    }

    pub fn lookup_index(&self, seat: &ClaimedSeat) -> DataIndex {
        if &seat.trader == main_trader_pk() {
            if is_main_seat_taken() {
                main_trader_index()
            } else {
                NIL
            }
        } else if &seat.trader == second_trader_pk() {
            if is_second_seat_taken() {
                second_trader_index()
            } else {
                NIL
            }
        } else {
            cvt_assert!(false);
            NIL
        }
    }

    pub fn remove_by_index(&mut self, index: DataIndex) {
        if index == main_trader_index() {
            release_main_seat();
        } else if index == second_trader_index() {
            release_second_seat();
        } else {
            cvt_assert!(false);
        }
    }

    pub fn insert(&mut self, index: DataIndex, seat: ClaimedSeat) {
        if index == main_trader_index() {
            let mut dynamic = [0u8; 8];
            let seat_node: &mut RBNode<ClaimedSeat> =
                get_mut_helper_seat(&mut dynamic, main_trader_index());
            let new_seat_node: RBNode<ClaimedSeat> = RBNode {
                left: NIL,
                right: NIL,
                parent: NIL,
                color: hypertree::Color::Red,
                value: seat,
                payload_type: 0,
                _unused_padding: 0,
            };
            take_main_seat();
            *seat_node = new_seat_node;
        } else if index == second_trader_index() {
            let mut dynamic = [0u8; 8];
            let seat_node: &mut RBNode<ClaimedSeat> =
                get_mut_helper_seat(&mut dynamic, second_trader_index());
            let new_seat_node: RBNode<ClaimedSeat> = RBNode {
                left: NIL,
                right: NIL,
                parent: NIL,
                color: hypertree::Color::Red,
                value: seat,
                payload_type: 0,
                _unused_padding: 0,
            };
            take_second_seat();
            *seat_node = new_seat_node;
        } else {
            cvt_assert!(false);
        }
    }
}

pub struct CvtBooksideReadOnlyIterator {
    pub calls: u64,
}

impl std::iter::Iterator for CvtBooksideReadOnlyIterator {
    type Item = (DataIndex, RestingOrder);

    fn next(&mut self) -> Option<Self::Item> {
        // TODO: this does not support when both main orders are present.
        if is_bid_order_taken() && self.calls < 1 {
            let order_index = main_bid_order_index();
            let dynamic = &mut [0; 8];
            let resting_order: &RestingOrder = get_helper_order(dynamic, order_index).get_value();
            self.calls += 1;
            Some((order_index, *resting_order))
        } else if is_ask_order_taken() && self.calls < 1 {
            let order_index = main_ask_order_index();
            let dynamic = &mut [0; 8];
            let resting_order: &RestingOrder = get_helper_order(dynamic, order_index).get_value();
            self.calls += 1;
            Some((order_index, *resting_order))
        } else {
            None
        }
    }
}

pub struct CvtBooksideReadOnly<'a> {
    root_index: DataIndex,
    max_index: DataIndex,
    phantom: std::marker::PhantomData<&'a [u8]>,
}

impl<'a> CvtBooksideReadOnly<'a> {
    pub fn new(_data: &'a [u8], root_index: DataIndex, max_index: DataIndex) -> Self {
        Self {
            root_index,
            max_index,
            phantom: PhantomData,
        }
    }

    pub fn get_root_index(&self) -> DataIndex {
        self.root_index
    }

    pub fn get_max_index(&self) -> DataIndex {
        self.max_index
    }

    pub fn get_next_lower_index<V>(&self, _index: DataIndex) -> DataIndex {
        nondet()
    }

    pub fn iter<V>(&self) -> CvtBooksideReadOnlyIterator {
        // -- non-deterministically, return an empty iterator
        let calls = if nondet::nondet::<bool>() { 0 } else { 1 };
        CvtBooksideReadOnlyIterator { calls }
    }
}

pub struct CvtBookside<'a> {
    root_index: DataIndex,
    max_index: DataIndex,
    phantom: std::marker::PhantomData<&'a mut [u8]>,
}

impl<'a> CvtBookside<'a> {
    pub fn new(_data: &'a mut [u8], root_index: DataIndex, max_index: DataIndex) -> Self {
        Self {
            root_index,
            max_index,
            phantom: PhantomData,
        }
    }

    pub fn get_root_index(&self) -> DataIndex {
        self.root_index
    }

    pub fn get_max_index(&self) -> DataIndex {
        self.max_index
    }

    /// The mock holds at most one order per side, so a lookup can only ever
    /// find that one. Used when a reverse order looks for an order at the same
    /// price to coalesce into.
    pub fn lookup_index(&self, order: &RestingOrder) -> DataIndex {
        let dynamic: &mut [u8; 8] = &mut [0; 8];
        if order.get_is_bid() {
            if is_bid_order_free() {
                return NIL;
            }
            let index: DataIndex = main_bid_order_index();
            let resting_order: &RestingOrder = get_helper_bid_order(dynamic, index).get_value();
            if resting_order == order {
                index
            } else {
                NIL
            }
        } else {
            if is_ask_order_free() {
                return NIL;
            }
            let index: DataIndex = main_ask_order_index();
            let resting_order: &RestingOrder = get_helper_ask_order(dynamic, index).get_value();
            if resting_order == order {
                index
            } else {
                NIL
            }
        }
    }

    pub fn remove_by_index(&mut self, index: DataIndex) {
        if index == main_bid_order_index() {
            release_bid_order();
        } else if index == main_ask_order_index() {
            release_ask_order();
        } else {
            cvt_assert!(false);
        }
    }

    pub fn insert(&mut self, index: DataIndex, order: RestingOrder) {
        let new_order_node: RBNode<RestingOrder> = RBNode {
            left: nondet(),
            right: nondet(),
            parent: nondet(),
            color: hypertree::Color::Red,
            value: order,
            payload_type: 0,
            _unused_padding: 0,
        };
        if index == main_bid_order_index() {
            let mut dynamic = [0u8; 8];
            let bid_order_node = get_mut_helper_bid_order(&mut dynamic, index);
            *bid_order_node = new_order_node;
            take_bid_order();
        } else if index == main_ask_order_index() {
            let mut dynamic = [0u8; 8];
            let ask_order_node = get_mut_helper_ask_order(&mut dynamic, index);
            *ask_order_node = new_order_node;
            take_ask_order();
        } else {
            cvt_assert!(false);
        }
    }
}
