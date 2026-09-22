//! Bit-vector equivalence checks for the deployed arithmetic, independently of
//! Certora's reduced price model. No restricted price/size sampling is used.
//! Run the entire module: caller proofs use exact specifications established
//! by the helper proofs, including the helper preconditions at every call.
//!
//! Multiplication uses scripts/kani/z3: it proves against the compiled MIR
//! using certified word decomposition and conservative product abstraction.
//! Division uses stock cvc5's width-preserving bit-vector-to-integer solver.
//! Neither solver is allowed to count unknown or timeouts as successful.
use super::*;

const LIMB_PRODUCT_LIMIT: u64 = (u32::MAX as u64) * (u32::MAX as u64);

fn reference_assembly(p00: u64, p01: u64, p10: u64, p11: u64) -> u64 {
    let bounds = p00 <= LIMB_PRODUCT_LIMIT
        && p01 <= LIMB_PRODUCT_LIMIT
        && p10 <= LIMB_PRODUCT_LIMIT
        && p11 <= LIMB_PRODUCT_LIMIT;
    assert!(bounds);
    kani::assume(bounds);
    let expanded = p00 as u128 + (((p01 as u128) + (p10 as u128)) << 32) + ((p11 as u128) << 64);
    (expanded >> 64) as u64
}

#[kani::proof]
#[kani::solver(cadical)]
fn product_carry_assembly_matches_u128() {
    let p: [u64; 4] = kani::any();
    kani::assume(p[0] <= LIMB_PRODUCT_LIMIT);
    kani::assume(p[1] <= LIMB_PRODUCT_LIMIT);
    kani::assume(p[2] <= LIMB_PRODUCT_LIMIT);
    kani::assume(p[3] <= LIMB_PRODUCT_LIMIT);
    assert!(
        assemble_product_high(p[0], p[1], p[2], p[3]) == reference_assembly(p[0], p[1], p[2], p[3])
    );
}

#[kani::proof]
#[kani::solver(z3)]
#[kani::stub(assemble_product_high, reference_assembly)]
fn high_product_carries_match_u128() {
    let a: u64 = kani::any();
    let b: u64 = kani::any();
    assert!(mul_high_u64(a, b) == ((a as u128 * b as u128) >> 64) as u64);
}

fn reference_high_product(a: u64, b: u64) -> u64 {
    ((a as u128 * b as u128) >> 64) as u64
}

fn check_price_product<const CASE: u8>() {
    let price: [u64; 2] = kani::any();
    let amount: u64 = kani::any();
    // These four disjoint cases exhaust every price/amount pair. All native
    // overflow checks remain enabled, including the partition calculations.
    if CASE == 0 {
        kani::assume(price[1] == 0);
    } else {
        kani::assume(price[1] != 0);
        let high = price[1] as u128 * amount as u128;
        if CASE == 1 {
            kani::assume(high > u64::MAX as u128);
        } else {
            kani::assume(high <= u64::MAX as u128);
            let low = price[0] as u128 * amount as u128;
            let combined_high = high + (low >> 64);
            kani::assume(if CASE == 2 {
                combined_high > u64::MAX as u128
            } else {
                combined_high <= u64::MAX as u128
            });
        }
    }
    match (
        checked_price_product(price, amount),
        u64_slice_to_u128(price).checked_mul(amount as u128),
    ) {
        (Some(actual), Some(expected)) => assert!(actual == expected),
        (None, None) => (),
        _ => panic!("product value or overflow rejection differs"),
    }
}

macro_rules! product_proofs {
    ($($name:ident: $case:expr),* $(,)?) => {$(
        #[kani::proof]
        #[kani::solver(z3)]
        #[kani::stub(mul_high_u64, reference_high_product)]
        fn $name() { check_price_product::<$case>(); }
    )*};
}
product_proofs! {
    price_product_low_word: 0,
    price_product_high_overflow: 1,
    price_product_carry_overflow: 2,
    price_product_fits: 3,
}

