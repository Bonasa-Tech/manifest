// We redefine this macro to avoid including error code conversion/formatting
#[macro_export]
macro_rules! require_keys_eq {
    ($value1: expr, $value2: expr, $error_code:expr $(,)?) => {
        cvt::CVT_assume($value1 == $value2);
    };
    ($value1: expr, $value2: expr $(,)?) => {
        cvt::CVT_assume($value1 == $value2);
    };
}

#[macro_export]
macro_rules! require_keys_neq {
    ($value1: expr, $value2: expr, $error_code:expr $(,)?) => {
        cvt::CVT_assume($value1 != $value2);
    };
    ($value1: expr, $value2: expr $(,)?) => {
        cvt::CVT_assume($value1 != $value2);
    };
}

// We redefine invoke and invoke_signed to avoid the signer checks
// TODO: we might want to move these macros to mocks
#[macro_export]
macro_rules! invoke {
    ($instruction: expr, $acc_infos: expr $(,)*) => {{
        solana_program::program::invoke_signed_unchecked($instruction, $acc_infos, &[])
    }};
}

#[macro_export]
macro_rules! invoke_signed {
    ($instruction: expr, $acc_infos: expr, $seeds: expr $(,)*) => {{
        solana_program::program::invoke_signed_unchecked($instruction, $acc_infos, $seeds)
    }};
}
