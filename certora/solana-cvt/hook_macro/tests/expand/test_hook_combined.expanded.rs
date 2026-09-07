use hook_macro::{cvt_hook_start, cvt_hook_end};
fn hook_start() {
    ();
}
fn hook_end() {
    ();
}
fn tmp() -> Result<()> {
    hook_start();
    hook_end();
    Ok(())
}
fn t3() {
    match (&tmp(), &Ok(())) {
        (left_val, right_val) => {
            if !(*left_val == *right_val) {
                let kind = ::core::panicking::AssertKind::Eq;
                ::core::panicking::assert_failed(
                    kind,
                    &*left_val,
                    &*right_val,
                    ::core::option::Option::None,
                );
            }
        }
    };
}
fn t4() {
    hook_start();
    match (&1, &1) {
        (left_val, right_val) => {
            if !(*left_val == *right_val) {
                let kind = ::core::panicking::AssertKind::Eq;
                ::core::panicking::assert_failed(
                    kind,
                    &*left_val,
                    &*right_val,
                    ::core::option::Option::None,
                );
            }
        }
    };
    hook_end();
}
fn abs(x: i32) -> i32 {
    hook_start();
    if x >= 0 {
        {
            ::std::io::_print(format_args!("x is positive\n"));
        };
        x
    } else {
        {
            ::std::io::_print(format_args!("x is negative\n"));
        };
        -x
    }
}
fn abs2(x: i32) -> i32 {
    hook_end();
    if x >= 0 {
        {
            ::std::io::_print(format_args!("x is positive\n"));
        };
        x
    } else {
        {
            ::std::io::_print(format_args!("x is negative\n"));
        };
        -x
    }
}
