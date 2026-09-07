#[derive(Eq, Debug, Copy, Clone)]
/// Nativeematical Integer (represented by u64 number)
///
/// The magic is that symbolically an SBF word is mapped to 256 bit symbolic
/// integer.
pub struct NativeIntU64(u64);

/// Declaration for external library for mathematical integers
///
/// This library is implemented symbolically by Certora Prover
/// Run-time under-approximation is provided in [rt_impls] module
mod rt_decls {
    type BoolU64 = u64;

    extern "C" {
        pub fn CVT_nativeint_u64_eq(_: u64, _: u64) -> BoolU64;
        pub fn CVT_nativeint_u64_lt(_: u64, _: u64) -> BoolU64;
        pub fn CVT_nativeint_u64_le(_: u64, _: u64) -> BoolU64;

        pub fn CVT_nativeint_u64_add(_: u64, _: u64) -> u64;
        pub fn CVT_nativeint_u64_mul(_: u64, _: u64) -> u64;
        pub fn CVT_nativeint_u64_div(_: u64, _: u64) -> u64;
        pub fn CVT_nativeint_u64_div_ceil(_: u64, _: u64) -> u64;
        pub fn CVT_nativeint_u64_muldiv(_: u64, _: u64, _: u64) -> u64;
        pub fn CVT_nativeint_u64_muldiv_ceil(_: u64, _: u64, _: u64) -> u64;

        pub fn CVT_nativeint_u64_nondet() -> u64;

        pub fn CVT_nativeint_u64_from_u128(w0: u64, w1: u64) -> u64;
        pub fn CVT_nativeint_u64_from_u256(w0: u64, w1: u64, w2: u64, w3: u64) -> u64;

        pub fn CVT_nativeint_u64_u64_max() -> u64;
        pub fn CVT_nativeint_u64_u128_max() -> u64;
        pub fn CVT_nativeint_u64_u256_max() -> u64;
    }
}

/// Run-time implementation of the external library
///
/// This implementation is intendent as an under-approximation of the symbolic
/// behaviour. It is intended to be used for testing.
#[cfg(feature = "rt")]
mod rt_impls {
    #[no_mangle]
    pub extern "C" fn CVT_nativeint_u64_eq(a: u64, b: u64) -> u64 {
        (a == b).into()
    }

    #[no_mangle]
    pub extern "C" fn CVT_nativeint_u64_lt(a: u64, b: u64) -> u64 {
        (a < b).into()
    }

    #[no_mangle]
    pub extern "C" fn CVT_nativeint_u64_le(a: u64, b: u64) -> u64 {
        (a <= b).into()
    }

    #[no_mangle]
    pub extern "C" fn CVT_nativeint_u64_add(a: u64, b: u64) -> u64 {
        a.checked_add(b).unwrap()
    }

    #[no_mangle]
    pub extern "C" fn CVT_nativeint_u64_mul(a: u64, b: u64) -> u64 {
        a.checked_mul(b).unwrap()
    }

    #[no_mangle]
    pub extern "C" fn CVT_nativeint_u64_div(a: u64, b: u64) -> u64 {
        a.checked_div(b).unwrap()
    }

    #[no_mangle]
    pub extern "C" fn CVT_nativeint_u64_div_ceil(a: u64, b: u64) -> u64 {
        a.div_ceil(b)
    }

    #[no_mangle]
    pub extern "C" fn CVT_nativeint_u64_muldiv(a: u64, b: u64, c: u64) -> u64 {
        a.checked_mul(b).unwrap().checked_div(c).unwrap()
    }

    #[no_mangle]
    pub extern "C" fn CVT_nativeint_u64_muldiv_ceil(a: u64, b: u64, c: u64) -> u64 {
        a.checked_mul(b).unwrap().div_ceil(c)
    }

    #[no_mangle]
    pub extern "C" fn CVT_nativeint_u64_nondet() -> u64 {
        // -- concrete implementation returns some specific number
        // -- it can, potentially, return a random number instead, or depend on
        // -- run-time of nondet
        0
    }

    #[no_mangle]
    pub extern "C" fn CVT_nativeint_u64_from_u128(w0: u64, w1: u64) -> u64 {
        if w1 != 0 {
            panic!();
        }
        w0
    }

    #[no_mangle]
    pub extern "C" fn CVT_nativeint_u64_from_u256(w0: u64, w1: u64, w2: u64, w3: u64) -> u64 {
        if w1 != 0 || w2 != 0 || w3 != 0 {
            panic!();
        }
        w0
    }

    #[no_mangle]
    pub extern "C" fn CVT_nativeint_u64_u64_max() -> u64 {
        u64::MAX
    }

    #[no_mangle]
    pub extern "C" fn CVT_nativeint_u64_u128_max() -> u64 {
        panic!();
    }

    #[no_mangle]
    pub extern "C" fn CVT_nativeint_u64_u256_max() -> u64 {
        panic!();
    }
}

use rt_decls::*;

impl NativeIntU64 {
    pub fn new<T>(v: T) -> Self
    where
        T: Into<NativeIntU64>,
    {
        v.into()
    }

    pub fn div_ceil(self, rhs: Self) -> Self {
        unsafe { Self(CVT_nativeint_u64_div_ceil(self.0, rhs.0)) }
    }

    pub fn muldiv(self, num: Self, den: Self) -> Self {
        unsafe { Self(CVT_nativeint_u64_muldiv(self.0, num.0, den.0)) }
    }

    pub fn muldiv_ceil(self, num: Self, den: Self) -> Self {
        unsafe { Self(CVT_nativeint_u64_muldiv_ceil(self.0, num.0, den.0)) }
    }

    pub fn from_u128(w0: u64, w1: u64) -> Self {
        unsafe { Self(CVT_nativeint_u64_from_u128(w0, w1)) }
    }

