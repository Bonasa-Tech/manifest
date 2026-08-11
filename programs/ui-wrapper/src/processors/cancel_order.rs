use std::cell::{Ref, RefMut};

use borsh::{BorshDeserialize, BorshSerialize};
use hypertree::{
    get_helper, get_mut_helper, trace, DataIndex, FreeList, HyperTreeReadOperations,
    HyperTreeValueIteratorTrait, HyperTreeWriteOperations, RBNode, NIL,
};
use manifest::{
    program::{
        batch_update::CancelOrderParams, batch_update_instruction, get_dynamic_account,
        get_mut_dynamic_account, invoke,
    },
    state::{claimed_seat::ClaimedSeat, DynamicAccount, MarketFixed},
    validation::{ManifestAccountInfo, Program, Signer},
};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    program_error::ProgramError,
    pubkey::Pubkey,
    system_program,
    sysvar::Sysvar,
};

use crate::{
    market_info::MarketInfo, open_order::WrapperOpenOrder,
    processors::shared::OpenOrdersTreeReadOnly, wrapper_user::ManifestWrapperUserFixed,
};

use super::shared::{
    check_signer, get_market_info_index_for_market, OpenOrdersTree, UnusedWrapperFreeListPadding,
    WrapperStateAccountInfo,
};

#[derive(BorshDeserialize, BorshSerialize, Clone)]
pub struct WrapperCancelOrderParams {
    client_order_id: u64,
}
impl WrapperCancelOrderParams {
    pub fn new(client_order_id: u64) -> Self {
        WrapperCancelOrderParams { client_order_id }
    }
}

pub(crate) fn process_cancel_order(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let account_iter: &mut std::slice::Iter<AccountInfo> = &mut accounts.iter();
    let wrapper_state: WrapperStateAccountInfo =
        WrapperStateAccountInfo::new(next_account_info(account_iter)?)?;
    let owner: Signer = Signer::new(next_account_info(account_iter)?)?;
    let trader_token_account: &AccountInfo = next_account_info(account_iter)?;
    let market: ManifestAccountInfo<MarketFixed> =
        ManifestAccountInfo::<MarketFixed>::new(next_account_info(account_iter)?)?;
    let vault: &AccountInfo = next_account_info(account_iter)?;
    let mint: &AccountInfo = next_account_info(account_iter)?;
    let system_program: Program =
        Program::new(next_account_info(account_iter)?, &system_program::id())?;
    let token_program: &AccountInfo = next_account_info(account_iter)?;
    let manifest_program: Program =
        Program::new(next_account_info(account_iter)?, &manifest::id())?;

    check_signer(&wrapper_state, owner.key);
    let market_info_index: DataIndex = get_market_info_index_for_market(&wrapper_state, market.key);

    let cancel = WrapperCancelOrderParams::try_from_slice(data)?;

    // prepare cancel
    let wrapper_data: Ref<&mut [u8]> = wrapper_state.info.try_borrow_data()?;
    let wrapper: DynamicAccount<&ManifestWrapperUserFixed, &[u8]> =
        get_dynamic_account(&wrapper_data);

    let market_info: MarketInfo =
        *get_helper::<RBNode<MarketInfo>>(wrapper.dynamic, market_info_index).get_value();
    let trader_index: DataIndex = market_info.trader_index;
    let orders_root_index = market_info.orders_root_index;

    let open_orders_tree: OpenOrdersTreeReadOnly =
        OpenOrdersTreeReadOnly::new(wrapper.dynamic, orders_root_index, NIL);

    // find order with same client order id
    let (wrapper_index, open_order): (DataIndex, &WrapperOpenOrder) = open_orders_tree
        .iter::<WrapperOpenOrder>()
        .find(|(_, o)| o.get_client_order_id() == cancel.client_order_id)
        .ok_or(ProgramError::InvalidArgument)?;

    let core_cancel = CancelOrderParams::new_with_hint(
        open_order.get_order_sequence_number(),
        Some(open_order.get_market_data_index()),
    );
    trace!("cancel index:{wrapper_index} order:{open_order:?} cpi:{core_cancel:?}");
    drop(wrapper_data);

    invoke(
        &batch_update_instruction(
            market.key,
            owner.key,
            Some(trader_index),
            vec![core_cancel],
            vec![],
            None,
            None,
            None,
            None,
        ),
        &[
            owner.info.clone(),
            system_program.info.clone(),
            manifest_program.info.clone(),
            owner.info.clone(),
            market.info.clone(),
            trader_token_account.clone(),
            vault.clone(),
            token_program.clone(),
            mint.clone(),
        ],
    )?;

    // Process the order result
    let mut wrapper_data: RefMut<&mut [u8]> = wrapper_state.info.try_borrow_mut_data().unwrap();
    let wrapper: DynamicAccount<&mut ManifestWrapperUserFixed, &mut [u8]> =
        get_mut_dynamic_account(&mut wrapper_data);

    // fetch current root first to not borrow wrapper.dynamic twice
    let orders_root_index = {
        let market_info: &mut MarketInfo =
            get_mut_helper::<RBNode<MarketInfo>>(wrapper.dynamic, market_info_index)
                .get_mut_value();
        market_info.orders_root_index
    };

    // remove node from order tree
    let orders_root_index = {
        let mut open_orders_tree: OpenOrdersTree =
            OpenOrdersTree::new(wrapper.dynamic, orders_root_index, NIL);
        open_orders_tree.remove_by_index(wrapper_index);
        open_orders_tree.get_root_index()
    };

    // save new root
    let market_info: &mut MarketInfo =
        get_mut_helper::<RBNode<MarketInfo>>(wrapper.dynamic, market_info_index).get_mut_value();
    market_info.orders_root_index = orders_root_index;

    // CancelOrder intentionally synchronizes only balances and the cumulative
    // volume baseline. Wrapper fees are assessed by the settlement path, not
    // by cancellation, and advancing the baseline here is the established
    // accounting contract rather than an omitted fee accrual.
    let market_data = market.info.data.borrow();
    let market_ref = get_dynamic_account::<MarketFixed>(&market_data);
    let claimed_seat: &ClaimedSeat =
        get_helper::<RBNode<ClaimedSeat>>(market_ref.dynamic, market_info.trader_index).get_value();
    market_info.base_balance = claimed_seat.base_withdrawable_balance;
    market_info.quote_balance = claimed_seat.quote_withdrawable_balance;
    market_info.quote_volume = claimed_seat.quote_volume;
    market_info.last_updated_slot = Clock::get().unwrap().slot as u32;

    // add node to freelist
    let mut free_list: FreeList<UnusedWrapperFreeListPadding> =
        FreeList::new(wrapper.dynamic, wrapper.fixed.free_list_head_index);
    free_list.add(wrapper_index);
    wrapper.fixed.free_list_head_index = free_list.get_head();

    Ok(())
}
