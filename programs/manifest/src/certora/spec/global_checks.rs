//! No loss of funds for global orders.
//!
//! A global order is not backed by the market vault. The maker's tokens sit in
//! the global vault, a token account shared by every market that trades the
//! mint, and are only pulled into the market vault at the moment the order is
//! matched. So the global vault is a second, independent source of funds and it
//! gets its own no-loss-of-funds property:
//!
//!     global_vault == sum of all global deposits
//!
//! on top of the market one, which must keep holding while global orders trade:
//!
//!     market_vault == withdrawable + orderbook
//!
//! Together they say that the tokens a global trade moves out of the global
//! vault land in the market vault, and are credited to somebody, in the same
//! instruction. Note that a resting global order contributes nothing to
//! `orderbook`, which is what makes the two invariants independent.
use crate::*;
use cvt::{cvt_assert, cvt_assume};
use cvt_macros::rule;
use nondet::*;

use solana_program::account_info::AccountInfo;

use crate::{
    certora::spec::{
        no_funds_loss_util::*, place_order_checks::place_single_order_nondet_inputs_with_type,
    },
    program::{
        get_mut_dynamic_account,
        global_deposit::{process_global_deposit_core, GlobalDepositParams},
        global_evict::{process_global_evict_core, GlobalEvictParams},
        global_withdraw::{process_global_withdraw_core, GlobalWithdrawParams},
    },
    quantities::{BaseAtoms, QuoteAtoms, WrapperU64},
    state::{
        main_trader_index,
        market::market_helpers::{AddOrderStatus, AddOrderToMarketInnerResult, AddSingleOrderCtx},
        AddOrderToMarketArgs, DynamicAccount, MarketRefMut,
    },
    validation::loaders::GlobalTradeAccounts,
};
use hypertree::DataIndex;
use solana_cvt::token::spl_token_account_get_amount;

/// The market vault on the side the global maker backs their order with. A
/// taker bid is matched by a global ask, which is backed by base.
fn global_side_market_vault<'a>(
    vault_base_token: &'a AccountInfo<'static>,
    vault_quote_token: &'a AccountInfo<'static>,
    is_bid: bool,
) -> &'a AccountInfo<'static> {
    if is_bid {
        vault_base_token
    } else {
        vault_quote_token
    }
}

/// Matching against a global maker moves exactly the traded amount from the
/// global vault into the market vault, and both no-loss-of-funds invariants
/// survive.
pub fn place_single_order_global_match_check<const IS_BID: bool, const IS_FULL_MATCH: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader: &AccountInfo = &acc_infos[0];
    let market_info: &AccountInfo = &acc_infos[1];
    let maker_trader: &AccountInfo = &acc_infos[7];
    let vault_base_token: &AccountInfo = &acc_infos[8];
    let vault_quote_token: &AccountInfo = &acc_infos[9];
    let global_info: &AccountInfo = &acc_infos[10];
    let global_vault_token: &AccountInfo = &acc_infos[11];

    // -- the maker order on the book is a global order
    let maker_order_index: DataIndex = cvt_assume_global_market_preconditions::<IS_BID>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );

    let market_vault_token: &AccountInfo =
        global_side_market_vault(vault_base_token, vault_quote_token, IS_BID);

    let global_trade_accounts_opts: [Option<GlobalTradeAccounts>; 2] =
        cvt_assume_global_trade_accounts(
            market_info,
            trader,
            maker_trader,
            global_info,
            global_vault_token,
            market_vault_token,
            IS_BID, // a global maker facing a taker bid is an ask, backed by base
        );

    // -- record balances before matching
    let balances_old: AllBalances = record_all_balances(
        market_info,
        vault_base_token,
        vault_quote_token,
        trader,
        maker_trader,
        maker_order_index,
    );
    let global_old: GlobalBalances =
        record_global_balances(global_info, global_vault_token, maker_trader);
    let market_vault_old: u64 = spl_token_account_get_amount(market_vault_token);

    // -- assume both no loss of funds invariants
    cvt_assume_funds_invariants(balances_old);
    cvt_assume_global_funds_invariants(global_old);

    let (args, remaining_base_atoms, now_slot) = place_single_order_nondet_inputs_with_type::<IS_BID>(
        market_info,
        state::OrderType::Limit,
        &global_trade_accounts_opts,
    );

    let (res, total_base_atoms_traded, total_quote_atoms_traded, global_atoms_transferred) = place_single_order_and_settle_global!(
        market_info,
        args,
        remaining_base_atoms,
        now_slot,
        maker_order_index
    );

    if IS_FULL_MATCH {
        cvt_assume!(res.status == AddOrderStatus::Filled);
    } else {
        cvt_assume!(res.status == AddOrderStatus::PartialFill);
    }

    let balances_new: AllBalances = record_all_balances(
        market_info,
        vault_base_token,
        vault_quote_token,
        trader,
        maker_trader,
        maker_order_index,
    );
    let global_new: GlobalBalances =
        record_global_balances(global_info, global_vault_token, maker_trader);
    let market_vault_new: u64 = spl_token_account_get_amount(market_vault_token);

    // -- the market keeps covering everything it owes
    cvt_assert_funds_invariants(balances_new);
    // -- and so does the global account
    cvt_assert_global_funds_invariants(global_new);

    // -- a global maker pays with base when it is an ask, quote when it is a bid
    let traded_atoms: u64 = if IS_BID {
        total_base_atoms_traded.as_u64()
    } else {
        total_quote_atoms_traded.as_u64()
    };
    cvt_assert!(global_atoms_transferred.as_u64() == traded_atoms);

    // -- everything that left the global vault landed in the market vault
    cvt_assert_global_funds_moved_to_market(
        global_old,
        global_new,
        market_vault_old,
        market_vault_new,
        traded_atoms,
    );

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_place_single_order_global_full_match_bid() {
    place_single_order_global_match_check::<true /* IS_BID */, true /* IS_FULL_MATCH */>();
}

