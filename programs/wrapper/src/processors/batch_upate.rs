use std::{
    cell::{Ref, RefMut},
    collections::HashSet,
    mem::size_of,
};

use borsh::{BorshDeserialize, BorshSerialize};
use hypertree::{
    get_helper, get_mut_helper, DataIndex, FreeList, HyperTreeReadOperations,
    HyperTreeValueIteratorTrait, HyperTreeWriteOperations, RBNode, NIL,
};
use manifest::{
    program::{
        batch_update::{BatchUpdateParams, BatchUpdateReturn, CancelOrderParams, PlaceOrderParams},
        get_dynamic_account, get_mut_dynamic_account, invoke, ManifestInstruction,
    },
    quantities::{BaseAtoms, QuoteAtoms, QuoteAtomsPerBaseAtom, WrapperU64},
    state::{
        utils::get_now_slot, DynamicAccount, MarketFixed, OrderType, RestingOrder,
        MARKET_FIXED_SIZE, NO_EXPIRATION_LAST_VALID_SLOT,
    },
    validation::{ManifestAccountInfo, Program, Signer},
};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    instruction::{AccountMeta, Instruction},
    program::get_return_data,
    program_error::ProgramError,
    pubkey::Pubkey,
    system_program,
};

use crate::{
    loader::{check_signer, WrapperStateAccountInfo},
    market_info::MarketInfo,
    open_order::WrapperOpenOrder,
    wrapper_state::ManifestWrapperStateFixed,
};

use super::shared::{
    expand_wrapper_if_needed, get_market_info_index_for_market, sync_fast, OpenOrdersTree,
    OpenOrdersTreeReadOnly, UnusedWrapperFreeListPadding, EXPECTED_ORDER_BATCH_SIZE,
};

#[derive(BorshDeserialize, BorshSerialize, Clone)]
pub struct WrapperPlaceOrderParams {
    client_order_id: u64,
    base_atoms: u64,
    price_mantissa: u32,
    price_exponent: i8,
    is_bid: bool,
    last_valid_slot: u32,
    order_type: OrderType,
}
impl WrapperPlaceOrderParams {
    pub fn new(
        client_order_id: u64,
        base_atoms: u64,
        price_mantissa: u32,
        price_exponent: i8,
        is_bid: bool,
        last_valid_slot: u32,
        order_type: OrderType,
    ) -> Self {
        WrapperPlaceOrderParams {
            client_order_id,
            base_atoms,
            price_mantissa,
            price_exponent,
            is_bid,
            last_valid_slot,
            order_type,
        }
    }
}

// TODO: Note that this does not cancel reverse orders which have been created
// at a new sequence number and address (partial fill).
#[derive(BorshDeserialize, BorshSerialize, Clone)]
pub struct WrapperCancelOrderParams {
    client_order_id: u64,
}
impl WrapperCancelOrderParams {
    pub fn new(client_order_id: u64) -> Self {
        WrapperCancelOrderParams { client_order_id }
    }
}

#[derive(BorshDeserialize, BorshSerialize)]
pub struct WrapperBatchUpdateParams {
    pub cancels: Vec<WrapperCancelOrderParams>,
    pub cancel_all: bool,
    pub orders: Vec<WrapperPlaceOrderParams>,
}
impl WrapperBatchUpdateParams {
    pub fn new(
        cancels: Vec<WrapperCancelOrderParams>,
        cancel_all: bool,
        orders: Vec<WrapperPlaceOrderParams>,
    ) -> Self {
        WrapperBatchUpdateParams {
            cancels,
            cancel_all,
            orders,
        }
    }
}

