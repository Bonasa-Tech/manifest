use crate::program::ManifestError;
use borsh::{BorshDeserialize as Deserialize, BorshSerialize as Serialize};
use bytemuck::{Pod, Zeroable};
use hypertree::trace;
use pinocchio::error::ProgramError;
use shank::ShankAccount;
use static_assertions::const_assert;
use std::{
    cmp::Ordering,
    fmt::Display,
    ops::{Add, AddAssign, Div, Sub, SubAssign},
    u128, u32, u64,
};

/// New and as_u64 for creating and switching to u64 when needing to use base or
/// quote
pub trait WrapperU64 {
    fn new(value: u64) -> Self;
    fn as_u64(&self) -> u64;
}

macro_rules! checked_math {
    ($type_name:ident) => {
        impl $type_name {
            #[inline(always)]
            pub fn checked_add(self, other: Self) -> Result<$type_name, ManifestError> {
                let result_or: Option<u64> = self.inner.checked_add(other.inner);
                if result_or.is_none() {
                    Err(ManifestError::Overflow)
                } else {
                    Ok($type_name::new(result_or.unwrap()))
                }
            }

            #[inline(always)]
            pub fn checked_sub(self, other: Self) -> Result<$type_name, ManifestError> {
                let result_or: Option<u64> = self.inner.checked_sub(other.inner);
                if result_or.is_none() {
                    Err(ManifestError::Overflow)
                } else {
                    Ok($type_name::new(result_or.unwrap()))
                }
            }
        }
    };
}

macro_rules! overflow_math {
    ($type_name:ident) => {
        impl $type_name {
            #[inline(always)]
            pub fn overflowing_add(self, other: Self) -> ($type_name, bool) {
                let (sum, overflow) = self.inner.overflowing_add(other.inner);
                ($type_name::new(sum), overflow)
            }

            #[inline(always)]
            pub fn saturating_add(self, other: Self) -> $type_name {
                let sum = self.inner.saturating_add(other.inner);
                $type_name::new(sum)
            }

            #[inline(always)]
            pub fn saturating_sub(self, other: Self) -> $type_name {
                let difference = self.inner.saturating_sub(other.inner);
                $type_name::new(difference)
            }

            #[inline(always)]
            pub fn wrapping_add(self, other: Self) -> $type_name {
                let sum = self.inner.wrapping_add(other.inner);
                $type_name::new(sum)
            }

            #[inline(always)]
            pub fn wrapping_sub(self, other: Self) -> $type_name {
                let difference = self.inner.wrapping_sub(other.inner);
                $type_name::new(difference)
            }
        }
    };
}

macro_rules! basic_math {
    ($type_name:ident) => {
        impl Add for $type_name {
            type Output = Self;

            #[inline(always)]
            fn add(self, other: Self) -> Self {
                $type_name::new(self.inner + other.inner)
            }
        }

        impl AddAssign for $type_name {
            #[inline(always)]
            fn add_assign(&mut self, other: Self) {
                *self = *self + other;
            }
        }

        impl Sub for $type_name {
            type Output = Self;

            #[inline(always)]
            fn sub(self, other: Self) -> Self {
                $type_name::new(self.inner - other.inner)
            }
        }

        impl SubAssign for $type_name {
            #[inline(always)]
            fn sub_assign(&mut self, other: Self) {
                *self = *self - other;
            }
        }

        impl Default for $type_name {
            #[inline(always)]
            fn default() -> Self {
                Self::ZERO
            }
        }

        impl Display for $type_name {
            #[inline(always)]
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                self.inner.fmt(f)
            }
        }

        impl PartialEq for $type_name {
            #[inline(always)]
            fn eq(&self, other: &Self) -> bool {
                self.inner == other.inner
            }
        }

        impl Eq for $type_name {}
    };
}

macro_rules! basic_u64 {
    ($type_name:ident) => {
        impl WrapperU64 for $type_name {
            #[inline(always)]
            fn new(value: u64) -> Self {
                $type_name { inner: value }
            }

            #[inline(always)]
            fn as_u64(&self) -> u64 {
                self.inner
            }
        }

        impl $type_name {
            pub const ZERO: Self = $type_name { inner: 0 };
            pub const ONE: Self = $type_name { inner: 1 };

            #[inline(always)]
            pub fn min(self, other: Self) -> Self {
                if self.inner <= other.inner {
                    self
                } else {
                    other
                }
            }
        }

        impl From<$type_name> for u64 {
            #[inline(always)]
            fn from(x: $type_name) -> u64 {
                x.inner
            }
        }

        // Below should only be used in tests.
        impl PartialEq<u64> for $type_name {
            #[inline(always)]
            fn eq(&self, other: &u64) -> bool {
                self.inner == *other
            }
        }

        impl PartialEq<$type_name> for u64 {
            #[inline(always)]
            fn eq(&self, other: &$type_name) -> bool {
                *self == other.inner
            }
        }

        basic_math!($type_name);
        checked_math!($type_name);
        overflow_math!($type_name);
    };
}

#[derive(
    Debug, Clone, Copy, PartialOrd, Ord, Zeroable, Pod, Deserialize, Serialize, ShankAccount,
)]
#[repr(transparent)]
pub struct QuoteAtoms {
    inner: u64,
}
basic_u64!(QuoteAtoms);

#[derive(
    Debug, Clone, Copy, PartialOrd, Ord, Zeroable, Pod, Deserialize, Serialize, ShankAccount,
)]
#[repr(transparent)]
pub struct BaseAtoms {
    inner: u64,
}
basic_u64!(BaseAtoms);

#[derive(
    Debug, Clone, Copy, PartialOrd, Ord, Zeroable, Pod, Deserialize, Serialize, ShankAccount,
)]
#[repr(transparent)]
pub struct GlobalAtoms {
    inner: u64,
}
basic_u64!(GlobalAtoms);

