//! Fee-aware summary for token-2022 transfers.
//!
//! The `solana_cvt` transfer summaries move the requested amount exactly, so
//! the received-amount-differs-from-requested handling for fee-bearing
//! token-2022 deposits was verified in the fee-less case only. This summary
//! models a token-2022 `transfer_checked` against a mint that may carry a
//! transfer fee: the source is debited the full requested amount, the
//! destination is credited the requested amount minus a nondeterministic fee.
//!
//! The fee is gated by a ghost switch so the pre-existing rules, whose
//! assertions state exact-amount movement, keep verifying unchanged:
//! `init_static` disables the fee, and only rules that opt in with
//! `cvt_enable_transfer_fee` explore the fee-bearing executions. The fee
//! charged by each call accumulates in a ghost that rules read back with
//! `transfer_fees_charged` to state exact deltas.

use solana_cvt::token::{spl_token_account_get_amount, spl_token_account_set_amount};
use solana_program::{account_info::AccountInfo, entrypoint::ProgramResult};

/// Whether transfers may charge a nondeterministic fee. Reset by
/// `init_static`; havoced in rules that do not initialize statics.
static mut TRANSFER_FEE_ENABLED: bool = false;

/// Ghost sum of the fees charged by every fee-aware transfer since
/// `init_static`.
static mut TRANSFER_FEES_CHARGED: u64 = 0;

pub fn init_transfer_fee() {
    unsafe {
        TRANSFER_FEE_ENABLED = false;
        TRANSFER_FEES_CHARGED = 0;
    }
}

/// Opt a rule into fee-bearing executions.
pub fn cvt_enable_transfer_fee() {
    unsafe {
        TRANSFER_FEE_ENABLED = true;
    }
}

pub fn transfer_fee_enabled() -> bool {
    unsafe { TRANSFER_FEE_ENABLED }
}

pub fn transfer_fees_charged() -> u64 {
    unsafe { TRANSFER_FEES_CHARGED }
}

/// (Summary) Token-2022 `transfer_checked` against a mint that may carry a
/// transfer fee: `amount` leaves the source, `amount - fee` reaches the
/// destination, for a nondeterministic `fee <= amount` (zero unless the rule
/// called `cvt_enable_transfer_fee`). Mirrors the assumes of the exact
/// `solana_cvt` summary: the source covers the amount, self-transfers are
/// no-ops.
pub fn spl_token_2022_transfer_with_fee<'a>(
    src_info: &AccountInfo<'a>,
    dst_info: &AccountInfo<'a>,
    _authority_info: &AccountInfo<'a>,
    amount: u64,
) -> ProgramResult {
    if src_info.key != dst_info.key {
        let fee: u64 = if transfer_fee_enabled() {
            let fee: u64 = ::nondet::nondet();
            cvt::cvt_assume!(fee <= amount);
            fee
        } else {
            0
        };

        let mut src_amount: u64 = spl_token_account_get_amount(src_info);
        let mut dst_amount: u64 = spl_token_account_get_amount(dst_info);

        // The token program fails a transfer the source cannot cover.
        cvt::cvt_assume!(src_amount >= amount);

        src_amount = src_amount.checked_sub(amount).unwrap();
        dst_amount = dst_amount
            .checked_add(amount.checked_sub(fee).unwrap())
            .unwrap();

        spl_token_account_set_amount(src_amount, src_info);
        spl_token_account_set_amount(dst_amount, dst_info);

        unsafe {
            TRANSFER_FEES_CHARGED = TRANSFER_FEES_CHARGED.checked_add(fee).unwrap();
        }
    }

    Ok(())
}
