use crate::*;
use cvt::{cvt_assert, cvt_assume};
use cvt_macros::rule;
use nondet::*;

use hypertree::DataIndex;
use solana_program::account_info::AccountInfo;

use crate::{
    program::get_mut_dynamic_account,
    quantities::WrapperU64,
    state::{get_helper_order, MarketRefMut, RestingOrder},
};

use crate::certora::spec::no_funds_loss_util::cvt_assume_market_preconditions;

pub fn cancel_order_by_index_no_revert<const IS_BID: bool>() {
    // IS_BID = true sets up the rule such that the order to
    // be canceled is ask, and vice-versa

    cvt_static_initializer!();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let trader: &AccountInfo = &acc_infos[0];
    let market_info: &AccountInfo = &acc_infos[1];
    let maker_trader: &AccountInfo = &acc_infos[7];
    let vault_base_token: &AccountInfo = &acc_infos[8];
    let vault_quote_token: &AccountInfo = &acc_infos[9];

    // -- market preconditions
    let maker_order_index: DataIndex = cvt_assume_market_preconditions::<IS_BID>(
        market_info,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    );

    // Assume that there will not be an overflow when adding to seat balance.
    let (maker_order_base, maker_order_quote) = get_order_atoms!(maker_order_index);
    let (maker_seat_base, maker_seat_quote) = get_trader_balance!(market_info, maker_trader.key);
    if IS_BID {
        cvt_assume!(maker_seat_base + maker_order_base.as_u64() <= u64::MAX);
    } else {
        cvt_assume!(maker_seat_quote + maker_order_quote.as_u64() <= u64::MAX);
    }

    // -- call to cancel_order_by_index
    let market_data: &mut std::cell::RefMut<&mut [u8]> =
        &mut market_info.try_borrow_mut_data().unwrap();
    let mut dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);
    let order_index: DataIndex = maker_order_index;
    let result: ProgramResult = dynamic_account.cancel_order_by_index(order_index, &[None, None]);
    cvt_assert!(result.is_ok());

    cvt_vacuity_check!();
}

#[rule]
pub fn rule_cancel_order_by_index_no_revert_bid() {
    cancel_order_by_index_no_revert::<true>();
}

#[rule]
pub fn rule_cancel_order_by_index_no_revert_ask() {
    cancel_order_by_index_no_revert::<false>();
}