    pub fn from_u256(w0: u64, w1: u64, w2: u64, w3: u64) -> Self {
        unsafe { Self(CVT_nativeint_u64_from_u256(w0, w1, w2, w3)) }
    }

    pub fn u64_max() -> Self {
        unsafe { Self(CVT_nativeint_u64_u64_max()) }
    }

    pub fn u128_max() -> Self {
        unsafe { Self(CVT_nativeint_u64_u128_max()) }
    }

    pub fn u256_max() -> Self {
        unsafe { Self(CVT_nativeint_u64_u256_max()) }
    }

    pub fn nondet() -> Self {
        nondet::nondet()
    }

    // Expose internal representation. Internal use only.
    pub fn as_internal(&self) -> u64 { 
        self.0
    }
}

impl PartialEq for NativeIntU64 {
    fn eq(&self, other: &Self) -> bool {
        unsafe { CVT_nativeint_u64_eq(self.0, other.0) != 0 }
    }
}

impl PartialOrd for NativeIntU64 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        let ord = if self.0 == other.0 {
            std::cmp::Ordering::Equal
        } else if self.0 < other.0 {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        };
        Some(ord)
    }
    fn lt(&self, other: &NativeIntU64) -> bool {
        unsafe { CVT_nativeint_u64_lt(self.0, other.0) != 0 }
    }
    fn le(&self, other: &NativeIntU64) -> bool {
        unsafe { CVT_nativeint_u64_le(self.0, other.0) != 0 }
    }
    fn gt(&self, other: &NativeIntU64) -> bool {
        other.lt(self)
    }
    fn ge(&self, other: &NativeIntU64) -> bool {
        other.le(self)
    }
}

impl Ord for NativeIntU64 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        if self.lt(&other) {
            std::cmp::Ordering::Less
        } else if self.gt(&other) {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    }

    fn max(self, other: Self) -> Self {
        if self.gt(&other) {
            self
        } else {
            other
        }
    }

    fn min(self, other: Self) -> Self {
        if self.gt(&other) {
            other
        } else {
            self
        }
    }

    fn clamp(self, min: Self, max: Self) -> Self {
        if self.gt(&max) {
            max
        } else if self.lt(&min) {
            min
        } else {
            self
        }
    }
}

impl std::ops::Add<NativeIntU64> for NativeIntU64 {
    type Output = Self;

    fn add(self, rhs: NativeIntU64) -> Self::Output {
        unsafe { Self(CVT_nativeint_u64_add(self.0, rhs.0)) }
    }
}

impl std::ops::Mul<NativeIntU64> for NativeIntU64 {
    type Output = Self;

    fn mul(self, rhs: NativeIntU64) -> Self::Output {
        unsafe { Self(CVT_nativeint_u64_mul(self.0, rhs.0)) }
    }
}

impl std::ops::Div<NativeIntU64> for NativeIntU64 {
    type Output = Self;

    fn div(self, rhs: NativeIntU64) -> Self::Output {
        unsafe { Self(CVT_nativeint_u64_div(self.0, rhs.0)) }
    }
}

impl std::ops::Add<u64> for NativeIntU64 {
    type Output = Self;

    fn add(self, rhs: u64) -> Self::Output {
        self + Self(rhs)
    }
}

impl std::ops::Mul<u64> for NativeIntU64 {
    type Output = Self;

    fn mul(self, rhs: u64) -> Self::Output {
        self * Self(rhs)
    }
}

impl std::ops::Div<u64> for NativeIntU64 {
    type Output = Self;

    fn div(self, rhs: u64) -> Self::Output {
        self / Self(rhs)
    }
}

impl From<u64> for NativeIntU64 {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<u128> for NativeIntU64 {
    fn from(value: u128) -> Self {
        let w0: u64 = (value & 0xffff_ffff_ffff_ffff) as u64;
        let w1: u64 = (value >> 64) as u64;

        Self::from_u128(w0, w1)
    }
}

impl From<&[u64; 2]> for NativeIntU64 {
    fn from(value: &[u64; 2]) -> Self {
        Self::from_u128(value[0], value[1])
    }
}

impl From<&[u64; 4]> for NativeIntU64 {
    fn from(value: &[u64; 4]) -> Self {
        Self::from_u256(value[0], value[1], value[2], value[3])
    }
}

impl From<&[u8; 32]> for NativeIntU64 {
    fn from(value: &[u8; 32]) -> Self {
        let (w0, rest) = value.split_at(8);
        let w0 = u64::from_le_bytes(w0.try_into().unwrap());
        let (w1, rest) = rest.split_at(8);
        let w1 = u64::from_le_bytes(w1.try_into().unwrap());
        let (w2, rest) = rest.split_at(8);
        let w2 = u64::from_le_bytes(w2.try_into().unwrap());
        let w3 = u64::from_le_bytes(rest.try_into().unwrap());
        unsafe { Self(CVT_nativeint_u64_from_u256(w0, w1, w2, w3)) }
    }
}

impl From<&[u8]> for NativeIntU64 {
    fn from(value: &[u8]) -> Self {
        let v: &[u8; 32] = value.try_into().unwrap();
        Self::from(v)
    }

}

impl ::nondet::Nondet for NativeIntU64 {
    fn nondet() -> NativeIntU64 {
        unsafe { Self(CVT_nativeint_u64_nondet()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let x = NativeIntU64(2);
        let y = NativeIntU64(4);
        assert_eq!(x + y, NativeIntU64(6));
        assert_eq!(x + y, 6u64.into());
        assert!(x < 6u64.into());
    }

    #[test]
    fn nondet_test() {
        let x: NativeIntU64 = nondet::nondet();
        assert_eq!(x, 0u64.into());
    }
}
