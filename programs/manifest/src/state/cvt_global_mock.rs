//! Mock of the global account for formal verification.
//!
//! The global account keeps two red-black trees, one of `GlobalTrader` and one
//! of `GlobalDeposit`. Reasoning about the real trees is out of reach for the
//! prover, so, exactly like `cvt_db_mock` does for the market, we replace them
//! with a fixed number of slots. The global account is modeled with two seats,
//! owned by the same two traders that the market mock knows about, so a maker
//! with a seat on the market can also have a global deposit.
//!
//! Everything that touches balances (`deposit_global`, `withdraw_global`,
//! `reduce`) is still the real production code. Only the lookup and the tree
//! rebalancing are mocked.
use std::marker::PhantomData;

use crate::{
    certora::utils::alloc_havoced,
    quantities::WrapperU64,
    state::{
        global::{GlobalDeposit, GlobalTrader},
        main_trader_pk, second_trader_pk, GLOBAL_BLOCK_SIZE,
    },
};
use cvt::{cvt_assert, cvt_assume};
use hypertree::{get_helper, get_mut_helper, DataIndex, RBNode, NIL};
use nondet::nondet;
use solana_program::pubkey::Pubkey;

const NUM_BLOCKS: usize = 8;
/// Must be big enough that every INDEX constant below is a legal index into an
/// array of that size.
const GLOBAL_DATA_LEN: usize = NUM_BLOCKS * GLOBAL_BLOCK_SIZE;

/// Indices of the two modeled global traders, as the rest of the program sees
/// them.
const MAIN_GLOBAL_TRADER_INDEX: DataIndex = 0;
const SECOND_GLOBAL_TRADER_INDEX: DataIndex = GLOBAL_BLOCK_SIZE as DataIndex;
/// Indices of the two modeled global deposits.
const MAIN_GLOBAL_DEPOSIT_INDEX: DataIndex = 2 * GLOBAL_BLOCK_SIZE as DataIndex;
const SECOND_GLOBAL_DEPOSIT_INDEX: DataIndex = 3 * GLOBAL_BLOCK_SIZE as DataIndex;

/// Where those nodes actually live in the mocked storage.
const MAIN_GLOBAL_TRADER_DATA_IDX: DataIndex = 0;
const SECOND_GLOBAL_TRADER_DATA_IDX: DataIndex = GLOBAL_BLOCK_SIZE as DataIndex;
const MAIN_GLOBAL_DEPOSIT_DATA_IDX: DataIndex = 0;
const SECOND_GLOBAL_DEPOSIT_DATA_IDX: DataIndex = GLOBAL_BLOCK_SIZE as DataIndex;

static mut GLOBAL_TRADER_DATA: *mut [u8; GLOBAL_DATA_LEN] = std::ptr::null_mut();
static mut GLOBAL_DEPOSIT_DATA: *mut [u8; GLOBAL_DATA_LEN] = std::ptr::null_mut();

/// A seat in the trader tree. Tracked separately from the deposit tree because
/// `reduce`, `deposit_global` and `withdraw_global` temporarily take the
/// deposit out of its tree to re-sort it while the trader stays put.
static mut IS_MAIN_GLOBAL_TRADER_TAKEN: u64 = 0;
static mut IS_SECOND_GLOBAL_TRADER_TAKEN: u64 = 0;
static mut IS_MAIN_GLOBAL_DEPOSIT_TAKEN: u64 = 0;
static mut IS_SECOND_GLOBAL_DEPOSIT_TAKEN: u64 = 0;

/// `add_trader` asks for two free blocks in a row, a trader block and then a
/// deposit block, before it inserts either of them. Alternate between the two
/// kinds so the second call does not hand out the block the first one did.
static mut NEXT_FREE_BLOCK_IS_DEPOSIT: bool = false;