// Manifest pricing
#[derive(Clone, Copy, Default, Zeroable, Pod, Deserialize, Serialize, ShankAccount)]
#[repr(C)]
pub struct QuoteAtomsPerBaseAtom {
    pub(crate) inner: [u64; 2],
}

// These conversions are necessary, bc. the compiler forces 16 byte alignment
// on the u128 type, which is not necessary given that the target architecture
// has no native support for u128 math and requires us only to be 8 byte
// aligned.
#[cfg(not(feature = "certora"))]
/// The little endian split of a u128 into its low and high words.
///
/// Done with shifts rather than by reinterpreting the bytes. A `u128` may
/// need 16 byte alignment while `[u64; 2]` guarantees only 8, so the pointer
/// version had to go through an unaligned read, and that made the compiler
/// round the value through the stack instead of keeping it in the register
/// pair it already occupies. These conversions sit under every price
/// comparison and every piece of price arithmetic, so that round trip was
/// paid at every level of every orderbook tree descent.
const fn u128_to_u64_slice(a: u128) -> [u64; 2] {
    [a as u64, (a >> 64) as u64]
}
pub(crate) const fn u64_slice_to_u128(a: [u64; 2]) -> u128 {
    ((a[1] as u128) << 64) | (a[0] as u128)
}

#[cfg(not(feature = "certora"))]
const ATOM_LIMIT: u128 = u64::MAX as u128;
const D18: u128 = 10u128.pow(18);
/// Exact `x / 10^18`. The common u64 quotient uses normalized long division
/// with native 64-bit operations. sBPF v3 lacks v2's high-multiply opcode,
/// which made the previous 256-bit reciprocal product expensive. Keeping the
/// remainder also makes ceiling division cheap without multiplying back.
#[cfg(all(not(feature = "certora"), test))]
#[inline(always)]
fn div_d18(x: u128) -> u128 {
    div_rem_d18(x).0
}

/// Formal verification keeps the plain division.
///
/// The Certora Solana prover does not model 128-bit arithmetic bit-precisely:
/// the compiler-rt helpers that implement u128 multiplication and division
/// (`__multi3`, `__udivti3`) are summarized in `certora/cvt_summaries.txt` as
/// typed but otherwise unconstrained numbers. Existing rules reason on top
/// of those summaries, so formal builds retain the original full-width
/// expressions. `div_d18_test` separately checks the optimized arithmetic
/// against Rust's full-width operations on boundaries, bit patterns, random
/// inputs, and complete price conversions, including overflow and rounding.
#[cfg(feature = "certora")]
#[allow(dead_code)]
#[inline(always)]
fn div_d18(x: u128) -> u128 {
    x / D18
}

/// `ceil(x / 10^18)`.
#[cfg(any(feature = "certora", test))]
#[cfg_attr(feature = "certora", allow(dead_code))]
#[inline(always)]
fn div_ceil_d18(x: u128) -> u128 {
    #[cfg(not(feature = "certora"))]
    {
        let (quotient, remainder) = div_rem_d18(x);
        quotient + u128::from(remainder != 0)
    }
    #[cfg(feature = "certora")]
    {
        let quotient: u128 = div_d18(x);
        // quotient * 10^18 <= x, so this cannot overflow.
        quotient + ((x != quotient * D18) as u128)
    }
}

/// Divide in base 2^32 using two quotient digits. Normalizing 10^18 by four
/// bits makes its high digit at least 2^31, so each quotient estimate needs
/// at most two corrections. All products fit u64; the wrapping operations
/// discard the high base-2^32 digits that cancel in the subtraction.
#[cfg(not(feature = "certora"))]
#[inline(always)]
fn div_rem_d18(x: u128) -> (u128, u64) {
    const DIVISOR: u64 = D18 as u64;
    const NORMALIZED: u64 = DIVISOR << 4;
    const HIGH: u64 = NORMALIZED >> 32;
    const LOW: u64 = NORMALIZED & u32::MAX as u64;
    const BASE: u64 = 1 << 32;
    const MASK: u64 = BASE - 1;

    #[inline(always)]
    fn digit(top: u64, next: u64) -> u64 {
        let mut quotient = top / HIGH;
        let mut remainder = top - quotient * HIGH;
        // LOW < HIGH, so quotient * LOW <= top and cannot overflow.
        while quotient >= BASE || quotient * LOW > ((remainder << 32) | next) {
            quotient -= 1;
            remainder += HIGH;
            if remainder >= BASE {
                break;
            }
        }
        quotient
    }

    let high = (x >> 64) as u64;
    let low = x as u64;
    if high == 0 {
        return ((low / DIVISOR) as u128, low % DIVISOR);
    }
    // The fast path has a u64 quotient. Larger results are rejected by the
    // caller's atom limit, but retain exact full-width semantics here.
    if high >= DIVISOR {
        return (x / D18, (x % D18) as u64);
    }
    let top = (high << 4) | (low >> 60);
    let bottom = low << 4;
    let next = bottom >> 32;
    let last = bottom & MASK;
    let q1 = digit(top, next);
    let middle = (top << 32)
        .wrapping_add(next)
        .wrapping_sub(q1.wrapping_mul(NORMALIZED));
    let q0 = digit(middle, last);
    let remainder = (middle << 32)
        .wrapping_add(last)
        .wrapping_sub(q0.wrapping_mul(NORMALIZED))
        >> 4;
    (((q1 << 32) | q0) as u128, remainder)
}

/// Checked 128-by-64 product using 32-bit limbs for the high half. sBPF v3
/// has native 64-bit low multiplication but no high multiplication opcode.
#[cfg(not(feature = "certora"))]
#[inline(always)]
fn checked_price_product(price: [u64; 2], amount: u64) -> Option<u128> {
    let a0 = price[0] & u32::MAX as u64;
    let a1 = price[0] >> 32;
    let b0 = amount & u32::MAX as u64;
    let b1 = amount >> 32;
    let p00 = a0 * b0;
    // Each limb is at most 2^32-1, so these sums fit u64.
    let middle = a1 * b0 + (p00 >> 32);
    let carry = (middle & u32::MAX as u64) + a0 * b1;
    let high = a1 * b1 + (middle >> 32) + (carry >> 32);
    let high = if price[1] == 0 {
        high
    } else {
        high.checked_add(price[1].checked_mul(amount)?)?
    };
    Some(((high as u128) << 64) | price[0].wrapping_mul(amount) as u128)
}

