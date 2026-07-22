#[cfg(not(feature = "certora"))]
mod free_addr_helpers {
    use crate::state::market::{MarketFixed, MarketUnusedFreeListPadding};
    use hypertree::{DataIndex, FreeList};

    pub fn get_free_address_on_market_fixed(
        fixed: &mut MarketFixed,
        dynamic: &mut [u8],
    ) -> DataIndex {
        let mut free_list: FreeList<MarketUnusedFreeListPadding> =
            FreeList::new(dynamic, fixed.free_list_head_index);
        let free_address: DataIndex = free_list.remove();
        fixed.free_list_head_index = free_list.get_head();
        free_address
    }

    pub fn get_free_address_on_market_fixed_for_seat(
        fixed: &mut MarketFixed,
        dynamic: &mut [u8],
    ) -> DataIndex {
        get_free_address_on_market_fixed(fixed, dynamic)
    }

    pub fn get_free_address_on_market_fixed_for_bid_order(
        fixed: &mut MarketFixed,
        dynamic: &mut [u8],
    ) -> DataIndex {
        get_free_address_on_market_fixed(fixed, dynamic)
    }

    pub fn get_free_address_on_market_fixed_for_ask_order(
        fixed: &mut MarketFixed,
        dynamic: &mut [u8],
    ) -> DataIndex {
        get_free_address_on_market_fixed(fixed, dynamic)
    }

    pub fn release_address_on_market_fixed(
        fixed: &mut MarketFixed,
        dynamic: &mut [u8],
        index: DataIndex,
    ) {
        let mut free_list: FreeList<MarketUnusedFreeListPadding> =
            FreeList::new(dynamic, fixed.free_list_head_index);
        free_list.add(index);
        fixed.free_list_head_index = index;
    }

    pub fn release_address_on_market_fixed_for_seat(
        fixed: &mut MarketFixed,
        dynamic: &mut [u8],
        index: DataIndex,
    ) {
        release_address_on_market_fixed(fixed, dynamic, index);
    }

    pub fn release_address_on_market_fixed_for_bid_order(
        fixed: &mut MarketFixed,
        dynamic: &mut [u8],
        index: DataIndex,
    ) {
        release_address_on_market_fixed(fixed, dynamic, index);
    }

    pub fn release_address_on_market_fixed_for_ask_order(
        fixed: &mut MarketFixed,
        dynamic: &mut [u8],
        index: DataIndex,
    ) {
        release_address_on_market_fixed(fixed, dynamic, index);
    }
}

#[cfg(feature = "certora")]
mod free_addr_helpers {
    use crate::state::market::MarketFixed;

    use super::{is_main_seat_free, is_second_seat_free, main_trader_index, second_trader_index};
    use hypertree::DataIndex;

    pub fn get_free_address_on_market_fixed_for_seat(
        _fixed: &mut MarketFixed,
        _dynamic: &mut [u8],
    ) -> DataIndex {
        // -- return index of the first available trader
        if is_main_seat_free() {
            main_trader_index()
        } else if is_second_seat_free() {
            second_trader_index()
        } else {
            cvt::cvt_assert!(false);
            crate::state::market::NIL
        }
    }

    pub fn get_free_address_on_market_fixed_for_bid_order(
        _fixed: &mut MarketFixed,
        _dynamic: &mut [u8],
    ) -> DataIndex {
        if super::is_bid_order_free() {
            super::main_bid_order_index()
        } else {
            cvt::cvt_assert!(false);
            super::NIL
        }
    }

    pub fn get_free_address_on_market_fixed_for_ask_order(
        _fixed: &mut MarketFixed,
        _dynamic: &mut [u8],
    ) -> DataIndex {
        if super::is_ask_order_free() {
            super::main_ask_order_index()
        } else {
            cvt::cvt_assert!(false);
            super::NIL
        }
    }

    pub fn release_address_on_market_fixed_for_seat(
        _fixed: &mut MarketFixed,
        _dynamic: &mut [u8],
        _index: DataIndex,
    ) {
    }

    pub fn release_address_on_market_fixed_for_bid_order(
        _fixed: &mut MarketFixed,
        _dynamic: &mut [u8],
        _index: DataIndex,
    ) {
    }

    pub fn release_address_on_market_fixed_for_ask_order(
        _fixed: &mut MarketFixed,
        _dynamic: &mut [u8],
        _index: DataIndex,
    ) {
    }
}

pub use free_addr_helpers::*;

// Refactoring of place_order

use super::*;
use crate::state::utils::{transfer_global_tokens, try_to_reduce_global_tokens};