/// Takes a vector of wrapper cancel params and returns two vectors, one with
/// the wrapper indices of the orders and the other with the cancel params for
/// the core. This function is responsible for looking up the order in the core
/// and converting wrapper cancel params into core cancel params.
fn prepare_cancels(
    wrapper_state: &WrapperStateAccountInfo,
    cancels: &Vec<WrapperCancelOrderParams>,
    cancel_all: bool,
    orders_root_index: DataIndex,
    remaining_base_atoms: &mut BaseAtoms,
    remaining_quote_atoms: &mut QuoteAtoms,
    market: &ManifestAccountInfo<MarketFixed>,
    trader_index: DataIndex,
) -> Result<(Vec<DataIndex>, Vec<CancelOrderParams>), ProgramError> {
    let wrapper_data: Ref<&mut [u8]> = wrapper_state.info.try_borrow_data().unwrap();
    let wrapper: DynamicAccount<&ManifestWrapperStateFixed, &[u8]> =
        get_dynamic_account(&wrapper_data);

    let open_orders_tree: OpenOrdersTreeReadOnly =
        OpenOrdersTreeReadOnly::new(wrapper.dynamic, orders_root_index, NIL);

    let client_ids_to_cancel: HashSet<u64> = {
        let mut set: HashSet<u64> = HashSet::<u64>::with_capacity(cancels.len());
        set.extend(cancels.iter().map(|c| c.client_order_id));
        set
    };

    let mut wrapper_indices: Vec<DataIndex> = Vec::with_capacity(EXPECTED_ORDER_BATCH_SIZE);
    let mut core_cancels: Vec<CancelOrderParams> = Vec::with_capacity(EXPECTED_ORDER_BATCH_SIZE);
    for (wrapper_index, open_order) in open_orders_tree.iter::<WrapperOpenOrder>() {
        if cancel_all || client_ids_to_cancel.contains(&open_order.get_client_order_id()) {
            wrapper_indices.push(wrapper_index);
            core_cancels.push(CancelOrderParams::new_with_hint(
                open_order.get_order_sequence_number(),
                Some(open_order.get_market_data_index()),
            ));
            if open_order.get_is_bid() {
                *remaining_quote_atoms += open_order
                    .get_price()
                    .checked_quote_for_base(open_order.get_num_base_atoms(), true)
                    .unwrap();
            } else {
                *remaining_base_atoms += open_order.get_num_base_atoms();
            };
        }
    }
    drop(wrapper_data);

    // When cancel_all is set, also cancel orders on the market's seat that the
    // wrapper does not track (e.g. placed directly via the manifest program).
    if cancel_all {
        let known_sequence_numbers: HashSet<u64> = core_cancels
            .iter()
            .map(|c| c.order_sequence_number())
            .collect();

        let market_data: Ref<&mut [u8]> = market.try_borrow_data().unwrap();
        let market_ref: DynamicAccount<&MarketFixed, &[u8]> =
            get_dynamic_account::<MarketFixed>(&market_data);

        for (index, resting_order) in market_ref.get_bids().iter::<RestingOrder>() {
            if resting_order.get_trader_index() == trader_index
                && !known_sequence_numbers.contains(&resting_order.get_sequence_number())
            {
                core_cancels.push(CancelOrderParams::new_with_hint(
                    resting_order.get_sequence_number(),
                    Some(index),
                ));
                *remaining_quote_atoms += resting_order
                    .get_price()
                    .checked_quote_for_base(resting_order.get_num_base_atoms(), true)
                    .unwrap();
            }
        }
        for (index, resting_order) in market_ref.get_asks().iter::<RestingOrder>() {
            if resting_order.get_trader_index() == trader_index
                && !known_sequence_numbers.contains(&resting_order.get_sequence_number())
            {
                core_cancels.push(CancelOrderParams::new_with_hint(
                    resting_order.get_sequence_number(),
                    Some(index),
                ));
                *remaining_base_atoms += resting_order.get_num_base_atoms();
            }
        }
    }

    Ok((wrapper_indices, core_cancels))
}

