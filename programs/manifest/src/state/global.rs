/// The global order state stores information about all global orders for a given token.
///
/// The reason for global orders to be sharded by token is that it will make it
/// more effective for landing transactions. Rather than requiring a write lock
/// for state that covers all markets, you just need to write lock state that
/// covers all orders involving a given token.
use std::{cmp::Ordering, mem::size_of};

use bytemuck::{Pod, Zeroable};
use hypertree::{
    get_helper, get_mut_helper, DataIndex, FreeList, Get, HyperTreeReadOperations,
    HyperTreeWriteOperations, RBNode, RedBlackTree, RedBlackTreeReadOnly, NIL,
};
use shank::ShankType;
use solana_program::{entrypoint::ProgramResult, pubkey::Pubkey};
use static_assertions::const_assert_eq;

use crate::{
    quantities::GlobalAtoms,
    require,
    validation::{get_global_address, get_global_vault_address, ManifestAccount},
};

use super::{
    DerefOrBorrow, DerefOrBorrowMut, DynamicAccount, RestingOrder, GLOBAL_BLOCK_SIZE,
    GLOBAL_DEPOSIT_SIZE, GLOBAL_FIXED_DISCRIMINANT, GLOBAL_FIXED_SIZE, GLOBAL_FREE_LIST_BLOCK_SIZE,
    GLOBAL_TRADER_SIZE, MAX_GLOBAL_SEATS,
};

#[repr(C)]
#[derive(Default, Copy, Clone, Zeroable, Pod, ShankType)]
pub struct GlobalFixed {
    /// Discriminant for identifying this type of account.
    pub discriminant: u64,

    /// Mint for this global
    mint: Pubkey,

    /// Vault address
    vault: Pubkey,

    /// Red-black tree root representing the global orders for the bank.
    global_traders_root_index: DataIndex,

    /// Red-black tree root representing the global deposits sorted by amount.
    global_deposits_root_index: DataIndex,
    /// Max, because the Hypertree provides access to max, but the sort key is
    /// reversed so this is the smallest balance.
    global_deposits_max_index: DataIndex,

    /// LinkedList representing all free blocks that could be used for ClaimedSeats or RestingOrders
    free_list_head_index: DataIndex,

    /// Number of bytes allocated so far.
    num_bytes_allocated: DataIndex,

    vault_bump: u8,

    /// Unused, but this byte wasnt being used anyways.
    global_bump: u8,

    num_seats_claimed: u16,
}
const_assert_eq!(
    size_of::<GlobalFixed>(),
    8  +  // discriminant
    32 +  // mint
    32 +  // vault
    4 +   // global_seats_root_index
    4 +   // global_amounts_root_index
    4 +   // global_amounts_max_index 
    4 +   // free_list_head_index
    4 +   // num_bytes_allocated
    1 +   // vault_bump
    1 +   // global_bump
    2 // num_seats_claimed
);
const_assert_eq!(size_of::<GlobalFixed>(), GLOBAL_FIXED_SIZE);
const_assert_eq!(size_of::<GlobalFixed>() % 8, 0);
impl Get for GlobalFixed {}

#[repr(C, packed)]
#[derive(Default, Copy, Clone, Pod, Zeroable)]
struct GlobalUnusedFreeListPadding {
    _padding: [u64; 7],
    _padding2: [u8; 4],
}
// 4 bytes are for the free list, rest is payload.
const_assert_eq!(
    size_of::<GlobalUnusedFreeListPadding>(),
    GLOBAL_FREE_LIST_BLOCK_SIZE
);
// Does not need to align to word boundaries because does not deserialize.

#[repr(C)]
#[derive(Default, Copy, Clone, Zeroable, Pod, ShankType)]
pub struct GlobalTrader {
    /// Trader who controls this global trader.
    trader: Pubkey,

    deposit_index: DataIndex,
    _padding: u32,
    _padding2: u64,
}
const_assert_eq!(size_of::<GlobalTrader>(), GLOBAL_TRADER_SIZE);
const_assert_eq!(size_of::<GlobalTrader>() % 8, 0);