/// Size a reverse bid being coalesced into an existing order so that the
/// existing order's allocation does not grow by more quote atoms than the
/// maker received from the fill.
pub(super) fn get_reverse_bid_coalesce_amounts(
    price: QuoteAtomsPerBaseAtom,
    old_base_atoms: BaseAtoms,
    requested_base_atoms: BaseAtoms,
    quote_atoms_received: QuoteAtoms,
) -> Result<(BaseAtoms, QuoteAtoms), ProgramError> {
    let previous_quote_allocated: QuoteAtoms =
        price.checked_quote_for_base(old_base_atoms, true)?;
    let requested_new_base_atoms: BaseAtoms = old_base_atoms.checked_add(requested_base_atoms)?;
    let requested_new_quote_allocated: QuoteAtoms =
        price.checked_quote_for_base(requested_new_base_atoms, true)?;
    let requested_quote_debit: QuoteAtoms =
        requested_new_quote_allocated.checked_sub(previous_quote_allocated)?;

    if requested_quote_debit <= quote_atoms_received {
        return Ok((requested_base_atoms, requested_quote_debit));
    }

    // This addition cannot overflow in this branch: requested allocation is a
    // u64 and is greater than previous allocation + received quote.
    let affordable_total_quote: QuoteAtoms =
        previous_quote_allocated.checked_add(quote_atoms_received)?;
    let affordable_total_base: BaseAtoms =
        price.checked_base_for_quote(affordable_total_quote, false)?;
    let base_atoms_to_add: BaseAtoms = affordable_total_base
        .checked_sub(old_base_atoms)?
        .min(requested_base_atoms);
    let new_quote_allocated: QuoteAtoms =
        price.checked_quote_for_base(old_base_atoms.checked_add(base_atoms_to_add)?, true)?;

    Ok((
        base_atoms_to_add,
        new_quote_allocated.checked_sub(previous_quote_allocated)?,
    ))
}

#[derive(Default, PartialEq, Clone, Copy)]
pub enum AddOrderStatus {
    #[default]
    Canceled,
    Filled,
    PartialFill,
    Unmatched,
    /// The maker was a global order that could not cover the trade, so it was
    /// removed from the book without trading.
    GlobalSkip,
    /// The maker was a global order but the global accounts were not passed in,
    /// so matching stops here.
    GlobalMissing,
}

#[derive(Default)]
pub struct AddOrderToMarketInnerResult {
    pub next_order_index: DataIndex,
    pub status: AddOrderStatus,
}

pub struct AddSingleOrderCtx<'a, 'b, 'info> {
    pub args: AddOrderToMarketArgs<'b, 'info>,
    fixed: &'a mut MarketFixed,
    dynamic: &'a mut [u8],
    pub now_slot: u32,
    pub remaining_base_atoms: BaseAtoms,
    pub total_base_atoms_traded: BaseAtoms,
    pub total_quote_atoms_traded: QuoteAtoms,
    pub global_atoms_to_transfer: GlobalAtoms,
    /// The taker's sequence number, claimed before matching so that the fill
    /// logs are correct even if sequence numbers are consumed by reverse
    /// orders during matching. Mirrors `Market::place_order`.
    pub this_order_sequence_number: u64,
}