/// Possibly update orders due to insufficient funds. Reduce the quantity of the
/// last orders in the vector so that they will not fail.
fn prepare_orders(
    orders: &Vec<WrapperPlaceOrderParams>,
    remaining_base_atoms: &mut BaseAtoms,
    remaining_quote_atoms: &mut QuoteAtoms,
    market: &ManifestAccountInfo<MarketFixed>,
) -> (Vec<PlaceOrderParams>, Vec<usize>) {
    let market_data: Ref<'_, &mut [u8]> = market.try_borrow_data().unwrap();
    let market_ref: DynamicAccount<&MarketFixed, &[u8]> =
        get_dynamic_account::<MarketFixed>(&market_data);
    let mut best_ask_index: DataIndex = market_ref.get_asks().get_max_index();
    let mut best_bid_index: DataIndex = market_ref.get_bids().get_max_index();

    // Walk the tree until you find a non-expired order since those can be
    // trivially ignored. Does not prevent unbacked global orders, but that
    // would require global accounts and be too complicated to do here because
    // this is only best-effort.
    // Also, changes orders with last_valid_slot < 1_000_000 to now +
    // last_valid_slot.
    let now_slot: u32 = get_now_slot();

    while best_ask_index != NIL
        && get_helper::<RBNode<RestingOrder>>(
            &market_data,
            best_ask_index + (MARKET_FIXED_SIZE as DataIndex),
        )
        .get_value()
        .is_expired(now_slot)
    {
        best_ask_index = market_ref
            .get_asks()
            .get_next_lower_index::<RestingOrder>(best_ask_index);
    }
    while best_bid_index != NIL
        && get_helper::<RBNode<RestingOrder>>(
            &market_data,
            best_bid_index + (MARKET_FIXED_SIZE as DataIndex),
        )
        .get_value()
        .is_expired(now_slot)
    {
        best_bid_index = market_ref
            .get_bids()
            .get_next_lower_index::<RestingOrder>(best_bid_index);
    }

    let best_ask_price: QuoteAtomsPerBaseAtom = if best_ask_index != NIL {
        get_helper::<RBNode<RestingOrder>>(
            &market_data,
            best_ask_index + (MARKET_FIXED_SIZE as DataIndex),
        )
        .get_value()
        .get_price()
    } else {
        QuoteAtomsPerBaseAtom::MAX
    };
    let best_bid_price: QuoteAtomsPerBaseAtom = if best_bid_index != NIL {
        get_helper::<RBNode<RestingOrder>>(
            &market_data,
            best_bid_index + (MARKET_FIXED_SIZE as DataIndex),
        )
        .get_value()
        .get_price()
    } else {
        QuoteAtomsPerBaseAtom::MIN
    };

    let mut result: Vec<PlaceOrderParams> = Vec::with_capacity(orders.len());
    let mut original_indices: Vec<usize> = Vec::with_capacity(orders.len());
    for (i, order) in orders.iter().enumerate() {
        let mut num_base_atoms: u64 = order.base_atoms;
        let price: QuoteAtomsPerBaseAtom = QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(
            order.price_mantissa,
            order.price_exponent,
        )
        .unwrap();
        if order.order_type != OrderType::Global {
            if order.is_bid {
                if price > best_ask_price && order.order_type == OrderType::PostOnly {
                    solana_program::msg!("Removing post only bid that would cross");
                    num_base_atoms = 0;
                } else {
                    let desired: QuoteAtoms = BaseAtoms::new(order.base_atoms)
                        .checked_mul(price, true)
                        .unwrap();
                    if desired > *remaining_quote_atoms {
                        solana_program::msg!("Removing bid for insufficient funds");
                        num_base_atoms = 0;
                    } else {
                        *remaining_quote_atoms -= desired;
                    }
                }
            } else {
                let desired: BaseAtoms = BaseAtoms::new(order.base_atoms);
                if price < best_bid_price && order.order_type == OrderType::PostOnly {
                    solana_program::msg!("Removing post only ask that would cross");
                    num_base_atoms = 0;
                } else {
                    if desired > *remaining_base_atoms {
                        solana_program::msg!("Removing ask for insufficient funds");
                        num_base_atoms = 0;
                    } else {
                        *remaining_base_atoms -= desired;
                    }
                }
            }
        }
        if num_base_atoms == 0 {
            continue;
        }
        let expiration = if order.last_valid_slot != NO_EXPIRATION_LAST_VALID_SLOT
            && order.last_valid_slot < 10_000_000
            && !order.order_type.is_reversible()
        {
            now_slot + order.last_valid_slot
        } else {
            order.last_valid_slot
        };
        result.push(PlaceOrderParams::new(
            num_base_atoms,
            order.price_mantissa,
            order.price_exponent,
            order.is_bid,
            order.order_type,
            expiration,
        ));
        original_indices.push(i);
    }
    (result, original_indices)
}

