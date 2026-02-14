use std::cell::RefMut;

use crate::{
    logs::{emit_stack, CancelOrderLog, PlaceOrderLog},
    program::get_trader_index_with_hint,
    quantities::{BaseAtoms, PriceConversionError, QuoteAtomsPerBaseAtom, WrapperU64},
    require,
    state::{
        utils::{get_now_slot, try_to_pay_all_global_gas_prepayment},
        AddOrderToMarketArgs, AddOrderToMarketResult, MarketRefMut, OrderType, RestingOrder,
        MARKET_BLOCK_SIZE,
    },
    validation::loaders::BatchUpdateContext,
};
use borsh::{BorshDeserialize, BorshSerialize};

use hypertree::{get_helper, trace, DataIndex, PodBool, RBNode};
use solana_program::{
    account_info::AccountInfo, entrypoint::ProgramResult, program_error::ProgramError,
    pubkey::Pubkey,
};

use super::{expand_market_if_needed, shared::get_mut_dynamic_account};

use crate::validation::loaders::GlobalTradeAccounts;
#[cfg(feature = "certora")]
use {
    crate::certora::mocks_batch_update::{mock_cancel_order, mock_place_order},
    early_panic::early_panic,
    vectors::no_resizable_vec::NoResizableVec,
};

#[derive(Debug, BorshDeserialize, BorshSerialize, Clone)]
pub struct CancelOrderParams {
    order_sequence_number: u64,
    order_index_hint: Option<DataIndex>,
}

impl CancelOrderParams {
    pub fn new(order_sequence_number: u64) -> Self {
        CancelOrderParams {
            order_sequence_number,
            order_index_hint: None,
        }
    }
    pub fn new_with_hint(order_sequence_number: u64, order_index_hint: Option<DataIndex>) -> Self {
        CancelOrderParams {
            order_sequence_number,
            order_index_hint,
        }
    }
    pub fn order_sequence_number(&self) -> u64 {
        self.order_sequence_number
    }
    pub fn order_index_hint(&self) -> Option<DataIndex> {
        self.order_index_hint
    }
}

#[derive(Debug, BorshDeserialize, BorshSerialize, Clone)]
pub struct PlaceOrderParams {
    base_atoms: u64,
    price_mantissa: u32,
    price_exponent: i8,
    is_bid: bool,
    last_valid_slot: u32,
    order_type: OrderType,
}

impl PlaceOrderParams {
    pub fn new(
        base_atoms: u64,
        price_mantissa: u32,
        price_exponent: i8,
        is_bid: bool,
        order_type: OrderType,
        last_valid_slot: u32,
    ) -> Self {
        PlaceOrderParams {
            base_atoms,
            price_mantissa,
            price_exponent,
            is_bid,
            order_type,
            last_valid_slot,
        }
    }
    pub fn base_atoms(&self) -> u64 {
        self.base_atoms
    }

    pub fn try_price(&self) -> Result<QuoteAtomsPerBaseAtom, PriceConversionError> {
        if self.price_exponent > self.order_type().max_exponent() {
            return Err(PriceConversionError(0x3));
        }
        QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(
            self.price_mantissa,
            self.price_exponent,
        )
    }
    pub fn is_bid(&self) -> bool {
        self.is_bid
    }
    pub fn last_valid_slot(&self) -> u32 {
        self.last_valid_slot
    }
    pub fn order_type(&self) -> OrderType {
        self.order_type
    }
}

#[derive(BorshDeserialize, BorshSerialize)]
pub struct BatchUpdateParams {
    /// Optional hint for what index the trader's ClaimedSeat is at.
    pub trader_index_hint: Option<DataIndex>,
    #[cfg(not(feature = "certora"))]
    pub cancels: Vec<CancelOrderParams>,
    #[cfg(feature = "certora")]
    pub cancels: NoResizableVec<CancelOrderParams>,
    #[cfg(not(feature = "certora"))]
    pub orders: Vec<PlaceOrderParams>,
    #[cfg(feature = "certora")]
    pub orders: NoResizableVec<PlaceOrderParams>,
}

impl BatchUpdateParams {
    pub fn new(
        trader_index_hint: Option<DataIndex>,
        #[cfg(not(feature = "certora"))] cancels: Vec<CancelOrderParams>,
        #[cfg(feature = "certora")] cancels: NoResizableVec<CancelOrderParams>,
        #[cfg(not(feature = "certora"))] orders: Vec<PlaceOrderParams>,
        #[cfg(feature = "certora")] orders: NoResizableVec<PlaceOrderParams>,
    ) -> Self {
        BatchUpdateParams {
            trader_index_hint,
            cancels,
            orders,
        }
    }
}

