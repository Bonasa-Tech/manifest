//! The trader pubkey stored in a claimed seat is written by `claim_seat` and
//! never mutated while the seat is held.
//!
//! This discharges `cvt_assume_seat_pubkeys` in `no_funds_loss_util`: the
//! matching code reads the maker's key back out of the seat node to decide
//! whose global deposit pays for a global order, so the funds properties for
//! global orders are only as strong as this invariant. The rules here are the
//! induction: `rule_claim_seat_writes_trader_pubkey` establishes it when a
//! seat is claimed, and the `rule_seat_pubkey_preserved_by_*` family shows
//! that every other verified operation leaves the stored pubkeys alone (they
//! only touch balances and volume). Once a seat is released its block returns
//! to the free list and may be legitimately reused, so preservation is only
//! stated while the seat is held; releasing one seat must still not touch the
//! other.
//!
//! KNOWN GAP: the matching, cancel, and rest-remaining rules below are built
//! with the non-global preconditions (`cvt_assume_market_preconditions` forces
//! a non-global maker) and pass no global accounts, so the global-specific
//! code paths (`try_to_reduce_global_tokens`, `remove_from_global`,
//! `transfer_global_tokens`) are never exercised by any preservation rule. If
//! one of those paths corrupted a market seat pubkey, no rule here would fail,
//! yet the global funds rules would still assume the pubkeys intact at entry —
//! the induction has a hole exactly on the global surface. Closing it means
//! adding MAKER_IS_GLOBAL variants of the matching/cancel/rest rules via
//! `cvt_assume_market_preconditions_gen`.
use super::verification_utils::init_static;
use crate::*;
use cvt::{cvt_assert, cvt_assume};
use cvt_macros::rule;
use nondet::*;

use solana_program::account_info::AccountInfo;

use crate::{
    certora::spec::{no_funds_loss_util::*, place_order_checks::place_single_order_nondet_inputs},
    program::{
        deposit::{process_deposit_core, DepositParams},
        get_mut_dynamic_account,
        withdraw::{process_withdraw_core, WithdrawParams},
    },
    quantities::{BaseAtoms, QuoteAtoms},
    state::{
        get_helper_seat, is_main_seat_free, is_main_seat_taken, is_second_seat_free,
        main_trader_index, main_trader_pk,
        market::market_helpers::{AddOrderToMarketInnerResult, AddSingleOrderCtx},
        second_trader_index, second_trader_pk, AddOrderToMarketArgs, DynamicAccount, MarketFixed,
        MarketRefMut,
    },
};
use hypertree::{get_mut_helper, DataIndex};

/// The pubkeys currently stored in the two seat nodes.
fn record_seat_pubkeys() -> (Pubkey, Pubkey) {
    let dynamic: &[u8; 8] = &[0; 8];
    (
        get_helper_seat(dynamic, main_trader_index())
            .get_value()
            .trader,
        get_helper_seat(dynamic, second_trader_index())
            .get_value()
            .trader,
    )
}

fn cvt_assert_seat_pubkeys_unchanged(old: (Pubkey, Pubkey)) {
    let new: (Pubkey, Pubkey) = record_seat_pubkeys();
    cvt_assert!(old.0 == new.0);
    cvt_assert!(old.1 == new.1);
}

/// Claiming a seat writes the trader's pubkey into the seat node, and claiming
/// a second seat does not touch the first.
#[rule]
pub fn rule_claim_seat_writes_trader_pubkey() {
    init_static();

    let market_info: AccountInfo = nondet();
    create_empty_market!(market_info);

    let trader1_key: Pubkey = *main_trader_pk();
    let trader2_key: Pubkey = *second_trader_pk();
    cvt_assume!(trader1_key != trader2_key);
    cvt_assume!(is_main_seat_free());
    cvt_assume!(is_second_seat_free());

    claim_seat!(market_info, &trader1_key);
    let dynamic: &[u8; 8] = &[0; 8];
    cvt_assert!(
        get_helper_seat(dynamic, main_trader_index())
            .get_value()
            .trader
            == trader1_key
    );

    claim_seat!(market_info, &trader2_key);
    cvt_assert!(
        get_helper_seat(dynamic, second_trader_index())
            .get_value()
            .trader
            == trader2_key
    );
    // -- claiming the second seat did not rewrite the first
    cvt_assert!(
        get_helper_seat(dynamic, main_trader_index())
            .get_value()
            .trader
            == trader1_key
    );

    cvt_vacuity_check!();
}

