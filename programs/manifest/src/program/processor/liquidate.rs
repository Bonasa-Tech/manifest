use crate::{
    logs::{emit_stack, LiquidateLog},
    program::{get_mut_dynamic_account, ManifestError},
    quantities::{BaseAtoms, QuoteAtoms, QuoteAtomsPerBaseAtom, WrapperU64},
    require,
    state::{claimed_seat::ClaimedSeat, MarketRefMut, RestingOrder},
    validation::loaders::{GlobalTradeAccounts, LiquidateContext},
};
use borsh::{BorshDeserialize, BorshSerialize};
use hypertree::{get_helper, get_mut_helper, DataIndex, HyperTreeValueIteratorTrait, RBNode};
use solana_program::{
    account_info::AccountInfo, entrypoint::ProgramResult, program_error::ProgramError,
    pubkey::Pubkey,
};
use std::cell::RefMut;

/// Liquidator reward in basis points (2.5%)
const LIQUIDATOR_REWARD_BPS: u64 = 250;

#[derive(BorshDeserialize, BorshSerialize)]
pub struct LiquidateParams {
    pub trader_to_liquidate: Pubkey,
}

impl LiquidateParams {
    pub fn new(trader_to_liquidate: Pubkey) -> Self {
        LiquidateParams {
            trader_to_liquidate,
        }
    }
}

pub(crate) fn process_liquidate(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let params = LiquidateParams::try_from_slice(data)?;
    let liquidate_context: LiquidateContext = LiquidateContext::load(accounts)?;

    let LiquidateContext {
        market,
        liquidator,
    } = liquidate_context;

    let market_data: &mut RefMut<&mut [u8]> = &mut market.try_borrow_mut_data()?;
    let mut dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);

    // Find the trader's seat
    let trader_index: DataIndex =
        dynamic_account.get_trader_index(&params.trader_to_liquidate);
    require!(
        trader_index != hypertree::NIL,
        ProgramError::InvalidArgument,
        "Trader not found on market",
    )?;

    let claimed_seat: &ClaimedSeat = get_helper::<RBNode<ClaimedSeat>>(
        &dynamic_account.dynamic,
        trader_index,
    )
    .get_value();

    let position_size: i64 = claimed_seat.get_position_size();
    require!(
        position_size != 0,
        ManifestError::NotLiquidatable,
        "Trader has no open position",
    )?;

    let quote_cost_basis: u64 = claimed_seat.get_quote_cost_basis();
    let margin_balance: u64 = claimed_seat.quote_withdrawable_balance.as_u64();

    // Cancel all open orders belonging to this trader before computing mark price
    // This releases reserved funds back to the trader's balance
    {
        let no_global_accounts: [Option<GlobalTradeAccounts>; 2] = [None, None];

        // Collect order indices from bids
        let bid_indices: Vec<DataIndex> = dynamic_account
            .get_bids()
            .iter::<RestingOrder>()
            .filter(|(_, order)| order.get_trader_index() == trader_index)
            .map(|(index, _)| index)
            .collect();

        // Collect order indices from asks
        let ask_indices: Vec<DataIndex> = dynamic_account
            .get_asks()
            .iter::<RestingOrder>()
            .filter(|(_, order)| order.get_trader_index() == trader_index)
            .map(|(index, _)| index)
            .collect();

        // Cancel all collected orders
        for order_index in bid_indices.iter().chain(ask_indices.iter()) {
            dynamic_account.cancel_order_by_index(*order_index, &no_global_accounts)?;
        }
    }

    // Re-read margin balance after order cancellations (funds released back)
    let margin_balance: u64 = {
        let seat: &ClaimedSeat = get_helper::<RBNode<ClaimedSeat>>(
            &dynamic_account.dynamic,
            trader_index,
        )
        .get_value();
        seat.quote_withdrawable_balance.as_u64()
    };

    // Compute mark price (prefers oracle, falls back to orderbook)
    let mark_price: QuoteAtomsPerBaseAtom = compute_mark_price(&dynamic_account)?;

    // Compute current market value of position: mark_price * |position_size|
    let abs_position: u64 = position_size.unsigned_abs();
    let current_value: u64 = mark_price
        .checked_quote_for_base(BaseAtoms::new(abs_position), false)?
        .as_u64();

    // Compute unrealized PnL
    let unrealized_pnl: i64 = if position_size > 0 {
        (current_value as i64).wrapping_sub(quote_cost_basis as i64)
    } else {
        (quote_cost_basis as i64).wrapping_sub(current_value as i64)
    };

    // Equity = margin + unrealized_pnl
    let equity: i128 = (margin_balance as i128) + (unrealized_pnl as i128);

    // Maintenance margin = current_value * maintenance_margin_bps / 10000
    let maintenance_margin_bps: u64 = dynamic_account.fixed.get_maintenance_margin_bps();
    let required_maintenance: u64 = current_value
        .checked_mul(maintenance_margin_bps)
        .unwrap_or(u64::MAX)
        / 10000;

    require!(
        equity < required_maintenance as i128,
        ManifestError::NotLiquidatable,
        "Trader equity {} >= maintenance margin {}, not liquidatable",
        equity,
        required_maintenance,
    )?;

    // Liquidate: settle position at mark price
    let settlement_pnl: i64 = unrealized_pnl;

    // Update the trader's seat: close position, settle PnL, deduct liquidator reward
    let liquidator_reward: u64;
    {
        let claimed_seat_mut: &mut ClaimedSeat = get_mut_helper::<RBNode<ClaimedSeat>>(
            &mut dynamic_account.dynamic,
            trader_index,
        )
        .get_mut_value();

        // Close position
        claimed_seat_mut.set_position_size(0);
        claimed_seat_mut.set_quote_cost_basis(0);

        // Settle PnL into margin balance
        let settled_margin = if settlement_pnl >= 0 {
            margin_balance.saturating_add(settlement_pnl as u64)
        } else {
            margin_balance.saturating_sub(settlement_pnl.unsigned_abs())
        };

        // Compute liquidator reward from remaining margin
        liquidator_reward = settled_margin
            .checked_mul(LIQUIDATOR_REWARD_BPS)
            .unwrap_or(0)
            / 10000;

        claimed_seat_mut.quote_withdrawable_balance =
            QuoteAtoms::new(settled_margin.saturating_sub(liquidator_reward));
    }

    // Credit liquidator reward (liquidator must have a seat)
    if liquidator_reward > 0 {
        let liquidator_index: DataIndex = dynamic_account.get_trader_index(liquidator.key);
        if liquidator_index != hypertree::NIL {
            let liquidator_seat: &mut ClaimedSeat =
                get_mut_helper::<RBNode<ClaimedSeat>>(
                    &mut dynamic_account.dynamic,
                    liquidator_index,
                )
                .get_mut_value();
            let current = liquidator_seat.quote_withdrawable_balance.as_u64();
            liquidator_seat.quote_withdrawable_balance =
                QuoteAtoms::new(current.saturating_add(liquidator_reward));
        }
    }

    // Update global position tracking
    #[cfg(not(feature = "certora"))]
    {
        if position_size > 0 {
            let current = dynamic_account.fixed.get_total_long_base_atoms();
            dynamic_account
                .fixed
                .set_total_long_base_atoms(current.saturating_sub(abs_position));
        } else {
            let current = dynamic_account.fixed.get_total_short_base_atoms();
            dynamic_account
                .fixed
                .set_total_short_base_atoms(current.saturating_sub(abs_position));
        }
    }

    emit_stack(LiquidateLog {
        market: *market.key,
        liquidator: *liquidator.key,
        trader: params.trader_to_liquidate,
        position_size: position_size as u64,
        settlement_price: current_value,
        pnl: settlement_pnl as u64,
        _padding: [0; 8],
    })?;

    Ok(())
}