impl<'a, 'b, 'info> AddSingleOrderCtx<'a, 'b, 'info> {
    pub fn new(
        args: AddOrderToMarketArgs<'b, 'info>,
        fixed: &'a mut MarketFixed,
        dynamic: &'a mut [u8],
        remaining_base_atoms: BaseAtoms,
        now_slot: u32,
    ) -> Self {
        // Claim this_order_sequence_number for the current order before any
        // matching happens, exactly as Market::place_order does.
        let this_order_sequence_number: u64 = fixed.order_sequence_number;
        fixed.order_sequence_number = this_order_sequence_number.wrapping_add(1);
        Self {
            args,
            fixed,
            dynamic,
            now_slot,
            remaining_base_atoms,
            total_base_atoms_traded: BaseAtoms::ZERO,
            total_quote_atoms_traded: QuoteAtoms::ZERO,
            global_atoms_to_transfer: GlobalAtoms::ZERO,
            this_order_sequence_number,
        }
    }
    /// One iteration of the matching loop in `Market::place_order`. Kept
    /// separate so that formal verification can reason about a single step
    /// instead of the whole loop. It must stay behaviourally identical to the
    /// body of that loop.
    pub fn place_single_order(
        &mut self,
        current_order_index: DataIndex,
    ) -> Result<AddOrderToMarketInnerResult, ProgramError> {
        let this_order_sequence_number: u64 = self.this_order_sequence_number;
        let fixed: &mut _ = self.fixed;
        let dynamic: &mut _ = self.dynamic;
        let now_slot = self.now_slot;
        let remaining_base_atoms = self.remaining_base_atoms;

        let AddOrderToMarketArgs {
            market,
            trader_index,
            num_base_atoms: _,
            price,
            is_bid,
            last_valid_slot: _,
            order_type,
            global_trade_accounts_opts,
            current_slot: _,
        } = self.args;

        let next_order_index: DataIndex =
            get_next_candidate_match_index(fixed, dynamic, current_order_index, is_bid);

        let other_order: &RestingOrder = get_helper_order(dynamic, current_order_index).get_value();

        // Remove the resting order if expired or somehow a zero order got on the book.
        if other_order.is_expired(now_slot) || other_order.get_num_base_atoms() == BaseAtoms::ZERO {
            remove_and_update_balances(
                fixed,
                dynamic,
                current_order_index,
                global_trade_accounts_opts,
            )?;
            return Ok(AddOrderToMarketInnerResult {
                next_order_index,
                status: AddOrderStatus::Canceled,
            });
        }

        // Stop trying to match if price no longer satisfies limit.
        if (is_bid && other_order.get_price() > price)
            || (!is_bid && other_order.get_price() < price)
        {
            return Ok(AddOrderToMarketInnerResult {
                next_order_index: NIL,
                status: AddOrderStatus::Unmatched,
            });
        }

        // Got a match. First make sure we are allowed to match. We check
        // inside the matching rather than skipping the matching altogether
        // because post only orders should fail, not produce a crossed book.
        trace!(
            "match {} {order_type:?} {price:?} with {other_order:?}",
            if is_bid { "bid" } else { "ask" }
        );
        assert_can_take(order_type)?;

        let maker_sequence_number: u64 = other_order.get_sequence_number();
        let maker_trader_index: DataIndex = other_order.get_trader_index();
        let did_fully_match_resting_order: bool =
            remaining_base_atoms >= other_order.get_num_base_atoms();
        let base_atoms_traded: BaseAtoms = if did_fully_match_resting_order {
            other_order.get_num_base_atoms()
        } else {
            remaining_base_atoms
        };

        let matched_price: QuoteAtomsPerBaseAtom = other_order.get_price();
        let maker_order_type: OrderType = other_order.get_order_type();
        let maker_price_reverse: Result<QuoteAtomsPerBaseAtom, _> = other_order.reverse_price();
        let is_global: bool = other_order.is_global();
        let is_maker_reverse: bool = other_order.is_reversible();
        let maker_reverse_spread: u16 = other_order.get_reverse_spread();

        // on full fill: round in favor of the taker
        // on partial fill: round in favor of the maker
        let quote_atoms_traded: QuoteAtoms = matched_price
            .checked_quote_for_base(base_atoms_traded, is_bid != did_fully_match_resting_order)?;

        // If it is a global order, just in time bring the funds over, or
        // remove from the tree and continue on to the next order.
        let maker: Pubkey = get_helper_seat(dynamic, maker_trader_index)
            .get_value()
            .trader;
        let taker: Pubkey = get_helper_seat(dynamic, trader_index).get_value().trader;

        if is_global {
            let global_trade_accounts_opt: &Option<GlobalTradeAccounts> = if is_bid {
                &global_trade_accounts_opts[0]
            } else {
                &global_trade_accounts_opts[1]
            };
            // When the global account is not included, a taker order can halt
            // here, but a possible maker order will need to crash since that
            // would result in a crossed book.
            if global_trade_accounts_opt.is_none() {
                if order_type_can_rest(order_type) {
                    return Err(ManifestError::MissingGlobal.into());
                }
                return Ok(AddOrderToMarketInnerResult {
                    next_order_index: NIL,
                    status: AddOrderStatus::GlobalMissing,
                });
            }
            // When is_bid, the taker is supplying quote, so the global maker
            // needs to supply base.
            let global_atoms_needed: GlobalAtoms = GlobalAtoms::new(if is_bid {
                base_atoms_traded.as_u64()
            } else {
                quote_atoms_traded.as_u64()
            });
            let has_enough_tokens: bool = try_to_reduce_global_tokens(
                global_trade_accounts_opt,
                &maker,
                global_atoms_needed,
            )?;
            if !has_enough_tokens {
                remove_and_update_balances(
                    fixed,
                    dynamic,
                    current_order_index,
                    global_trade_accounts_opts,
                )?;
                return Ok(AddOrderToMarketInnerResult {
                    next_order_index,
                    status: AddOrderStatus::GlobalSkip,
                });
            }
            // Accumulate for batch transfer after matching completes
            self.global_atoms_to_transfer = self
                .global_atoms_to_transfer
                .checked_add(global_atoms_needed)?;
        }

        self.total_base_atoms_traded = self
            .total_base_atoms_traded
            .checked_add(base_atoms_traded)?;
        self.total_quote_atoms_traded = self
            .total_quote_atoms_traded
            .checked_add(quote_atoms_traded)?;

        // Possibly increase bonus atom maker gets from the rounding the
        // quote in their favor. They will get one less than expected when
        // cancelling because of rounding, this counters that. This ensures
        // that the amount of quote that the maker has credit for when they
        // cancel/expire is always the maximum amount that could have been
        // used in matching that order.
        // Example:
        // Maker deposits 11            | Balance: 0 base 11 quote | Orders: []
        // Maker bid for 10@1.15        | Balance: 0 base 0 quote  | Orders: [bid 10@1.15]
        // Swap    5 base <--> 5 quote  | Balance: 5 base 0 quote  | Orders: [bid 5@1.15]
        //     <this code block>        | Balance: 5 base 1 quote  | Orders: [bid 5@1.15]
        // Maker cancel                 | Balance: 5 base 6 quote  | Orders: []
        //
        // The swapper deposited 5 base and withdrew 5 quote. The maker deposited 11 quote.
        // If we didnt do this adjustment, there would be an unaccounted for
        // quote atom.
        // Note that we do not have to do this on the other direction
        // because the amount of atoms that a maker needs to support an ask
        // is exact. The rounding is always on quote.
        //
        // Do not credit the bonus atom on global orders. Only the atoms
        // required for the trade were brought over from the global account, so
        // there is no spare atom on the market to credit.
        if !is_bid && !is_global {
            // These are only used when is_bid, included up here for borrow checker reasons.
            let other_order: &RestingOrder =
                get_helper_order(dynamic, current_order_index).get_value();
            let previous_maker_quote_atoms_allocated: QuoteAtoms =
                matched_price.checked_quote_for_base(other_order.get_num_base_atoms(), true)?;
            let new_maker_quote_atoms_allocated: QuoteAtoms = matched_price
                .checked_quote_for_base(
                    other_order
                        .get_num_base_atoms()
                        .checked_sub(base_atoms_traded)?,
                    true,
                )?;
            update_balance(
                fixed,
                dynamic,
                maker_trader_index,
                is_bid,
                true,
                (previous_maker_quote_atoms_allocated
                    .checked_sub(new_maker_quote_atoms_allocated)?
                    .checked_sub(quote_atoms_traded)?)
                .as_u64(),
            )?;
        }

        // Certora : the manifest code first increased the maker for the matched amount,
        // then decreased the taker. This causes an overflow on withdrawable_balances.
        // Thus, we changed it to first decrease the taker, and then increase the maker.

        // Decrease taker
        update_balance(
            fixed,
            dynamic,
            trader_index,
            !is_bid,
            false,
            if is_bid {
                quote_atoms_traded.into()
            } else {
                base_atoms_traded.into()
            },
        )?;
        // Increase maker from the matched amount in the trade.
        update_balance(
            fixed,
            dynamic,
            maker_trader_index,
            !is_bid,
            true,
            if is_bid {
                quote_atoms_traded.into()
            } else {
                base_atoms_traded.into()
            },
        )?;
        // Increase taker
        update_balance(
            fixed,
            dynamic,
            trader_index,
            is_bid,
            true,
            if is_bid {
                base_atoms_traded.into()
            } else {
                quote_atoms_traded.into()
            },
        )?;

        // record maker & taker volume
        record_volume_by_trader_index(dynamic, maker_trader_index, quote_atoms_traded);
        record_volume_by_trader_index(dynamic, trader_index, quote_atoms_traded);

        emit_stack(FillLog {
            market,
            maker,
            taker,
            base_atoms: base_atoms_traded,
            quote_atoms: quote_atoms_traded,
            price: matched_price,
            maker_sequence_number,
            taker_sequence_number: this_order_sequence_number,
            taker_is_buy: PodBool::from(is_bid),
            base_mint: *fixed.get_base_mint(),
            quote_mint: *fixed.get_quote_mint(),
            is_maker_global: PodBool::from(is_global),
            _padding: [0; 14],
        })?;

        let status: AddOrderStatus = if did_fully_match_resting_order {
            // Get paid for removing a global order.
            if is_global {
                if is_bid {
                    remove_from_global(&global_trade_accounts_opts[0])?;
                } else {
                    remove_from_global(&global_trade_accounts_opts[1])?;
                }
            }

            remove_order_from_tree_and_free(fixed, dynamic, current_order_index, !is_bid)?;
            self.remaining_base_atoms = self.remaining_base_atoms.checked_sub(base_atoms_traded)?;
            AddOrderStatus::Filled
        } else {
            #[cfg(feature = "certora")]
            remove_from_orderbook_balance(fixed, dynamic, current_order_index);
            let other_order: &mut RestingOrder =
                get_mut_helper_order(dynamic, current_order_index).get_mut_value();
            other_order.reduce(base_atoms_traded)?;
            #[cfg(feature = "certora")]
            add_to_orderbook_balance(fixed, dynamic, current_order_index);
            self.remaining_base_atoms = BaseAtoms::ZERO;
            AddOrderStatus::PartialFill
        };

        // Place the reverse order if the maker was a reverse order type. This
        // is non-trivial because in order to prevent tons of orders filling the
        // books on partial fills, we coalesce on top of book.
        if is_maker_reverse {
            if let Ok(price_reverse) = maker_price_reverse {
                place_reverse_order(
                    fixed,
                    dynamic,
                    maker_trader_index,
                    maker_order_type,
                    maker_reverse_spread,
                    price_reverse,
                    base_atoms_traded,
                    quote_atoms_traded,
                    is_bid,
                )?;
            }
        }

        Ok(AddOrderToMarketInnerResult {
            next_order_index: if status == AddOrderStatus::Filled {
                next_order_index
            } else {
                NIL
            },
            status,
        })
    }
}