/// Releasing one seat does not touch the pubkey stored in the other.
#[rule]
pub fn rule_seat_pubkey_preserved_by_release_seat() {
    init_static();

    let market_info: AccountInfo = nondet();
    create_empty_market!(market_info);

    let trader_key: Pubkey = *main_trader_pk();
    cvt_assume!(is_main_seat_taken());

    let old: (Pubkey, Pubkey) = record_seat_pubkeys();

    {
        let market_data: &mut std::cell::RefMut<&mut [u8]> =
            &mut market_info.try_borrow_mut_data().unwrap();
        let mut dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);
        dynamic_account.release_seat(&trader_key).unwrap();
    }

    cvt_assert!(is_main_seat_free());
    let dynamic: &[u8; 8] = &[0; 8];
    cvt_assert!(
        get_helper_seat(dynamic, second_trader_index())
            .get_value()
            .trader
            == old.1
    );

    cvt_vacuity_check!();
}

/// Deposits credit balances; they never rewrite who owns a seat.
fn seat_pubkey_preserved_by_deposit_or_withdraw_check<const IS_DEPOSIT: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let used_acc_infos: &[AccountInfo] = &acc_infos[..6];
    let trader: &AccountInfo = &used_acc_infos[0];
    let market_info: &AccountInfo = &used_acc_infos[1];
    let trader_token: &AccountInfo = &used_acc_infos[2];
    let vault_token: &AccountInfo = &used_acc_infos[3];

    let maker_trader: &AccountInfo = &acc_infos[7];
    let vault_base_token: &AccountInfo = &acc_infos[8];
    let vault_quote_token: &AccountInfo = &acc_infos[9];

    // the IS_BID parameter only shapes the resting maker order, which these
    // rules do not touch
    let _maker_order_index: DataIndex = cvt_assume_market_preconditions::<true /* IS_BID */>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );

    let market_base_vault_pk: Pubkey = get_base_vault!(market_info);
    cvt_assume!(vault_token.key == &market_base_vault_pk);
    cvt_assume!(trader_token.key != vault_token.key);

    let old: (Pubkey, Pubkey) = record_seat_pubkeys();

    let amount_arg: u64 = nondet();
    if IS_DEPOSIT {
        process_deposit_core(
            &crate::id(),
            used_acc_infos,
            DepositParams::new(amount_arg, None),
        )
        .unwrap();
    } else {
        process_withdraw_core(
            &crate::id(),
            used_acc_infos,
            WithdrawParams::new(amount_arg, None),
        )
        .unwrap();
    }

    cvt_assert_seat_pubkeys_unchanged(old);

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_seat_pubkey_preserved_by_deposit() {
    seat_pubkey_preserved_by_deposit_or_withdraw_check::<true /* IS_DEPOSIT */>();
}

#[rule]
pub fn rule_seat_pubkey_preserved_by_withdraw() {
    seat_pubkey_preserved_by_deposit_or_withdraw_check::<false /* IS_DEPOSIT */>();
}