impl Ord for GlobalTrader {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.trader).cmp(&(other.trader))
    }
}
impl PartialOrd for GlobalTrader {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for GlobalTrader {
    fn eq(&self, other: &Self) -> bool {
        (self.trader) == (other.trader)
    }
}
impl Eq for GlobalTrader {}
impl std::fmt::Display for GlobalTrader {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.trader)
    }
}

#[repr(C)]
#[derive(Default, Copy, Clone, Zeroable, Pod, ShankType)]
pub struct GlobalDeposit {
    /// Trader who controls this global trader.
    trader: Pubkey,

    /// Token balance in the global account for this trader. The tokens received
    /// in trades stay in the market.
    balance_atoms: GlobalAtoms,
    _padding: u64,
}
const_assert_eq!(size_of::<GlobalDeposit>(), GLOBAL_DEPOSIT_SIZE);
const_assert_eq!(size_of::<GlobalDeposit>() % 8, 0);

impl Ord for GlobalDeposit {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reversed order so that the max according to the tree is actually the min.
        (other.balance_atoms).cmp(&(self.balance_atoms))
    }
}
impl PartialOrd for GlobalDeposit {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for GlobalDeposit {
    fn eq(&self, other: &Self) -> bool {
        (self.trader) == (other.trader)
    }
}
impl Eq for GlobalDeposit {}
impl std::fmt::Display for GlobalDeposit {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.trader)
    }
}
impl GlobalFixed {
    pub fn new_empty(mint: &Pubkey) -> Self {
        let (vault, vault_bump) = get_global_vault_address(mint);
        let (_, global_bump) = get_global_address(mint);
        GlobalFixed {
            discriminant: GLOBAL_FIXED_DISCRIMINANT,
            mint: *mint,
            vault,
            global_traders_root_index: NIL,
            global_deposits_root_index: NIL,
            global_deposits_max_index: NIL,
            free_list_head_index: NIL,
            num_bytes_allocated: 0,
            vault_bump,
            global_bump,
            num_seats_claimed: 0,
        }
    }
    pub fn get_mint(&self) -> &Pubkey {
        &self.mint
    }
    pub fn get_vault(&self) -> &Pubkey {
        &self.vault
    }
    pub fn get_vault_bump(&self) -> u8 {
        self.vault_bump
    }
    pub fn needs_eviction(&self) -> bool {
        self.num_seats_claimed >= MAX_GLOBAL_SEATS
    }
}

impl ManifestAccount for GlobalFixed {
    fn verify_discriminant(&self) -> ProgramResult {
        // Check the discriminant to make sure it is a global account.
        require!(
            self.discriminant == GLOBAL_FIXED_DISCRIMINANT,
            solana_program::program_error::ProgramError::InvalidAccountData,
            "Invalid market discriminant actual: {} expected: {}",
            self.discriminant,
            GLOBAL_FIXED_DISCRIMINANT
        )?;
        Ok(())
    }
}

impl GlobalTrader {
    pub fn new_empty(trader: &Pubkey, deposit_index: DataIndex) -> Self {
        GlobalTrader {
            trader: *trader,
            deposit_index,
            _padding: 0,
            _padding2: 0,
        }
    }
}

impl GlobalDeposit {
    pub fn new_empty(trader: &Pubkey) -> Self {
        GlobalDeposit {
            trader: *trader,
            balance_atoms: GlobalAtoms::ZERO,
            _padding: 0,
        }
    }

    pub fn get_balance_atoms(&self) -> GlobalAtoms {
        self.balance_atoms
    }
}

pub type GlobalTraderTree<'a> = RedBlackTree<'a, GlobalTrader>;
pub type GlobalTraderTreeReadOnly<'a> = RedBlackTreeReadOnly<'a, GlobalTrader>;
pub type GlobalDepositTree<'a> = RedBlackTree<'a, GlobalDeposit>;
pub type GlobalDepositTreeReadOnly<'a> = RedBlackTreeReadOnly<'a, GlobalDeposit>;

