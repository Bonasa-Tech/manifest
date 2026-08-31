use std::{
    cell::{Ref, RefMut},
    mem::size_of,
};

use hypertree::{
    get_mut_helper, DataIndex, FreeList, HyperTreeReadOperations, HyperTreeWriteOperations, RBNode,
    NIL,
};
use manifest::{
    program::{claim_seat_instruction, expand_market_instruction, get_dynamic_account, invoke},
    state::{MarketFixed, MarketRef},
    validation::ManifestAccountInfo,
};

use crate::{
    loader::{check_signer, WrapperStateAccountInfo},
    market_info::MarketInfo,
    wrapper_state::ManifestWrapperStateFixed,
};
use manifest::validation::{Program, Signer};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
    system_program,
};

use super::shared::{
    expand_wrapper_if_needed, MarketInfosTree, UnusedWrapperFreeListPadding, ORDERS_LAYOUT_LIST,
};

pub(crate) fn process_claim_seat(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    _data: &[u8],
) -> ProgramResult {
    let account_iter: &mut std::slice::Iter<AccountInfo> = &mut accounts.iter();
    let manifest_program: Program =
        Program::new(next_account_info(account_iter)?, &manifest::id())?;
    let owner: Signer = Signer::new(next_account_info(account_iter)?)?;
    let market: ManifestAccountInfo<MarketFixed> =
        ManifestAccountInfo::<MarketFixed>::new(next_account_info(account_iter)?)?;
    let system_program: Program =
        Program::new(next_account_info(account_iter)?, &system_program::id())?;
    let wrapper_state: WrapperStateAccountInfo =
        WrapperStateAccountInfo::new(next_account_info(account_iter)?)?;
    check_signer(&wrapper_state, owner.key);

    // Lookup if the trader already has a seat. This will prevent failing when
    // they already had a seat on a different wrapper.
    let trader_index: DataIndex = {
        let trader_index: DataIndex = {
            let market_data: &Ref<&mut [u8]> = &market.try_borrow_data()?;
            let dynamic_account: MarketRef = get_dynamic_account(market_data);
            dynamic_account.get_trader_index(owner.key)
        };

        if trader_index != NIL {
            // if core seat was already initialized, nothing to do here
            trader_index
        } else {
            // Call the Expand CPI.
            invoke(
                &expand_market_instruction(market.key, owner.key),
                &[
                    manifest_program.info.clone(),
                    owner.info.clone(),
                    market.info.clone(),
                    system_program.info.clone(),
                ],
            )?;

            // Call the ClaimSeat CPI.
            invoke(
                &claim_seat_instruction(market.key, owner.key),
                &[
                    manifest_program.info.clone(),
                    owner.info.clone(),
                    market.info.clone(),
                    system_program.info.clone(),
                ],
            )?;

            // fetch newly assigned trader index after claiming core seat
            let market_data: &Ref<&mut [u8]> = &mut market.try_borrow_data()?;
            let dynamic_account: MarketRef = get_dynamic_account(market_data);
            dynamic_account.get_trader_index(owner.key)
        }
    };

    // Insert the seat into the wrapper state.

    // Make sure the wrapper is big enough.
    expand_wrapper_if_needed(&wrapper_state, &owner, &system_program)?;

    // Load the market_infos tree and insert a new one.
    let wrapper_state_info: &AccountInfo = wrapper_state.info;
    let mut wrapper_data: RefMut<&mut [u8]> = wrapper_state_info.try_borrow_mut_data().unwrap();
    let (fixed_data, wrapper_dynamic_data) =
        wrapper_data.split_at_mut(size_of::<ManifestWrapperStateFixed>());
    let wrapper_fixed: &mut ManifestWrapperStateFixed = get_mut_helper(fixed_data, 0);

    // Get the free block and setup the new MarketInfo there.
    let market_info: MarketInfo = MarketInfo::new_empty(*market.key, trader_index);

    // Put that market_info at the free list head.
    let mut free_list: FreeList<UnusedWrapperFreeListPadding> =
        FreeList::new(wrapper_dynamic_data, wrapper_fixed.free_list_head_index);
    let free_address: DataIndex = free_list.remove();
    wrapper_fixed.free_list_head_index = free_list.get_head();

    // Insert into the MarketInfosTree.
    let mut market_infos_tree: MarketInfosTree = MarketInfosTree::new(
        wrapper_dynamic_data,
        wrapper_fixed.market_infos_root_index,
        NIL,
    );
    market_infos_tree.insert(free_address, market_info);
    wrapper_fixed.market_infos_root_index = market_infos_tree.get_root_index();
    // New market infos keep their open orders in the list layout.
    get_mut_helper::<RBNode<MarketInfo>>(wrapper_dynamic_data, free_address)
        .set_payload_type(ORDERS_LAYOUT_LIST);

    Ok(())
}