#[rule]
pub fn rule_place_single_order_global_full_match_ask() {
    place_single_order_global_match_check::<false /* IS_BID */, true /* IS_FULL_MATCH */>();
}

#[rule]
pub fn rule_place_single_order_global_partial_match_bid() {
    place_single_order_global_match_check::<true /* IS_BID */, false /* IS_FULL_MATCH */>();
}

#[rule]
pub fn rule_place_single_order_global_partial_match_ask() {
    place_single_order_global_match_check::<false /* IS_BID */, false /* IS_FULL_MATCH */>();
}

/// An unbacked global order is dropped from the book without trading, and
/// without moving a single atom anywhere.
pub fn place_single_order_global_unbacked_check<const IS_BID: bool>() {
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

    let market_vault_token: &AccountInfo =
        global_side_market_vault(vault_base_token, vault_quote_token, IS_BID);

    let global_trade_accounts_opts: [Option<GlobalTradeAccounts>; 2] =
        cvt_assume_global_trade_accounts(
            market_info,
            trader,
            maker_trader,
            global_info,
            global_vault_token,
            market_vault_token,
            IS_BID, // a global maker facing a taker bid is an ask, backed by base
        );

    let balances_old: AllBalances = record_all_balances(
        market_info,
        vault_base_token,
        vault_quote_token,
        trader,
        maker_trader,
        maker_order_index,
    );
    let global_old: GlobalBalances =
        record_global_balances(global_info, global_vault_token, maker_trader);

    cvt_assume_funds_invariants(balances_old);
    cvt_assume_global_funds_invariants(global_old);

    let (args, remaining_base_atoms, now_slot) = place_single_order_nondet_inputs_with_type::<IS_BID>(
        market_info,
        state::OrderType::Limit,
        &global_trade_accounts_opts,
    );

    let (res, total_base_atoms_traded, total_quote_atoms_traded, global_atoms_transferred) = place_single_order_and_settle_global!(
        market_info,
        args,
        remaining_base_atoms,
        now_slot,
        maker_order_index
    );
    cvt_assume!(res.status == AddOrderStatus::GlobalSkip);

    let balances_new: AllBalances = record_all_balances(
        market_info,
        vault_base_token,
        vault_quote_token,
        trader,
        maker_trader,
        maker_order_index,
    );
    let global_new: GlobalBalances =
        record_global_balances(global_info, global_vault_token, maker_trader);

    cvt_assert_funds_invariants(balances_new);
    cvt_assert_global_funds_invariants(global_new);

    // -- nothing traded
    cvt_assert!(total_base_atoms_traded == BaseAtoms::ZERO);
    cvt_assert!(total_quote_atoms_traded == QuoteAtoms::ZERO);
    cvt_assert!(global_atoms_transferred.as_u64() == 0);

    // -- nothing moved on the global account
    cvt_assert_global_funds_unchanged(global_old, global_new);

    // -- and nothing moved on the market either. Removing a global order gives
    // -- nothing back because nothing was ever taken.
    cvt_assert!(balances_old.vault_base == balances_new.vault_base);
    cvt_assert!(balances_old.vault_quote == balances_new.vault_quote);
    cvt_assert!(balances_old.withdrawable_base == balances_new.withdrawable_base);
    cvt_assert!(balances_old.withdrawable_quote == balances_new.withdrawable_quote);
    cvt_assert!(balances_old.orderbook_base == balances_new.orderbook_base);
    cvt_assert!(balances_old.orderbook_quote == balances_new.orderbook_quote);

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_place_single_order_global_unbacked_bid() {
    place_single_order_global_unbacked_check::<true /* IS_BID */>();
}

#[rule]
pub fn rule_place_single_order_global_unbacked_ask() {
    place_single_order_global_unbacked_check::<false /* IS_BID */>();
}

/// Resting a global order moves no funds. The order is backed by the global
/// account, so nothing may be taken from the trader's market balance and
/// nothing may be added to the orderbook aggregate.
pub fn rest_remaining_global_check<const IS_BID: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader: &AccountInfo = &acc_infos[0];
    let market_info: &AccountInfo = &acc_infos[1];
    let maker_trader: &AccountInfo = &acc_infos[7];
    let vault_base_token: &AccountInfo = &acc_infos[8];
    let vault_quote_token: &AccountInfo = &acc_infos[9];
    let global_info: &AccountInfo = &acc_infos[10];
    let global_vault_token: &AccountInfo = &acc_infos[11];

    let maker_order_index: DataIndex = cvt_assume_market_preconditions::<IS_BID>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );

    // The order being rested is the taker's own, so it is backed with base when
    // it is an ask and quote when it is a bid. That is the opposite side from a
    // maker sitting on the book, which is why the vault picked here is the
    // mirror of the matching rules.
    let market_vault_token: &AccountInfo =
        global_side_market_vault(vault_base_token, vault_quote_token, !IS_BID);

    // The trader resting the global order needs the global seat, not the maker.
    let global_trade_accounts_opts: [Option<GlobalTradeAccounts>; 2] =
        cvt_assume_global_trade_accounts(
            market_info,
            trader,
            trader,
            global_info,
            global_vault_token,
            market_vault_token,
            !IS_BID, // the trader's own global bid is backed by quote
        );

    let balances_old: AllBalances = record_all_balances(
        market_info,
        vault_base_token,
        vault_quote_token,
        trader,
        maker_trader,
        maker_order_index,
    );
    let global_old: GlobalBalances =
        record_global_balances(global_info, global_vault_token, trader);

    cvt_assume_funds_invariants(balances_old);
    cvt_assume_global_funds_invariants(global_old);

    let args: AddOrderToMarketArgs = AddOrderToMarketArgs {
        market: *market_info.key,
        trader_index: main_trader_index(),
        num_base_atoms: nondet(),
        price: crate::quantities::QuoteAtomsPerBaseAtom::nondet_price_u32(),
        is_bid: IS_BID,
        last_valid_slot: nondet(),
        order_type: state::OrderType::Global,
        global_trade_accounts_opts: &global_trade_accounts_opts,
        current_slot: Some(nondet()),
    };

    rest_remaining!(
        market_info,
        args,
        nondet::<BaseAtoms>(),
        nondet::<u64>(),
        nondet::<BaseAtoms>(),
        nondet::<QuoteAtoms>()
    );

    let balances_new: AllBalances = record_all_balances(
        market_info,
        vault_base_token,
        vault_quote_token,
        trader,
        maker_trader,
        maker_order_index,
    );
    let global_new: GlobalBalances =
        record_global_balances(global_info, global_vault_token, trader);

    cvt_assert_funds_invariants(balances_new);
    cvt_assert_global_funds_invariants(global_new);

    // -- placing a global order does not move any funds at all
    cvt_assert_global_funds_unchanged(global_old, global_new);
    cvt_assert!(balances_old.vault_base == balances_new.vault_base);
    cvt_assert!(balances_old.vault_quote == balances_new.vault_quote);
    cvt_assert!(balances_old.withdrawable_base == balances_new.withdrawable_base);
    cvt_assert!(balances_old.withdrawable_quote == balances_new.withdrawable_quote);
    cvt_assert!(balances_old.trader_base == balances_new.trader_base);
    cvt_assert!(balances_old.trader_quote == balances_new.trader_quote);
    // -- and it reserves nothing on the market, the global account backs it
    cvt_assert!(balances_old.orderbook_base == balances_new.orderbook_base);
    cvt_assert!(balances_old.orderbook_quote == balances_new.orderbook_quote);

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_rest_remaining_global_bid() {
    rest_remaining_global_check::<true /* IS_BID */>();
}