pub fn init_global_mock() {
    unsafe {
        GLOBAL_TRADER_DATA = alloc_havoced::<[u8; GLOBAL_DATA_LEN]>();
        GLOBAL_DEPOSIT_DATA = alloc_havoced::<[u8; GLOBAL_DATA_LEN]>();
        NEXT_FREE_BLOCK_IS_DEPOSIT = false;

        let is_main_taken: u64 = nondet();
        let is_second_taken: u64 = nondet();
        cvt_assume!(is_main_taken <= 1);
        cvt_assume!(is_second_taken <= 1);
        IS_MAIN_GLOBAL_TRADER_TAKEN = is_main_taken;
        IS_SECOND_GLOBAL_TRADER_TAKEN = is_second_taken;
        // A trader and its deposit are claimed and released together.
        IS_MAIN_GLOBAL_DEPOSIT_TAKEN = is_main_taken;
        IS_SECOND_GLOBAL_DEPOSIT_TAKEN = is_second_taken;
    }

    // The havoced bytes have to be made self consistent: a trader node points
    // at the deposit node of the same trader, and both carry the trader key.
    // Balances stay nondeterministic.
    let dynamic: &mut [u8; 8] = &mut [0; 8];
    *get_mut_helper_global_trader(dynamic, MAIN_GLOBAL_TRADER_INDEX) = new_node(
        GlobalTrader::new_empty(main_trader_pk(), MAIN_GLOBAL_DEPOSIT_INDEX),
    );
    *get_mut_helper_global_trader(dynamic, SECOND_GLOBAL_TRADER_INDEX) = new_node(
        GlobalTrader::new_empty(second_trader_pk(), SECOND_GLOBAL_DEPOSIT_INDEX),
    );
    *get_mut_helper_global_deposit(dynamic, MAIN_GLOBAL_DEPOSIT_INDEX) =
        new_node(GlobalDeposit::new_nondet(main_trader_pk()));
    *get_mut_helper_global_deposit(dynamic, SECOND_GLOBAL_DEPOSIT_INDEX) =
        new_node(GlobalDeposit::new_nondet(second_trader_pk()));
}

fn new_node<V: Copy>(value: V) -> RBNode<V> {
    RBNode {
        left: NIL,
        right: NIL,
        parent: NIL,
        color: hypertree::Color::Red,
        value,
        payload_type: 0,
        _unused_padding: 0,
    }
}

pub fn main_global_trader_index() -> DataIndex {
    MAIN_GLOBAL_TRADER_INDEX
}
pub fn second_global_trader_index() -> DataIndex {
    SECOND_GLOBAL_TRADER_INDEX
}
pub fn main_global_deposit_index() -> DataIndex {
    MAIN_GLOBAL_DEPOSIT_INDEX
}
pub fn second_global_deposit_index() -> DataIndex {
    SECOND_GLOBAL_DEPOSIT_INDEX
}

pub fn is_main_global_seat_taken() -> bool {
    unsafe { IS_MAIN_GLOBAL_TRADER_TAKEN == 1 }
}
pub fn is_main_global_seat_free() -> bool {
    !is_main_global_seat_taken()
}
pub fn is_second_global_seat_taken() -> bool {
    unsafe { IS_SECOND_GLOBAL_TRADER_TAKEN == 1 }
}
pub fn is_second_global_seat_free() -> bool {
    !is_second_global_seat_taken()
}

/// Whether the trader identified by `pk` holds a global seat.
pub fn has_mock_global_seat(pk: &Pubkey) -> bool {
    lookup_global_trader_index(pk) != NIL
}

/// Assume the trader identified by `pk` holds a global seat.
pub fn cvt_assume_has_global_seat(pk: &Pubkey) {
    cvt_assume!(has_mock_global_seat(pk));
}

/// Balance of `pk` in the mocked global account. Zero when the trader holds no
/// seat, which is what the production `get_balance_atoms` answers for an
/// evicted trader.
pub fn global_balance_atoms(pk: &Pubkey) -> u64 {
    let trader_index: DataIndex = lookup_global_trader_index(pk);
    if trader_index == NIL {
        return 0;
    }
    let dynamic: &mut [u8; 8] = &mut [0; 8];
    let deposit_index: DataIndex = get_helper_global_trader(dynamic, trader_index)
        .get_value()
        .get_deposit_index();
    get_helper_global_deposit(dynamic, deposit_index)
        .get_value()
        .get_balance_atoms()
        .as_u64()
}

pub fn get_helper_global_trader(_data: &[u8], index: DataIndex) -> &'static RBNode<GlobalTrader> {
    get_helper::<RBNode<GlobalTrader>>(unsafe { &*GLOBAL_TRADER_DATA }, trader_data_idx(index))
}

