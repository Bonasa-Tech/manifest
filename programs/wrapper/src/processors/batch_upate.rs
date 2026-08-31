use std::{
    cell::{Ref, RefMut},
    mem::size_of,
};

use borsh::{BorshDeserialize, BorshSerialize};
use hypertree::{
    get_helper, get_mut_helper, DataIndex, FreeList, HyperTreeReadOperations,
    HyperTreeValueIteratorTrait, HyperTreeWriteOperations, RBNode, NIL,
};
use manifest::{
    program::{
        batch_update::{BatchUpdateParams, CancelOrderParams, PlaceOrderParams},
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
    ensure_free_slots, get_market_info_index_for_market, sync_fast, CancelMatcher, OpenOrdersList,
    UnusedWrapperFreeListPadding, EXPECTED_ORDER_BATCH_SIZE,
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
    pub fn client_order_id(&self) -> u64 {
        self.client_order_id
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

/// For `cancel_all`, also cancels orders on the market's seat that the
/// wrapper does not track (e.g. placed directly via the manifest program).
/// The wrapper's own open orders were already matched while syncing.
fn prepare_cancel_all(
    matcher: &mut CancelMatcher,
    market: &ManifestAccountInfo<MarketFixed>,
    trader_index: DataIndex,
) {
    let mut remaining_cancel_all_scans: usize =
        EXPECTED_ORDER_BATCH_SIZE.saturating_sub(matcher.core_cancels.len());
    let market_data: Ref<&mut [u8]> = market.try_borrow_data().unwrap();
    let market_ref: DynamicAccount<&MarketFixed, &[u8]> =
        get_dynamic_account::<MarketFixed>(&market_data);
    let is_known = |order_sequence_number: u64, core_cancels: &Vec<CancelOrderParams>| {
        core_cancels.iter().any(|cancel: &CancelOrderParams| {
            cancel.order_sequence_number() == order_sequence_number
        })
    };
    for (index, resting_order) in market_ref.get_bids().iter::<RestingOrder>() {
        if remaining_cancel_all_scans == 0 {
            break;
        }
        remaining_cancel_all_scans -= 1;
        if resting_order.get_trader_index() == trader_index
            && !is_known(resting_order.get_sequence_number(), &matcher.core_cancels)
        {
            matcher.core_cancels.push(CancelOrderParams::new_with_hint(
                resting_order.get_sequence_number(),
                Some(index),
            ));
            if matcher.needs_quote {
                matcher.freed_quote_atoms += resting_order
                    .get_price()
                    .checked_quote_for_base(resting_order.get_num_base_atoms(), true)
                    .unwrap();
            }
        }
    }
    for (index, resting_order) in market_ref.get_asks().iter::<RestingOrder>() {
        if remaining_cancel_all_scans == 0 {
            break;
        }
        remaining_cancel_all_scans -= 1;
        if resting_order.get_trader_index() == trader_index
            && !is_known(resting_order.get_sequence_number(), &matcher.core_cancels)
        {
            matcher.core_cancels.push(CancelOrderParams::new_with_hint(
                resting_order.get_sequence_number(),
                Some(index),
            ));
            if matcher.needs_base {
                matcher.freed_base_atoms += resting_order.get_num_base_atoms();
            }
        }
    }
}

/// Possibly update orders due to insufficient funds. Reduce the quantity of the
/// last orders in the vector so that they will not fail.
fn prepare_orders(
    orders: &[WrapperPlaceOrderParams],
    mut remaining_base_atoms: BaseAtoms,
    mut remaining_quote_atoms: QuoteAtoms,
    market: &ManifestAccountInfo<MarketFixed>,
    now_slot: u32,
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
                    // Exact, like the core: a bid sized to the whole balance
                    // must pass. The division is the reciprocal fast path in
                    // quantities.
                    let desired: QuoteAtoms = BaseAtoms::new(order.base_atoms)
                        .checked_mul(price, true)
                        .unwrap();
                    if desired > remaining_quote_atoms {
                        solana_program::msg!("Removing bid for insufficient funds");
                        num_base_atoms = 0;
                    } else {
                        remaining_quote_atoms -= desired;
                    }
                }
            } else {
                let desired: BaseAtoms = BaseAtoms::new(order.base_atoms);
                if price < best_bid_price && order.order_type == OrderType::PostOnly {
                    solana_program::msg!("Removing post only ask that would cross");
                    num_base_atoms = 0;
                } else {
                    if desired > remaining_base_atoms {
                        solana_program::msg!("Removing ask for insufficient funds");
                        num_base_atoms = 0;
                    } else {
                        remaining_base_atoms -= desired;
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

/// Forwards the batch to the core.
///
/// CU note: besides the 1,000 CU invoke base cost, the runtime charges every
/// account passed to a CPI `data_len / 250` CU when it translates the
/// caller's `AccountInfo`s (`cpi_bytes_per_unit`), whether or not account
/// data direct mapping is enabled; direct mapping only removes the copy of
/// the data, not the charge. For the market that is 4 CU per KB per batch
/// update, e.g. about 4,000 CU on a 1 MB market, and the only ways around it
/// are smaller markets or not going through a CPI.
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

/// Removes the cancelled orders from the wrapper's open orders.
fn process_cancels(
    wrapper_state: &WrapperStateAccountInfo,
    cancel_indices: &[DataIndex],
    market_info_index: DataIndex,
) {
    let mut wrapper_data: RefMut<&mut [u8]> = wrapper_state.info.try_borrow_mut_data().unwrap();
    let wrapper: DynamicAccount<&mut ManifestWrapperStateFixed, &mut [u8]> =
        get_mut_dynamic_account(&mut wrapper_data);
    let (orders_root_index, mut num_open_global_orders): (DataIndex, u32) = {
        let market_info: &MarketInfo =
            get_helper::<RBNode<MarketInfo>>(wrapper.dynamic, market_info_index).get_value();
        (
            market_info.orders_root_index,
            market_info.num_open_global_orders,
        )
    };
    for order_wrapper_index in cancel_indices {
        if get_helper::<RBNode<WrapperOpenOrder>>(wrapper.dynamic, *order_wrapper_index)
            .get_value()
            .get_order_type()
            == OrderType::Global
        {
            num_open_global_orders = num_open_global_orders.saturating_sub(1);
        }
    }
    let orders_root_index: DataIndex = {
        let mut open_orders: OpenOrdersList =
            OpenOrdersList::new(wrapper.dynamic, orders_root_index);
        for order_wrapper_index in cancel_indices {
            open_orders.remove_by_index(*order_wrapper_index);
        }
        open_orders.get_root_index()
    };
    let market_info: &mut MarketInfo =
        get_mut_helper::<RBNode<MarketInfo>>(wrapper.dynamic, market_info_index).get_mut_value();
    market_info.orders_root_index = orders_root_index;
    market_info.num_open_global_orders = num_open_global_orders;

    let mut free_list: FreeList<UnusedWrapperFreeListPadding> =
        FreeList::new(wrapper.dynamic, wrapper.fixed.free_list_head_index);
    for order_wrapper_index in cancel_indices {
        if *order_wrapper_index != NIL {
            free_list.add(*order_wrapper_index);
        }
    }
    // Update free list head.
    wrapper.fixed.free_list_head_index = free_list.get_head();
}

/// Records the orders that rested on the core in the wrapper's open orders.
fn process_orders<'a, 'info>(
    payer: &Signer<'a, 'info>,
    system_program: &Program<'a, 'info>,
    wrapper_state: &WrapperStateAccountInfo<'a, 'info>,
    orders: &[WrapperPlaceOrderParams],
    original_indices: &[usize],
    market_info_index: DataIndex,
) -> ProgramResult {
    // The core returns `BatchUpdateReturn`, borsh: a u32 count followed by
    // (u64 order sequence number, u32 order index) records. Read them in
    // place instead of deserializing into vectors.
    let (_, return_data): (Pubkey, Vec<u8>) = get_return_data().unwrap();
    let num_records: usize = u32::from_le_bytes(return_data[..4].try_into().unwrap()) as usize;
    let records: &[u8] = &return_data[4..4 + num_records * 12];
    let record = |index: usize| -> (u64, DataIndex) {
        let bytes: &[u8] = &records[index * 12..index * 12 + 12];
        (
            u64::from_le_bytes(bytes[..8].try_into().unwrap()),
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        )
    };

    // Order index is NIL when it did not rest, those need no slot. Grow the
    // wrapper once for all the rest instead of checking per order.
    let num_resting: usize = (0..num_records).filter(|&i| record(i).1 != NIL).count();
    if num_resting == 0 {
        return Ok(());
    }
    ensure_free_slots(wrapper_state, payer, system_program, num_resting)?;

    let mut wrapper_data: RefMut<&mut [u8]> = wrapper_state.info.try_borrow_mut_data().unwrap();
    let wrapper: DynamicAccount<&mut ManifestWrapperStateFixed, &mut [u8]> =
        get_mut_dynamic_account(&mut wrapper_data);
    let (mut orders_root_index, mut num_open_global_orders): (DataIndex, u32) = {
        let market_info: &MarketInfo =
            get_helper::<RBNode<MarketInfo>>(wrapper.dynamic, market_info_index).get_value();
        (
            market_info.orders_root_index,
            market_info.num_open_global_orders,
        )
    };
    let mut free_list_head_index: DataIndex = wrapper.fixed.free_list_head_index;
    for index in 0..num_records {
        let (order_sequence_number, order_index): (u64, DataIndex) = record(index);
        if order_index == NIL {
            continue;
        }
        let wrapper_new_order_index: DataIndex = {
            let mut free_list: FreeList<UnusedWrapperFreeListPadding> =
                FreeList::new(wrapper.dynamic, free_list_head_index);
            let new_index: DataIndex = free_list.remove();
            free_list_head_index = free_list.get_head();
            new_index
        };

        let original_order: &WrapperPlaceOrderParams = &orders[original_indices[index]];
        // Price and remaining size are left at zero here and filled in by the
        // sync that follows every placement in `process_batch_update`, which
        // is the only thing that knows how much of the order actually rested.
        // That sync is not optional: a zero here would otherwise be read as an
        // order that frees nothing when it is cancelled, and a later
        // cancel and replace would drop the replacement for want of funds.
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
        if original_order.order_type == OrderType::Global {
            num_open_global_orders += 1;
        }

        let mut open_orders: OpenOrdersList =
            OpenOrdersList::new(wrapper.dynamic, orders_root_index);
        open_orders.insert(wrapper_new_order_index, order);
        orders_root_index = open_orders.get_root_index();
    }
    wrapper.fixed.free_list_head_index = free_list_head_index;
    let market_info: &mut MarketInfo =
        get_mut_helper::<RBNode<MarketInfo>>(wrapper.dynamic, market_info_index).get_mut_value();
    market_info.orders_root_index = orders_root_index;
    market_info.num_open_global_orders = num_open_global_orders;
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

    // One clock read for the whole instruction.
    let now_slot: u32 = get_now_slot();

    // Cancels are mutable because the user may have mistakenly sent the same
    // one multiple times and the wrapper will take the responsibility for
    // deduping before forwarding to the core.
    let WrapperBatchUpdateParams {
        orders,
        cancel_all,
        cancels,
    } = WrapperBatchUpdateParams::try_from_slice(data)?;

    // Only price the funds that cancels free up when a new order needs the
    // balance check.
    let needs_base: bool = orders.iter().any(|order: &WrapperPlaceOrderParams| {
        !order.is_bid && order.order_type != OrderType::Global
    });
    let needs_quote: bool = orders.iter().any(|order: &WrapperPlaceOrderParams| {
        order.is_bid && order.order_type != OrderType::Global
    });

    // Sync to get all existing orders and balances fresh (needed for
    // modifying user orders for insufficient funds), matching the cancels
    // against the open orders in the same walk.
    let mut matcher: CancelMatcher =
        CancelMatcher::new(&cancels, cancel_all, needs_base, needs_quote);
    sync_fast(
        &wrapper_state,
        &market,
        market_info_index,
        now_slot,
        false,
        Some(&mut matcher),
    )?;

    let market_info: MarketInfo = {
        let wrapper_data: Ref<&mut [u8]> = wrapper_state.info.try_borrow_data()?;
        let (_fixed_data, wrapper_dynamic_data) =
            wrapper_data.split_at(size_of::<ManifestWrapperStateFixed>());
        *get_helper::<RBNode<MarketInfo>>(wrapper_dynamic_data, market_info_index).get_value()
    };
    let trader_index_hint: Option<DataIndex> = Some(market_info.trader_index);
    if cancel_all {
        prepare_cancel_all(&mut matcher, &market, market_info.trader_index);
    }
    let remaining_base_atoms: BaseAtoms = market_info.base_balance + matcher.freed_base_atoms;
    let remaining_quote_atoms: QuoteAtoms = market_info.quote_balance + matcher.freed_quote_atoms;
    let CancelMatcher {
        wrapper_indices: cancel_indices,
        core_cancels,
        ..
    } = matcher;

    let (core_orders, original_indices) = prepare_orders(
        &orders,
        remaining_base_atoms,
        remaining_quote_atoms,
        &market,
        now_slot,
    );

    // Whether the core ran its matching loop, which is the only thing in a
    // batch update that can touch orders other than the ones named in it.
    let placed_orders: bool = !core_orders.is_empty();

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

    // Forwarding a placement runs the core's matching loop, and that loop can
    // change this trader's other orders in ways nothing here can predict: it
    // fills them, and it removes any it finds expired on its way down the
    // book. So whenever an order was placed, read the orders back rather than
    // declaring the view exact.
    //
    // Seat quote volume was used for this and is not enough. The matching loop
    // removes an expired maker without recording any volume, and a fill of a
    // small amount at a low price rounds down to zero quote atoms, so either
    // can move this trader's orders while the volume stands still.
    //
    // A batch that only cancels runs no matching, so nothing can have touched
    // the orders that this instruction did not touch itself, and the cheap
    // path still applies. This is also what heals a view that went stale
    // outside the wrapper, see `sync_fast`: the next placement re-reads.
    sync_fast(
        &wrapper_state,
        &market,
        market_info_index,
        now_slot,
        !placed_orders,
        None,
    )?;

    // Collect fee.
    collect_fee(&payer, &wrapper_state)?;

    Ok(())
}
