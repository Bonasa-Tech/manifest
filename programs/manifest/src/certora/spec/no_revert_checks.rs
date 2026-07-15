//! No unexpected reverts on the paths this work added.
//!
//! The rest of the rules unwrap the result of the operation, so an execution
//! that fails is pruned instead of reported: they say "if it succeeds, no funds
//! are lost", not "it succeeds". These rules close that hole for the paths that
//! are new here, the same way `rule_withdraw_does_not_revert` and
//! `rule_cancel_order_by_index_no_revert_*` do for the older ones.
//!
//! Under the `certora` feature `require!` compiles to an assumption, so a
//! violation of `result.is_ok()` can only come from an *unexpected* failure --
//! an arithmetic overflow or a `?` on a checked operation -- and never from a
//! deliberate `require!`. That is exactly the property the original rules
//! state.
use crate::*;
use cvt::{cvt_assert, cvt_assume};
use cvt_macros::rule;
use nondet::*;

use solana_program::{account_info::AccountInfo, entrypoint::ProgramResult};

use crate::{
    certora::spec::{
        no_funds_loss_util::*, place_order_checks::place_single_order_nondet_inputs_with_type,
    },
    program::{
        get_mut_dynamic_account,
        global_deposit::{process_global_deposit_core, GlobalDepositParams},
        global_withdraw::{process_global_withdraw_core, GlobalWithdrawParams},
    },
    quantities::{BaseAtoms, QuoteAtoms, QuoteAtomsPerBaseAtom, WrapperU64},
    state::{
        get_helper_order, main_trader_index,
        market::market_helpers::{AddOrderToMarketInnerResult, AddSingleOrderCtx},
        AddOrderToMarketArgs, DynamicAccount, MarketRefMut, OrderType, RestingOrder,
    },
    validation::loaders::GlobalTradeAccounts,
};
use hypertree::DataIndex;

/// The arithmetic a trade performs must not overflow.
///
/// `place_order` can legitimately return an error: an order whose price times
/// size exceeds a u64, or a credit that overflows a seat balance, is rejected
/// and the transaction fails. That is correct behaviour, not an unexpected
/// revert, so the no-revert rules assume it away -- exactly as the existing
/// rules for withdraw and cancel_order_by_index do ("assume that there will not
/// be an overflow when adding to seat balance").
///
/// Everything the trade can move is bounded by the maker order: at most its
/// size in base, and at most its size times its price in quote. `remaining` is
/// what the taker brings, and is priced at the same price.
fn cvt_assume_trade_arithmetic_does_not_overflow(
    balances: AllBalances,
    maker_order_index: DataIndex,
    remaining_base_atoms: BaseAtoms,
) {
    let dynamic: &[u8; 8] = &[0; 8];
    let maker_order: &RestingOrder = get_helper_order(dynamic, maker_order_index).get_value();
    let price: QuoteAtomsPerBaseAtom = maker_order.get_price();
    let maker_base: BaseAtoms = maker_order.get_num_base_atoms();

    // -- neither side of the trade prices out beyond a u64
    let maker_quote_or: Result<QuoteAtoms, _> = price.checked_quote_for_base(maker_base, true);
    cvt_assume!(maker_quote_or.is_ok());
    cvt_assume!(price
        .checked_quote_for_base(remaining_base_atoms, true)
        .is_ok());
    let max_quote: u64 = maker_quote_or.unwrap().as_u64();
    let max_base: u64 = maker_base.as_u64();

    // -- and crediting either trader with it does not overflow their seat
    cvt_assume!(balances.trader_base.checked_add(max_base).is_some());
    cvt_assume!(balances.trader_quote.checked_add(max_quote).is_some());
    cvt_assume!(balances.maker_trader_base.checked_add(max_base).is_some());
    cvt_assume!(balances.maker_trader_quote.checked_add(max_quote).is_some());
}

