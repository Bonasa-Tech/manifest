use crate::program::ManifestError;
use borsh::{BorshDeserialize as Deserialize, BorshSerialize as Serialize};
use bytemuck::{Pod, Zeroable};
use hypertree::trace;
use shank::ShankAccount;
use solana_program::program_error::ProgramError;
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
const fn u128_to_u64_slice(a: u128) -> [u64; 2] {
    unsafe {
        let ptr: *const u128 = &a;
        *ptr.cast::<[u64; 2]>()
    }
}
pub(crate) fn u64_slice_to_u128(a: [u64; 2]) -> u128 {
    // `[u64; 2]` guarantees only 8-byte alignment, while Rust may require
    // 16-byte alignment for `u128` even on SBF. The runtime supports the
    // underlying 8-byte loads, but the Rust pointer dereference must still be
    // explicitly unaligned to avoid undefined behavior.
    unsafe {
        let ptr: *const [u64; 2] = &a;
        ptr.cast::<u128>().read_unaligned()
    }
}

#[cfg(not(feature = "certora"))]
const ATOM_LIMIT: u128 = u64::MAX as u128;
const D18: u128 = 10u128.pow(18);
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
        // can not overflow 10^26 * u32::MAX < u128::MAX
        let inner: u128 = DECIMAL_CONSTANTS[offset].wrapping_mul(mantissa as u128);
        QuoteAtomsPerBaseAtom {
            inner: u128_to_u64_slice(inner),
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
    ) -> Result<u128, ProgramError> {
        let inner: u128 = u64_slice_to_u128(self.inner);
        let product: u128 = inner
            .checked_mul(base_atoms.inner as u128)
            .ok_or(PriceConversionError(0x8))?;
        let quote_atoms: u128 = if round_up {
            product.div_ceil(D18)
        } else {
            product.div(D18)
        };
        if quote_atoms <= ATOM_LIMIT {
            Ok(quote_atoms)
        } else {
            Err(PriceConversionError(0x9).into())
        }
    }

    #[inline(always)]
    pub fn checked_quote_for_base(
        self,
        other: BaseAtoms,
        round_up: bool,
    ) -> Result<QuoteAtoms, ProgramError> {
        self.checked_quote_for_base_(other, round_up)
            .map(|r| QuoteAtoms::new(r as u64))
    }
}

impl Ord for QuoteAtomsPerBaseAtom {
    #[inline(always)]
    fn cmp(&self, other: &Self) -> Ordering {
        (u64_slice_to_u128(self.inner)).cmp(&u64_slice_to_u128(other.inner))
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