#[rule]
pub fn rule_rest_remaining_global_ask() {
    rest_remaining_global_check::<false /* IS_BID */>();
}

/// Cancelling a global order gives nothing back on the market, because nothing
/// was ever taken, and leaves the global account untouched.
pub fn cancel_global_order_check<const IS_BID: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader: &AccountInfo = &acc_infos[0];
    let market_info: &AccountInfo = &acc_infos[1];
    let maker_trader: &AccountInfo = &acc_infos[7];
    let vault_base_token: &AccountInfo = &acc_infos[8];
    let vault_quote_token: &AccountInfo = &acc_infos[9];
    let global_info: &AccountInfo = &acc_infos[10];
    let global_vault_token: &AccountInfo = &acc_infos[11];

    // The global order to cancel rests on the book opposite a taker on IS_BID.
    let order_index: DataIndex = cvt_assume_global_market_preconditions::<IS_BID>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );

    let market_vault_token: &AccountInfo =
        global_side_market_vault(vault_base_token, vault_quote_token, IS_BID);

    let global_trade_accounts_opts: [Option<GlobalTradeAccounts>; 2] =
        cvt_assume_global_trade_accounts(
            market_info,
            trader,
            maker_trader,
            global_info,
            global_vault_token,
            market_vault_token,
            IS_BID, // a global maker facing a taker bid is an ask, backed by base
        );

    let balances_old: AllBalances = record_all_balances(
        market_info,
        vault_base_token,
        vault_quote_token,
        trader,
        maker_trader,
        order_index,
    );
    let global_old: GlobalBalances =
        record_global_balances(global_info, global_vault_token, maker_trader);

    cvt_assume_funds_invariants(balances_old);
    cvt_assume_global_funds_invariants(global_old);

    cancel_order_by_index!(market_info, order_index, &global_trade_accounts_opts);

    let balances_new: AllBalances = record_all_balances(
        market_info,
        vault_base_token,
        vault_quote_token,
        trader,
        maker_trader,
        order_index,
    );
    let global_new: GlobalBalances =
        record_global_balances(global_info, global_vault_token, maker_trader);

    cvt_assert_funds_invariants(balances_new);
    cvt_assert_global_funds_invariants(global_new);

    cvt_assert_global_funds_unchanged(global_old, global_new);
    cvt_assert!(balances_old.vault_base == balances_new.vault_base);
    cvt_assert!(balances_old.vault_quote == balances_new.vault_quote);
    cvt_assert!(balances_old.withdrawable_base == balances_new.withdrawable_base);
    cvt_assert!(balances_old.withdrawable_quote == balances_new.withdrawable_quote);
    cvt_assert!(balances_old.orderbook_base == balances_new.orderbook_base);
    cvt_assert!(balances_old.orderbook_quote == balances_new.orderbook_quote);
    // -- in particular the maker is not credited for an order they never
    // -- funded from the market
    cvt_assert!(balances_old.maker_trader_base == balances_new.maker_trader_base);
    cvt_assert!(balances_old.maker_trader_quote == balances_new.maker_trader_quote);

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_cancel_global_order_bid() {
    cancel_global_order_check::<true /* IS_BID */>();
}

