use std::{
    cell::{Ref, RefMut},
    mem::size_of,
};

use borsh::{BorshDeserialize, BorshSerialize};
use hypertree::{
    get_helper, get_mut_helper, trace, DataIndex, FreeList, HyperTreeReadOperations,
    HyperTreeValueIteratorTrait, HyperTreeWriteOperations, RBNode, NIL,
};
use manifest::{
    program::{
        batch_update::{BatchUpdateReturn, CancelOrderParams, PlaceOrderParams},
        batch_update_instruction, deposit_instruction, expand_market_instruction,
        get_dynamic_account, get_mut_dynamic_account,
    },
    quantities::{BaseAtoms, QuoteAtoms, QuoteAtomsPerBaseAtom, WrapperU64},
    state::{DynamicAccount, MarketFixed, MarketRef, OrderType, NO_EXPIRATION_LAST_VALID_SLOT},
    validation::{ManifestAccountInfo, Program, Signer},
};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    program::{get_return_data, invoke},
    pubkey::Pubkey,
    system_program,
};

use crate::{
    market_info::MarketInfo, open_order::WrapperOpenOrder,
    processors::shared::OpenOrdersTreeReadOnly, wrapper_user::ManifestWrapperUserFixed,
};

use super::shared::{
    check_signer, expand_wrapper_if_needed, get_market_info_index_for_market, sync_fast,
    OpenOrdersTree, UnusedWrapperFreeListPadding, WrapperStateAccountInfo,
};

#[derive(BorshDeserialize, BorshSerialize, Clone)]
pub struct WrapperEditOrderParams {
    client_order_id: u64,
    expected_base_atoms: u64,
    adjusted_base_atoms: u64,
    price_mantissa: u32,
    price_exponent: i8,
    is_bid: bool,
    last_valid_slot: u32,
    order_type: OrderType,
}
impl WrapperEditOrderParams {
    pub fn new(
        client_order_id: u64,
        expected_base_atoms: u64,
        adjusted_base_atoms: u64,
        price_mantissa: u32,
        price_exponent: i8,
        is_bid: bool,
        last_valid_slot: u32,
        order_type: OrderType,
    ) -> Self {
        WrapperEditOrderParams {
            client_order_id,
            expected_base_atoms,
            adjusted_base_atoms,
            price_mantissa,
            price_exponent,
            is_bid,
            last_valid_slot,
            order_type,
        }
    }
}

