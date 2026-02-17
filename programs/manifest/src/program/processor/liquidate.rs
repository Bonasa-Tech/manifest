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
    pubkey::Pubkey, sysvar::Sysvar,
};
use std::cell::RefMut;

/// Liquidator reward in basis points of closed notional (2.5%)
const LIQUIDATOR_REWARD_BPS: u64 = 250;
/// Minimum position size in base atoms to keep after partial liquidation.
/// If the remaining position would be smaller, do a full liquidation instead.
const MIN_POSITION_SIZE_ATOMS: u64 = 1000;

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

    // Prevent self-liquidation (extracting insurance fund via self-reward)
    require!(
        *liquidator.key != params.trader_to_liquidate,
        ManifestError::InvalidPerpsOperation,
        "Cannot liquidate your own position",
    )?;

    // Find the trader's seat
    let trader_index: DataIndex =
        dynamic_account.get_trader_index(&params.trader_to_liquidate);
    require!(
        trader_index != hypertree::NIL,
        ProgramError::InvalidArgument,
        "Trader not found on market",
    )?;

    // Lazy funding settlement for the trader being liquidated.
    // Must happen before reading margin/position to ensure accurate equity computation.
    dynamic_account.settle_funding_for_trader(trader_index)?;

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

    // Cancel all open orders belonging to this trader before computing mark price
    // This releases reserved funds back to the trader's balance
    {
        let no_global_accounts: [Option<GlobalTradeAccounts>; 2] = [None, None];

        let bid_indices: Vec<DataIndex> = dynamic_account
            .get_bids()
            .iter::<RestingOrder>()
            .filter(|(_, order)| order.get_trader_index() == trader_index)
            .map(|(index, _)| index)
            .collect();

        let ask_indices: Vec<DataIndex> = dynamic_account
            .get_asks()
            .iter::<RestingOrder>()
            .filter(|(_, order)| order.get_trader_index() == trader_index)
            .map(|(index, _)| index)
            .collect();

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

    // Require oracle has been updated recently (within 1 hour = 3600 seconds).
    // This prevents liquidation at stale cached prices.
    {
        let last_funding_ts: i64 = dynamic_account.fixed.get_last_funding_timestamp();
        let clock = solana_program::clock::Clock::get()?;
        let now = clock.unix_timestamp;
        let staleness = now.saturating_sub(last_funding_ts);
        require!(
            last_funding_ts > 0 && staleness <= 3600,
            ManifestError::InvalidPerpsOperation,
            "Oracle price is stale: last updated {} seconds ago",
            staleness,
        )?;
    }

    // Compute mark price (prefers oracle, falls back to orderbook)
    let mark_price: QuoteAtomsPerBaseAtom = compute_mark_price(&dynamic_account)?;

    // Compute current market value of position: mark_price * |position_size|
    let abs_position: u64 = position_size.unsigned_abs();
    let current_value: u64 = mark_price
        .checked_quote_for_base(BaseAtoms::new(abs_position), false)?
        .as_u64();

    // Compute unrealized PnL using i128 to avoid overflow on large u64 values
    let unrealized_pnl: i128 = if position_size > 0 {
        (current_value as i128) - (quote_cost_basis as i128)
    } else {
        (quote_cost_basis as i128) - (current_value as i128)
    };

    // Equity = margin + unrealized_pnl
    let equity: i128 = (margin_balance as i128) + unrealized_pnl;

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

    // --- Determine close amount: partial vs full liquidation ---
    //
    // After closing fraction f of position at mark price:
    //   new_equity = equity - f * current_value * REWARD_BPS / 10000
    //   new_notional = (1 - f) * current_value
    //   Target: new_equity >= new_notional * target_bps / 10000
    //
    // Solving: f = (target_bps - equity_bps) / (target_bps - REWARD_BPS)
    //   where equity_bps = equity * 10000 / current_value
    let liquidation_buffer_bps: u64 = dynamic_account.fixed.get_liquidation_buffer_bps();
    let target_bps: i128 = (maintenance_margin_bps + liquidation_buffer_bps) as i128;

    let close_amount: u64 = if current_value == 0 {
        abs_position
    } else {
        let equity_bps: i128 = equity * 10000 / current_value as i128;
        let reward_bps: i128 = LIQUIDATOR_REWARD_BPS as i128;

        let f_numerator: i128 = target_bps - equity_bps;
        let f_denominator: i128 = target_bps - reward_bps;

        if f_denominator <= 0 || f_numerator <= 0 {
            // f_denominator <= 0: target_bps <= reward_bps (unusual config), do full
            // f_numerator <= 0: equity already above target after order cancel, do full
            if equity < required_maintenance as i128 {
                abs_position
            } else {
                0 // Orders being cancelled was enough
            }
        } else {
            // f = f_numerator / f_denominator, close_amount = ceil(f * abs_position)
            let close: u128 = (f_numerator as u128 * abs_position as u128
                + f_denominator as u128
                - 1)
                / f_denominator as u128;
            (close.min(abs_position as u128)) as u64
        }
    };

    // If close_amount is 0, orders being cancelled was sufficient
    if close_amount == 0 {
        return Ok(());
    }

    // Round up to full liquidation if remaining position would be dust
    let close_amount: u64 = if abs_position.saturating_sub(close_amount) < MIN_POSITION_SIZE_ATOMS {
        abs_position
    } else {
        close_amount
    };

    let is_full_liquidation: bool = close_amount >= abs_position;

    // Proportional cost basis for the closed portion
    let closed_cost_basis: u64 = if is_full_liquidation {
        quote_cost_basis
    } else {
        ((quote_cost_basis as u128 * close_amount as u128) / abs_position as u128) as u64
    };

    // Compute notional of the closed portion
    let closed_notional: u64 = mark_price
        .checked_quote_for_base(BaseAtoms::new(close_amount), false)?
        .as_u64();

    // PnL on the closed portion (use i128 to avoid overflow)
    let closed_pnl: i128 = if position_size > 0 {
        (closed_notional as i128) - (closed_cost_basis as i128)
    } else {
        (closed_cost_basis as i128) - (closed_notional as i128)
    };

    // Liquidator reward = % of closed notional (always incentivizes liquidation)
    let liquidator_reward: u64 = closed_notional
        .checked_mul(LIQUIDATOR_REWARD_BPS)
        .unwrap_or(0)
        / 10000;

    // Settlement: apply PnL to margin, deduct reward
    let margin_after_pnl: i128 = margin_balance as i128 + closed_pnl;
    let margin_after_reward: i128 = margin_after_pnl - liquidator_reward as i128;

    // Insurance fund draw: if margin goes negative, there's bad debt
    let (final_trader_margin, actual_liquidator_reward) = if margin_after_reward >= 0 {
        (margin_after_reward as u64, liquidator_reward)
    } else {
        // Bad debt scenario
        let deficit: u64 = (-margin_after_reward) as u64;
        let drawn = dynamic_account.fixed.draw_from_insurance_fund(deficit);
        if drawn >= deficit {
            // Insurance fund fully covers the deficit
            (0u64, liquidator_reward)
        } else {
            // Insurance fund insufficient; reduce liquidator reward
            let remaining_deficit = deficit - drawn;
            let adjusted_reward = liquidator_reward.saturating_sub(remaining_deficit);
            (0u64, adjusted_reward)
        }
    };

    // Update trader's seat
    {
        let claimed_seat_mut: &mut ClaimedSeat = get_mut_helper::<RBNode<ClaimedSeat>>(
            &mut dynamic_account.dynamic,
            trader_index,
        )
        .get_mut_value();

        if is_full_liquidation {
            claimed_seat_mut.set_position_size(0);
            claimed_seat_mut.set_quote_cost_basis(0);
        } else {
            let new_position: i64 = if position_size > 0 {
                position_size - close_amount as i64
            } else {
                position_size + close_amount as i64
            };
            claimed_seat_mut.set_position_size(new_position);
            claimed_seat_mut.set_quote_cost_basis(
                quote_cost_basis.saturating_sub(closed_cost_basis),
            );
        }

        claimed_seat_mut.quote_withdrawable_balance = QuoteAtoms::new(final_trader_margin);
    }

    // Credit liquidator reward (liquidator must have a seat)
    if actual_liquidator_reward > 0 {
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
                QuoteAtoms::new(current.saturating_add(actual_liquidator_reward));
        }
    }

    // Update global position tracking
    #[cfg(not(feature = "certora"))]
    {
        if position_size > 0 {
            let current = dynamic_account.fixed.get_total_long_base_atoms();
            dynamic_account
                .fixed
                .set_total_long_base_atoms(current.saturating_sub(close_amount));
        } else {
            let current = dynamic_account.fixed.get_total_short_base_atoms();
            dynamic_account
                .fixed
                .set_total_short_base_atoms(current.saturating_sub(close_amount));
        }
    }

    // Store current global cumulative funding checkpoint for both trader and liquidator.
    dynamic_account.store_cumulative_for_trader(trader_index);
    {
        let liquidator_index: DataIndex = dynamic_account.get_trader_index(liquidator.key);
        if liquidator_index != hypertree::NIL {
            dynamic_account.store_cumulative_for_trader(liquidator_index);
        }
    }

    emit_stack(LiquidateLog {
        market: *market.key,
        liquidator: *liquidator.key,
        trader: params.trader_to_liquidate,
        position_size: abs_position,
        settlement_price: current_value,
        pnl: closed_pnl as i64 as u64,
        close_amount,
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
        // Use midpoint of bid and ask for a fair mark price
        let bid_inner = crate::quantities::u64_slice_to_u128(best_bid.get_price().inner);
        let ask_inner = crate::quantities::u64_slice_to_u128(best_ask.get_price().inner);
        let mid_inner = (bid_inner / 2) + (ask_inner / 2) + ((bid_inner % 2 + ask_inner % 2) / 2);
        Ok(QuoteAtomsPerBaseAtom { inner: [mid_inner as u64, (mid_inner >> 64) as u64] })
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