#[rule]
pub fn rule_cancel_global_order_ask() {
    cancel_global_order_check::<false /* IS_BID */>();
}

/// Cancelling a global order with the system program present pays the gas
/// prepayment refund: exactly GAS_DEPOSIT_LAMPORTS move from the global account
/// to the gas receiver, and the token side stays untouched.
pub fn cancel_global_order_gas_refund_check<const IS_BID: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader: &AccountInfo = &acc_infos[0];
    let market_info: &AccountInfo = &acc_infos[1];
    let maker_trader: &AccountInfo = &acc_infos[7];
    let vault_base_token: &AccountInfo = &acc_infos[8];
    let vault_quote_token: &AccountInfo = &acc_infos[9];
    let global_info: &AccountInfo = &acc_infos[10];
    let global_vault_token: &AccountInfo = &acc_infos[11];
    let system_program_info: &AccountInfo = &acc_infos[12];

    let order_index: DataIndex = cvt_assume_global_market_preconditions::<IS_BID>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );

    let market_vault_token: &AccountInfo =
        global_side_market_vault(vault_base_token, vault_quote_token, IS_BID);

    let global_trade_accounts_opts: [Option<GlobalTradeAccounts>; 2] =
        cvt_assume_global_trade_accounts_with_gas(
            market_info,
            trader,
            maker_trader,
            global_info,
            global_vault_token,
            market_vault_token,
            system_program_info,
            IS_BID,
        );

    let global_old: GlobalBalances =
        record_global_balances(global_info, global_vault_token, maker_trader);
    cvt_assume_global_funds_invariants(global_old);

    // -- lamports before. The gas prepayment paid when the order was placed
    // -- guarantees the global account can cover the refund.
    let global_lamports_old: u64 = **global_info.lamports.borrow();
    let receiver_lamports_old: u64 = **trader.lamports.borrow();
    cvt_assume!(global_lamports_old >= crate::state::GAS_DEPOSIT_LAMPORTS);
    cvt_assume!(receiver_lamports_old <= u64::MAX - crate::state::GAS_DEPOSIT_LAMPORTS);

    cancel_order_by_index!(market_info, order_index, &global_trade_accounts_opts);
    crate::state::utils::settle_global_gas_refunds(&global_trade_accounts_opts).unwrap();

    let global_new: GlobalBalances =
        record_global_balances(global_info, global_vault_token, maker_trader);
    cvt_assert_global_funds_invariants(global_new);

    // -- tokens did not move
    cvt_assert_global_funds_unchanged(global_old, global_new);

    // -- the refund moved, exactly once and exactly GAS_DEPOSIT_LAMPORTS
    let global_lamports_new: u64 = **global_info.lamports.borrow();
    let receiver_lamports_new: u64 = **trader.lamports.borrow();
    cvt_assert!(global_lamports_new == global_lamports_old - crate::state::GAS_DEPOSIT_LAMPORTS);
    cvt_assert!(
        receiver_lamports_new == receiver_lamports_old + crate::state::GAS_DEPOSIT_LAMPORTS
    );

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_cancel_global_order_gas_refund_bid() {
    cancel_global_order_gas_refund_check::<true /* IS_BID */>();
}

#[rule]
pub fn rule_cancel_global_order_gas_refund_ask() {
    cancel_global_order_gas_refund_check::<false /* IS_BID */>();
}