/// Put the maker of a filled reverse order back on the other side of the book,
/// coalescing into an existing order at the same price when there is one.
/// Mirrors the reverse block of `Market::place_order`.
#[allow(clippy::too_many_arguments)]
fn place_reverse_order(
    fixed: &mut MarketFixed,
    dynamic: &mut [u8],
    maker_trader_index: DataIndex,
    maker_order_type: OrderType,
    maker_reverse_spread: u16,
    price_reverse: QuoteAtomsPerBaseAtom,
    base_atoms_traded: BaseAtoms,
    quote_atoms_traded: QuoteAtoms,
    is_bid: bool,
) -> ProgramResult {
    let num_base_atoms_reverse: BaseAtoms = if is_bid {
        // Maker is now buying with the exact number of quote atoms. Do not
        // round_up because there might not be enough atoms for that.
        price_reverse.checked_base_for_quote(quote_atoms_traded, false)?
    } else {
        base_atoms_traded
    };

    let mut coalesced: bool = false;
    // Quote atoms the maker pays for the reverse bid. See the twin block in
    // Market::place_order: on coalesce, this is the exact growth of the
    // coalesced order's backing at that order's own price, not
    // num_base_atoms_reverse * price_reverse.
    let mut reverse_quote_atoms_debited: QuoteAtoms = QuoteAtoms::ZERO;
    {
        let other_tree: Bookside = if is_bid {
            Bookside::new(dynamic, fixed.bids_root_index, fixed.bids_best_index)
        } else {
            Bookside::new(dynamic, fixed.asks_root_index, fixed.asks_best_index)
        };
        let lookup_resting_order: RestingOrder = RestingOrder::new(
            maker_trader_index,
            BaseAtoms::ZERO, // Size does not matter, just price.
            price_reverse,
            0, // Sequence number does not matter, just price
            NO_EXPIRATION_LAST_VALID_SLOT,
            is_bid,
            maker_order_type,
        )?;

        let lookup_index: DataIndex = other_tree.lookup_index(&lookup_resting_order);
        if lookup_index != NIL {
            #[cfg(feature = "certora")]
            remove_from_orderbook_balance(fixed, dynamic, lookup_index);
            let order_to_coalesce_into: &mut RestingOrder =
                get_mut_helper_order(dynamic, lookup_index).get_mut_value();
            if is_bid {
                let (base_atoms_to_add, quote_atoms_to_debit) = get_reverse_bid_coalesce_amounts(
                    order_to_coalesce_into.get_price(),
                    order_to_coalesce_into.get_num_base_atoms(),
                    num_base_atoms_reverse,
                    quote_atoms_traded,
                )?;
                order_to_coalesce_into.increase(base_atoms_to_add)?;
                reverse_quote_atoms_debited = quote_atoms_to_debit;
            } else {
                order_to_coalesce_into.increase(num_base_atoms_reverse)?;
            }
            #[cfg(feature = "certora")]
            add_to_orderbook_balance(fixed, dynamic, lookup_index);
            coalesced = true;
        }
    }
    if !coalesced && is_bid {
        reverse_quote_atoms_debited = num_base_atoms_reverse.checked_mul(price_reverse, true)?;
    }

    // If there was 1 atom and because taker rounding is in effect, then this
    // would result in an empty order.
    if !coalesced && num_base_atoms_reverse > BaseAtoms::ZERO {
        let reverse_order_sequence_number: u64 = fixed.order_sequence_number;
        fixed.order_sequence_number = reverse_order_sequence_number.wrapping_add(1);

        let free_address: DataIndex = if is_bid {
            get_free_address_on_market_fixed_for_bid_order(fixed, dynamic)
        } else {
            get_free_address_on_market_fixed_for_ask_order(fixed, dynamic)
        };

        let mut new_reverse_resting_order: RestingOrder = RestingOrder::new(
            maker_trader_index,
            num_base_atoms_reverse,
            price_reverse,
            reverse_order_sequence_number,
            // Does not expire.
            NO_EXPIRATION_LAST_VALID_SLOT,
            is_bid,
            maker_order_type,
        )?;
        new_reverse_resting_order.set_reverse_spread(maker_reverse_spread);
        insert_order_into_tree(
            is_bid,
            fixed,
            dynamic,
            free_address,
            &new_reverse_resting_order,
        );
        set_payload_order(dynamic, free_address);
    }

    update_balance(
        fixed,
        dynamic,
        maker_trader_index,
        !is_bid,
        false,
        if is_bid {
            reverse_quote_atoms_debited.into()
        } else {
            num_base_atoms_reverse.into()
        },
    )?;

    Ok(())
}

