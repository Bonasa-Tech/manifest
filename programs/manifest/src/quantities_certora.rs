#![cfg(feature = "certora")]
use super::*;

impl QuoteAtomsPerBaseAtom {
    pub const ZERO: Self = QuoteAtomsPerBaseAtom { inner: [0; 2] };
    pub const MIN: Self = QuoteAtomsPerBaseAtom { inner: [1, 0] };
    pub const MAX: Self = QuoteAtomsPerBaseAtom {
        inner: [u32::MAX as u64, 0],
    };

    pub const MIN_EXP: i8 = -18;
    pub const MAX_EXP: i8 = 8;

    // 4 decimal points to fit price into 32 bit
    const DECIMALS: u64 = 10u64.pow(4);

    #[inline(always)]
    pub fn from_mantissa_and_exponent_(_mantissa: u32, _exponent: i8) -> Self {
        return Self {
            inner: [::nondet::nondet::<u64>(), ::nondet::nondet::<u64>()],
        };
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

    #[inline(never)]
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
        let dividend = Self::DECIMALS.wrapping_mul(quote_atoms.inner);
        let inner = self.inner[0];

        let base_atoms = if round_up {
            dividend.div_ceil(inner)
        } else {
            dividend.div(inner)
        };

        Ok(BaseAtoms::new(base_atoms))
    }

    /// Not inlined: with `#[inline(always)]` the compiler merges the writes
    /// that build the Result, and the prover then cannot read the payload back
    /// off the stack ("[3002] stack location is not accessible"). That only
    /// bites rules which keep the error path alive rather than unwrapping it,
    /// i.e. the no-revert rules.
    #[inline(never)]
    fn checked_quote_for_base_(
        self,
        base_atoms: BaseAtoms,
        round_up: bool,
    ) -> Result<u64, ProgramError> {
        let inner = self.inner[0];
        let product = inner
            .checked_mul(base_atoms.inner)
            .ok_or(PriceConversionError(0x8))?;
        let quote_atoms = if round_up {
            product.div_ceil(Self::DECIMALS)
        } else {
            product.div(Self::DECIMALS)
        };
        Ok(quote_atoms)
    }

    /// Written as a match rather than `.map(..)`. The prover's static analysis
    /// cannot follow the Result payload back off the stack through the closure,
    /// which shows up as "[3002] stack location is not accessible" in any rule
    /// that keeps the error path alive instead of unwrapping it -- that is, in
    /// the no-revert rules.
    #[inline(never)]
    pub fn checked_quote_for_base(
        self,
        other: BaseAtoms,
        round_up: bool,
    ) -> Result<QuoteAtoms, ProgramError> {
        match self.checked_quote_for_base_(other, round_up) {
            Ok(r) => Ok(QuoteAtoms::new(r)),
            Err(e) => Err(e),
        }
    }

    #[inline(always)]
    pub fn checked_effective_price(
        self,
        _num_base_atoms: BaseAtoms,
        _is_bid: bool,
    ) -> Result<QuoteAtomsPerBaseAtom, ProgramError> {
        cvt::cvt_assert!(false);
        unreachable!();
    }

    pub fn nondet_price_u32() -> Self {
        let x = ::nondet::nondet();
        cvt::cvt_assume!(x <= u32::MAX as u64);
        Self { inner: [x, 0] }
    }

    /// `self * numerator / denominator` on the single-limb certora price.
    ///
    /// This derives the price a reverse order comes back at. It has to be a
    /// real function of its inputs, not a nondet summary, because the reverse
    /// coalesce rule computes the reverse price twice (once in the rule and
    /// once inside the matching code) and the two have to agree. The product
    /// cannot overflow: the price fits u32 and the numerator is at most 10^8,
    /// so the product is at most ~4*10^17 < u64::MAX.
    pub fn checked_multiply_rational(
        self,
        numerator: u32,
        denominator: u32,
        round_up: bool,
    ) -> Result<Self, PriceConversionError> {
        if denominator == 0 {
            return Err(PriceConversionError(0x9));
        }
        let product: u64 = self.inner[0]
            .checked_mul(numerator as u64)
            .ok_or(PriceConversionError(0xa))?;
        let result: u64 = if round_up {
            product.div_ceil(denominator as u64)
        } else {
            product.div(denominator as u64)
        };
        Ok(Self { inner: [result, 0] })
    }
}
