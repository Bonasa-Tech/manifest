#[macro_export]
macro_rules! cvt_assert {
    ($cond: expr $(,)?) => {
        ::cvt::CVT_assert($cond)
    };
}

#[macro_export]
macro_rules! cvt_assume {
    ($cond: expr $(,)?) => {
        ::cvt::CVT_assume($cond)
    };
}

#[cfg(feature = "vacuity")]
#[macro_export]
macro_rules! cvt_vacuity_check {
    () => {
        cvt_assert!(false)
    };
}

#[cfg(not(feature = "vacuity"))]
#[macro_export]
macro_rules! cvt_vacuity_check {
    () => {
        ::cvt::cvt_assert!(true)
    };
}

#[macro_export]
macro_rules! cvt_satisfy {
    ($cond: expr $(,)?) => {
        ::cvt::CVT_satisfy($cond)
    };
}

#[macro_export]
macro_rules! cvt_assert_eq {
    ($value1: expr, $value2: expr) => {
        ::cvt::cvt_assert!($value1 == $value2)
    };
}

#[macro_export]
macro_rules! cvt_assert_neq {
    ($value1: expr, $value2: expr) => {
        ::cvt::cvt_assert!($value1 != $value2)
    };
}

// local macro, used by asserts below
#[macro_export]
macro_rules! cmp_pubkeys {
    ($value1: expr, $value2: expr $(,)?) => {
        $value1 == $value2
    };
}

#[macro_export]
macro_rules! assert_pk_eq {
    ($value1: expr, $value2: expr $(,)?) => {
        ::cvt::cvt_assert!(cmp_pubkeys!($value1, $value2))
    };
}

#[macro_export]
macro_rules! cvt_assert_key_neq {
    ($acc_info_1: expr, $acc_info_2: expr) => {
        ::cvt::assert_pk_neq!($acc_info_1.key, $acc_info_2.key)
    };
}

#[macro_export]
macro_rules! assert_pk_neq {
    ($value1: expr, $value2: expr $(,)?) => {
        ::cvt::cvt_assert!(!cmp_pubkeys!($value1, $value2))
    };
}

#[macro_export]
macro_rules! assert_some_pk_eq {
    ($value1: expr, $value2: expr $(,)?) => {{
        let value1 = &$value1;
        ::cvt::cvt_assert!(value1.is_some());
        ::cvt::assert_pk_eq!(&value1.unwrap(), $value2);
    }};
}

#[macro_export]
macro_rules! require_pk_eq {
    ($value1: expr, $value2: expr $(,)?) => {
        ::cvt::cvt_assume!(cmp_pubkeys!($value1, $value2))
    };
}

#[macro_export]
macro_rules! require_pk_neq {
    ($value1: expr, $value2: expr $(,)?) => {
        ::cvt::cvt_assume!(!cmp_pubkeys!($value1, $value2))
    };
}