pub fn place_order_helper<
    Fixed: DerefOrBorrowMut<MarketFixed> + DerefOrBorrow<MarketFixed>,
    Dynamic: DerefOrBorrowMut<[u8]> + DerefOrBorrow<[u8]>,
>(
    self_: &mut DynamicAccount<Fixed, Dynamic>,
    args: AddOrderToMarketArgs,
) -> Result<AddOrderToMarketResult, ProgramError> {
    let AddOrderToMarketArgs {
        market: _,
        trader_index,
        num_base_atoms,
        price,
        is_bid,
        last_valid_slot,
        order_type,
        global_trade_accounts_opts: _,
        current_slot,
    } = args;
    assert_already_has_seat(trader_index)?;
    let now_slot: u32 = current_slot.unwrap_or_else(|| get_now_slot());

    // Reverse orders will have their last valid slot overriden to no expiration.
    if !order_type.is_reversible() {
        assert_not_already_expired(last_valid_slot, now_slot)?;
    }

    let DynamicAccount { fixed, dynamic } = self_.borrow_mut();

    let mut current_order_index: DataIndex = if is_bid {
        fixed.asks_best_index
    } else {
        fixed.bids_best_index
    };

    let mut total_base_atoms_traded: BaseAtoms = BaseAtoms::ZERO;
    let mut total_quote_atoms_traded: QuoteAtoms = QuoteAtoms::ZERO;

    let mut remaining_base_atoms: BaseAtoms = num_base_atoms;

    let mut ctx: AddSingleOrderCtx =
        AddSingleOrderCtx::new(args, fixed, dynamic, remaining_base_atoms, now_slot);

    while remaining_base_atoms > BaseAtoms::ZERO && is_not_nil!(current_order_index) {
        // one step of placing an order
        let AddOrderToMarketInnerResult {
            next_order_index,
            status,
        } = ctx.place_single_order(current_order_index)?;

        // update global state based on the context
        // this ensures that each iteration of the loop updates all
        // variables in scope just as it did originally.
        current_order_index = next_order_index;
        remaining_base_atoms = ctx.remaining_base_atoms;
        total_base_atoms_traded = ctx.total_base_atoms_traded;
        total_quote_atoms_traded = ctx.total_quote_atoms_traded;

        if status == AddOrderStatus::Unmatched {
            break;
        } else if status == AddOrderStatus::PartialFill {
            break;
        }
    }

    // Batch transfer global tokens after all matching is complete
    let global_atoms_to_transfer: GlobalAtoms = ctx.global_atoms_to_transfer;
    if global_atoms_to_transfer > GlobalAtoms::ZERO {
        let global_trade_accounts_opt: &Option<GlobalTradeAccounts> = if is_bid {
            &ctx.args.global_trade_accounts_opts[0]
        } else {
            &ctx.args.global_trade_accounts_opts[1]
        };
        transfer_global_tokens(global_trade_accounts_opt, global_atoms_to_transfer)?;
    }

    // move out args so that they can be used later. The taker's sequence
    // number was claimed by AddSingleOrderCtx::new before matching started.
    let this_order_sequence_number: u64 = ctx.this_order_sequence_number;
    let args: AddOrderToMarketArgs = ctx.args;
    // ctx is dead from this point onward

    // Record volume on market
    fixed.quote_volume = fixed.quote_volume.wrapping_add(total_quote_atoms_traded);

    // If there is nothing left to rest, then return before resting.
    if !order_type_can_rest(order_type)
        || remaining_base_atoms == BaseAtoms::ZERO
        || price == QuoteAtomsPerBaseAtom::ZERO
    {
        return Ok(AddOrderToMarketResult {
            order_sequence_number: this_order_sequence_number,
            order_index: NIL,
            base_atoms_traded: total_base_atoms_traded,
            quote_atoms_traded: total_quote_atoms_traded,
        });
    }

    self_.rest_remaining(
        args,
        remaining_base_atoms,
        this_order_sequence_number,
        total_base_atoms_traded,
        total_quote_atoms_traded,
    )
}