pub fn get_mut_helper_global_trader(
    _data: &mut [u8],
    index: DataIndex,
) -> &'static mut RBNode<GlobalTrader> {
    get_mut_helper::<RBNode<GlobalTrader>>(
        unsafe { &mut *GLOBAL_TRADER_DATA },
        trader_data_idx(index),
    )
}

pub fn get_helper_global_deposit(_data: &[u8], index: DataIndex) -> &'static RBNode<GlobalDeposit> {
    get_helper::<RBNode<GlobalDeposit>>(unsafe { &*GLOBAL_DEPOSIT_DATA }, deposit_data_idx(index))
}

pub fn get_mut_helper_global_deposit(
    _data: &mut [u8],
    index: DataIndex,
) -> &'static mut RBNode<GlobalDeposit> {
    get_mut_helper::<RBNode<GlobalDeposit>>(
        unsafe { &mut *GLOBAL_DEPOSIT_DATA },
        deposit_data_idx(index),
    )
}

fn trader_data_idx(index: DataIndex) -> DataIndex {
    if index == MAIN_GLOBAL_TRADER_INDEX {
        MAIN_GLOBAL_TRADER_DATA_IDX
    } else if index == SECOND_GLOBAL_TRADER_INDEX {
        SECOND_GLOBAL_TRADER_DATA_IDX
    } else {
        cvt_assert!(false);
        MAIN_GLOBAL_TRADER_DATA_IDX
    }
}

fn deposit_data_idx(index: DataIndex) -> DataIndex {
    if index == MAIN_GLOBAL_DEPOSIT_INDEX {
        MAIN_GLOBAL_DEPOSIT_DATA_IDX
    } else if index == SECOND_GLOBAL_DEPOSIT_INDEX {
        SECOND_GLOBAL_DEPOSIT_DATA_IDX
    } else {
        cvt_assert!(false);
        MAIN_GLOBAL_DEPOSIT_DATA_IDX
    }
}

fn deposit_balance_at(index: DataIndex) -> u64 {
    let dynamic: &mut [u8; 8] = &mut [0; 8];
    get_helper_global_deposit(dynamic, index)
        .get_value()
        .get_balance_atoms()
        .as_u64()
}

/// Index of the deposit with the smallest balance, which is what the real tree
/// exposes as its max because `GlobalDeposit::cmp` is reversed.
pub fn min_balance_deposit_index() -> DataIndex {
    let main_taken: bool = unsafe { IS_MAIN_GLOBAL_DEPOSIT_TAKEN == 1 };
    let second_taken: bool = unsafe { IS_SECOND_GLOBAL_DEPOSIT_TAKEN == 1 };
    if main_taken && second_taken {
        if deposit_balance_at(MAIN_GLOBAL_DEPOSIT_INDEX)
            <= deposit_balance_at(SECOND_GLOBAL_DEPOSIT_INDEX)
        {
            MAIN_GLOBAL_DEPOSIT_INDEX
        } else {
            SECOND_GLOBAL_DEPOSIT_INDEX
        }
    } else if main_taken {
        MAIN_GLOBAL_DEPOSIT_INDEX
    } else if second_taken {
        SECOND_GLOBAL_DEPOSIT_INDEX
    } else {
        NIL
    }
}

/// The global account never frees blocks, so a free address is just a block of
/// the requested kind belonging to a seat nobody holds.
pub fn get_free_address_on_global_fixed(
    _fixed: &mut crate::state::GlobalFixed,
    _dynamic: &mut [u8],
) -> DataIndex {
    let want_deposit: bool = unsafe { NEXT_FREE_BLOCK_IS_DEPOSIT };
    unsafe { NEXT_FREE_BLOCK_IS_DEPOSIT = !want_deposit };

    if is_main_global_seat_free() {
        if want_deposit {
            MAIN_GLOBAL_DEPOSIT_INDEX
        } else {
            MAIN_GLOBAL_TRADER_INDEX
        }
    } else if is_second_global_seat_free() {
        if want_deposit {
            SECOND_GLOBAL_DEPOSIT_INDEX
        } else {
            SECOND_GLOBAL_TRADER_INDEX
        }
    } else {
        cvt_assert!(false);
        NIL
    }
}

