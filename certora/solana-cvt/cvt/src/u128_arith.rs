#[allow(improper_ctypes)]
extern "C" {
    fn CVT_u128_leq_c(x: u128, y: u128) -> bool;
    fn CVT_u128_gt0_c(x: u128) -> bool;
    fn CVT_u128_ceil_div_c(x: u128, y: u128) -> u128;
}

#[inline(never)]
#[allow(non_snake_case)]
pub fn CVT_u128_leq(x: u128, y: u128) -> bool {
    return unsafe { CVT_u128_leq_c(x, y) };
}

#[inline(never)]
#[allow(non_snake_case)]
pub fn CVT_u128_gt0(x: u128) -> bool {
    return unsafe { CVT_u128_gt0_c(x) };
}

#[inline(never)]
#[allow(non_snake_case)]
pub fn CVT_u128_ceil_div(x: u128, y: u128) -> u128 {
    return unsafe { CVT_u128_ceil_div_c(x, y) };
}