/// Differential check that `place_order_helper` -- the formal-verification
/// model whose loop body, `place_single_order`, has to be kept behaviourally
/// identical to the loop in `Market::place_order` by hand -- actually agrees
/// with `Market::place_order`. Every scenario runs the same taker order
/// through both implementations on identical markets and compares the result
/// and every byte of market state, including multi-level matching, which the
/// prover's one-order-per-side mock book cannot represent.
#[cfg(all(test, not(feature = "certora")))]
mod place_order_equivalence_tests {
    use super::*;
    use crate::state::{
        market::{create_empty_market, MarketValue},
        OrderType, MARKET_BLOCK_SIZE, NO_EXPIRATION_LAST_VALID_SLOT,
    };
    use solana_program::pubkey::Pubkey;

    const NOW_SLOT: u32 = 100;

    fn new_market_with_seats() -> (MarketValue, DataIndex, DataIndex, Pubkey, Pubkey) {
        let market_key: Pubkey = Pubkey::new_unique();
        let mint_authority: Pubkey = Pubkey::new_unique();
        let fixed: MarketFixed = create_empty_market(
            "So11111111111111111111111111111111111111112",
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            9,
            6,
            &mint_authority,
            &market_key,
        );
        let mut market: MarketValue = MarketValue {
            fixed,
            dynamic: vec![0; MARKET_BLOCK_SIZE * 32],
        };
        for _ in 0..32 {
            market.market_expand().unwrap();
        }

        let maker_pk: Pubkey = Pubkey::new_unique();
        let taker_pk: Pubkey = Pubkey::new_unique();
        market.claim_seat(&maker_pk).unwrap();
        market.claim_seat(&taker_pk).unwrap();
        let maker_index: DataIndex = market.get_trader_index(&maker_pk);
        let taker_index: DataIndex = market.get_trader_index(&taker_pk);
        for (index, is_base) in [
            (maker_index, true),
            (maker_index, false),
            (taker_index, true),
            (taker_index, false),
        ] {
            market.deposit(index, 1_000_000_000_000, is_base).unwrap();
        }
        (market, maker_index, taker_index, maker_pk, taker_pk)
    }