/// Fully owned Global, used in clients that can copy.
pub type GlobalValue = DynamicAccount<GlobalFixed, Vec<u8>>;
/// Full global reference type.
pub type GlobalRef<'a> = DynamicAccount<&'a GlobalFixed, &'a [u8]>;
/// Full global reference type.
pub type GlobalRefMut<'a> = DynamicAccount<&'a mut GlobalFixed, &'a mut [u8]>;

impl<Fixed: DerefOrBorrow<GlobalFixed>, Dynamic: DerefOrBorrow<[u8]>>
    DynamicAccount<Fixed, Dynamic>
{
    fn borrow_global(&self) -> GlobalRef {
        GlobalRef {
            fixed: self.fixed.deref_or_borrow(),
            dynamic: self.dynamic.deref_or_borrow(),
        }
    }

    pub fn has_global_seat(&self, trader: &Pubkey) -> bool {
        let DynamicAccount { fixed, dynamic } = self.borrow_global();
        get_global_deposit(fixed, dynamic, trader).is_some()
    }

    pub fn get_balance_atoms(&self, trader: &Pubkey) -> GlobalAtoms {
        let DynamicAccount { fixed, dynamic } = self.borrow_global();
        // If the trader got evicted, then they wont be found.
        let global_balance_or: Option<&GlobalDeposit> = get_global_deposit(fixed, dynamic, trader);
        if let Some(global_deposit) = global_balance_or {
            global_deposit.balance_atoms
        } else {
            GlobalAtoms::ZERO
        }
    }

    pub fn verify_min_balance(&self, trader: &Pubkey) -> ProgramResult {
        let DynamicAccount { fixed, dynamic } = self.borrow_global();

        let existing_global_trader_opt: Option<&GlobalTrader> =
            get_global_trader(fixed, dynamic, trader);
        require!(
            existing_global_trader_opt.is_some(),
            crate::program::ManifestError::MissingGlobal,
            "Could not find global trader for {}",
            trader
        )?;
        let existing_global_trader: GlobalTrader = *existing_global_trader_opt.unwrap();
        let global_trader_tree: GlobalTraderTreeReadOnly = GlobalTraderTreeReadOnly::new(
            dynamic,
            fixed.global_traders_root_index,
            fixed.global_deposits_max_index,
        );
        let existing_trader_index: DataIndex =
            global_trader_tree.lookup_index(&existing_global_trader);
        let existing_global_trader: &GlobalTrader =
            get_helper::<RBNode<GlobalTrader>>(dynamic, existing_trader_index).get_value();
        let existing_deposit_index: DataIndex = existing_global_trader.deposit_index;

        require!(
            existing_deposit_index == fixed.global_deposits_max_index,
            crate::program::ManifestError::GlobalInsufficient,
            "Only can remove trader with lowest deposit"
        )?;

        Ok(())
    }
}