/// Matching moves balances between the maker's and taker's seats (and, for a
/// reverse maker, back onto the book); the stored pubkeys never change.
fn seat_pubkey_preserved_by_matching_check<const IS_BID: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader: &AccountInfo = &acc_infos[0];
    let market_info: &AccountInfo = &acc_infos[1];
    let maker_trader: &AccountInfo = &acc_infos[7];
    let vault_base_token: &AccountInfo = &acc_infos[8];
    let vault_quote_token: &AccountInfo = &acc_infos[9];

    let maker_order_index: DataIndex = cvt_assume_market_preconditions::<IS_BID>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );

    let old: (Pubkey, Pubkey) = record_seat_pubkeys();

    let (args, remaining_base_atoms, now_slot) =
        place_single_order_nondet_inputs::<IS_BID>(market_info);

    let (_res, _total_base_atoms_traded, _total_quote_atoms_traded) = place_single_order!(
        market_info,
        args,
        remaining_base_atoms,
        now_slot,
        maker_order_index
    );

    cvt_assert_seat_pubkeys_unchanged(old);

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_seat_pubkey_preserved_by_matching_bid() {
    seat_pubkey_preserved_by_matching_check::<true /* IS_BID */>();
}

#[rule]
pub fn rule_seat_pubkey_preserved_by_matching_ask() {
    seat_pubkey_preserved_by_matching_check::<false /* IS_BID */>();
}

/// Resting an order reserves funds out of the trader's seat; the stored
/// pubkeys never change.
fn seat_pubkey_preserved_by_rest_remaining_check<const IS_BID: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader: &AccountInfo = &acc_infos[0];
    let market_info: &AccountInfo = &acc_infos[1];
    let maker_trader: &AccountInfo = &acc_infos[7];
    let vault_base_token: &AccountInfo = &acc_infos[8];
    let vault_quote_token: &AccountInfo = &acc_infos[9];

    let _maker_order_index: DataIndex = cvt_assume_market_preconditions::<IS_BID>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );

    let old: (Pubkey, Pubkey) = record_seat_pubkeys();

    let args: AddOrderToMarketArgs = AddOrderToMarketArgs {
        market: *market_info.key,
        trader_index: main_trader_index(),
        num_base_atoms: nondet(),
        price: crate::quantities::QuoteAtomsPerBaseAtom::nondet_price_u32(),
        is_bid: IS_BID,
        last_valid_slot: nondet(),
        order_type: state::OrderType::Limit,
        global_trade_accounts_opts: &[None, None],
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

    cvt_assert_seat_pubkeys_unchanged(old);

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_seat_pubkey_preserved_by_rest_remaining_bid() {
    seat_pubkey_preserved_by_rest_remaining_check::<true /* IS_BID */>();
}

#[rule]
pub fn rule_seat_pubkey_preserved_by_rest_remaining_ask() {
    seat_pubkey_preserved_by_rest_remaining_check::<false /* IS_BID */>();
}

/// Cancelling refunds the maker's seat; the stored pubkeys never change.
fn seat_pubkey_preserved_by_cancel_check<const IS_BID: bool>() {
    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader: &AccountInfo = &acc_infos[0];
    let market_info: &AccountInfo = &acc_infos[1];
    let maker_trader: &AccountInfo = &acc_infos[2];
    let vault_base_token: &AccountInfo = &acc_infos[3];
    let vault_quote_token: &AccountInfo = &acc_infos[4];

    let maker_order_index: DataIndex = cvt_assume_market_preconditions::<IS_BID>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );

    let old: (Pubkey, Pubkey) = record_seat_pubkeys();

    cancel_order_by_index!(market_info, maker_order_index);

    cvt_assert_seat_pubkeys_unchanged(old);

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_seat_pubkey_preserved_by_cancel_bid() {
    seat_pubkey_preserved_by_cancel_check::<true /* IS_BID */>();
}

#[rule]
pub fn rule_seat_pubkey_preserved_by_cancel_ask() {
    seat_pubkey_preserved_by_cancel_check::<false /* IS_BID */>();
}

// There is deliberately no swap rule in this family. A swap is, in the
// verified model, a composition of operations that are each covered: its only
// writes to seat nodes go through claim_seat (established by
// rule_claim_seat_writes_trader_pubkey; a seatless swapper's lazy claim is
// exactly that case) and update_balance (preserved by the deposit, withdraw
// and matching rules above). A dedicated swap rule additionally hits a prover
// pointer-analysis limitation in the swap account loader (error 3003,
// "dereference of a register with unknown provenance") when the seat nodes are
// read around process_swap_core, so the composition argument is the coverage.