    fn place(
        market: &mut MarketValue,
        trader_index: DataIndex,
        num_base_atoms: u64,
        price: f64,
        is_bid: bool,
        order_type: OrderType,
        last_valid_slot: u32,
        current_slot: u32,
    ) -> Result<AddOrderToMarketResult, ProgramError> {
        market.place_order(AddOrderToMarketArgs {
            market: Pubkey::new_unique(),
            trader_index,
            num_base_atoms: BaseAtoms::new(num_base_atoms),
            price: price.try_into().unwrap(),
            is_bid,
            last_valid_slot,
            order_type,
            global_trade_accounts_opts: &[None, None],
            current_slot: Some(current_slot),
        })
    }

    /// Runs the taker order through `place_order` on one copy of the market
    /// and through `place_order_` (the model) on another, and asserts they
    /// agree on the result and on every byte of market state.
    fn assert_equivalent_taker(
        market: &MarketValue,
        trader_index: DataIndex,
        num_base_atoms: u64,
        price: f64,
        is_bid: bool,
        order_type: OrderType,
        last_valid_slot: u32,
    ) {
        let market_key: Pubkey = Pubkey::new_unique();
        let mut production: MarketValue = MarketValue {
            fixed: market.fixed,
            dynamic: market.dynamic.clone(),
        };
        let mut model: MarketValue = MarketValue {
            fixed: market.fixed,
            dynamic: market.dynamic.clone(),
        };

        let production_result: Result<AddOrderToMarketResult, ProgramError> = production
            .place_order(AddOrderToMarketArgs {
                market: market_key,
                trader_index,
                num_base_atoms: BaseAtoms::new(num_base_atoms),
                price: price.try_into().unwrap(),
                is_bid,
                last_valid_slot,
                order_type,
                global_trade_accounts_opts: &[None, None],
                current_slot: Some(NOW_SLOT),
            });
        let model_result: Result<AddOrderToMarketResult, ProgramError> =
            model.place_order_(AddOrderToMarketArgs {
                market: market_key,
                trader_index,
                num_base_atoms: BaseAtoms::new(num_base_atoms),
                price: price.try_into().unwrap(),
                is_bid,
                last_valid_slot,
                order_type,
                global_trade_accounts_opts: &[None, None],
                current_slot: Some(NOW_SLOT),
            });

        match (production_result, model_result) {
            (Ok(production_result), Ok(model_result)) => {
                assert_eq!(
                    production_result.order_sequence_number, model_result.order_sequence_number,
                    "order_sequence_number diverged"
                );
                assert_eq!(
                    production_result.order_index, model_result.order_index,
                    "order_index diverged"
                );
                assert_eq!(
                    production_result.base_atoms_traded, model_result.base_atoms_traded,
                    "base_atoms_traded diverged"
                );
                assert_eq!(
                    production_result.quote_atoms_traded, model_result.quote_atoms_traded,
                    "quote_atoms_traded diverged"
                );
            }
            (Err(production_err), Err(model_err)) => {
                assert_eq!(production_err, model_err, "errors diverged");
            }
            (production_result, model_result) => {
                panic!(
                    "one implementation failed and the other did not: production ok={} model ok={}",
                    production_result.is_ok(),
                    model_result.is_ok()
                );
            }
        }

        assert_eq!(
            bytemuck::bytes_of(&production.fixed),
            bytemuck::bytes_of(&model.fixed),
            "market fixed state diverged"
        );
        assert_eq!(
            production.dynamic, model.dynamic,
            "market dynamic state diverged"
        );
    }

    /// Two ask levels, both consumed, the remainder rests.
    #[test]
    fn test_equivalence_multi_level_full_sweep_and_rest() {
        let (mut market, maker_index, taker_index, _, _) = new_market_with_seats();
        place(
            &mut market,
            maker_index,
            100,
            0.150,
            false,
            OrderType::Limit,
            NO_EXPIRATION_LAST_VALID_SLOT,
            NOW_SLOT,
        )
        .unwrap();
        place(
            &mut market,
            maker_index,
            100,
            0.180,
            false,
            OrderType::Limit,
            NO_EXPIRATION_LAST_VALID_SLOT,
            NOW_SLOT,
        )
        .unwrap();

        assert_equivalent_taker(
            &market,
            taker_index,
            250,
            0.200,
            true,
            OrderType::Limit,
            NO_EXPIRATION_LAST_VALID_SLOT,
        );
    }