fn execute_cpi(
    accounts: &[AccountInfo],
    trader_index_hint: Option<DataIndex>,
    core_cancels: Vec<CancelOrderParams>,
    core_orders: Vec<PlaceOrderParams>,
) -> ProgramResult {
    let mut acc_metas: Vec<AccountMeta> = Vec::with_capacity(accounts.len());
    // First two accounts are for wrapper and manifest program itself the
    // remainder is passed through directly to manifest.
    acc_metas.extend(accounts[2..].iter().map(|ai| {
        if ai.is_writable {
            AccountMeta::new(*ai.key, ai.is_signer)
        } else {
            AccountMeta::new_readonly(*ai.key, ai.is_signer)
        }
    }));

    let ix: Instruction = Instruction {
        program_id: manifest::id(),
        accounts: acc_metas,
        data: [
            ManifestInstruction::BatchUpdate.to_vec(),
            BatchUpdateParams::new(trader_index_hint, core_cancels, core_orders).try_to_vec()?,
        ]
        .concat(),
    };

    invoke(&ix, &accounts[1..])
}

fn process_cancels(
    wrapper_state: &WrapperStateAccountInfo,
    cancel_indices: &Vec<DataIndex>,
    market_info_index: DataIndex,
) {
    let mut wrapper_data: RefMut<&mut [u8]> = wrapper_state.info.try_borrow_mut_data().unwrap();
    let wrapper: DynamicAccount<&mut ManifestWrapperStateFixed, &mut [u8]> =
        get_mut_dynamic_account(&mut wrapper_data);

    // Fetch current root first to not borrow wrapper.dynamic twice.
    let orders_root_index: DataIndex = {
        let market_info: &mut MarketInfo =
            get_mut_helper::<RBNode<MarketInfo>>(wrapper.dynamic, market_info_index)
                .get_mut_value();
        market_info.orders_root_index
    };

    let orders_root_index: DataIndex = {
        let mut open_orders_tree: OpenOrdersTree =
            OpenOrdersTree::new(wrapper.dynamic, orders_root_index, NIL);

        // Remove nodes from order tree.
        for order_wrapper_index in cancel_indices {
            let order_wrapper_index = *order_wrapper_index;
            open_orders_tree.remove_by_index(order_wrapper_index);
        }
        open_orders_tree.get_root_index()
    };

    // Save new root.
    let market_info: &mut MarketInfo =
        get_mut_helper::<RBNode<MarketInfo>>(wrapper.dynamic, market_info_index).get_mut_value();
    market_info.orders_root_index = orders_root_index;

    // Add nodes to FreeList.
    let mut free_list: FreeList<UnusedWrapperFreeListPadding> =
        FreeList::new(wrapper.dynamic, wrapper.fixed.free_list_head_index);
    for order_wrapper_index in cancel_indices {
        let order_wrapper_index = *order_wrapper_index;

        if order_wrapper_index != NIL {
            free_list.add(order_wrapper_index);
        }
    }

    // Update free list head.
    wrapper.fixed.free_list_head_index = free_list.get_head();
}

fn process_orders<'a, 'info>(
    payer: &Signer<'a, 'info>,
    system_program: &Program<'a, 'info>,
    wrapper_state: &WrapperStateAccountInfo<'a, 'info>,
    orders: &Vec<WrapperPlaceOrderParams>,
    original_indices: &Vec<usize>,
    market_info_index: DataIndex,
) -> ProgramResult {
    let cpi_return_data: Option<(Pubkey, Vec<u8>)> = get_return_data();
    let BatchUpdateReturn {
        orders: batch_update_orders,
    } = BatchUpdateReturn::try_from_slice(&cpi_return_data.unwrap().1[..])?;
    for (index, &(order_sequence_number, order_index)) in batch_update_orders.iter().enumerate() {
        // Order index is NIL when it did not rest. In that case, do not need to store in wrapper.
        if order_index == NIL {
            continue;
        }

        // Does not expand all at once because expand checks if there is no spots available.
        expand_wrapper_if_needed(wrapper_state, payer, system_program)?;

        let mut wrapper_data: RefMut<&mut [u8]> = wrapper_state.info.try_borrow_mut_data().unwrap();
        let wrapper: DynamicAccount<&mut ManifestWrapperStateFixed, &mut [u8]> =
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

        let original_order: &WrapperPlaceOrderParams = &orders[original_indices[index]];
        // Base atoms & price can be wrong, will be fixed in the sync.
        let order: WrapperOpenOrder = WrapperOpenOrder::new(
            original_order.client_order_id,
            order_sequence_number,
            QuoteAtomsPerBaseAtom::ZERO,
            BaseAtoms::ZERO,
            original_order.last_valid_slot,
            order_index,
            original_order.is_bid,
            original_order.order_type,
        );

        let mut open_orders_tree: OpenOrdersTree =
            OpenOrdersTree::new(wrapper.dynamic, orders_root_index, NIL);
        open_orders_tree.insert(wrapper_new_order_index, order);
        let new_root_index: DataIndex = open_orders_tree.get_root_index();
        let market_info: &mut MarketInfo =
            get_mut_helper::<RBNode<MarketInfo>>(wrapper.dynamic, market_info_index)
                .get_mut_value();
        market_info.orders_root_index = new_root_index;
    }
    Ok(())
}

