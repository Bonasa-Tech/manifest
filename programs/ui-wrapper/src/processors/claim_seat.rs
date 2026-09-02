use std::{
    mem::size_of,
};

use hypertree::{
    get_helper, get_mut_helper, DataIndex, FreeList, HyperTreeReadOperations,
    HyperTreeWriteOperations, RBNode, NIL,
};
use manifest::{
    program::{claim_seat_instruction, expand_market_instruction, get_dynamic_account, invoke},
    state::{claimed_seat::ClaimedSeat, MarketFixed, MarketRef},
    validation::ManifestAccountInfo,
};

use crate::{market_info::MarketInfo, wrapper_user::ManifestWrapperUserFixed};
use pinocchio::account_info::{Ref, RefMut};
use manifest::validation::next_account_info;
use manifest::validation::AccountInfoExt;
use pinocchio::account_info::AccountInfo;
use pinocchio::ProgramResult;
use manifest::validation::{Program, Signer};
use solana_program::{
    pubkey::Pubkey,
    system_program,
};

use super::shared::{
    check_signer, expand_wrapper_if_needed, MarketInfosTree, UnusedWrapperFreeListPadding,
    WrapperStateAccountInfo,
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
    let payer: Signer = Signer::new(next_account_info(account_iter)?)?;
    let wrapper_state: WrapperStateAccountInfo =
        WrapperStateAccountInfo::new(next_account_info(account_iter)?)?;
    check_signer(&wrapper_state, owner.pubkey());

    let trader_index: DataIndex = {
        let trader_index: DataIndex = {
            let market_data: &Ref<[u8]> = &market.try_borrow_data()?;
            let dynamic_account: MarketRef = get_dynamic_account(market_data);
            dynamic_account.get_trader_index(owner.pubkey())
        };

        if trader_index != NIL {
            // if core seat was already initialized, nothing to do here
            trader_index
        } else {
            // Need to intialize a new seat on core
            // Call expand so claim seat has enough free space
            // and owner doesn't get charged rent
            invoke(
                &expand_market_instruction(market.pubkey(), payer.pubkey()),
                &[
                    manifest_program.info,
                    payer.info,
                    market.info,
                    system_program.info,
                ],
            )?;

            // Call the ClaimSeat CPI
            invoke(
                &claim_seat_instruction(market.pubkey(), owner.pubkey()),
                &[
                    manifest_program.info,
                    owner.info,
                    market.info,
                    system_program.info,
                ],
            )?;

            // fetch newly assigned trader index after claiming core seat
            let market_data: &Ref<[u8]> = &mut market.try_borrow_data()?;
            let dynamic_account: MarketRef = get_dynamic_account(market_data);
            dynamic_account.get_trader_index(owner.pubkey())
        }
    };

    // Insert the seat into the wrapper state
    expand_wrapper_if_needed(&wrapper_state, &payer, &system_program)?;

    // Load the market_infos tree and insert a new one
    let wrapper_state_info: &AccountInfo = wrapper_state.info;
    let mut wrapper_data: RefMut<[u8]> = wrapper_state_info.try_borrow_mut_data()?;
    let (fixed_data, wrapper_dynamic_data) =
        wrapper_data.split_at_mut(size_of::<ManifestWrapperUserFixed>());
    let wrapper_fixed: &mut ManifestWrapperUserFixed = get_mut_helper(fixed_data, 0);
    let mut market_info: MarketInfo = MarketInfo::new_empty(*market.pubkey(), trader_index);
    market_info.quote_volume = {
        // sync volume from core seat to prevent double billing if seat
        // existed before wrapper invocation
        let market_data: &Ref<[u8]> = &market.try_borrow_data()?;
        let dynamic_account: MarketRef = get_dynamic_account(market_data);
        let claimed_seat: &ClaimedSeat =
            get_helper::<RBNode<ClaimedSeat>>(dynamic_account.dynamic, trader_index).get_value();
        claimed_seat.quote_volume
    };

    // Put that market_info at the free list head
    let mut free_list: FreeList<UnusedWrapperFreeListPadding> =
        FreeList::new(wrapper_dynamic_data, wrapper_fixed.free_list_head_index);
    let free_address: DataIndex = free_list.remove();
    wrapper_fixed.free_list_head_index = free_list.get_head();

    // Insert into the MarketInfosTree
    let mut market_infos_tree: MarketInfosTree = MarketInfosTree::new(
        wrapper_dynamic_data,
        wrapper_fixed.market_infos_root_index,
        NIL,
    );
    market_infos_tree.insert(free_address, market_info);
    wrapper_fixed.market_infos_root_index = market_infos_tree.get_root_index();

    Ok(())
}