#[kani::proof]
#[kani::solver(cvc5)]
#[kani::unwind(4)]
fn division_digit_matches_u128() {
    let top: u64 = kani::any();
    let next: u32 = kani::any();
    kani::assume(top < (D18 as u64) << 4);
    let quotient = div_d18_digit(top, next as u64);
    assert!(quotient as u128 == (((top as u128) << 32) | next as u128) / (D18 << 4));
}

fn reference_digit(top: u64, next: u64) -> u64 {
    // Prove the callers meet the independently verified helper's preconditions.
    assert!(top < (D18 as u64) << 4);
    assert!(next <= u32::MAX as u64);
    ((((top as u128) << 32) | next as u128) / (D18 << 4)) as u64
}

#[kani::proof]
#[kani::solver(cvc5)]
#[kani::stub(div_d18_digit, reference_digit)]
fn division_matches_u128_quotient_and_remainder() {
    let value: u128 = kani::any();
    let (quotient, remainder) = div_rem_d18(value);
    assert!(quotient == value / D18);
    assert!(remainder as u128 == value % D18);
}

fn check_mantissa<const EXPONENT: i8>() {
    let mantissa: u32 = kani::any();
    let decimal: u128 = DECIMAL_CONSTANTS[(8 - EXPONENT) as usize];
    let price = QuoteAtomsPerBaseAtom::from_mantissa_and_exponent_(mantissa, EXPONENT);
    assert!(u64_slice_to_u128(price.inner) == decimal * mantissa as u128);
}

// Partition only the 27 valid exponent values; each check retains every u32
// mantissa. This avoids one large symbolic lookup without sampling any inputs.
macro_rules! mantissa_proofs {
    ($($name:ident: $exponent:expr),* $(,)?) => {$(
        #[kani::proof]
        #[kani::solver(cvc5)]
        fn $name() { check_mantissa::<$exponent>(); }
    )*};
}
mantissa_proofs! {
    mantissa_p8: 8, mantissa_p7: 7, mantissa_p6: 6, mantissa_p5: 5,
    mantissa_p4: 4, mantissa_p3: 3, mantissa_p2: 2, mantissa_p1: 1,
    mantissa_zero: 0,
    mantissa_m1: -1, mantissa_m2: -2, mantissa_m3: -3, mantissa_m4: -4,
    mantissa_m5: -5, mantissa_m6: -6, mantissa_m7: -7, mantissa_m8: -8,
    mantissa_m9: -9, mantissa_m10: -10, mantissa_m11: -11, mantissa_m12: -12,
    mantissa_m13: -13, mantissa_m14: -14, mantissa_m15: -15, mantissa_m16: -16,
    mantissa_m17: -17, mantissa_m18: -18,
}

fn reference_product(price: [u64; 2], amount: u64) -> Option<u128> {
    u64_slice_to_u128(price).checked_mul(amount as u128)
}

fn reference_division(value: u128) -> (u128, u64) {
    (value / D18, (value % D18) as u64)
}

// Compositional proof: the multiplication/division harnesses above establish
// these exact replacements for every input. All must pass with this caller.
#[kani::proof]
#[kani::solver(cvc5)]
#[kani::stub(checked_price_product, reference_product)]
#[kani::stub(div_rem_d18, reference_division)]
fn quote_conversion_matches_u128_including_rounding_and_errors() {
    let price = QuoteAtomsPerBaseAtom { inner: kani::any() };
    let amount: u64 = kani::any();
    let round_up: bool = kani::any();
    let expected: Result<u64, ProgramError> =
        match u64_slice_to_u128(price.inner).checked_mul(amount as u128) {
            None => Err(PriceConversionError(0x8).into()),
            Some(product) => {
                let quote = product / D18 + u128::from(round_up && product % D18 != 0);
                if quote <= u64::MAX as u128 {
                    Ok(quote as u64)
                } else {
                    Err(PriceConversionError(0x9).into())
                }
            }
        };
    assert_eq!(
        price.checked_quote_for_base_(BaseAtoms::new(amount), round_up),
        expected
    );
}