/// Placing global orders deposits the gas prepayment: exactly
/// GAS_DEPOSIT_LAMPORTS per order move from the gas payer to the global
/// account, where they wait for whoever removes the orders.
#[rule]
pub fn rule_global_gas_prepayment() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader: &AccountInfo = &acc_infos[0];
    let market_info: &AccountInfo = &acc_infos[1];
    let maker_trader: &AccountInfo = &acc_infos[7];
    let market_vault_token: &AccountInfo = &acc_infos[8];
    let global_info: &AccountInfo = &acc_infos[10];
    let global_vault_token: &AccountInfo = &acc_infos[11];
    let system_program_info: &AccountInfo = &acc_infos[12];

    crate::state::cvt_assume_main_trader_has_seat(trader.key);

    let global_trade_accounts_opts: [Option<GlobalTradeAccounts>; 2] =
        cvt_assume_global_trade_accounts_with_gas(
            market_info,
            trader,
            maker_trader,
            global_info,
            global_vault_token,
            market_vault_token,
            system_program_info,
            true, // side does not matter for the lamport accounting
        );

    let payer_lamports_old: u64 = **trader.lamports.borrow();
    let global_lamports_old: u64 = **global_info.lamports.borrow();

    let num_orders: u64 = nondet();
    cvt_assume!(num_orders >= 1 && num_orders <= 4);
    crate::state::utils::pay_global_gas_prepayment(
        global_trade_accounts_opts[0].as_ref().unwrap(),
        num_orders,
    )
    .unwrap();

    let payer_lamports_new: u64 = **trader.lamports.borrow();
    let global_lamports_new: u64 = **global_info.lamports.borrow();

    let expected: u64 = crate::state::GAS_DEPOSIT_LAMPORTS * num_orders;
    cvt_assert!(payer_lamports_new == payer_lamports_old - expected);
    cvt_assert!(global_lamports_new == global_lamports_old + expected);

    cvt_vacuity_check!();
}

/// Evicting the global trader with the smallest (zero) deposit hands their
/// seat to the new trader without moving any tokens: the vault, the deposit
/// aggregate, and everyone else's balances are exactly what they were, and the
/// new trader starts from zero.
///
/// The deposit-aggregate assertion below reads the `global_deposited_atoms`
/// ghost, which is only updated where a balance field is mutated — destroying
/// a `GlobalDeposit` node leaves it untouched. The only guard against
/// evicting a trader with a nonzero deposit is the production
/// `require!(existing_global_atoms_deposited == ZERO)` in
/// `evict_and_take_seat`, which compiles to an *assume* under certora. If
/// that check were removed from production, the assume would vanish with it
/// and eviction would confiscate the deposit while the vault keeps the
/// tokens; the ghost and the vault would not notice. That is why the rule
/// also asserts the delta of the real mock state
/// (`modeled_global_deposits()`) across the call: a confiscated nonzero
/// deposit shrinks the modeled sum and fails the rule.
#[rule]
pub fn rule_global_evict() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let global_info: &AccountInfo = &acc_infos[10];
    let global_vault_token: &AccountInfo = &acc_infos[11];

    cvt_assume!(global_info.owner == &crate::id());
    create_global!(global_info);

    // -- the evictee holds the only seat, the new trader holds none
    let evictee_pk: Pubkey = *crate::state::second_trader_pk();
    let new_trader_pk: Pubkey = *crate::state::main_trader_pk();
    cvt_assume!(evictee_pk != new_trader_pk);
    cvt_assume!(crate::state::is_second_global_seat_taken());
    cvt_assume!(crate::state::is_main_global_seat_free());

    let global_vault_old: u64 = spl_token_account_get_amount(global_vault_token);
    let global_deposits_old: u64 = get_global_deposited_atoms!(global_info);
    let modeled_deposits_old: u64 = crate::state::modeled_global_deposits();
    cvt_assume!(modeled_deposits_old <= global_deposits_old);
    cvt_assume!(global_vault_old == global_deposits_old);

    {
        let global_data: &mut std::cell::RefMut<&mut [u8]> =
            &mut global_info.try_borrow_mut_data().unwrap();
        let mut global_dynamic_account: crate::state::GlobalRefMut =
            get_mut_dynamic_account(global_data);
        global_dynamic_account
            .evict_and_take_seat(&evictee_pk, &new_trader_pk)
            .unwrap();
    }

    let global_vault_new: u64 = spl_token_account_get_amount(global_vault_token);
    let global_deposits_new: u64 = get_global_deposited_atoms!(global_info);

    // -- no tokens moved and the vault still covers every deposit
    cvt_assert!(global_vault_new == global_vault_old);
    cvt_assert!(global_deposits_new == global_deposits_old);
    cvt_assert!(global_vault_new == global_deposits_new);

    // -- the real mock state agrees: an eviction is only allowed to destroy a
    // -- zero deposit, so the modeled sum of deposits is unchanged. This is
    // -- what catches a confiscated nonzero deposit, which the ghost above
    // -- cannot see (destroying a node does not go through a balance write).
    cvt_assert!(crate::state::modeled_global_deposits() == modeled_deposits_old);

    // -- the seat changed hands
    cvt_assert!(crate::state::has_mock_global_seat(&new_trader_pk));
    cvt_assert!(!crate::state::has_mock_global_seat(&evictee_pk));

    // -- and the new trader starts with nothing to withdraw
    cvt_assert!(crate::state::global_balance_atoms(&new_trader_pk) == 0);

    cvt_vacuity_check!();
}