/// Compute mark price, preferring cached oracle price over orderbook.
///
/// If the oracle price is set (oracle_price_mantissa > 0), converts it to
/// QuoteAtomsPerBaseAtom using the market's decimal configuration.
/// Falls back to orderbook best bid/ask if oracle is not available.
pub(crate) fn compute_mark_price(market: &MarketRefMut) -> Result<QuoteAtomsPerBaseAtom, ProgramError> {
    let oracle_mantissa = market.fixed.get_oracle_price_mantissa();
    if oracle_mantissa > 0 {
        // Oracle price = mantissa * 10^expo (USD per unit of base asset)
        // Convert to QuoteAtomsPerBaseAtom:
        //   qapba = mantissa * 10^(expo + quote_decimals - base_decimals)
        let expo = market.fixed.get_oracle_price_expo() as i64;
        let base_decimals = market.fixed.get_base_mint_decimals() as i64;
        let quote_decimals = market.fixed.get_quote_mint_decimals() as i64;

        let adjusted_expo = expo + quote_decimals - base_decimals;

        // Normalize mantissa to fit in u32 while adjusting exponent
        let mut m = oracle_mantissa as u128;
        let mut e = adjusted_expo;
        while m > u32::MAX as u128 && e < i8::MAX as i64 {
            m /= 10;
            e += 1;
        }

        if m <= u32::MAX as u128 && e >= i8::MIN as i64 && e <= i8::MAX as i64 {
            if let Ok(price) =
                QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(m as u32, e as i8)
            {
                return Ok(price);
            }
        }
        // If conversion fails, fall through to orderbook
    }

    // Fallback: orderbook best bid/ask
    let best_bid_index = market.fixed.get_bids_best_index();
    let best_ask_index = market.fixed.get_asks_best_index();

    require!(
        best_bid_index != hypertree::NIL || best_ask_index != hypertree::NIL,
        ManifestError::InvalidPerpsOperation,
        "Cannot compute mark price: empty orderbook",
    )?;

    if best_bid_index != hypertree::NIL && best_ask_index != hypertree::NIL {
        let best_bid: &RestingOrder =
            get_helper::<RBNode<RestingOrder>>(&market.dynamic, best_bid_index).get_value();
        let best_ask: &RestingOrder =
            get_helper::<RBNode<RestingOrder>>(&market.dynamic, best_ask_index).get_value();
        if best_bid.get_price() <= best_ask.get_price() {
            Ok(best_bid.get_price())
        } else {
            Ok(best_ask.get_price())
        }
    } else if best_bid_index != hypertree::NIL {
        let best_bid: &RestingOrder =
            get_helper::<RBNode<RestingOrder>>(&market.dynamic, best_bid_index).get_value();
        Ok(best_bid.get_price())
    } else {
        let best_ask: &RestingOrder =
            get_helper::<RBNode<RestingOrder>>(&market.dynamic, best_ask_index).get_value();
        Ok(best_ask.get_price())
    }
}