#[cfg(test)]
mod div_d18_test {
    use super::*;

    /// Reference: the plain division the fast path replaces.
    fn reference(x: u128, round_up: bool) -> u128 {
        if round_up {
            x.div_ceil(D18)
        } else {
            x / D18
        }
    }

    fn check(x: u128) {
        assert_eq!(div_d18(x), reference(x, false), "floor {x}");
        assert_eq!(div_ceil_d18(x), reference(x, true), "ceil {x}");
    }

    /// Simple deterministic generator so the tests need no dependencies.
    struct XorShift(u128);
    impl XorShift {
        fn next(&mut self) -> u128 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    #[test]
    fn edge_values() {
        for x in [
            0,
            1,
            2,
            D18 - 2,
            D18 - 1,
            D18,
            D18 + 1,
            D18 + 2,
            u32::MAX as u128,
            u64::MAX as u128 - 1,
            u64::MAX as u128,
            u64::MAX as u128 + 1,
            1u128 << 64,
            (1u128 << 64) + 1,
            1u128 << 100,
            1u128 << 127,
            (1u128 << 127) + 1,
            u128::MAX - 2,
            u128::MAX - 1,
            u128::MAX,
            D18 * D18 - 1,
            D18 * D18,
            D18 * D18 + 1,
            ATOM_LIMIT * D18 - 1,
            ATOM_LIMIT * D18,
            ATOM_LIMIT * D18 + 1,
            (ATOM_LIMIT + 1) * D18 - 1,
            (ATOM_LIMIT + 1) * D18,
        ] {
            check(x);
        }
    }

    /// Every power of two and its neighbours across the whole width.
    #[test]
    fn powers_of_two() {
        for shift in 0..128u32 {
            let p: u128 = 1u128 << shift;
            for x in [p.wrapping_sub(2), p - 1, p, p + 1, p.wrapping_add(2)] {
                check(x);
            }
            // Also every power of two scaled by the divisor and by 10^9.
            for scale in [D18, 1_000_000_000u128] {
                if let Some(v) = p.checked_mul(scale) {
                    for x in [v - 1, v, v + 1] {
                        check(x);
                    }
                }
            }
        }
    }

    /// Exact multiples of the divisor and their neighbours, which are the
    /// values where floor/ceil rounding flips, for small, large and random
    /// multipliers.
    #[test]
    fn around_multiples_of_the_divisor() {
        for k in 0..=200_000u128 {
            let base: u128 = k * D18;
            for x in [base.wrapping_sub(1), base, base + 1] {
                check(x);
            }
        }
        for shift in 0..64u32 {
            for k in [(1u128 << shift) - 1, 1u128 << shift, (1u128 << shift) + 1] {
                let base: u128 = k * D18;
                for x in [
                    base.wrapping_sub(2),
                    base.wrapping_sub(1),
                    base,
                    base + 1,
                    base + 2,
                ] {
                    check(x);
                }
            }
        }
        // Exact multiples are where a wrong magic number or shift shows
        // itself, so sample the multiplier across the whole range a quotient
        // can take, up to `u128::MAX / 10^18`, which is about 2^68. Drawing
        // 64 bit multipliers would leave the top of that range untested.
        const K_MAX: u128 = u128::MAX / D18;
        let mut rng = XorShift(0x243f6a8885a308d313198a2e03707344);
        for k in [0, 1, 2, K_MAX - 1, K_MAX] {
            check_multiple(k);
        }
        for _ in 0..300_000 {
            check_multiple(rng.next() % (K_MAX + 1));
        }
    }

    /// `k * 10^18` and the two values on either side of it, skipping the ones
    /// that fall outside a `u128`.
    fn check_multiple(k: u128) {
        let base: u128 = k * D18;
        for delta in [2, 1] {
            if base >= delta {
                check(base - delta);
            }
        }
        check(base);
        for delta in [1, 2] {
            if let Some(x) = base.checked_add(delta) {
                check(x);
            }
        }
    }

    /// Every 16-bit pattern in every 16-bit lane of the dividend, with the
    /// other lanes zero, all ones, or the divisor's bits.
    #[test]
    fn bit_lanes() {
        for lane in 0..8u32 {
            for pattern in 0..=u16::MAX {
                let placed: u128 = (pattern as u128) << (16 * lane);
                check(placed);
                check(placed | !((0xffffu128) << (16 * lane)));
                check(placed ^ D18);
            }
        }
    }

    /// Random dividends of every bit length, plus uniformly random ones.
    #[test]
    fn random_dividends() {
        let mut rng = XorShift(0x9e3779b97f4a7c15f39cc0605cedc835);
        for i in 0..2_000_000u32 {
            let raw: u128 = rng.next();
            let x: u128 = match i % 5 {
                0 => raw,
                1 => raw >> (i % 128),
                2 => raw >> 64,
                3 => (raw >> 64).wrapping_mul(D18).wrapping_add(raw % 3),
                _ => raw | (1u128 << 127),
            };
            check(x);
        }
    }

    #[test]
    fn mantissa_conversion_matches_full_width_multiplication() {
        let mut rng = XorShift(0x13198a2e03707344243f6a8885a308d3);
        for exponent in -18..=8i8 {
            let scale = 10u128.pow((exponent + 18) as u32);
            for mantissa in [0, 1, 2, 0x7fff_ffff, 0x8000_0000, u32::MAX]
                .into_iter()
                .chain((0..10_000).map(|_| rng.next() as u32))
            {
                let price =
                    QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(mantissa, exponent)
                        .unwrap();
                assert_eq!(u64_slice_to_u128(price.inner), scale * mantissa as u128);
            }
        }
    }