    /// First level consumed, second level partially filled.
    #[test]
    fn test_equivalence_partial_fill_second_level() {
        let (mut market, maker_index, taker_index, _, _) = new_market_with_seats();
        place(
            &mut market,
            maker_index,
            100,
            0.150,
            false,
            OrderType::Limit,
            NO_EXPIRATION_LAST_VALID_SLOT,
            NOW_SLOT,
        )
        .unwrap();
        place(
            &mut market,
            maker_index,
            100,
            0.180,
            false,
            OrderType::Limit,
            NO_EXPIRATION_LAST_VALID_SLOT,
            NOW_SLOT,
        )
        .unwrap();

        assert_equivalent_taker(
            &market,
            taker_index,
            150,
            0.200,
            true,
            OrderType::Limit,
            NO_EXPIRATION_LAST_VALID_SLOT,
        );
    }

    /// The taker's bid does not cross the book, so it rests.
    #[test]
    fn test_equivalence_unmatched_rests() {
        let (mut market, maker_index, taker_index, _, _) = new_market_with_seats();
        place(
            &mut market,
            maker_index,
            100,
            0.150,
            false,
            OrderType::Limit,
            NO_EXPIRATION_LAST_VALID_SLOT,
            NOW_SLOT,
        )
        .unwrap();

        assert_equivalent_taker(
            &market,
            taker_index,
            100,
            0.100,
            true,
            OrderType::Limit,
            NO_EXPIRATION_LAST_VALID_SLOT,
        );
    }

    /// Immediate-or-cancel matches what it can and rests nothing.
    #[test]
    fn test_equivalence_ioc_partial() {
        let (mut market, maker_index, taker_index, _, _) = new_market_with_seats();
        place(
            &mut market,
            maker_index,
            100,
            0.150,
            false,
            OrderType::Limit,
            NO_EXPIRATION_LAST_VALID_SLOT,
            NOW_SLOT,
        )
        .unwrap();

        assert_equivalent_taker(
            &market,
            taker_index,
            150,
            0.150,
            true,
            OrderType::ImmediateOrCancel,
            NO_EXPIRATION_LAST_VALID_SLOT,
        );
    }

    /// A crossing post-only order fails identically in both implementations.
    #[test]
    fn test_equivalence_post_only_crossing() {
        let (mut market, maker_index, taker_index, _, _) = new_market_with_seats();
        place(
            &mut market,
            maker_index,
            100,
            0.150,
            false,
            OrderType::Limit,
            NO_EXPIRATION_LAST_VALID_SLOT,
            NOW_SLOT,
        )
        .unwrap();

        assert_equivalent_taker(
            &market,
            taker_index,
            100,
            0.200,
            true,
            OrderType::PostOnly,
            NO_EXPIRATION_LAST_VALID_SLOT,
        );
    }

    /// An expired maker on the first level is removed, the second level trades.
    #[test]
    fn test_equivalence_expired_maker_skipped() {
        let (mut market, maker_index, taker_index, _, _) = new_market_with_seats();
        // Expires at slot 10, taker arrives at NOW_SLOT = 100.
        place(
            &mut market,
            maker_index,
            100,
            0.150,
            false,
            OrderType::Limit,
            10,
            5,
        )
        .unwrap();
        place(
            &mut market,
            maker_index,
            100,
            0.180,
            false,
            OrderType::Limit,
            NO_EXPIRATION_LAST_VALID_SLOT,
            5,
        )
        .unwrap();

        assert_equivalent_taker(
            &market,
            taker_index,
            150,
            0.200,
            true,
            OrderType::Limit,
            NO_EXPIRATION_LAST_VALID_SLOT,
        );
    }

    /// Matching a reverse maker places its come-back order on the other side,
    /// which consumes a sequence number during matching. The taker's own
    /// sequence number and the resting state must still agree.
    #[test]
    fn test_equivalence_reverse_maker_come_back() {
        let (mut market, maker_index, taker_index, _, _) = new_market_with_seats();
        place(
            &mut market,
            maker_index,
            100,
            0.150,
            false,
            OrderType::Reverse,
            NO_EXPIRATION_LAST_VALID_SLOT,
            NOW_SLOT,
        )
        .unwrap();

        // Full match plus a remainder that rests, after the reverse come-back
        // consumed a sequence number.
        assert_equivalent_taker(
            &market,
            taker_index,
            150,
            0.150,
            true,
            OrderType::Limit,
            NO_EXPIRATION_LAST_VALID_SLOT,
        );
    }

    /// A zero-price order matches nothing and does not rest.
    #[test]
    fn test_equivalence_zero_price_returns_early() {
        let (market, _, taker_index, _, _) = new_market_with_seats();

        assert_equivalent_taker(
            &market,
            taker_index,
            100,
            0.0,
            true,
            OrderType::Limit,
            NO_EXPIRATION_LAST_VALID_SLOT,
        );
    }

    /// Reversible order types are exempt from the expiration check.
    #[test]
    fn test_equivalence_reverse_taker_expiration_exempt() {
        let (market, _, taker_index, _, _) = new_market_with_seats();

        // Expired for a limit order, but reverse orders skip the check.
        assert_equivalent_taker(
            &market,
            taker_index,
            100,
            0.150,
            true,
            OrderType::Reverse,
            10,
        );
    }
}
