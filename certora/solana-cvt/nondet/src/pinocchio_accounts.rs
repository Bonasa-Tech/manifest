//! Non-deterministic account views, for programs that read accounts the way
//! the runtime lays them out.
//!
//! The `solana` module here builds a `solana_program::AccountInfo`, which is
//! several independent pointers, and has to pin each of them to a fixed
//! address so counterexamples stay readable. An
//! `AccountView` is one pointer to a `RuntimeAccount` header immediately
//! followed by that account's data, which is what the runtime actually hands
//! a program, so pinning it needs one assumption per account and no
//! uninterpreted constructor: the pointer is non-deterministic, and the memory
//! it addresses is unconstrained, which is exactly the account being symbolic.
//!
//! Accounts are spaced by the maximum an account can hold so their regions
//! cannot overlap, mirroring the `solana` module's choice of a fixed layout
//! over an arbitrary one.

use crate::nondet;
use pinocchio::account::{AccountView, RuntimeAccount};

/// Start of the input memory region in the SVM, where the runtime serializes
/// the accounts an instruction is called with.
const CONTEXT_START: u64 = 0x400_000_008;

/// Bytes reserved per account: the header, then the most data an account can
/// hold. Spacing by this keeps each account's region clear of the next.
const ACCOUNT_STRIDE: u64 = 10_485_760 + 88;

/// An account view whose contents are unconstrained.
///
/// The pointer is non-deterministic; nothing is assumed about the header or
/// the data behind it, so every field a program reads is symbolic.
pub fn nondet_account_view() -> AccountView {
    let address: u64 = nondet::<u64>();
    // SAFETY: for verification. The prover reasons about the memory this
    // addresses symbolically rather than dereferencing anything real, and
    // `account_views_with_mem_layout` pins the address to the input region.
    unsafe { AccountView::new_unchecked(address as *mut RuntimeAccount) }
}

/// Sixteen account views laid out in the input memory region, one after
/// another, with unconstrained contents.
///
/// Same purpose as `fun_acc_infos_with_mem_layout`: fixing the addresses
/// removes counterexamples that differ only in where the runtime happened to
/// put an account, which no program should branch on.
pub fn fun_account_views_with_mem_layout() -> [AccountView; 16] {
    let views: [AccountView; 16] = core::array::from_fn(|_| nondet_account_view());
    for (index, view) in views.iter().enumerate() {
        let expected: u64 = CONTEXT_START + (index as u64) * ACCOUNT_STRIDE;
        cvt::CVT_assume(view.account_ptr() as u64 == expected);
    }
    views
}

#[macro_export]
macro_rules! account_views_with_mem_layout {
    () => {
        nondet::fun_account_views_with_mem_layout()
    };
}