/// The reverse come-back order coalesces into an existing order, growing it.
/// The come-back size and the grown order must stay within a u64, or the trade
/// is rejected for the same legitimate reason as above. The two sides differ:
/// a bid come-back is a division by the reverse price and the grown bid is a
/// further multiply; an ask come-back is just `base_traded` and the grown ask
/// reserves base directly, with no division or multiply. So the ask needs only
/// the add bounded, and applying the bid's division chain to it would inject an
/// intermediate that is not on the ask's code path.
fn cvt_assume_coalesce_arithmetic_does_not_overflow<const IS_BID: bool>(
    maker_order_index: DataIndex,
    coalesce_order_index: DataIndex,
) {
    let dynamic: &[u8; 8] = &[0; 8];
    let maker_order: &RestingOrder = get_helper_order(dynamic, maker_order_index).get_value();
    let coalesce_order: &RestingOrder = get_helper_order(dynamic, coalesce_order_index).get_value();
    let maker_base: BaseAtoms = maker_order.get_num_base_atoms();
    let coalesce_base: u64 = coalesce_order.get_num_base_atoms().as_u64();

    // An upper bound on the come-back size, bounded through quantities that
    // upper-bound what the matching code computes internally. Each step is a
    // legitimate rejection point: an order too large to price or to add is
    // refused, which is not an unexpected revert.
    let come_back_bound: u64 = if IS_BID {
        //   quote_traded  <= maker_max_quote                 (the maker's full value)
        //   come_back_size = base_for_quote(quote_traded, price_reverse)
        //                 <= base_for_quote(maker_max_quote, price_reverse)
        // base_for_quote grows with the quote, so the max case bounds the
        // actual one. price_reverse is derived from the maker exactly as the
        // matching code derives it.
        let maker_max_quote_or: Result<QuoteAtoms, _> = maker_order
            .get_price()
            .checked_quote_for_base(maker_base, true);
        cvt_assume!(maker_max_quote_or.is_ok());
        let price_reverse: QuoteAtomsPerBaseAtom = maker_order.reverse_price().unwrap();
        let max_reverse_or: Result<BaseAtoms, _> =
            price_reverse.checked_base_for_quote(maker_max_quote_or.unwrap(), false);
        cvt_assume!(max_reverse_or.is_ok());
        max_reverse_or.unwrap().as_u64()
    } else {
        // An ask come-back is base_atoms_traded, at most the maker's size.
        maker_base.as_u64()
    };

    // Adding the come-back size to the coalesce order does not overflow.
    let grown_or: Option<BaseAtoms> = coalesce_base
        .checked_add(come_back_bound)
        .map(BaseAtoms::new);
    cvt_assume!(grown_or.is_some());

    // A bid coalesce order reserves quote, so the grown order must still price
    // out within a u64. An ask coalesce order reserves base directly, no price
    // multiply, so there is nothing further to bound.
    if IS_BID {
        cvt_assume!(coalesce_order
            .get_price()
            .checked_quote_for_base(grown_or.unwrap(), true)
            .is_ok());
    }
}

/// Matching a global maker does not revert unexpectedly.
pub fn place_single_order_global_no_revert_check<const IS_BID: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader: &AccountInfo = &acc_infos[0];
    let market_info: &AccountInfo = &acc_infos[1];
    let maker_trader: &AccountInfo = &acc_infos[7];
    let vault_base_token: &AccountInfo = &acc_infos[8];
    let vault_quote_token: &AccountInfo = &acc_infos[9];
    let global_info: &AccountInfo = &acc_infos[10];
    let global_vault_token: &AccountInfo = &acc_infos[11];

    let maker_order_index: DataIndex = cvt_assume_global_market_preconditions::<IS_BID>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );

    let market_vault_token: &AccountInfo = if IS_BID {
        vault_base_token
    } else {
        vault_quote_token
    };

    let global_trade_accounts_opts: [Option<GlobalTradeAccounts>; 2] =
        cvt_assume_global_trade_accounts(
            market_info,
            trader,
            maker_trader,
            global_info,
            global_vault_token,
            market_vault_token,
            IS_BID,
        );

    let balances: AllBalances = record_all_balances(
        market_info,
        vault_base_token,
        vault_quote_token,
        trader,
        maker_trader,
        maker_order_index,
    );
    cvt_assume_funds_invariants(balances);

    let global_balances: GlobalBalances =
        record_global_balances(global_info, global_vault_token, maker_trader);
    cvt_assume_global_funds_invariants(global_balances);

    let (args, remaining_base_atoms, now_slot) = place_single_order_nondet_inputs_with_type::<IS_BID>(
        market_info,
        OrderType::Limit,
        &global_trade_accounts_opts,
    );

    cvt_assume_trade_arithmetic_does_not_overflow(
        balances,
        maker_order_index,
        remaining_base_atoms,
    );

    let (res, _base, _quote) = place_single_order_res!(
        market_info,
        args,
        remaining_base_atoms,
        now_slot,
        maker_order_index
    );

    cvt_assert!(res.is_ok());

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_place_single_order_global_no_revert_bid() {
    place_single_order_global_no_revert_check::<true /* IS_BID */>();
}