/// Give the mocked global account an arbitrary number of claimed seats.
fn set_nondet_num_seats_claimed(global_info: &AccountInfo) {
    let global_data: &mut std::cell::RefMut<&mut [u8]> =
        &mut global_info.try_borrow_mut_data().unwrap();
    let global_dynamic_account: crate::state::GlobalRefMut = get_mut_dynamic_account(global_data);
    global_dynamic_account.fixed.set_num_seats_claimed(nondet());
}

/// The `global_evict` processor around `evict_and_take_seat`: the evictor
/// pays the seat + eviction fee in lamports (received in full by the global
/// account), the evictee is paid out their whole balance from the global
/// vault, the evictor's deposit lands in the vault and is credited to them,
/// and the vault covers every deposit throughout. Fee-less transfers; the
/// token-2022 transfer-fee path is covered by
/// `rule_global_evict_processor_with_fee`.
#[rule]
pub fn rule_global_evict_processor() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let used_acc_infos: &[AccountInfo] = &acc_infos[..8];
    let payer: &AccountInfo = &used_acc_infos[0]; // the evictor
    let global_info: &AccountInfo = &used_acc_infos[1];
    let global_vault_token: &AccountInfo = &used_acc_infos[3];
    let trader_token: &AccountInfo = &used_acc_infos[4];
    let evictee_token: &AccountInfo = &used_acc_infos[5];

    cvt_assume!(global_info.owner == &crate::id());
    create_global!(global_info);
    // create_global! writes an account with zero seats, but eviction is only
    // allowed at capacity: give it an arbitrary seat count so the capacity
    // check does not make the rule vacuous.
    set_nondet_num_seats_claimed(global_info);

    // -- the evictee holds the only modeled seat, the evictor holds none
    cvt_assume!(payer.key == crate::state::main_trader_pk());
    cvt_assume!(
        &evictee_token.try_borrow_data().unwrap()[32..64]
            == crate::state::second_trader_pk().as_ref()
    );
    cvt_assume!(payer.key != crate::state::second_trader_pk());
    cvt_assume!(crate::state::is_second_global_seat_taken());
    cvt_assume!(crate::state::is_main_global_seat_free());

    // -- distinct token accounts
    cvt_assume!(trader_token.key != global_vault_token.key);
    cvt_assume!(evictee_token.key != global_vault_token.key);
    cvt_assume!(evictee_token.key != trader_token.key);

    let global_old: GlobalBalances = record_global_balances(global_info, global_vault_token, payer);
    cvt_assume_global_funds_invariants(global_old);
    cvt_assume!(crate::state::modeled_global_deposits() <= global_old.global_deposits);

    let evictee_balance_old: u64 =
        crate::state::global_balance_atoms(crate::state::second_trader_pk());
    let trader_token_old: u64 = spl_token_account_get_amount(trader_token);
    let evictee_token_old: u64 = spl_token_account_get_amount(evictee_token);
    let payer_lamports_old: u64 = **payer.lamports.borrow();
    let global_lamports_old: u64 = **global_info.lamports.borrow();

    let amount_atoms: u64 = nondet();
    process_global_evict_core(
        &crate::id(),
        used_acc_infos,
        GlobalEvictParams::new(amount_atoms),
    )
    .unwrap();

    let global_new: GlobalBalances = record_global_balances(global_info, global_vault_token, payer);
    let trader_token_new: u64 = spl_token_account_get_amount(trader_token);
    let evictee_token_new: u64 = spl_token_account_get_amount(evictee_token);
    let payer_lamports_new: u64 = **payer.lamports.borrow();
    let global_lamports_new: u64 = **global_info.lamports.borrow();

    // -- the eviction fee moved from the payer to the global account, and
    // nowhere else
    cvt_assert!(payer_lamports_new <= payer_lamports_old);
    let fee_lamports: u64 = payer_lamports_old - payer_lamports_new;
    cvt_assert!(global_lamports_new == global_lamports_old + fee_lamports);

    // -- an eviction is only valid when the evictor deposits more than the
    // evictee held
    cvt_assert!(evictee_balance_old < amount_atoms);

    // -- the evictee was paid out their whole balance
    cvt_assert!(evictee_token_new == evictee_token_old + evictee_balance_old);

    // -- the evictor paid their deposit into the vault
    cvt_assert!(trader_token_new == trader_token_old.saturating_sub(amount_atoms));
    cvt_assert!(
        global_new.global_vault == global_old.global_vault - evictee_balance_old + amount_atoms
    );

    // -- the deposit aggregate moved in step and the vault still covers it
    cvt_assert!(
        global_new.global_deposits
            == global_old.global_deposits - evictee_balance_old + amount_atoms
    );
    cvt_assert_global_funds_invariants(global_new);

    // -- the seat changed hands and the balances ended where they should
    cvt_assert!(crate::state::has_mock_global_seat(payer.key));
    cvt_assert!(!crate::state::has_mock_global_seat(
        crate::state::second_trader_pk()
    ));
    cvt_assert!(crate::state::global_balance_atoms(payer.key) == amount_atoms);
    cvt_assert!(crate::state::global_balance_atoms(crate::state::second_trader_pk()) == 0);

    cvt_vacuity_check!();
}