pub struct CvtGlobalTraderTree<'a> {
    root_index: DataIndex,
    phantom: PhantomData<&'a mut [u8]>,
}

impl<'a> CvtGlobalTraderTree<'a> {
    pub fn new(_data: &'a mut [u8], root_index: DataIndex, _max_index: DataIndex) -> Self {
        Self {
            root_index,
            phantom: PhantomData,
        }
    }

    pub fn get_root_index(&self) -> DataIndex {
        self.root_index
    }

    pub fn lookup_index(&self, global_trader: &GlobalTrader) -> DataIndex {
        lookup_global_trader_index(global_trader.get_trader())
    }

    pub fn insert(&mut self, index: DataIndex, global_trader: GlobalTrader) {
        let dynamic: &mut [u8; 8] = &mut [0; 8];
        *get_mut_helper_global_trader(dynamic, index) = new_node(global_trader);
        if index == MAIN_GLOBAL_TRADER_INDEX {
            unsafe {
                cvt_assert!(IS_MAIN_GLOBAL_TRADER_TAKEN == 0);
                IS_MAIN_GLOBAL_TRADER_TAKEN = 1;
            }
        } else if index == SECOND_GLOBAL_TRADER_INDEX {
            unsafe {
                cvt_assert!(IS_SECOND_GLOBAL_TRADER_TAKEN == 0);
                IS_SECOND_GLOBAL_TRADER_TAKEN = 1;
            }
        } else {
            cvt_assert!(false);
        }
    }

    pub fn remove_by_index(&mut self, index: DataIndex) {
        if index == MAIN_GLOBAL_TRADER_INDEX {
            unsafe {
                cvt_assert!(IS_MAIN_GLOBAL_TRADER_TAKEN == 1);
                IS_MAIN_GLOBAL_TRADER_TAKEN = 0;
            }
        } else if index == SECOND_GLOBAL_TRADER_INDEX {
            unsafe {
                cvt_assert!(IS_SECOND_GLOBAL_TRADER_TAKEN == 1);
                IS_SECOND_GLOBAL_TRADER_TAKEN = 0;
            }
        } else {
            cvt_assert!(false);
        }
    }
}

pub struct CvtGlobalTraderTreeReadOnly<'a> {
    root_index: DataIndex,
    phantom: PhantomData<&'a [u8]>,
}

impl<'a> CvtGlobalTraderTreeReadOnly<'a> {
    pub fn new(_data: &'a [u8], root_index: DataIndex, _max_index: DataIndex) -> Self {
        Self {
            root_index,
            phantom: PhantomData,
        }
    }

    pub fn get_root_index(&self) -> DataIndex {
        self.root_index
    }

    pub fn lookup_index(&self, global_trader: &GlobalTrader) -> DataIndex {
        lookup_global_trader_index(global_trader.get_trader())
    }
}

/// Scan the two seats for a stored trader key. The pubkey held by a slot can
/// change over the life of a rule (eviction replaces it), so the lookup has to
/// read the node instead of relying on a fixed trader-to-slot assignment.
fn lookup_global_trader_index(trader: &Pubkey) -> DataIndex {
    let dynamic: &mut [u8; 8] = &mut [0; 8];
    if is_main_global_seat_taken()
        && get_helper_global_trader(dynamic, MAIN_GLOBAL_TRADER_INDEX)
            .get_value()
            .get_trader()
            == trader
    {
        MAIN_GLOBAL_TRADER_INDEX
    } else if is_second_global_seat_taken()
        && get_helper_global_trader(dynamic, SECOND_GLOBAL_TRADER_INDEX)
            .get_value()
            .get_trader()
            == trader
    {
        SECOND_GLOBAL_TRADER_INDEX
    } else {
        NIL
    }
}

pub struct CvtGlobalDepositTree<'a> {
    root_index: DataIndex,
    phantom: PhantomData<&'a mut [u8]>,
}

impl<'a> CvtGlobalDepositTree<'a> {
    pub fn new(_data: &'a mut [u8], root_index: DataIndex, _max_index: DataIndex) -> Self {
        Self {
            root_index,
            phantom: PhantomData,
        }
    }

    pub fn get_root_index(&self) -> DataIndex {
        self.root_index
    }

    pub fn get_max_index(&self) -> DataIndex {
        min_balance_deposit_index()
    }