impl<Fixed: DerefOrBorrowMut<GlobalFixed>, Dynamic: DerefOrBorrowMut<[u8]>>
    DynamicAccount<Fixed, Dynamic>
{
    fn borrow_mut_global(&mut self) -> GlobalRefMut {
        GlobalRefMut {
            fixed: self.fixed.deref_or_borrow_mut(),
            dynamic: self.dynamic.deref_or_borrow_mut(),
        }
    }

    pub fn global_expand(&mut self) -> ProgramResult {
        let DynamicAccount { fixed, dynamic } = self.borrow_mut_global();

        require!(
            fixed.free_list_head_index == NIL,
            crate::program::ManifestError::InvalidFreeList,
            "Expected empty free list, but expand wasnt needed",
        )?;

        let mut free_list: FreeList<GlobalUnusedFreeListPadding> =
            FreeList::new(dynamic, fixed.free_list_head_index);

        // Expand twice since there are two trees.
        free_list.add(fixed.num_bytes_allocated);
        free_list.add(fixed.num_bytes_allocated + GLOBAL_BLOCK_SIZE as u32);
        fixed.num_bytes_allocated += 2 * GLOBAL_BLOCK_SIZE as u32;
        fixed.free_list_head_index = free_list.get_head();
        Ok(())
    }

    pub fn reduce(&mut self, trader: &Pubkey, num_atoms: GlobalAtoms) -> ProgramResult {
        let DynamicAccount { fixed, dynamic } = self.borrow_mut_global();
        let deposit_index: DataIndex = get_deposit_index(fixed, dynamic, trader)?;

        // Remove from tree first, then update balance, then reinsert.
        // Split into separate scopes to satisfy the borrow checker.
        {
            let mut deposit_tree: GlobalDepositTree = GlobalDepositTree::new(
                dynamic,
                fixed.global_deposits_root_index,
                fixed.global_deposits_max_index,
            );
            deposit_tree.remove_by_index(deposit_index);
            fixed.global_deposits_root_index = deposit_tree.get_root_index();
            fixed.global_deposits_max_index = deposit_tree.get_max_index();
        }

        let global_deposit: &mut GlobalDeposit =
            get_mut_helper::<RBNode<GlobalDeposit>>(dynamic, deposit_index).get_mut_value();
        global_deposit.balance_atoms = global_deposit.balance_atoms.checked_sub(num_atoms)?;
        let updated_deposit: GlobalDeposit = *global_deposit;

        {
            let mut deposit_tree: GlobalDepositTree = GlobalDepositTree::new(
                dynamic,
                fixed.global_deposits_root_index,
                fixed.global_deposits_max_index,
            );
            deposit_tree.insert(deposit_index, updated_deposit);
            fixed.global_deposits_root_index = deposit_tree.get_root_index();
            fixed.global_deposits_max_index = deposit_tree.get_max_index();
        }
        Ok(())
    }

    /// Add GlobalTrader to the tree of global traders
    pub fn add_trader(&mut self, trader: &Pubkey) -> ProgramResult {
        let DynamicAccount { fixed, dynamic } = self.borrow_mut_global();

        let free_address_trader: DataIndex = get_free_address_on_global_fixed(fixed, dynamic);
        let free_address_deposit: DataIndex = get_free_address_on_global_fixed(fixed, dynamic);
        let mut global_trader_tree: GlobalTraderTree =
            GlobalTraderTree::new(dynamic, fixed.global_traders_root_index, NIL);
        let global_trader: GlobalTrader = GlobalTrader::new_empty(trader, free_address_deposit);

        require!(
            global_trader_tree.lookup_index(&global_trader) == NIL,
            crate::program::ManifestError::AlreadyClaimedSeat,
            "Already claimed global trader seat",
        )?;

        global_trader_tree.insert(free_address_trader, global_trader);
        fixed.global_traders_root_index = global_trader_tree.get_root_index();
        require!(
            !fixed.needs_eviction(),
            crate::program::ManifestError::TooManyGlobalSeats,
            "There is a strict limit on number of seats available in a global, use evict",
        )?;

        fixed.num_seats_claimed += 1;

        let global_deposit: GlobalDeposit = GlobalDeposit::new_empty(trader);
        let mut global_deposit_tree: GlobalDepositTree = GlobalDepositTree::new(
            dynamic,
            fixed.global_deposits_root_index,
            fixed.global_deposits_max_index,
        );
        global_deposit_tree.insert(free_address_deposit, global_deposit);
        fixed.global_deposits_root_index = global_deposit_tree.get_root_index();
        fixed.global_deposits_max_index = global_deposit_tree.get_max_index();

        Ok(())
    }

    /// Evict from the global account and steal their seat
    pub fn evict_and_take_seat(
        &mut self,
        existing_trader: &Pubkey,
        new_trader: &Pubkey,
    ) -> ProgramResult {
        let DynamicAccount { fixed, dynamic } = self.borrow_mut_global();

        let existing_global_trader_opt: Option<&GlobalTrader> =
            get_global_trader(fixed, dynamic, existing_trader);
        require!(
            existing_global_trader_opt.is_some(),
            crate::program::ManifestError::MissingGlobal,
            "Could not find global trader for {}",
            existing_trader
        )?;
        let existing_global_trader: GlobalTrader = *existing_global_trader_opt.unwrap();

        let existing_global_deposit_opt: Option<&mut GlobalDeposit> =
            get_mut_global_deposit(fixed, dynamic, existing_trader);
        require!(
            existing_global_deposit_opt.is_some(),
            crate::program::ManifestError::MissingGlobal,
            "Could not find global deposit for {}",
            existing_trader
        )?;
        let existing_global_deposit: &mut GlobalDeposit = existing_global_deposit_opt.unwrap();

        let existing_global_atoms_deposited: GlobalAtoms = existing_global_deposit.balance_atoms;
        require!(
            existing_global_atoms_deposited == GlobalAtoms::ZERO,
            crate::program::ManifestError::GlobalInsufficient,
            "Error in emptying the existing global",
        )?;

        // Verification that the max index is the deposit index we are taking happens before withdraw.
        let global_trader_tree: GlobalTraderTree =
            GlobalTraderTree::new(dynamic, fixed.global_traders_root_index, NIL);
        let existing_trader_index: DataIndex =
            global_trader_tree.lookup_index(&existing_global_trader);
        let existing_global_trader: &GlobalTrader =
            get_helper::<RBNode<GlobalTrader>>(dynamic, existing_trader_index).get_value();
        let existing_deposit_index: DataIndex = existing_global_trader.deposit_index;

        // Update global trader
        {
            let mut global_trader_tree: GlobalTraderTree =
                GlobalTraderTree::new(dynamic, fixed.global_traders_root_index, NIL);
            require!(
                existing_deposit_index == fixed.global_deposits_max_index,
                crate::program::ManifestError::GlobalInsufficient,
                "Only can remove trader with lowest deposit"
            )?;
            let new_global_trader: GlobalTrader =
                GlobalTrader::new_empty(new_trader, fixed.global_deposits_max_index);

            global_trader_tree.remove_by_index(existing_trader_index);

            // Cannot claim an extra seat.
            require!(
                global_trader_tree.lookup_index(&new_global_trader) == NIL,
                crate::program::ManifestError::AlreadyClaimedSeat,
                "Already claimed global trader seat",
            )?;

            global_trader_tree.insert(existing_trader_index, new_global_trader);
            fixed.global_traders_root_index = global_trader_tree.get_root_index();
        }

        // Update global deposits
        {
            let new_global_deposit: GlobalDeposit = GlobalDeposit::new_empty(new_trader);
            let mut global_deposit_tree: GlobalDepositTree = GlobalDepositTree::new(
                dynamic,
                fixed.global_deposits_root_index,
                fixed.global_deposits_max_index,
            );
            global_deposit_tree.remove_by_index(existing_deposit_index);
            global_deposit_tree.insert(existing_deposit_index, new_global_deposit);
            fixed.global_deposits_max_index = global_deposit_tree.get_max_index();
            fixed.global_deposits_root_index = global_deposit_tree.get_root_index();
        }

        Ok(())
    }

    /// Adding to global only checks whether they have a seat. It was previously unnecessarily
    /// checking if the trader had enough balance to not immediately get their order cancellable.
    pub fn add_order(
        &mut self,
        _resting_order: &RestingOrder,
        global_trade_owner: &Pubkey,
    ) -> ProgramResult {
        let DynamicAccount { fixed, dynamic } = self.borrow_mut_global();

        require!(
            get_global_trader(fixed, dynamic, global_trade_owner,).is_some(),
            crate::program::ManifestError::GlobalInsufficient,
            "Trying to place global order but did not have global seat"
        )?;

        Ok(())
    }

    /// Deposit to global account.
    pub fn deposit_global(&mut self, trader: &Pubkey, num_atoms: GlobalAtoms) -> ProgramResult {
        let DynamicAccount { fixed, dynamic } = self.borrow_mut_global();
        let deposit_index: DataIndex = get_deposit_index(fixed, dynamic, trader)?;

        {
            let mut deposit_tree: GlobalDepositTree = GlobalDepositTree::new(
                dynamic,
                fixed.global_deposits_root_index,
                fixed.global_deposits_max_index,
            );
            deposit_tree.remove_by_index(deposit_index);
            fixed.global_deposits_root_index = deposit_tree.get_root_index();
            fixed.global_deposits_max_index = deposit_tree.get_max_index();
        }

        let global_deposit: &mut GlobalDeposit =
            get_mut_helper::<RBNode<GlobalDeposit>>(dynamic, deposit_index).get_mut_value();
        global_deposit.balance_atoms = global_deposit.balance_atoms.checked_add(num_atoms)?;
        let updated_deposit: GlobalDeposit = *global_deposit;

        {
            let mut deposit_tree: GlobalDepositTree = GlobalDepositTree::new(
                dynamic,
                fixed.global_deposits_root_index,
                fixed.global_deposits_max_index,
            );
            deposit_tree.insert(deposit_index, updated_deposit);
            fixed.global_deposits_root_index = deposit_tree.get_root_index();
            fixed.global_deposits_max_index = deposit_tree.get_max_index();
        }

        Ok(())
    }

    /// Withdraw from global account.
    pub fn withdraw_global(&mut self, trader: &Pubkey, num_atoms: GlobalAtoms) -> ProgramResult {
        let DynamicAccount { fixed, dynamic } = self.borrow_mut_global();
        let deposit_index: DataIndex = get_deposit_index(fixed, dynamic, trader)?;

        {
            let mut deposit_tree: GlobalDepositTree = GlobalDepositTree::new(
                dynamic,
                fixed.global_deposits_root_index,
                fixed.global_deposits_max_index,
            );
            deposit_tree.remove_by_index(deposit_index);
            fixed.global_deposits_root_index = deposit_tree.get_root_index();
            fixed.global_deposits_max_index = deposit_tree.get_max_index();
        }

        let global_deposit: &mut GlobalDeposit =
            get_mut_helper::<RBNode<GlobalDeposit>>(dynamic, deposit_index).get_mut_value();
        global_deposit.balance_atoms = global_deposit.balance_atoms.checked_sub(num_atoms)?;
        let updated_deposit: GlobalDeposit = *global_deposit;

        {
            let mut deposit_tree: GlobalDepositTree = GlobalDepositTree::new(
                dynamic,
                fixed.global_deposits_root_index,
                fixed.global_deposits_max_index,
            );
            deposit_tree.insert(deposit_index, updated_deposit);
            fixed.global_deposits_root_index = deposit_tree.get_root_index();
            fixed.global_deposits_max_index = deposit_tree.get_max_index();
        }

        Ok(())
    }
}