// Fee here is 5_000 lamports stored on the wrapper state. This is stored on the
// wrapper state because it prevents the need for a contentious extra write
// lock. Users who do not wish to pay this fee should use their own wrapper or
// interact directly with the manifest program.
fn collect_fee<'a, 'info>(
    payer: &Signer<'a, 'info>,
    wrapper_state: &WrapperStateAccountInfo<'a, 'info>,
) -> ProgramResult {
    invoke(
        &solana_program::system_instruction::transfer(
            &payer.as_ref().key,
            &wrapper_state.key,
            manifest::state::GAS_DEPOSIT_LAMPORTS,
        ),
        &[payer.as_ref().clone(), wrapper_state.info.clone()],
    )?;

    Ok(())
}

pub(crate) fn process_batch_update(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let account_iter: &mut std::slice::Iter<AccountInfo> = &mut accounts.iter();
    let wrapper_state: WrapperStateAccountInfo =
        WrapperStateAccountInfo::new(next_account_info(account_iter)?)?;
    let _manifest_program: Program =
        Program::new(next_account_info(account_iter)?, &manifest::id())?;
    let payer: Signer = Signer::new(next_account_info(account_iter)?)?;
    let market: ManifestAccountInfo<MarketFixed> =
        ManifestAccountInfo::<MarketFixed>::new(next_account_info(account_iter)?)?;
    let system_program: Program =
        Program::new(next_account_info(account_iter)?, &system_program::id())?;

    check_signer(&wrapper_state, payer.key);
    let market_info_index: DataIndex = get_market_info_index_for_market(&wrapper_state, market.key);

    // Do an initial sync to get all existing orders and balances fresh. This is
    // needed for modifying user orders for insufficient funds.
    sync_fast(&wrapper_state, &market, market_info_index)?;

    // Cancels are mutable because the user may have mistakenly sent the same
    // one multiple times and the wrapper will take the responsibility for
    // deduping before forwarding to the core.
    let WrapperBatchUpdateParams {
        orders,
        cancel_all,
        cancels,
    } = WrapperBatchUpdateParams::try_from_slice(data)?;

    let wrapper_data: Ref<&mut [u8]> = wrapper_state.info.try_borrow_data()?;
    let (_fixed_data, wrapper_dynamic_data) =
        wrapper_data.split_at(size_of::<ManifestWrapperStateFixed>());

    let market_info: MarketInfo =
        *get_helper::<RBNode<MarketInfo>>(wrapper_dynamic_data, market_info_index).get_value();

    let trader_index_hint: Option<DataIndex> = Some(market_info.trader_index);
    let mut remaining_base_atoms: BaseAtoms = market_info.base_balance;
    let mut remaining_quote_atoms: QuoteAtoms = market_info.quote_balance;
    drop(wrapper_data);

    let (cancel_indices, core_cancels) = prepare_cancels(
        &wrapper_state,
        &cancels,
        cancel_all,
        market_info.orders_root_index,
        &mut remaining_base_atoms,
        &mut remaining_quote_atoms,
        &market,
        market_info.trader_index,
    )?;
    let (core_orders, original_indices) = prepare_orders(
        &orders,
        &mut remaining_base_atoms,
        &mut remaining_quote_atoms,
        &market,
    );

    execute_cpi(accounts, trader_index_hint, core_cancels, core_orders)?;

    process_cancels(&wrapper_state, &cancel_indices, market_info_index);
    process_orders(
        &payer,
        &system_program,
        &wrapper_state,
        &orders,
        &original_indices,
        market_info_index,
    )?;

    // Sync to get the balance correct and remove any expired orders.
    sync_fast(&wrapper_state, &market, market_info_index)?;

    // Collect fee.
    collect_fee(&payer, &wrapper_state)?;

    Ok(())
}
