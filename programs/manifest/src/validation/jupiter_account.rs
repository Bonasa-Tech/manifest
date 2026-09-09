//! Account views for the Jupiter quoter, built away from the runtime.
//!
//! The quoter in `client/rust/jup` is the only caller. It answers a swap quote
//! without sending a transaction, which means running this program's own state
//! and matching code over market bytes fetched from RPC; anything less would
//! quote something other than what the program does.
//!
//! That code reads accounts as [`AccountView`], a pointer to a
//! [`RuntimeAccount`] header immediately followed by that account's data. On
//! chain the runtime lays that out. Off chain nothing does, so [`OwnedAccount`]
//! allocates it: one buffer holding the header and the data, from which a view
//! can be taken.
//!
//! It is not a way to fake an account to the runtime: nothing here can make
//! the runtime treat these bytes as an account, and a CPI made with one would
//! be rejected. It exists so code that only reads can be exercised off chain.

use std::alloc::{alloc_zeroed, dealloc, Layout};

use pinocchio::account::{AccountView, RuntimeAccount, MAX_PERMITTED_DATA_INCREASE, NOT_BORROWED};
use solana_program::pubkey::Pubkey;

use crate::validation::as_raw_key;

/// An account laid out the way the runtime lays one out, on the heap.
pub struct OwnedAccount {
    /// Header followed by `data_len` bytes of account data, then the slack a
    /// resize is allowed to use.
    buffer: *mut u8,
    layout: Layout,
}

impl OwnedAccount {
    /// Lays out an account with `data` and returns it.
    ///
    /// The data is copied, so later writes through the view do not touch the
    /// caller's bytes.
    pub fn new(address: &Pubkey, owner: &Pubkey, lamports: u64, data: &[u8]) -> Self {
        let size: usize = size_of::<RuntimeAccount>() + data.len() + MAX_PERMITTED_DATA_INCREASE;
        // The header is read as a `RuntimeAccount`, so the allocation has to
        // satisfy that type's alignment.
        let layout: Layout =
            Layout::from_size_align(size, align_of::<RuntimeAccount>()).expect("valid layout");
        // SAFETY: the layout is non zero and correctly aligned.
        let buffer: *mut u8 = unsafe { alloc_zeroed(layout) };
        assert!(!buffer.is_null(), "out of memory building an account");

        // SAFETY: the allocation is at least the size of the header and is
        // aligned for it.
        unsafe {
            buffer.cast::<RuntimeAccount>().write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: 0,
                is_writable: 1,
                executable: 0,
                resize_delta: 0,
                address: as_raw_key(address).clone(),
                owner: as_raw_key(owner).clone(),
                lamports,
                data_len: data.len() as u64,
            });
            std::ptr::copy_nonoverlapping(
                data.as_ptr(),
                buffer.add(size_of::<RuntimeAccount>()),
                data.len(),
            );
        }

        Self { buffer, layout }
    }

    /// A view of this account.
    ///
    /// # Safety
    ///
    /// The returned view is a bare pointer into this buffer. It carries no
    /// lifetime, and `AccountView` is `Clone`, so nothing in the type system
    /// keeps it from outliving the `OwnedAccount` that owns the memory. The
    /// caller must not use the view, or anything cloned or borrowed from it,
    /// after this `OwnedAccount` is dropped, and must not alias the data
    /// except through the view's own `try_borrow` and `try_borrow_mut`.
    pub unsafe fn view(&self) -> AccountView {
        // SAFETY: `buffer` holds a `RuntimeAccount` followed by exactly
        // `data_len` bytes of data, which is the invariant `AccountView`
        // requires. Outliving the buffer is the caller's contract above.
        unsafe { AccountView::new_unchecked(self.buffer.cast::<RuntimeAccount>()) }
    }

    /// Marks the account as a signer, which some loaders require.
    pub fn set_signer(&mut self, is_signer: bool) {
        // SAFETY: the header is at the start of the buffer.
        unsafe { (*self.buffer.cast::<RuntimeAccount>()).is_signer = u8::from(is_signer) };
    }
}

impl Drop for OwnedAccount {
    fn drop(&mut self) {
        // SAFETY: allocated with this layout in `new` and not freed since.
        unsafe { dealloc(self.buffer, self.layout) };
    }
}

/// The address an `AccountView` reports, for callers holding a `Pubkey`.
pub fn view_address(view: &AccountView) -> &Pubkey {
    crate::validation::as_pubkey(view.address())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_built_account_reads_back() {
        let address: Pubkey = Pubkey::new_unique();
        let owner: Pubkey = Pubkey::new_unique();
        let data: Vec<u8> = (0..64u8).collect();
        let account: OwnedAccount = OwnedAccount::new(&address, &owner, 42, &data);

        // SAFETY: `account` outlives every use of the view below.
        let view = unsafe { account.view() };
        assert_eq!(view_address(&view), &address);
        assert_eq!(view.lamports(), 42);
        assert_eq!(view.data_len(), data.len());
        assert!(view.owned_by(as_raw_key(&owner)));
        assert_eq!(&*view.try_borrow().unwrap(), &data[..]);
    }

    #[test]
    fn writes_through_the_view_are_visible() {
        let account: OwnedAccount =
            OwnedAccount::new(&Pubkey::new_unique(), &crate::ID, 0, &[0u8; 8]);
        {
            // SAFETY: `account` outlives every use of the view below.
            let view = unsafe { account.view() };
            let mut borrowed = view.try_borrow_mut().unwrap();
            borrowed.copy_from_slice(&[7u8; 8]);
        }
        // Read back through a fresh view, which is the only way in: the data
        // is reached through the view's own borrow tracking.
        // SAFETY: `account` outlives every use of the view below.
        let view = unsafe { account.view() };
        assert_eq!(&*view.try_borrow().unwrap(), &[7u8; 8]);
    }

    #[test]
    fn two_views_of_one_account_share_borrow_state() {
        let account: OwnedAccount =
            OwnedAccount::new(&Pubkey::new_unique(), &crate::ID, 0, &[0u8; 8]);
        // SAFETY: `account` outlives every use of the view below.
        let first = unsafe { account.view() };
        // SAFETY: `account` outlives every use of the view below.
        let second = unsafe { account.view() };
        let _held = first.try_borrow_mut().unwrap();
        // The borrow flag lives in the shared header, so the second view sees it.
        assert!(second.try_borrow().is_err());
    }
}