fn get_deposit_index(
    fixed: &GlobalFixed,
    dynamic: &[u8],
    trader: &Pubkey,
) -> Result<DataIndex, solana_program::program_error::ProgramError> {
    let global_trader_tree: GlobalTraderTreeReadOnly =
        GlobalTraderTreeReadOnly::new(dynamic, fixed.global_traders_root_index, NIL);
    let global_trader_index: DataIndex =
        global_trader_tree.lookup_index(&GlobalTrader::new_empty(trader, NIL));
    require!(
        global_trader_index != NIL,
        crate::program::ManifestError::MissingGlobal,
        "Could not find global trader for {}",
        trader
    )?;
    let global_trader: &GlobalTrader =
        get_helper::<RBNode<GlobalTrader>>(dynamic, global_trader_index).get_value();
    Ok(global_trader.deposit_index)
}

fn get_free_address_on_global_fixed(fixed: &mut GlobalFixed, dynamic: &mut [u8]) -> DataIndex {
    let mut free_list: FreeList<GlobalUnusedFreeListPadding> =
        FreeList::new(dynamic, fixed.free_list_head_index);
    let free_address: DataIndex = free_list.remove();
    fixed.free_list_head_index = free_list.get_head();
    free_address
}

fn get_global_trader<'a>(
    fixed: &'a GlobalFixed,
    dynamic: &'a [u8],
    trader: &'a Pubkey,
) -> Option<&'a GlobalTrader> {
    let global_trader_tree: GlobalTraderTreeReadOnly =
        GlobalTraderTreeReadOnly::new(dynamic, fixed.global_traders_root_index, NIL);
    let global_trader_index: DataIndex =
        global_trader_tree.lookup_index(&GlobalTrader::new_empty(trader, NIL));
    if global_trader_index == NIL {
        return None;
    }
    let global_trader: &GlobalTrader =
        get_helper::<RBNode<GlobalTrader>>(dynamic, global_trader_index).get_value();
    Some(global_trader)
}