    #[cfg(not(feature = "certora"))]
    #[test]
    fn price_product_matches_checked_u128_multiplication() {
        let mut rng = XorShift(0x082efa98ec4e6c89452821e638d01377);
        for i in 0..500_000 {
            let raw = rng.next();
            let price = match i % 4 {
                0 => raw,
                1 => raw >> 64,
                2 => u128::MAX >> (i % 128),
                _ => 1u128 << (i % 128),
            };
            let amount = rng.next() as u64 >> (i % 64);
            assert_eq!(
                checked_price_product(u128_to_u64_slice(price), amount),
                price.checked_mul(amount as u128)
            );
        }
        for price in [0, 1, u64::MAX as u128, 1u128 << 64, u128::MAX] {
            for amount in [0, 1, 2, u32::MAX as u64, 1u64 << 32, u64::MAX] {
                assert_eq!(
                    checked_price_product(u128_to_u64_slice(price), amount),
                    price.checked_mul(amount as u128)
                );
            }
        }
    }

    #[test]
    fn decimal_factor_fast_path_and_fallback_match_reference() {
        let mut rng = XorShift(0x9e3779b97f4a7c15d1b54a32d192ed03);
        for i in 0..100_000 {
            let raw = rng.next();
            let factor = if i % 8 < 4 {
                1_000_000_000
            } else {
                1_000_000_000_000
            };
            let factored = ((raw as u64 as u128) / factor) * factor;
            let inner = match i % 4 {
                0 => factored,
                1 => factored + 1,
                2 => factored.saturating_sub(1),
                _ => raw,
            };
            let size = (rng.next() as u64) >> (i % 64);
            let price = QuoteAtomsPerBaseAtom {
                inner: u128_to_u64_slice(inner),
            };
            for round_up in [false, true] {
                let expected = inner
                    .checked_mul(size as u128)
                    .ok_or_else(|| ProgramError::from(PriceConversionError(0x8)))
                    .and_then(|product| {
                        let quote = reference(product, round_up);
                        if quote <= ATOM_LIMIT {
                            Ok(QuoteAtoms::new(quote as u64))
                        } else {
                            Err(PriceConversionError(0x9).into())
                        }
                    });
                assert_eq!(
                    price.checked_quote_for_base(BaseAtoms::new(size), round_up),
                    expected
                );
            }
        }
    }

    #[test]
    fn decimal_factor_product_boundaries() {
        for factor in [1_000_000_000u64, 1_000_000_000_000] {
            for reduced in [1, 999, 1000, u64::MAX / factor] {
                let inner = (reduced * factor) as u128;
                let limit = u64::MAX / reduced;
                let price = QuoteAtomsPerBaseAtom {
                    inner: u128_to_u64_slice(inner),
                };
                for size in [0, 1, limit - 1, limit, limit.saturating_add(1), u64::MAX] {
                    for round_up in [false, true] {
                        let expected = reference(inner * size as u128, round_up);
                        let expected = if expected <= ATOM_LIMIT {
                            Ok(QuoteAtoms::new(expected as u64))
                        } else {
                            Err(PriceConversionError(0x9).into())
                        };
                        assert_eq!(
                            price.checked_quote_for_base(BaseAtoms::new(size), round_up),
                            expected
                        );
                    }
                }
            }
        }
        for size in [0, 1, u64::MAX] {
            for round_up in [false, true] {
                assert_eq!(
                    QuoteAtomsPerBaseAtom::ZERO
                        .checked_quote_for_base(BaseAtoms::new(size), round_up)
                        .unwrap(),
                    QuoteAtoms::ZERO
                );
            }
        }
    }

    /// The fast path is only reachable through `checked_quote_for_base`;
    /// compare that whole function against the reference on a price and size
    /// grid covering every exponent, extreme mantissas and extreme sizes.
    #[test]
    fn checked_quote_for_base_matches_reference() {
        let mantissas: [u32; 7] = [1, 2, 7, 999, 123_456_789, u32::MAX - 1, u32::MAX];
        let sizes: [u64; 9] = [
            1,
            2,
            999,
            1_000_000,
            1_000_000_000,
            1 << 32,
            u64::MAX / 3,
            u64::MAX - 1,
            u64::MAX,
        ];
        let mut cases: u32 = 0;
        for exponent in -18..=8i8 {
            for mantissa in mantissas {
                let Ok(price) =
                    QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(mantissa, exponent)
                else {
                    continue;
                };
                let inner: u128 = u64_slice_to_u128(price.inner);
                for size in sizes {
                    for round_up in [false, true] {
                        let expected: Result<u128, ()> = inner
                            .checked_mul(size as u128)
                            .map(|product| reference(product, round_up))
                            .filter(|quote| *quote <= ATOM_LIMIT)
                            .ok_or(());
                        let actual: Result<u128, ()> = price
                            .checked_quote_for_base(BaseAtoms::new(size), round_up)
                            .map(|quote| quote.as_u64() as u128)
                            .map_err(|_| ());
                        assert_eq!(
                            actual, expected,
                            "{mantissa}e{exponent} x {size} round_up={round_up}"
                        );
                        cases += 1;
                    }
                }
            }
        }
        assert!(cases > 1_000, "grid covered {cases} cases");
        // And random price/size pairs.
        let mut rng = XorShift(0x452821e638d01377be5466cf34e90c6c);
        for _ in 0..200_000 {
            let raw: u128 = rng.next();
            let mantissa: u32 = (raw >> 96) as u32;
            let exponent: i8 = ((raw >> 64) as u8 % 27) as i8 - 18;
            let size: u64 = raw as u64 >> (raw >> 90 & 63);
            let Ok(price) =
                QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(mantissa, exponent)
            else {
                continue;
            };
            let inner: u128 = u64_slice_to_u128(price.inner);
            for round_up in [false, true] {
                let expected: Result<u128, ()> = inner
                    .checked_mul(size as u128)
                    .map(|product| reference(product, round_up))
                    .filter(|quote| *quote <= ATOM_LIMIT)
                    .ok_or(());
                let actual: Result<u128, ()> = price
                    .checked_quote_for_base(BaseAtoms::new(size), round_up)
                    .map(|quote| quote.as_u64() as u128)
                    .map_err(|_| ());
                assert_eq!(
                    actual, expected,
                    "{mantissa}e{exponent} x {size} round_up={round_up}"
                );
            }
        }
    }