pub(crate) fn process_edit_order(
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
    let payer: Signer = Signer::new(next_account_info(account_iter)?)?;

    let base_mint: &AccountInfo = next_account_info(account_iter)?;
    let base_global: &AccountInfo = next_account_info(account_iter)?;
    let base_global_vault: &AccountInfo = next_account_info(account_iter)?;
    let base_market_vault: &AccountInfo = next_account_info(account_iter)?;
    let base_token_program: &AccountInfo = next_account_info(account_iter)?;
    let quote_mint: &AccountInfo = next_account_info(account_iter)?;
    let quote_global: &AccountInfo = next_account_info(account_iter)?;
    let quote_global_vault: &AccountInfo = next_account_info(account_iter)?;
    let quote_market_vault: &AccountInfo = next_account_info(account_iter)?;
    let quote_token_program: &AccountInfo = next_account_info(account_iter)?;

    if spl_token_2022::id() == *token_program.key {
        unimplemented!("token2022 not yet supported")
    }

    check_signer(&wrapper_state, owner.key);
    let market_info_index: DataIndex = get_market_info_index_for_market(&wrapper_state, market.key);

    let (base_mint_key, quote_mint_key) = {
        let market_data: &Ref<&mut [u8]> = &market.try_borrow_data()?;
        let dynamic_account: MarketRef = get_dynamic_account(market_data);
        let base_mint_key: Pubkey = *dynamic_account.get_base_mint();
        let quote_mint_key: Pubkey = *dynamic_account.get_quote_mint();
        (base_mint_key, quote_mint_key)
    };

    // Do an initial sync to get all existing orders and balances fresh. This is
    // needed for modifying user orders in response to in-flight fills since the
    // tx was signed.
    sync_fast(&wrapper_state, &market, market_info_index)?;

    let edit_params = WrapperEditOrderParams::try_from_slice(data)?;

    let wrapper_data: Ref<&mut [u8]> = wrapper_state.info.try_borrow_data()?;
    let (_fixed_data, wrapper_dynamic_data) =
        wrapper_data.split_at(size_of::<ManifestWrapperUserFixed>());

    let market_info: MarketInfo =
        *get_helper::<RBNode<MarketInfo>>(wrapper_dynamic_data, market_info_index).get_value();
    let trader_index: DataIndex = market_info.trader_index;

    drop(wrapper_data);

    // find order with same client order id
    let order_to_edit: Option<WrapperOpenOrder> = {
        let mut wrapper_data: RefMut<&mut [u8]> = wrapper_state.info.try_borrow_mut_data()?;
        let (_fixed_data, wrapper_dynamic_data) =
            wrapper_data.split_at_mut(size_of::<ManifestWrapperUserFixed>());

        let orders_root_index: DataIndex = {
            let market_info: &mut MarketInfo =
                get_mut_helper::<RBNode<MarketInfo>>(wrapper_dynamic_data, market_info_index)
                    .get_mut_value();
            market_info.orders_root_index
        };

        if orders_root_index != NIL {
            let orders_tree: OpenOrdersTreeReadOnly =
                OpenOrdersTreeReadOnly::new(wrapper_dynamic_data, orders_root_index, NIL);

            orders_tree
                .iter::<WrapperOpenOrder>()
                .find(|(_, o)| o.get_client_order_id() == edit_params.client_order_id)
                .map(|(_, o)| o.clone())
        } else {
            None
        }
    };

    let to_place_base_atoms = match order_to_edit {
        // found order to edit on wrapper
        Some(order) => {
            // calculate partial fill while tx was inflight
            let recently_filled =
                BaseAtoms::new(edit_params.expected_base_atoms) - order.get_num_base_atoms();

            // cancel existing before placing an updated orders
            let core_cancel = CancelOrderParams::new_with_hint(
                order.get_order_sequence_number(),
                Some(order.get_market_data_index()),
            );
            trace!("cancel order:{order:?} cpi:{core_cancel:?}");

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

            BaseAtoms::new(edit_params.adjusted_base_atoms) - recently_filled
        }
        // if the order was fully filled in flight, sync would remove it from the wrapper
        // and the iterator would not return it anymore. No cancel is needed in this case.
        None => BaseAtoms::new(edit_params.adjusted_base_atoms - edit_params.expected_base_atoms),
    };

    // sync again after cancellation and read out unlocked amounts
    sync_fast(&wrapper_state, &market, market_info_index)?;

    let wrapper_data: Ref<&mut [u8]> = wrapper_state.info.try_borrow_data()?;
    let (_fixed_data, wrapper_dynamic_data) =
        wrapper_data.split_at(size_of::<ManifestWrapperUserFixed>());

    let market_info: MarketInfo =
        *get_helper::<RBNode<MarketInfo>>(wrapper_dynamic_data, market_info_index).get_value();
    let remaining_base_atoms: BaseAtoms = market_info.base_balance;
    let remaining_quote_atoms: QuoteAtoms = market_info.quote_balance;
    drop(wrapper_data);

    let price = QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(
        edit_params.price_mantissa,
        edit_params.price_exponent,
    )?;

    let deposit_amount_atoms: u64 = if edit_params.is_bid {
        let required_quote_atoms = to_place_base_atoms.checked_mul(price, true)?;
        required_quote_atoms
            .saturating_sub(remaining_quote_atoms)
            .as_u64()
    } else {
        to_place_base_atoms
            .saturating_sub(remaining_base_atoms)
            .as_u64()
    };

    trace!("deposit amount:{deposit_amount_atoms} mint:{:?}", mint.key);
    if deposit_amount_atoms > 0 {
        invoke(
            &deposit_instruction(
                market.key,
                owner.key,
                mint.key,
                deposit_amount_atoms,
                trader_token_account.key,
                *token_program.key,
                Some(trader_index),
            ),
            &[
                manifest_program.info.clone(),
                owner.info.clone(),
                market.info.clone(),
                trader_token_account.clone(),
                vault.clone(),
                token_program.clone(),
                mint.clone(),
            ],
        )?;
    }

    // Call expand so claim seat has enough free space
    // and owner doesn't get charged rent
    // TODO: could check if needed before
    invoke(
        &expand_market_instruction(market.key, payer.key),
        &[
            manifest_program.info.clone(),
            payer.info.clone(),
            market.info.clone(),
            system_program.info.clone(),
        ],
    )?;

    let core_place: PlaceOrderParams = PlaceOrderParams::new(
        to_place_base_atoms.as_u64(),
        edit_params.price_mantissa,
        edit_params.price_exponent,
        edit_params.is_bid,
        edit_params.order_type,
        NO_EXPIRATION_LAST_VALID_SLOT,
    );

    trace!("place {core_place:?}");

    invoke(
        &batch_update_instruction(
            market.key,
            owner.key,
            Some(trader_index),
            vec![],
            vec![core_place],
            Some(base_mint_key),
            None,
            Some(quote_mint_key),
            None,
        ),
        &[
            system_program.info.clone(),
            manifest_program.info.clone(),
            owner.info.clone(),
            market.info.clone(),
            trader_token_account.clone(),
            vault.clone(),
            token_program.clone(),
            mint.clone(),
            base_mint.clone(),
            base_global.clone(),
            base_global_vault.clone(),
            base_market_vault.clone(),
            base_token_program.clone(),
            quote_mint.clone(),
            quote_global.clone(),
            quote_global_vault.clone(),
            quote_market_vault.clone(),
            quote_token_program.clone(),
        ],
    )?;

    // Process the order result

    let cpi_return_data: Option<(Pubkey, Vec<u8>)> = get_return_data();
    let BatchUpdateReturn {
        orders: batch_update_orders,
    } = BatchUpdateReturn::try_from_slice(&cpi_return_data.unwrap().1[..])?;

    trace!("cpi return orders:{batch_update_orders:?}");

    let (order_sequence_number, order_index) = batch_update_orders[0];
    // Order index is NIL when it did not rest. In that case, do not need to store in wrapper.
    if order_index != NIL {
        expand_wrapper_if_needed(&wrapper_state, &payer, &system_program)?;

        let mut wrapper_data: RefMut<&mut [u8]> = wrapper_state.info.try_borrow_mut_data().unwrap();
        let wrapper: DynamicAccount<&mut ManifestWrapperUserFixed, &mut [u8]> =
            get_mut_dynamic_account(&mut wrapper_data);

        let orders_root_index: DataIndex = {
            let market_info: &mut MarketInfo =
                get_mut_helper::<RBNode<MarketInfo>>(wrapper.dynamic, market_info_index)
                    .get_mut_value();
            market_info.orders_root_index
        };

        let wrapper_new_order_index: DataIndex = {
            let mut free_list: FreeList<UnusedWrapperFreeListPadding> =
                FreeList::new(wrapper.dynamic, wrapper.fixed.free_list_head_index);
            let new_index: DataIndex = free_list.remove();
            wrapper.fixed.free_list_head_index = free_list.get_head();
            new_index
        };

        let wrapper_order: WrapperOpenOrder = WrapperOpenOrder::new(
            edit_params.client_order_id,
            order_sequence_number,
            price,
            // Base atoms can be wrong, will be fixed in the next sync.
            0,
            edit_params.last_valid_slot,
            order_index,
            edit_params.is_bid,
            edit_params.order_type,
        );

        let mut open_orders_tree: OpenOrdersTree =
            OpenOrdersTree::new(wrapper.dynamic, orders_root_index, NIL);
        open_orders_tree.insert(wrapper_new_order_index, wrapper_order);
        let new_root_index: DataIndex = open_orders_tree.get_root_index();
        let market_info: &mut MarketInfo =
            get_mut_helper::<RBNode<MarketInfo>>(wrapper.dynamic, market_info_index)
                .get_mut_value();
        market_info.orders_root_index = new_root_index;
    }

    // Sync to get the balance correct and remove any expired orders.
    sync_fast(&wrapper_state, &market, market_info_index)?;

    Ok(())
}