fn get_mut_global_deposit<'a>(
    fixed: &'a mut GlobalFixed,
    dynamic: &'a mut [u8],
    trader: &'a Pubkey,
) -> Option<&'a mut GlobalDeposit> {
    let global_trader_tree: GlobalTraderTree =
        GlobalTraderTree::new(dynamic, fixed.global_traders_root_index, NIL);
    let global_trader_index: DataIndex =
        global_trader_tree.lookup_index(&GlobalTrader::new_empty(trader, NIL));
    if global_trader_index == NIL {
        return None;
    }
    let global_trader: &GlobalTrader =
        get_helper::<RBNode<GlobalTrader>>(dynamic, global_trader_index).get_value();
    let global_deposit_index: DataIndex = global_trader.deposit_index;
    Some(get_mut_helper::<RBNode<GlobalDeposit>>(dynamic, global_deposit_index).get_mut_value())
}

fn get_global_deposit<'a>(
    fixed: &'a GlobalFixed,
    dynamic: &'a [u8],
    trader: &'a Pubkey,
) -> Option<&'a GlobalDeposit> {
    let global_trader_tree: GlobalTraderTreeReadOnly =
        GlobalTraderTreeReadOnly::new(dynamic, fixed.global_traders_root_index, NIL);
    let global_trader_index: DataIndex =
        global_trader_tree.lookup_index(&GlobalTrader::new_empty(trader, NIL));
    if global_trader_index == NIL {
        return None;
    }
    let global_trader: &GlobalTrader =
        get_helper::<RBNode<GlobalTrader>>(dynamic, global_trader_index).get_value();
    let global_deposit_index: DataIndex = global_trader.deposit_index;
    Some(get_helper::<RBNode<GlobalDeposit>>(dynamic, global_deposit_index).get_value())
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::quantities::WrapperU64;

    #[test]
    fn test_display_trader() {
        format!("{}", GlobalTrader::default());
    }

    #[test]
    fn test_cmp_trader() {
        // Just use token program ids since those have known sort order.
        let global_trader1: GlobalTrader = GlobalTrader::new_empty(&spl_token::id(), NIL);
        let global_trader2: GlobalTrader = GlobalTrader::new_empty(&spl_token_2022::id(), NIL);
        assert!(global_trader1 < global_trader2);
        assert!(global_trader1 != global_trader2);
    }

    #[test]
    fn test_display_deposit() {
        format!("{}", GlobalDeposit::default());
    }

    #[test]
    fn test_cmp_deposit() {
        let global_deposit1: GlobalDeposit = GlobalDeposit::new_empty(&Pubkey::new_unique());
        let mut global_deposit2: GlobalDeposit = GlobalDeposit::new_empty(&Pubkey::new_unique());
        global_deposit2.balance_atoms = GlobalAtoms::new(1);
        // Reversed order than expected because Hypertrees give max pointer, but we want a min balance.
        assert!(global_deposit1 > global_deposit2);
        assert!(global_deposit1 != global_deposit2);
    }
}