    /// Rounding invariants that hold independently of the reference.
    #[test]
    fn rounding_invariants() {
        let mut rng = XorShift(0xc0ac29b7c97c50dd3f84d5b5b5470917);
        for _ in 0..500_000 {
            let x: u128 = rng.next();
            let floor: u128 = div_d18(x);
            let ceil: u128 = div_ceil_d18(x);
            assert!(floor * D18 <= x);
            assert!(x - floor * D18 < D18);
            assert!(ceil == floor || ceil == floor + 1);
            assert_eq!(ceil == floor, x == floor * D18);
        }
    }
}
const D18F: f64 = D18 as f64;

#[cfg(not(feature = "certora"))]
const DECIMAL_CONSTANTS: [u128; 27] = [
    10u128.pow(26),
    10u128.pow(25),
    10u128.pow(24),
    10u128.pow(23),
    10u128.pow(22),
    10u128.pow(21),
    10u128.pow(20),
    10u128.pow(19),
    10u128.pow(18),
    10u128.pow(17),
    10u128.pow(16),
    10u128.pow(15),
    10u128.pow(14),
    10u128.pow(13),
    10u128.pow(12),
    10u128.pow(11),
    10u128.pow(10),
    10u128.pow(09),
    10u128.pow(08),
    10u128.pow(07),
    10u128.pow(06),
    10u128.pow(05),
    10u128.pow(04),
    10u128.pow(03),
    10u128.pow(02),
    10u128.pow(01),
    10u128.pow(00),
];
// ensures that the index lookup is correct when converting from floating point
#[cfg(not(feature = "certora"))]
static_assertions::const_assert_eq!(
    DECIMAL_CONSTANTS[QuoteAtomsPerBaseAtom::MAX_EXP as usize],
    D18
);

// ensures that we can remove bounds checks on certain multiplications
#[cfg(not(feature = "certora"))]
const_assert!(DECIMAL_CONSTANTS[0] * (u32::MAX as u128) < u128::MAX);

const_assert!(D18 * (u64::MAX as u128) < u128::MAX);

#[cfg(feature = "certora")]
#[path = "quantities_certora.rs"]
mod quantities_certora;

#[cfg(not(feature = "certora"))]
impl QuoteAtomsPerBaseAtom {
    pub const ZERO: Self = QuoteAtomsPerBaseAtom { inner: [0; 2] };
    pub const MIN: Self = QuoteAtomsPerBaseAtom::from_mantissa_and_exponent_(1, Self::MIN_EXP);
    pub const MAX: Self =
        QuoteAtomsPerBaseAtom::from_mantissa_and_exponent_(u32::MAX, Self::MAX_EXP);
    pub const MIN_EXP: i8 = -18;
    pub const MAX_EXP: i8 = 8;

    #[inline(always)]
    const fn from_mantissa_and_exponent_(mantissa: u32, exponent: i8) -> Self {
        /* map exponent to array range
          8 ->  [0] -> D26
          0 ->  [8] -> D18
        -10 -> [18] -> D08
        -18 -> [26] ->  D0
        */
        let offset: usize = (Self::MAX_EXP as i64).wrapping_sub(exponent as i64) as usize;
        #[cfg(feature = "certora")]
        {
            let inner: u128 = DECIMAL_CONSTANTS[offset].wrapping_mul(mantissa as u128);
            QuoteAtomsPerBaseAtom {
                inner: u128_to_u64_slice(inner),
            }
        }
        #[cfg(not(feature = "certora"))]
        {
            // Three native 64-bit products instead of a generic u128 multiply.
            // The low and middle products fit because mantissa has 32 bits;
            // the final high word fits because 10^26 * u32::MAX < u128::MAX.
            let decimal = u128_to_u64_slice(DECIMAL_CONSTANTS[offset]);
            let mantissa = mantissa as u64;
            let low = (decimal[0] & u32::MAX as u64).wrapping_mul(mantissa);
            let middle = (decimal[0] >> 32)
                .wrapping_mul(mantissa)
                .wrapping_add(low >> 32);
            QuoteAtomsPerBaseAtom {
                inner: [
                    (middle << 32) | (low & u32::MAX as u64),
                    decimal[1].wrapping_mul(mantissa).wrapping_add(middle >> 32),
                ],
            }
        }
    }

    #[inline(always)]
    pub fn checked_multiply_rational(
        self,
        numerator: u32,
        denominator: u32,
        round_up: bool,
    ) -> Result<Self, PriceConversionError> {
        // Stored as u128 * 10^-26
        let inner: u128 = u64_slice_to_u128(self.inner);
        // multiply then divide
        let Some(product) = inner.checked_mul(numerator as u128) else {
            return Err(PriceConversionError(0x4));
        };
        let new_inner: u128 = if round_up {
            product.div_ceil(denominator as u128)
        } else {
            product.div(denominator as u128)
        };
        Ok(QuoteAtomsPerBaseAtom {
            inner: u128_to_u64_slice(new_inner),
        })
    }

    pub fn try_from_mantissa_and_exponent(
        mantissa: u32,
        exponent: i8,
    ) -> Result<Self, PriceConversionError> {
        if exponent > Self::MAX_EXP {
            trace!("invalid exponent {exponent} > 8 would truncate",);
            return Err(PriceConversionError(0x1));
        }
        if exponent < Self::MIN_EXP {
            trace!("invalid exponent {exponent} < -18 would truncate",);
            return Err(PriceConversionError(0x2));
        }
        Ok(Self::from_mantissa_and_exponent_(mantissa, exponent))
    }

