use super::verification_utils::init_static;
use crate::{
    claim_seat, create_empty_market, cvt_assert_is_nil, cvt_vacuity_check, deposit,
    get_trader_balance, get_trader_index,
};
use calltrace::cvt_cex_print_u64;
use cvt::{cvt_assert, cvt_assume};
use cvt_macros::rule;
use nondet::nondet;

use crate::{
    program::get_mut_dynamic_account,
    state::{
        is_main_seat_free, is_main_seat_taken, is_second_seat_free, main_trader_index,
        main_trader_pk, second_trader_index, second_trader_pk, MarketFixed, MarketRefMut,
    },
};
use hypertree::{get_mut_helper, is_nil, DataIndex, NIL};
use solana_program::{account_info::AccountInfo, pubkey::Pubkey};

#[rule]
pub fn rule_market_empty() {
    init_static();

    let market_info: AccountInfo = nondet();

    // Create an empty market
    create_empty_market!(market_info);

    let trader_key: Pubkey = *main_trader_pk();
    cvt_assume!(is_main_seat_free());

    cvt_assert_is_nil!(get_trader_index!(market_info, &trader_key));
    cvt_vacuity_check!();
}

#[rule]
pub fn rule_market_claim_seat_once() {
    init_static();

    let market_info: AccountInfo = nondet();

    // Create an empty market
    create_empty_market!(market_info);

    let trader_key: Pubkey = *main_trader_pk();
    cvt_assume!(is_main_seat_free());

    cvt_assert_is_nil!(get_trader_index!(market_info, &trader_key));
    claim_seat!(market_info, &trader_key);
    cvt_assert!(get_trader_index!(market_info, &trader_key) == main_trader_index());
    cvt_vacuity_check!();
}

#[rule]
pub fn rule_market_claim_seat_twice_same_trader() {
    init_static();

    let market_info: AccountInfo = nondet();

    // Create an empty market
    create_empty_market!(market_info);

    let trader_key: Pubkey = *main_trader_pk();
    cvt_assume!(is_main_seat_free());
    cvt_assume!(is_second_seat_free());

    cvt_assert_is_nil!(get_trader_index!(market_info, &trader_key));
    claim_seat!(market_info, &trader_key);
    cvt_assert!(get_trader_index!(market_info, &trader_key) == main_trader_index());

    // second call to claim_seat will make the rule pass vacuously
    claim_seat!(market_info, &trader_key);
    cvt_assert!(get_trader_index!(market_info, &trader_key) == main_trader_index());
    cvt_vacuity_check!();
}

#[rule]
pub fn rule_market_claim_seat_twice_different_trader() {
    init_static();

    let market_info: AccountInfo = nondet();

    // Create an empty market
    create_empty_market!(market_info);

    let trader1_key: Pubkey = *main_trader_pk();
    cvt_assume!(is_main_seat_free());

    let trader2_key: Pubkey = *second_trader_pk();
    cvt_assume!(is_second_seat_free());
    cvt_assume!(trader2_key != trader1_key);

    cvt_assert_is_nil!(get_trader_index!(market_info, &trader1_key));
    claim_seat!(market_info, &trader1_key);
    cvt_assert!(get_trader_index!(market_info, &trader1_key) == main_trader_index());

    cvt_assert_is_nil!(get_trader_index!(market_info, &trader2_key));
    claim_seat!(market_info, &trader2_key);
    cvt_assert!(get_trader_index!(market_info, &trader2_key) == second_trader_index());

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_market_deposit() {
    init_static();

    let market_info: AccountInfo = nondet();

    // Create an empty market
    create_empty_market!(market_info);

    let trader_key: Pubkey = *main_trader_pk();
    cvt_assume!(is_main_seat_free());

    cvt_assert_is_nil!(get_trader_index!(market_info, &trader_key));
    claim_seat!(market_info, &trader_key);
    let trader_index: DataIndex = get_trader_index!(market_info, &trader_key);
    cvt_assert!(trader_index == main_trader_index());

    deposit!(market_info, trader_index, 100, true);
    let (base_atoms, quote_atoms) = get_trader_balance!(market_info, &trader_key);
    cvt_cex_print_u64!(1, u64::from(base_atoms), u64::from(quote_atoms));
    cvt_assert!(u64::from(base_atoms) == 100);
    cvt_vacuity_check!();
}

#[rule]
pub fn rule_market_release_seat() {
    init_static();

    let market_info: AccountInfo = nondet();

    // Create an empty market
    create_empty_market!(market_info);

    let trader_key: Pubkey = *main_trader_pk();
    cvt_assume!(is_main_seat_taken());

    {
        let market_data: &mut std::cell::RefMut<&mut [u8]> =
            &mut market_info.try_borrow_mut_data().unwrap();
        let mut dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);
        dynamic_account.release_seat(&trader_key).unwrap();
    }

    cvt_assert!(is_main_seat_free());
    cvt_vacuity_check!();
}