/// The `global_evict` processor when the mint carries a token-2022 transfer
/// fee on both transfer legs. The evictor is debited the full requested
/// amount but credited only what the vault received; the evictee is debited
/// their full balance from the vault but receives it minus the fee; the vault
/// still covers every deposit.
#[rule]
pub fn rule_global_evict_processor_with_fee() {
    use crate::certora::summaries::token::{cvt_enable_transfer_fee, transfer_fees_charged};

    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let used_acc_infos: &[AccountInfo] = &acc_infos[..8];
    let payer: &AccountInfo = &used_acc_infos[0]; // the evictor
    let global_info: &AccountInfo = &used_acc_infos[1];
    let global_vault_token: &AccountInfo = &used_acc_infos[3];
    let trader_token: &AccountInfo = &used_acc_infos[4];
    let evictee_token: &AccountInfo = &used_acc_infos[5];

    cvt_assume!(global_info.owner == &crate::id());
    create_global!(global_info);
    // create_global! writes an account with zero seats, but eviction is only
    // allowed at capacity: give it an arbitrary seat count so the capacity
    // check does not make the rule vacuous.
    set_nondet_num_seats_claimed(global_info);

    // -- the vault is a token-2022 account, the only path where a transfer
    // fee exists, and the mint may charge one
    cvt_assume!(global_vault_token.owner == &spl_token_2022::id());
    cvt_enable_transfer_fee();

    // -- the evictee holds the only modeled seat, the evictor holds none
    cvt_assume!(payer.key == crate::state::main_trader_pk());
    cvt_assume!(
        &evictee_token.try_borrow_data().unwrap()[32..64]
            == crate::state::second_trader_pk().as_ref()
    );
    cvt_assume!(payer.key != crate::state::second_trader_pk());
    cvt_assume!(crate::state::is_second_global_seat_taken());
    cvt_assume!(crate::state::is_main_global_seat_free());

    // -- distinct token accounts
    cvt_assume!(trader_token.key != global_vault_token.key);
    cvt_assume!(evictee_token.key != global_vault_token.key);
    cvt_assume!(evictee_token.key != trader_token.key);

    let global_old: GlobalBalances = record_global_balances(global_info, global_vault_token, payer);
    cvt_assume_global_funds_invariants(global_old);
    cvt_assume!(crate::state::modeled_global_deposits() <= global_old.global_deposits);

    let evictee_balance_old: u64 =
        crate::state::global_balance_atoms(crate::state::second_trader_pk());
    let trader_token_old: u64 = spl_token_account_get_amount(trader_token);
    let evictee_token_old: u64 = spl_token_account_get_amount(evictee_token);

    let amount_atoms: u64 = nondet();
    process_global_evict_core(
        &crate::id(),
        used_acc_infos,
        GlobalEvictParams::new(amount_atoms),
    )
    .unwrap();

    let global_new: GlobalBalances = record_global_balances(global_info, global_vault_token, payer);
    let trader_token_new: u64 = spl_token_account_get_amount(trader_token);
    let evictee_token_new: u64 = spl_token_account_get_amount(evictee_token);

    // The two legs charge one fee each. The evictor's credited balance
    // recovers the deposit-leg fee; the rest was charged on the withdraw leg.
    let deposited: u64 = crate::state::global_balance_atoms(payer.key);
    cvt_assert!(deposited <= amount_atoms);
    let deposit_fee: u64 = amount_atoms - deposited;
    let fees: u64 = transfer_fees_charged();
    cvt_assert!(fees >= deposit_fee);
    let withdraw_fee: u64 = fees - deposit_fee;
    cvt_assert!(withdraw_fee <= evictee_balance_old);

    // -- an eviction is only valid when the vault received more than the
    // evictee held
    cvt_assert!(evictee_balance_old < deposited);

    // -- the evictee is debited their whole balance and receives it minus the
    // withdraw-leg fee
    cvt_assert!(evictee_token_new == evictee_token_old + (evictee_balance_old - withdraw_fee));

    // -- the evictor pays the requested amount, the vault receives it minus
    // the deposit-leg fee
    cvt_assert!(trader_token_new == trader_token_old.saturating_sub(amount_atoms));
    cvt_assert!(
        global_new.global_vault == global_old.global_vault - evictee_balance_old + deposited
    );

    // -- the deposit aggregate moved in step and the vault still covers it
    cvt_assert!(
        global_new.global_deposits == global_old.global_deposits - evictee_balance_old + deposited
    );
    cvt_assert_global_funds_invariants(global_new);

    // -- the seat changed hands and the balances ended where they should
    cvt_assert!(crate::state::has_mock_global_seat(payer.key));
    cvt_assert!(!crate::state::has_mock_global_seat(
        crate::state::second_trader_pk()
    ));
    cvt_assert!(crate::state::global_balance_atoms(crate::state::second_trader_pk()) == 0);

    cvt_vacuity_check!();
}