#[derive(BorshDeserialize, BorshSerialize)]
pub struct BatchUpdateReturn {
    /// Vector of tuples of (order_sequence_number, DataIndex)
    pub orders: Vec<(u64, DataIndex)>,
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Default)]
pub enum MarketDataTreeNodeType {
    // 0 is reserved because zeroed byte arrays should be empty.
    Empty = 0,
    #[default]
    ClaimedSeat = 1,
    RestingOrder = 2,
}

pub(crate) fn process_batch_update(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let params: BatchUpdateParams = BatchUpdateParams::try_from_slice(data)?;
    process_batch_update_core(program_id, accounts, params)
}

#[cfg(not(feature = "certora"))]
fn batch_cancel_order(
    dynamic_account: &mut MarketRefMut,
    trader_index: DataIndex,
    order_sequence_number: u64,
    global_trade_accounts_opts: &[Option<GlobalTradeAccounts>; 2],
) -> ProgramResult {
    dynamic_account.cancel_order(
        trader_index,
        order_sequence_number,
        &global_trade_accounts_opts,
    )
}

#[cfg(feature = "certora")]
fn batch_cancel_order(
    dynamic_account: &mut MarketRefMut,
    trader_index: DataIndex,
    order_sequence_number: u64,
    global_trade_accounts_opts: &[Option<GlobalTradeAccounts>; 2],
) -> ProgramResult {
    mock_cancel_order(
        &dynamic_account,
        trader_index,
        order_sequence_number,
        &global_trade_accounts_opts,
    )
}

#[cfg(not(feature = "certora"))]
fn batch_place_order(
    dynamic_account: &mut MarketRefMut,
    args: AddOrderToMarketArgs,
) -> Result<AddOrderToMarketResult, ProgramError> {
    dynamic_account.place_order(args)
}

#[cfg(feature = "certora")]
fn batch_place_order(
    dynamic_account: &mut MarketRefMut,
    args: AddOrderToMarketArgs,
) -> Result<AddOrderToMarketResult, ProgramError> {
    mock_place_order(dynamic_account, args)
}