#[rule]
pub fn rule_place_single_order_global_no_revert_ask() {
    place_single_order_global_no_revert_check::<false /* IS_BID */>();
}

/// Cancelling a global order does not revert unexpectedly.
pub fn cancel_global_order_no_revert_check<const IS_BID: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader: &AccountInfo = &acc_infos[0];
    let market_info: &AccountInfo = &acc_infos[1];
    let maker_trader: &AccountInfo = &acc_infos[7];
    let vault_base_token: &AccountInfo = &acc_infos[8];
    let vault_quote_token: &AccountInfo = &acc_infos[9];
    let global_info: &AccountInfo = &acc_infos[10];
    let global_vault_token: &AccountInfo = &acc_infos[11];

    let order_index: DataIndex = cvt_assume_global_market_preconditions::<IS_BID>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );

    let market_vault_token: &AccountInfo = if IS_BID {
        vault_base_token
    } else {
        vault_quote_token
    };

    let global_trade_accounts_opts: [Option<GlobalTradeAccounts>; 2] =
        cvt_assume_global_trade_accounts(
            market_info,
            trader,
            maker_trader,
            global_info,
            global_vault_token,
            market_vault_token,
            IS_BID,
        );

    let market_data: &mut std::cell::RefMut<&mut [u8]> =
        &mut market_info.try_borrow_mut_data().unwrap();
    let mut dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);
    let result: ProgramResult =
        dynamic_account.cancel_order_by_index(order_index, &global_trade_accounts_opts);

    cvt_assert!(result.is_ok());

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_cancel_global_order_no_revert_bid() {
    cancel_global_order_no_revert_check::<true /* IS_BID */>();
}

#[rule]
pub fn rule_cancel_global_order_no_revert_ask() {
    cancel_global_order_no_revert_check::<false /* IS_BID */>();
}

/// Resting a global order does not revert unexpectedly.
pub fn rest_remaining_global_no_revert_check<const IS_BID: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader: &AccountInfo = &acc_infos[0];
    let market_info: &AccountInfo = &acc_infos[1];
    let maker_trader: &AccountInfo = &acc_infos[7];
    let vault_base_token: &AccountInfo = &acc_infos[8];
    let vault_quote_token: &AccountInfo = &acc_infos[9];
    let global_info: &AccountInfo = &acc_infos[10];
    let global_vault_token: &AccountInfo = &acc_infos[11];

    let _maker_order_index: DataIndex = cvt_assume_market_preconditions::<IS_BID>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );

    // The trader rests their own global order, backed by quote when it is a bid.
    let market_vault_token: &AccountInfo = if IS_BID {
        vault_quote_token
    } else {
        vault_base_token
    };

    let global_trade_accounts_opts: [Option<GlobalTradeAccounts>; 2] =
        cvt_assume_global_trade_accounts(
            market_info,
            trader,
            trader,
            global_info,
            global_vault_token,
            market_vault_token,
            !IS_BID,
        );

    let args: AddOrderToMarketArgs = AddOrderToMarketArgs {
        market: *market_info.key,
        trader_index: main_trader_index(),
        num_base_atoms: nondet(),
        price: QuoteAtomsPerBaseAtom::nondet_price_u32(),
        is_bid: IS_BID,
        last_valid_slot: nondet(),
        order_type: OrderType::Global,
        global_trade_accounts_opts: &global_trade_accounts_opts,
        current_slot: Some(nondet()),
    };

    let market_data: &mut std::cell::RefMut<&mut [u8]> =
        &mut market_info.try_borrow_mut_data().unwrap();
    let mut dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);
    let result = dynamic_account.certora_rest_remaining(
        args,
        nondet::<BaseAtoms>(),
        nondet::<u64>(),
        nondet::<BaseAtoms>(),
        nondet::<QuoteAtoms>(),
    );

    cvt_assert!(result.is_ok());

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_rest_remaining_global_no_revert_bid() {
    rest_remaining_global_no_revert_check::<true /* IS_BID */>();
}