    #[inline(always)]
    pub fn checked_base_for_quote(
        self,
        quote_atoms: QuoteAtoms,
        round_up: bool,
    ) -> Result<BaseAtoms, ProgramError> {
        // prevents division by zero further down the line. zero is not an
        // ideal answer, but this is only used in impact_base_atoms, which
        // is used to calculate error free order sizes and for that purpose
        // it works well.
        if self == Self::ZERO {
            return Ok(BaseAtoms::ZERO);
        }
        // this doesn't need a check, will never overflow: u64::MAX * D18 < u128::MAX
        let dividend: u128 = D18.wrapping_mul(quote_atoms.inner as u128);
        let inner: u128 = u64_slice_to_u128(self.inner);
        let base_atoms: u128 = if round_up {
            dividend.div_ceil(inner)
        } else {
            dividend.div(inner)
        };
        if base_atoms <= ATOM_LIMIT {
            Ok(BaseAtoms::new(base_atoms as u64))
        } else {
            Err(PriceConversionError(0x5).into())
        }
    }

    #[inline(always)]
    fn checked_quote_for_base_(
        self,
        base_atoms: BaseAtoms,
        round_up: bool,
    ) -> Result<u64, ProgramError> {
        #[cfg(not(feature = "certora"))]
        {
            // Prices on decimal grids often contain these exact factors.
            // Cancel one before multiplying so ordinary order sizes fit u64.
            // A successful reduced product also proves the original product
            // fits u128: u64::MAX * 10^12 < u128::MAX. Otherwise use the full
            // checked path, preserving precision, overflow and rounding.
            if self.inner[1] == 0 {
                let (reduced_price, divisor) = if self.inner[0] % 1_000_000_000_000 == 0 {
                    (self.inner[0] / 1_000_000_000_000, 1_000_000)
                } else if self.inner[0] % 1_000_000_000 == 0 {
                    (self.inner[0] / 1_000_000_000, 1_000_000_000)
                } else {
                    (0, 0)
                };
                if divisor != 0 {
                    if let Some(product) = reduced_price.checked_mul(base_atoms.inner) {
                        return Ok(
                            product / divisor + u64::from(round_up && product % divisor != 0)
                        );
                    }
                }
            }
        }
        #[cfg(feature = "certora")]
        let product: u128 = u64_slice_to_u128(self.inner)
            .checked_mul(base_atoms.inner as u128)
            .ok_or(PriceConversionError(0x8))?;
        #[cfg(not(feature = "certora"))]
        let product: u128 =
            checked_price_product(self.inner, base_atoms.inner).ok_or(PriceConversionError(0x8))?;
        #[cfg(not(feature = "certora"))]
        {
            // A high word at least 10^18 makes the floor quotient at least
            // 2^64. Reject it before division; every remaining quotient fits
            // u64 and only ceiling's final carry can overflow the atom limit.
            if product >> 64 >= D18 {
                return Err(PriceConversionError(0x9).into());
            }
            let (quotient, remainder) = div_rem_d18(product);
            return (quotient as u64)
                .checked_add(u64::from(round_up && remainder != 0))
                .ok_or_else(|| PriceConversionError(0x9).into());
        }
        #[cfg(feature = "certora")]
        {
            let quote_atoms: u128 = if round_up {
                div_ceil_d18(product)
            } else {
                div_d18(product)
            };
            if quote_atoms <= ATOM_LIMIT {
                Ok(quote_atoms as u64)
            } else {
                Err(PriceConversionError(0x9).into())
            }
        }
    }

    #[inline(always)]
    pub fn checked_quote_for_base(
        self,
        other: BaseAtoms,
        round_up: bool,
    ) -> Result<QuoteAtoms, ProgramError> {
        self.checked_quote_for_base_(other, round_up)
            .map(QuoteAtoms::new)
    }
}

impl Ord for QuoteAtomsPerBaseAtom {
    #[inline(always)]
    fn cmp(&self, other: &Self) -> Ordering {
        // `inner` is the little endian u64 split of the u128, so comparing the
        // high word and then the low word is the same ordering as comparing
        // the u128 itself, and it keeps both operands in registers.
        // The old conversion took the array by value and materialized the
        // u128 through an unaligned read, which cost a stack round trip.
        // This comparison runs at every level of every orderbook tree descent,
        // so that round trip is paid once per level per order.
        (self.inner[1], self.inner[0]).cmp(&(other.inner[1], other.inner[0]))
    }
}

impl PartialOrd for QuoteAtomsPerBaseAtom {
    #[inline(always)]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for QuoteAtomsPerBaseAtom {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        (self.inner) == (other.inner)
    }
}

impl Eq for QuoteAtomsPerBaseAtom {}

impl std::fmt::Display for QuoteAtomsPerBaseAtom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "{}",
            &(u64_slice_to_u128(self.inner) as f64 / D18F)
        ))
    }
}

impl std::fmt::Debug for QuoteAtomsPerBaseAtom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuoteAtomsPerBaseAtom")
            .field("value", &(u64_slice_to_u128(self.inner) as f64 / D18F))
            .finish()
    }
}

#[derive(Debug)]
pub struct PriceConversionError(pub u32);

const PRICE_CONVERSION_ERROR_BASE: u32 = 100;

impl From<PriceConversionError> for ProgramError {
    fn from(value: PriceConversionError) -> Self {
        ProgramError::Custom(value.0 + PRICE_CONVERSION_ERROR_BASE)
    }
}