#[cfg_attr(all(feature = "certora", not(feature = "certora-test")), early_panic)]
pub(crate) fn process_batch_update_core(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: BatchUpdateParams,
) -> ProgramResult {
    let batch_update_context: BatchUpdateContext = BatchUpdateContext::load(accounts)?;

    let BatchUpdateContext {
        market,
        payer,
        global_trade_accounts_opts,
        ..
    } = batch_update_context;

    let BatchUpdateParams {
        trader_index_hint,
        cancels,
        orders,
    } = params;

    let current_slot: Option<u32> = Some(get_now_slot());

    trace!("batch_update trader_index_hint:{trader_index_hint:?} cancels:{cancels:?} orders:{orders:?}");

    let trader_index: DataIndex = {
        let market_data: &mut RefMut<&mut [u8]> = &mut market.try_borrow_mut_data()?;

        let mut dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);
        let trader_index: DataIndex =
            get_trader_index_with_hint(trader_index_hint, &dynamic_account, &payer)?;

        for cancel_order_params in cancels {
            // Hinted is preferred because that is O(1) to find and O(log n) to
            // remove. Without the hint, we lookup by order_sequence_number and
            // that is O(n) lookup and O(log n) delete.
            match cancel_order_params.order_index_hint() {
                None => {
                    // Cancels must succeed otherwise we fail the tx.
                    batch_cancel_order(
                        &mut dynamic_account,
                        trader_index,
                        cancel_order_params.order_sequence_number(),
                        &global_trade_accounts_opts,
                    )?;
                }
                Some(hinted_cancel_index) => {
                    // Simple sanity check on the hint given. Make sure that it
                    // aligns with block boundaries. We do a check that it is an
                    // order owned by the payer inside the handler.
                    require!(
                        hinted_cancel_index % (MARKET_BLOCK_SIZE as DataIndex) == 0,
                        crate::program::ManifestError::WrongIndexHintParams,
                        "Invalid cancel hint index {}",
                        hinted_cancel_index,
                    )?;
                    require!(
                        get_helper::<RBNode<RestingOrder>>(
                            &dynamic_account.dynamic,
                            hinted_cancel_index,
                        )
                        .get_payload_type()
                            == MarketDataTreeNodeType::RestingOrder as u8,
                        crate::program::ManifestError::WrongIndexHintParams,
                        "Invalid cancel hint index {}",
                        hinted_cancel_index,
                    )?;
                    let order: &RestingOrder =
                        dynamic_account.get_order_by_index(hinted_cancel_index);
                    require!(
                        trader_index == order.get_trader_index(),
                        crate::program::ManifestError::WrongIndexHintParams,
                        "Invalid cancel hint index {}",
                        hinted_cancel_index,
                    )?;
                    require!(
                        cancel_order_params.order_sequence_number() == order.get_sequence_number(),
                        crate::program::ManifestError::WrongIndexHintParams,
                        "Invalid cancel hint sequence number index {}",
                        hinted_cancel_index,
                    )?;
                    dynamic_account
                        .cancel_order_by_index(hinted_cancel_index, &global_trade_accounts_opts)?;
                }
            };

            emit_stack(CancelOrderLog {
                market: *market.key,
                trader: *payer.key,
                order_sequence_number: cancel_order_params.order_sequence_number(),
            })?;
        }
        trader_index
    };

    // Formal verification does not cover global gas prepayment.
    #[cfg(not(feature = "certora"))]
    {
        try_to_pay_all_global_gas_prepayment(&orders, &global_trade_accounts_opts)?;
    }

    // Result is a vector of (order_sequence_number, data_index)
    #[cfg(not(feature = "certora"))]
    let mut result: Vec<(u64, DataIndex)> = Vec::with_capacity(orders.len());
    #[cfg(feature = "certora")]
    let mut result = NoResizableVec::<(u64, DataIndex)>::new(10);
    for place_order_params in orders {
        {
            let base_atoms: BaseAtoms = BaseAtoms::new(place_order_params.base_atoms());
            let price: QuoteAtomsPerBaseAtom = place_order_params.try_price()?;
            let order_type: OrderType = place_order_params.order_type();
            let last_valid_slot: u32 = place_order_params.last_valid_slot();

            // Need to reborrow every iteration so we can borrow later for expanding.
            let market_data: &mut RefMut<&mut [u8]> = &mut market.try_borrow_mut_data()?;
            let mut dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);

            // For asks: virtually credit base atoms so the engine can
            // reserve them for the resting order. Base is virtual.
            if !place_order_params.is_bid() {
                dynamic_account.deposit(trader_index, place_order_params.base_atoms(), true)?;
            }

            let add_order_to_market_result: AddOrderToMarketResult = batch_place_order(
                &mut dynamic_account,
                AddOrderToMarketArgs {
                    market: *market.key,
                    trader_index,
                    num_base_atoms: base_atoms,
                    price,
                    is_bid: place_order_params.is_bid(),
                    last_valid_slot,
                    order_type,
                    global_trade_accounts_opts: &global_trade_accounts_opts,
                    current_slot,
                },
            )?;

            let AddOrderToMarketResult {
                order_index,
                order_sequence_number,
                ..
            } = add_order_to_market_result;

            // Initial margin check after order placement
            #[cfg(not(feature = "certora"))]
            {
                let claimed_seat: &crate::state::claimed_seat::ClaimedSeat =
                    get_helper::<hypertree::RBNode<crate::state::claimed_seat::ClaimedSeat>>(
                        &dynamic_account.dynamic,
                        trader_index,
                    )
                    .get_value();
                let position_size: i64 = claimed_seat.get_position_size();
                if position_size != 0 {
                    let abs_position: u64 = position_size.unsigned_abs();
                    let mark_price =
                        super::liquidate::compute_mark_price(&dynamic_account)?;
                    let notional: u64 = mark_price
                        .checked_quote_for_base(
                            crate::quantities::BaseAtoms::new(abs_position),
                            false,
                        )?
                        .as_u64();
                    let initial_margin_bps: u64 =
                        dynamic_account.fixed.get_initial_margin_bps();
                    let required_margin: u64 = notional
                        .checked_mul(initial_margin_bps)
                        .unwrap_or(u64::MAX)
                        / 10000;

                    let cost_basis = claimed_seat.get_quote_cost_basis();
                    let unrealized_pnl: i64 = if position_size > 0 {
                        (notional as i64).wrapping_sub(cost_basis as i64)
                    } else {
                        (cost_basis as i64).wrapping_sub(notional as i64)
                    };

                    let margin: u64 = claimed_seat.quote_withdrawable_balance.as_u64();
                    let equity: i128 = (margin as i128) + (unrealized_pnl as i128);
                    crate::require!(
                        equity >= required_margin as i128,
                        crate::program::ManifestError::InsufficientMargin,
                        "Initial margin check failed: equity {} < required {}",
                        equity,
                        required_margin,
                    )?;
                }
            }

            emit_stack(PlaceOrderLog {
                market: *market.key,
                trader: *payer.key,
                base_atoms,
                price,
                order_type,
                is_bid: PodBool::from(place_order_params.is_bid()),
                _padding: [0; 6],
                order_sequence_number,
                order_index,
                last_valid_slot,
            })?;
            result.push((order_sequence_number, order_index));
        }
        expand_market_if_needed(&payer, &market)?;
    }

    // Formal verification does not cover return values.
    #[cfg(not(feature = "certora"))]
    {
        let mut buffer: Vec<u8> = Vec::with_capacity(
            std::mem::size_of::<BatchUpdateReturn>()
                + result.len() * 2 * std::mem::size_of::<u64>(),
        );
        let return_data: BatchUpdateReturn = BatchUpdateReturn { orders: result };
        return_data.serialize(&mut buffer).unwrap();
        solana_program::program::set_return_data(&buffer[..]);
    }

    Ok(())
}
