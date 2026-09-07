use solana_program::{account_info::AccountInfo, pubkey::Pubkey};

extern "C" {
    #[allow(improper_ctypes)]
    fn mk_account_info_unchecked() -> AccountInfo<'static>;
    #[allow(improper_ctypes)]
    fn mk_pubkey_unchecked() -> Pubkey;
}

#[allow(non_snake_case)]
pub fn CVT_nondet_account_info_impl() -> AccountInfo<'static> {
    unsafe {
        return mk_account_info_unchecked();
    }
}

#[allow(non_snake_case)]
pub fn CVT_nondet_pubkey_impl() -> Pubkey {
    unsafe {
        return mk_pubkey_unchecked();
    }
}

#[cfg(feature = "impls")]
mod impls {
    //! Default implementations of external functions
    //!
    //! These functions are not expected to be called. They are here only to
    //! allow compining test modules that require all external functions to be
    //! resolved

    use super::{AccountInfo, Pubkey};
    use solana_sdk::account::Account;

    #[allow(improper_ctypes)]
    #[no_mangle]
    extern "C" fn mk_pubkey_unchecked() -> Pubkey {
        Default::default()
    }
    #[allow(improper_ctypes, improper_ctypes_definitions)]
    #[no_mangle]
    extern "C" fn mk_account_info_unchecked() -> AccountInfo<'static> {
        let owner: &'static mut _ = Box::leak(Box::new(Pubkey::new_unique()));
        let acc = Account::new(1, 2, &owner);
        let pk: &'static mut _ = Box::leak(Box::new(Pubkey::new_unique()));
        let v: &'static mut _ = Box::leak(Box::new((*pk, acc)));
        v.into()
    }
}