#[inline(always)]
fn encode_mantissa_and_exponent(value: f64) -> (u32, i8) {
    let mut exponent: i8 = 0;
    // prevent overflow when casting to u32
    while exponent < QuoteAtomsPerBaseAtom::MAX_EXP
        && calculate_mantissa(value, exponent) > u32::MAX as f64
    {
        exponent += 1;
    }
    // prevent underflow and maximize precision available
    while exponent > QuoteAtomsPerBaseAtom::MIN_EXP
        && calculate_mantissa(value, exponent) < (u32::MAX / 10) as f64
    {
        exponent -= 1;
    }
    (calculate_mantissa(value, exponent) as u32, exponent)
}

#[inline(always)]
fn calculate_mantissa(value: f64, exp: i8) -> f64 {
    (value * 10f64.powi(-exp as i32)).round()
}

impl TryFrom<f64> for QuoteAtomsPerBaseAtom {
    type Error = PriceConversionError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        if value.is_infinite() {
            trace!("infinite can not be expressed as fixed point decimal");
            return Err(PriceConversionError(0xC));
        }
        if value.is_nan() {
            trace!("nan can not be expressed as fixed point decimal");
            return Err(PriceConversionError(0xD));
        }
        if value.is_sign_negative() {
            trace!("price {value} can not be negative");
            return Err(PriceConversionError(0xE));
        }
        if calculate_mantissa(value, Self::MAX_EXP) > u32::MAX as f64 {
            trace!("price {value} is too large");
            return Err(PriceConversionError(0xF));
        }

        let (mantissa, exponent) = encode_mantissa_and_exponent(value);

        Self::try_from_mantissa_and_exponent(mantissa, exponent)
    }
}

impl BaseAtoms {
    #[inline(always)]
    pub fn checked_mul(
        self,
        other: QuoteAtomsPerBaseAtom,
        round_up: bool,
    ) -> Result<QuoteAtoms, ProgramError> {
        other.checked_quote_for_base(self, round_up)
    }
}

#[cfg(feature = "certora")]
mod nondet {
    use super::*;

    impl ::nondet::Nondet for BaseAtoms {
        fn nondet() -> Self {
            Self::new(::nondet::nondet())
        }
    }

    impl ::nondet::Nondet for QuoteAtoms {
        fn nondet() -> Self {
            Self::new(::nondet::nondet())
        }
    }

    impl ::nondet::Nondet for QuoteAtomsPerBaseAtom {
        fn nondet() -> Self {
            Self {
                inner: [::nondet::nondet(), ::nondet::nondet()],
            }
        }
    }
}

#[test]
fn test_new_constructor_macro() {
    let base_atoms_1: BaseAtoms = BaseAtoms::new(5);
    let base_atoms_2: BaseAtoms = BaseAtoms::new(10);

    assert_eq!(base_atoms_1 + base_atoms_2, BaseAtoms::new(15));
    assert!((base_atoms_1 + base_atoms_2).eq(&BaseAtoms::new(15)));
    assert!((base_atoms_1 + base_atoms_2).eq(&15_u64));
    assert!(15u64.eq(&(base_atoms_1 + base_atoms_2)));
}

#[test]
fn test_checked_add() {
    let base_atoms_1: BaseAtoms = BaseAtoms::new(1);
    let base_atoms_2: BaseAtoms = BaseAtoms::new(2);
    assert_eq!(
        base_atoms_1.checked_add(base_atoms_2).unwrap(),
        BaseAtoms::new(3)
    );

    let base_atoms_1: BaseAtoms = BaseAtoms::new(u64::MAX - 1);
    let base_atoms_2: BaseAtoms = BaseAtoms::new(2);
    assert!(base_atoms_1.checked_add(base_atoms_2).is_err());
}

#[test]
fn test_checked_sub() {
    let base_atoms_1: BaseAtoms = BaseAtoms::new(1);
    let base_atoms_2: BaseAtoms = BaseAtoms::new(2);
    assert_eq!(
        base_atoms_2.checked_sub(base_atoms_1).unwrap(),
        BaseAtoms::new(1)
    );

    assert!(base_atoms_1.checked_sub(base_atoms_2).is_err());
}

#[test]
fn test_overflowing_add() {
    let base_atoms: BaseAtoms = BaseAtoms::new(u64::MAX);
    let (sum, overflow_detected) = base_atoms.overflowing_add(base_atoms);
    assert!(overflow_detected);

    let expected = base_atoms - BaseAtoms::ONE;
    assert_eq!(sum, expected);
}

#[test]
fn test_wrapping_add() {
    let base_atoms: BaseAtoms = BaseAtoms::new(u64::MAX);
    let sum = base_atoms.wrapping_add(base_atoms);
    let expected = base_atoms - BaseAtoms::ONE;
    assert_eq!(sum, expected);
}

#[test]
fn test_checked_base_for_quote_edge_cases() {
    let quote_atoms_per_base_atom: QuoteAtomsPerBaseAtom =
        QuoteAtomsPerBaseAtom::from_mantissa_and_exponent_(0, 0);
    assert_eq!(
        quote_atoms_per_base_atom
            .checked_base_for_quote(QuoteAtoms::new(1), false)
            .unwrap(),
        BaseAtoms::new(0)
    );

    let quote_atoms_per_base_atom: QuoteAtomsPerBaseAtom =
        QuoteAtomsPerBaseAtom::from_mantissa_and_exponent_(1, -18);
    assert!(quote_atoms_per_base_atom
        .checked_base_for_quote(QuoteAtoms::new(u64::MAX), false)
        .is_err(),);
}

#[test]
fn test_checked_quote_for_base_edge_cases() {
    // edge case is where u64MAX * 10**18  < product < u128MAX
    let quote_atoms_per_base_atom: QuoteAtomsPerBaseAtom = QuoteAtomsPerBaseAtom::MAX;
    assert!(quote_atoms_per_base_atom
        .checked_quote_for_base(BaseAtoms::new(u64::MAX - 1), false)
        .is_err(),);
}

#[test]
fn test_quote_atoms_per_base_atom_edge_case() {
    assert!(QuoteAtomsPerBaseAtom::try_from(f64::NAN).is_err());
}