#[rule]
pub fn rule_rest_remaining_global_no_revert_ask() {
    rest_remaining_global_no_revert_check::<false /* IS_BID */>();
}

/// Matching a reverse maker, whose come-back order coalesces into an existing
/// order, does not revert unexpectedly. This is the path that computes the
/// maker's debit from the difference of two allocations (F-01), so it is the
/// one most likely to hit a checked_sub that does not hold.
pub fn reverse_coalesce_no_revert_check<const IS_BID: bool, const IS_TIGHT: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader: &AccountInfo = &acc_infos[0];
    let market_info: &AccountInfo = &acc_infos[1];
    let maker_trader: &AccountInfo = &acc_infos[7];
    let vault_base_token: &AccountInfo = &acc_infos[8];
    let vault_quote_token: &AccountInfo = &acc_infos[9];

    let (maker_order_index, coalesce_order_index) =
        cvt_assume_reverse_coalesce_preconditions::<IS_BID, IS_TIGHT>(
            market_info,
            trader,
            vault_base_token,
            vault_quote_token,
            maker_trader,
        );

    let balances: AllBalances = record_all_balances(
        market_info,
        vault_base_token,
        vault_quote_token,
        trader,
        maker_trader,
        maker_order_index,
    );
    cvt_assume_funds_invariants(balances);

    let (args, remaining_base_atoms, now_slot) = place_single_order_nondet_inputs_with_type::<IS_BID>(
        market_info,
        OrderType::Limit,
        &[None, None],
    );

    cvt_assume_trade_arithmetic_does_not_overflow(
        balances,
        maker_order_index,
        remaining_base_atoms,
    );
    cvt_assume_coalesce_arithmetic_does_not_overflow::<IS_BID>(
        maker_order_index,
        coalesce_order_index,
    );

    let (res, _base, _quote) = place_single_order_res!(
        market_info,
        args,
        remaining_base_atoms,
        now_slot,
        maker_order_index
    );

    cvt_assert!(res.is_ok());

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_reverse_coalesce_no_revert_bid() {
    reverse_coalesce_no_revert_check::<true /* IS_BID */, false /* IS_TIGHT */>();
}

#[rule]
pub fn rule_reverse_coalesce_no_revert_ask() {
    reverse_coalesce_no_revert_check::<false /* IS_BID */, false /* IS_TIGHT */>();
}

/// Global deposit and withdraw do not revert unexpectedly.
fn global_deposit_withdraw_no_revert_check<const IS_DEPOSIT: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let used_acc_infos: &[AccountInfo] = &acc_infos[..6];
    let trader: &AccountInfo = &used_acc_infos[0];
    let global_info: &AccountInfo = &used_acc_infos[1];
    let global_vault_token: &AccountInfo = &used_acc_infos[3];
    let trader_token: &AccountInfo = &used_acc_infos[4];

    cvt_assume!(global_info.owner == &crate::id());
    create_global!(global_info);
    crate::state::cvt_assume_main_trader_has_seat(trader.key);
    crate::state::cvt_assume_has_global_seat(trader.key);
    cvt_assume!(trader_token.key != global_vault_token.key);

    let global_balances: GlobalBalances =
        record_global_balances(global_info, global_vault_token, trader);
    cvt_assume_global_funds_invariants(global_balances);

    let amount_atoms: u64 = nondet();
    let result: ProgramResult = if IS_DEPOSIT {
        // The deposit must not overflow what the depositor already holds.
        cvt_assume!(global_balances
            .maker_deposit
            .checked_add(amount_atoms)
            .is_some());
        cvt_assume!(global_balances
            .global_deposits
            .checked_add(amount_atoms)
            .is_some());
        process_global_deposit_core(
            &crate::id(),
            used_acc_infos,
            GlobalDepositParams::new(amount_atoms),
        )
    } else {
        // A trader can only withdraw what they have.
        cvt_assume!(amount_atoms <= global_balances.maker_deposit);
        process_global_withdraw_core(
            &crate::id(),
            used_acc_infos,
            GlobalWithdrawParams::new(amount_atoms),
        )
    };

    cvt_assert!(result.is_ok());

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_global_deposit_no_revert() {
    global_deposit_withdraw_no_revert_check::<true /* IS_DEPOSIT */>();
}

#[rule]
pub fn rule_global_withdraw_no_revert() {
    global_deposit_withdraw_no_revert_check::<false /* IS_DEPOSIT */>();
}