    pub fn lookup_index(&self, global_deposit: &GlobalDeposit) -> DataIndex {
        lookup_global_deposit_index(global_deposit.get_trader())
    }

    pub fn insert(&mut self, index: DataIndex, global_deposit: GlobalDeposit) {
        let dynamic: &mut [u8; 8] = &mut [0; 8];
        *get_mut_helper_global_deposit(dynamic, index) = new_node(global_deposit);
        if index == MAIN_GLOBAL_DEPOSIT_INDEX {
            unsafe {
                cvt_assert!(IS_MAIN_GLOBAL_DEPOSIT_TAKEN == 0);
                IS_MAIN_GLOBAL_DEPOSIT_TAKEN = 1;
            }
        } else if index == SECOND_GLOBAL_DEPOSIT_INDEX {
            unsafe {
                cvt_assert!(IS_SECOND_GLOBAL_DEPOSIT_TAKEN == 0);
                IS_SECOND_GLOBAL_DEPOSIT_TAKEN = 1;
            }
        } else {
            cvt_assert!(false);
        }
    }

    pub fn remove_by_index(&mut self, index: DataIndex) {
        if index == MAIN_GLOBAL_DEPOSIT_INDEX {
            unsafe {
                cvt_assert!(IS_MAIN_GLOBAL_DEPOSIT_TAKEN == 1);
                IS_MAIN_GLOBAL_DEPOSIT_TAKEN = 0;
            }
        } else if index == SECOND_GLOBAL_DEPOSIT_INDEX {
            unsafe {
                cvt_assert!(IS_SECOND_GLOBAL_DEPOSIT_TAKEN == 1);
                IS_SECOND_GLOBAL_DEPOSIT_TAKEN = 0;
            }
        } else {
            cvt_assert!(false);
        }
    }
}

pub struct CvtGlobalDepositTreeReadOnly<'a> {
    root_index: DataIndex,
    phantom: PhantomData<&'a [u8]>,
}

impl<'a> CvtGlobalDepositTreeReadOnly<'a> {
    pub fn new(_data: &'a [u8], root_index: DataIndex, _max_index: DataIndex) -> Self {
        Self {
            root_index,
            phantom: PhantomData,
        }
    }

    pub fn get_root_index(&self) -> DataIndex {
        self.root_index
    }

    pub fn get_max_index(&self) -> DataIndex {
        min_balance_deposit_index()
    }

    pub fn lookup_index(&self, global_deposit: &GlobalDeposit) -> DataIndex {
        lookup_global_deposit_index(global_deposit.get_trader())
    }
}

/// Same scan as the trader lookup, over the deposit slots.
fn lookup_global_deposit_index(trader: &Pubkey) -> DataIndex {
    let dynamic: &mut [u8; 8] = &mut [0; 8];
    if unsafe { IS_MAIN_GLOBAL_DEPOSIT_TAKEN == 1 }
        && get_helper_global_deposit(dynamic, MAIN_GLOBAL_DEPOSIT_INDEX)
            .get_value()
            .get_trader()
            == trader
    {
        MAIN_GLOBAL_DEPOSIT_INDEX
    } else if unsafe { IS_SECOND_GLOBAL_DEPOSIT_TAKEN == 1 }
        && get_helper_global_deposit(dynamic, SECOND_GLOBAL_DEPOSIT_INDEX)
            .get_value()
            .get_trader()
            == trader
    {
        SECOND_GLOBAL_DEPOSIT_INDEX
    } else {
        NIL
    }
}

/// Sum of the deposits held by the two modeled seats. The ghost aggregate on
/// `GlobalFixed` also covers traders outside the mock, so it is only ever
/// greater than or equal to this.
pub fn modeled_global_deposits() -> u64 {
    let main: u64 = if unsafe { IS_MAIN_GLOBAL_DEPOSIT_TAKEN == 1 } {
        deposit_balance_at(MAIN_GLOBAL_DEPOSIT_INDEX)
    } else {
        0
    };
    let second: u64 = if unsafe { IS_SECOND_GLOBAL_DEPOSIT_TAKEN == 1 } {
        deposit_balance_at(SECOND_GLOBAL_DEPOSIT_INDEX)
    } else {
        0
    };
    main.saturating_add(second)
}