/// A deposit into the global account increases the vault and what the depositor
/// is owed by the same amount.
#[rule]
pub fn rule_global_deposit() {
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

    let global_old: GlobalBalances =
        record_global_balances(global_info, global_vault_token, trader);
    cvt_assume_global_funds_invariants(global_old);
    let trader_token_old: u64 = spl_token_account_get_amount(trader_token);

    let amount_atoms: u64 = nondet();
    process_global_deposit_core(
        &crate::id(),
        used_acc_infos,
        GlobalDepositParams::new(amount_atoms),
    )
    .unwrap();

    let global_new: GlobalBalances =
        record_global_balances(global_info, global_vault_token, trader);
    let trader_token_new: u64 = spl_token_account_get_amount(trader_token);

    // -- the vault still covers every deposit
    cvt_assert_global_funds_invariants(global_new);

    // -- the tokens came out of the trader and went into the vault
    cvt_assert!(trader_token_new == trader_token_old.saturating_sub(amount_atoms));
    cvt_assert!(global_new.global_vault == global_old.global_vault.saturating_add(amount_atoms));
    // -- and the depositor is credited for exactly that
    cvt_assert!(global_new.maker_deposit == global_old.maker_deposit.saturating_add(amount_atoms));
    cvt_assert!(
        global_new.global_deposits == global_old.global_deposits.saturating_add(amount_atoms)
    );

    cvt_vacuity_check!();
}

/// A global deposit through a token-2022 mint that carries a transfer fee:
/// the trader pays the requested amount, the global vault receives the
/// requested amount minus the fee, and the depositor is credited with exactly
/// what the vault received -- never the requested amount. This verifies the
/// vault balance-delta crediting in `process_global_deposit_core` that the
/// exact-transfer summary could not exercise.
#[rule]
pub fn rule_global_deposit_with_fee() {
    use crate::certora::summaries::token::{cvt_enable_transfer_fee, transfer_fees_charged};

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

    // -- the vault is a token-2022 account, the only path where a transfer fee
    // exists, and the mint may charge one
    cvt_assume!(global_vault_token.owner == &spl_token_2022::id());
    cvt_enable_transfer_fee();

    let global_old: GlobalBalances =
        record_global_balances(global_info, global_vault_token, trader);
    cvt_assume_global_funds_invariants(global_old);
    let trader_token_old: u64 = spl_token_account_get_amount(trader_token);

    let amount_atoms: u64 = nondet();
    process_global_deposit_core(
        &crate::id(),
        used_acc_infos,
        GlobalDepositParams::new(amount_atoms),
    )
    .unwrap();

    // The fee the transfer summary chose for this execution
    let fee: u64 = transfer_fees_charged();
    cvt_assert!(fee <= amount_atoms);
    let received: u64 = amount_atoms - fee;

    let global_new: GlobalBalances =
        record_global_balances(global_info, global_vault_token, trader);
    let trader_token_new: u64 = spl_token_account_get_amount(trader_token);

    // -- the vault still covers every deposit
    cvt_assert_global_funds_invariants(global_new);

    // -- the trader pays the requested amount, the vault receives it minus the
    // fee
    cvt_assert!(trader_token_new == trader_token_old.saturating_sub(amount_atoms));
    cvt_assert!(global_new.global_vault == global_old.global_vault.saturating_add(received));
    // -- and the depositor is credited for exactly what the vault received
    cvt_assert!(global_new.maker_deposit == global_old.maker_deposit.saturating_add(received));
    cvt_assert!(global_new.global_deposits == global_old.global_deposits.saturating_add(received));

    cvt_vacuity_check!();
}

/// A withdraw from the global account can only take out what the trader has
/// deposited, and the vault shrinks by the same amount.
#[rule]
pub fn rule_global_withdraw() {
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

    let global_old: GlobalBalances =
        record_global_balances(global_info, global_vault_token, trader);
    cvt_assume_global_funds_invariants(global_old);
    let trader_token_old: u64 = spl_token_account_get_amount(trader_token);

    let amount_atoms: u64 = nondet();
    process_global_withdraw_core(
        &crate::id(),
        used_acc_infos,
        GlobalWithdrawParams::new(amount_atoms),
    )
    .unwrap();

    let global_new: GlobalBalances =
        record_global_balances(global_info, global_vault_token, trader);
    let trader_token_new: u64 = spl_token_account_get_amount(trader_token);

    cvt_assert_global_funds_invariants(global_new);

    // -- a trader cannot withdraw more than they deposited
    cvt_assert!(amount_atoms <= global_old.maker_deposit);

    // -- the tokens left the vault and reached the trader
    cvt_assert!(trader_token_new == trader_token_old.saturating_add(amount_atoms));
    cvt_assert!(global_new.global_vault == global_old.global_vault.saturating_sub(amount_atoms));
    cvt_assert!(global_new.maker_deposit == global_old.maker_deposit.saturating_sub(amount_atoms));
    cvt_assert!(
        global_new.global_deposits == global_old.global_deposits.saturating_sub(amount_atoms)
    );

    cvt_vacuity_check!();
}
