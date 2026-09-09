#![allow(unused_macros)]

// Please, don't import directly these functions in your project.
// Instead, import only the macros.
#[allow(improper_ctypes)]
extern "C" {
    #[allow(dead_code)]
    pub fn CVT_calltrace_print_tag(tag: &str);

    #[allow(dead_code)]
    pub fn CVT_calltrace_print_u64_1(tag: &str, x: u64);
    #[allow(dead_code)]
    pub fn CVT_calltrace_print_u64_2(tag: &str, x: u64, y: u64);
    #[allow(dead_code)]
    pub fn CVT_calltrace_print_u64_3(tag: &str, x: u64, y: u64, z: u64);

    #[allow(dead_code)]
    pub fn CVT_calltrace_print_i64_1(tag: &str, x: i64);
    #[allow(dead_code)]
    pub fn CVT_calltrace_print_i64_2(tag: &str, x: i64, y: i64);
    #[allow(dead_code)]
    pub fn CVT_calltrace_print_i64_3(tag: &str, x: i64, y: i64, z: i64);
}

#[macro_export]
macro_rules! cvt_cex_print_tag {
    ($tag: expr) => {
        unsafe { ::calltrace::CVT_calltrace_print_tag(stringify!($tag)) }
    };
}

#[macro_export]
macro_rules! cvt_cex_print_u64 {
    ($tag: expr $(,)?) => {};
    ($tag: expr, $x: expr) => { unsafe {::calltrace::CVT_calltrace_print_u64_1(stringify!($tag), $x as u64)}};
    ($tag: expr, $x: expr, $y: expr) => { unsafe {::calltrace::CVT_calltrace_print_u64_2(stringify!($tag), $x as u64,$y as u64)}};
    ($tag: expr, $v: expr, $x: expr, $y: expr $(,$z:expr)*) =>
    {
	unsafe {::calltrace::CVT_calltrace_print_u64_3(stringify!($tag), $v as u64, $x as u64,$y as u64)};
	cvt_cex_print_u64!($tag $(,$z)*);
	() // it needs to return an expression
    };
}

#[macro_export]
macro_rules! cvt_cex_print_i64 {
    ($tag: expr $(,)?) => {};
    ($tag: expr, $x: expr) => { unsafe {::calltrace::CVT_calltrace_print_i64_1(stringify!($tag), $x as i64)}};
    ($tag: expr, $x: expr, $y: expr) => { unsafe {::calltrace::CVT_calltrace_print_i64_2(stringify!($tag), $x as i64,$y as i64)}};
    ($tag: expr, $v: expr, $x: expr, $y: expr $(,$z:expr)*) =>
    {
	unsafe {::calltrace::CVT_calltrace_print_i64_3(stringify!($tag), $v as i64, $x as i64,$y as i64)};
	cvt_cex_print_i64!($tag $(,$z)*);
	() // it needs to return an expression
    };
}
