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

#[derive(Default, PartialEq)]
pub enum AddOrderStatus {
    #[default]
    Canceled,
    Filled,
    PartialFill,
    Unmatched,
    GlobalSkip,
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
}

impl<'a, 'b, 'info> AddSingleOrderCtx<'a, 'b, 'info> {
    pub fn new(
        args: AddOrderToMarketArgs<'b, 'info>,
        fixed: &'a mut MarketFixed,
        dynamic: &'a mut [u8],
        remaining_base_atoms: BaseAtoms,
        now_slot: u32,
    ) -> Self {
        Self {
            args,
            fixed,
            dynamic,
            now_slot,
            remaining_base_atoms,
            total_base_atoms_traded: BaseAtoms::ZERO,
            total_quote_atoms_traded: QuoteAtoms::ZERO,
        }
    }
    // TODO: Clean this up or prove that it is the same as market::place_order
    pub fn place_single_order(
        &mut self,
        current_order_index: DataIndex,
    ) -> Result<AddOrderToMarketInnerResult, ProgramError> {
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

        // Remove the resting order if expired.
        if other_order.is_expired(now_slot) {
            remove_and_update_balances(
                fixed,
                dynamic,
                current_order_index,
                global_trade_accounts_opts,
            )?;
            return Ok(AddOrderToMarketInnerResult {
                next_order_index,
                status: AddOrderStatus::Canceled,
                ..Default::default()
            });
        }

        // Stop trying to match if price no longer satisfies limit.
        if (is_bid && other_order.get_price() > price)
            || (!is_bid && other_order.get_price() < price)
        {
            return Ok(AddOrderToMarketInnerResult {
                next_order_index: NIL,
                status: AddOrderStatus::Unmatched,
                ..Default::default()
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

        let maker_sequence_number = other_order.get_sequence_number();
        let other_trader_index: DataIndex = other_order.get_trader_index();
        let did_fully_match_resting_order: bool =
            remaining_base_atoms >= other_order.get_num_base_atoms();
        let base_atoms_traded: BaseAtoms = if did_fully_match_resting_order {
            other_order.get_num_base_atoms()
        } else {
            remaining_base_atoms
        };

        let matched_price: QuoteAtomsPerBaseAtom = other_order.get_price();

        // on full fill: round in favor of the taker
        // on partial fill: round in favor of the maker
        let quote_atoms_traded: QuoteAtoms = matched_price
            .checked_quote_for_base(base_atoms_traded, is_bid != did_fully_match_resting_order)?;

        // If it is a global order, just in time bring the funds over, or
        // remove from the tree and continue on to the next order.
        let maker: Pubkey = get_helper_seat(dynamic, other_order.get_trader_index())
            .get_value()
            .trader;
        let taker: Pubkey = get_helper_seat(dynamic, trader_index).get_value().trader;

        if other_order.is_global() {
            let global_trade_accounts_opt: &Option<GlobalTradeAccounts> = if is_bid {
                &global_trade_accounts_opts[0]
            } else {
                &global_trade_accounts_opts[1]
            };
            let has_enough_tokens: bool = try_to_move_global_tokens(
                global_trade_accounts_opt,
                &maker,
                GlobalAtoms::new(if is_bid {
                    quote_atoms_traded.as_u64()
                } else {
                    base_atoms_traded.as_u64()
                }),
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
                    ..Default::default()
                });
            }
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
        if !is_bid {
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
                other_trader_index,
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
        // In perps, only credit quote to maker. Skip base credits since base
        // is virtual and base_withdrawable_balance stores cumulative funding.
        if is_bid {
            update_balance(
                fixed,
                dynamic,
                other_trader_index,
                false, // quote side
                true,
                quote_atoms_traded.into(),
            )?;
        }
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

        // Update perps position tracking for maker and taker
        update_perps_position(
            fixed,
            dynamic,
            other_trader_index,
            base_atoms_traded.as_u64(),
            quote_atoms_traded.as_u64(),
            !is_bid,
        )?;
        update_perps_position(
            fixed,
            dynamic,
            trader_index,
            base_atoms_traded.as_u64(),
            quote_atoms_traded.as_u64(),
            is_bid,
        )?;

        emit_stack(FillLog {
            market,
            maker,
            taker,
            base_atoms: base_atoms_traded,
            quote_atoms: quote_atoms_traded,
            price: matched_price,
            maker_sequence_number,
            taker_sequence_number: fixed.order_sequence_number,
            taker_is_buy: PodBool::from(is_bid),
            base_mint: Pubkey::default(),
            quote_mint: *fixed.get_quote_mint(),
            // TODO: Fix this
            is_maker_global: PodBool::from(false),
            _padding: [0; 14],
        })?;

        if did_fully_match_resting_order {
            // Get paid for removing a global order.
            if get_helper_order(dynamic, current_order_index)
                .get_value()
                .get_order_type()
                == OrderType::Global
            {
                if is_bid {
                    remove_from_global(&global_trade_accounts_opts[0])?;
                } else {
                    remove_from_global(&global_trade_accounts_opts[1])?;
                }
            }

            remove_order_from_tree_and_free(fixed, dynamic, current_order_index, !is_bid)?;
            self.remaining_base_atoms = self.remaining_base_atoms.checked_sub(base_atoms_traded)?;
            return Ok(AddOrderToMarketInnerResult {
                next_order_index,
                status: AddOrderStatus::Filled,
                ..Default::default()
            });
        } else {
            #[cfg(feature = "certora")]
            remove_from_orderbook_balance(fixed, dynamic, current_order_index);
            let other_order: &mut RestingOrder =
                get_mut_helper_order(dynamic, current_order_index).get_mut_value();
            other_order.reduce(base_atoms_traded)?;
            #[cfg(feature = "certora")]
            add_to_orderbook_balance(fixed, dynamic, current_order_index);
            self.remaining_base_atoms = BaseAtoms::ZERO;
            return Ok(AddOrderToMarketInnerResult {
                next_order_index: NIL,
                status: AddOrderStatus::PartialFill,
                ..Default::default()
            });
        }
    }
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
        price: _,
        is_bid,
        last_valid_slot,
        order_type,
        global_trade_accounts_opts: _,
        current_slot,
    } = args;
    assert_already_has_seat(trader_index)?;
    let now_slot: u32 = current_slot.unwrap_or_else(|| get_now_slot());

    assert_not_already_expired(last_valid_slot, now_slot)?;

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
    // move out args so that they can be used later
    let args: AddOrderToMarketArgs = ctx.args;
    // ctx is dead from this point onward

    // Record volume on market
    fixed.quote_volume = fixed.quote_volume.wrapping_add(total_quote_atoms_traded);

    // Bump the order sequence number even for orders which do not end up
    // resting.
    let order_sequence_number: u64 = fixed.order_sequence_number;
    fixed.order_sequence_number = order_sequence_number.wrapping_add(1);

    // If there is nothing left to rest, then return before resting.
    if !order_type_can_rest(order_type) || remaining_base_atoms == BaseAtoms::ZERO {
        return Ok(AddOrderToMarketResult {
            order_sequence_number,
            order_index: NIL,
            base_atoms_traded: total_base_atoms_traded,
            quote_atoms_traded: total_quote_atoms_traded,
        });
    }

    self_.rest_remaining(
        args,
        remaining_base_atoms,
        order_sequence_number,
        total_base_atoms_traded,
        total_quote_atoms_traded,
    )
}