#[test]
fn test_multiply_macro() {
    let base_atoms: BaseAtoms = BaseAtoms::new(5);
    let quote_atoms_per_base_atom: QuoteAtomsPerBaseAtom = QuoteAtomsPerBaseAtom {
        inner: u128_to_u64_slice(100 * D18 - 1),
    };
    assert_eq!(
        base_atoms
            .checked_mul(quote_atoms_per_base_atom, true)
            .unwrap(),
        QuoteAtoms::new(500)
    );
}

#[test]
fn test_price_limits() {
    assert!(QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(
        1,
        QuoteAtomsPerBaseAtom::MAX_EXP
    )
    .is_ok());
    assert!(QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(
        u32::MAX,
        QuoteAtomsPerBaseAtom::MAX_EXP
    )
    .is_ok());
    assert!(QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(
        1,
        QuoteAtomsPerBaseAtom::MIN_EXP
    )
    .is_ok());
    assert!(QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(
        u32::MAX,
        QuoteAtomsPerBaseAtom::MIN_EXP
    )
    .is_ok());
    assert!(QuoteAtomsPerBaseAtom::try_from(0f64).is_ok());
    assert!(QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(0, 0).is_ok());
    assert!(QuoteAtomsPerBaseAtom::try_from(
        u32::MAX as f64 * 10f64.powi(QuoteAtomsPerBaseAtom::MAX_EXP as i32)
    )
    .is_ok());

    // failures
    assert!(QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(
        1,
        QuoteAtomsPerBaseAtom::MAX_EXP + 1
    )
    .is_err());
    assert!(QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(
        1,
        QuoteAtomsPerBaseAtom::MIN_EXP - 1
    )
    .is_err());
    assert!(QuoteAtomsPerBaseAtom::try_from(-1f64).is_err());
    assert!(QuoteAtomsPerBaseAtom::try_from(u128::MAX as f64).is_err());
    assert!(QuoteAtomsPerBaseAtom::try_from(1f64 / 0f64).is_err());
}

#[allow(dead_code)]
#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
struct AlignmentTest {
    _alignment_fix: u128,
    _pad: u64,
    price: QuoteAtomsPerBaseAtom,
}

#[test]
fn test_alignment() {
    let mut t = AlignmentTest::default();
    t.price = QuoteAtomsPerBaseAtom::from_mantissa_and_exponent_(u32::MAX, 0);
    let mut s = t.clone();
    t.price = s.price.clone();
    let q = t
        .price
        .checked_base_for_quote(QuoteAtoms::new(u32::MAX as u64), true)
        .unwrap();
    t._pad = q.as_u64();
    s._pad = s.price.checked_quote_for_base(q, true).unwrap().as_u64();

    println!("s:{s:?} t:{t:?}");
}

#[test]
fn test_print() {
    println!("{}", BaseAtoms::new(1));
    println!("{}", QuoteAtoms::new(2));
    println!(
        "{}",
        QuoteAtomsPerBaseAtom {
            inner: u128_to_u64_slice(123 * D18 / 100),
        }
    );
}

#[test]
fn test_debug() {
    println!("{:?}", BaseAtoms::new(1));
    println!("{:?}", QuoteAtoms::new(2));
    println!(
        "{:?}",
        QuoteAtomsPerBaseAtom {
            inner: u128_to_u64_slice(123 * D18 / 100),
        }
    );
}

#[cfg(test)]
mod price_ordering_tests {
    use super::*;

    /// The word wise comparison in `Ord for QuoteAtomsPerBaseAtom` has to
    /// agree with comparing the u128 it stands for, including across the
    /// 64 bit boundary where only the high word separates two prices.
    #[test]
    fn word_wise_price_order_matches_u128() {
        let interesting: [u128; 14] = [
            0,
            1,
            2,
            u64::MAX as u128 - 1,
            u64::MAX as u128,
            u64::MAX as u128 + 1,
            u64::MAX as u128 + 2,
            1u128 << 64,
            (1u128 << 64) | 1,
            (1u128 << 64) | (u64::MAX as u128),
            2u128 << 64,
            u128::MAX - 1,
            u128::MAX,
            D18,
        ];
        for left in interesting.iter() {
            for right in interesting.iter() {
                let a: QuoteAtomsPerBaseAtom = QuoteAtomsPerBaseAtom {
                    inner: u128_to_u64_slice(*left),
                };
                let b: QuoteAtomsPerBaseAtom = QuoteAtomsPerBaseAtom {
                    inner: u128_to_u64_slice(*right),
                };
                assert_eq!(
                    a.cmp(&b),
                    left.cmp(right),
                    "comparing {left} against {right}"
                );
                assert_eq!(a == b, left == right, "equality of {left} and {right}");
            }
        }
    }
}

#[cfg(test)]
mod word_split_tests {
    use super::*;

    /// The shift based split has to be the same little endian split the byte
    /// reinterpretation produced, in both directions, or every stored price
    /// changes meaning.
    #[test]
    fn split_and_join_round_trip() {
        let interesting: [u128; 10] = [
            0,
            1,
            u64::MAX as u128,
            u64::MAX as u128 + 1,
            1u128 << 64,
            (1u128 << 64) | 0xdead_beef,
            u128::MAX,
            D18,
            D18 * 1_000,
            0x0123_4567_89ab_cdef_fedc_ba98_7654_3210,
        ];
        for value in interesting.iter() {
            let words: [u64; 2] = u128_to_u64_slice(*value);
            assert_eq!(words[0], *value as u64, "low word of {value}");
            assert_eq!(words[1], (*value >> 64) as u64, "high word of {value}");
            assert_eq!(u64_slice_to_u128(words), *value, "round trip of {value}");
            // What the pointer reinterpretation did, on a little endian target.
            let reinterpreted: [u64; 2] = unsafe {
                let ptr: *const u128 = value;
                *ptr.cast::<[u64; 2]>()
            };
            assert_eq!(words, reinterpreted, "byte order for {value}");
        }
    }
}
