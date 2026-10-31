use bytemuck::{Pod, Zeroable};
use pinocchio::error::ProgramError;
use solana_program::pubkey::Pubkey;
use static_assertions::const_assert_eq;

/// First 8 bytes of every position account.
pub const POSITION_DISCRIMINANT: u64 = u64::from_le_bytes(*b"mfxposn2");

/// How far the creation time named in the PDA seeds may be from the cluster
/// clock. The client picks the time before signing, so it has to allow for the
/// transaction's lifetime and for clock skew either way.
pub const MAX_CREATED_AT_DRIFT_SECONDS: i64 = 300;

/// Upper bound on the ladder stored after the header.
pub const MAX_LADDER_BYTES: usize = 1024;
/// Upper bound on the orders one ladder describes.
pub const MAX_TOTAL_ORDERS: usize = 1024;

pub const POSITION_SEED: &[u8] = b"position";
/// Seeds of the position's trader: [TRADER_SEED, position]. A system account
/// without data that the program signs for; it holds the position's seat on
/// the market and its two token accounts.
pub const TRADER_SEED: &[u8] = b"trader";

/// One balance snapshot. Prices are quote tokens per base token scaled by
/// 1e9, so they do not depend on either mint's decimals.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct Snapshot {
    pub base_atoms: u64,
    pub quote_atoms: u64,
    pub price_e9: u64,
}

/// Fixed header of a position account. `ladder_len` bytes of the ladder
/// follow it, then one bit per order, set once that order went out.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
pub struct Position {
    pub discriminant: u64,
    pub owner: Pubkey,
    pub market: Pubkey,
    pub created_at: i64,
    /// 0 while the position is open.
    pub closed_at: i64,
    /// What placing the ladder took from the owner so far, at the opening
    /// price. Grows with every batch.
    pub created_with: Snapshot,
    /// What closing handed back to the owner, at the closing price, which
    /// the first CancelOrders or ClosePosition records (nonzero once
    /// closing began).
    pub closed_with: Snapshot,
    pub total_orders: u16,
    pub placed_orders: u16,
    pub ladder_len: u16,
    pub bump: u8,
    pub trader_bump: u8,
}
pub const POSITION_HEADER_SIZE: usize = 144;
const_assert_eq!(core::mem::size_of::<Position>(), POSITION_HEADER_SIZE);

impl Position {
    pub fn is_closed(&self) -> bool {
        self.closed_at != 0
    }

    /// A CancelOrders or ClosePosition recorded the closing price.
    pub fn is_closing(&self) -> bool {
        self.closed_with.price_e9 != 0
    }
}

pub const fn placed_bitmap_len(total_orders: usize) -> usize {
    total_orders.div_ceil(8)
}

pub fn get_position_address(owner: &Pubkey, market: &Pubkey, created_at: i64) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            POSITION_SEED,
            owner.as_ref(),
            market.as_ref(),
            &created_at.to_le_bytes(),
        ],
        &crate::ID,
    )
}

pub fn get_trader_address(position: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[TRADER_SEED, position.as_ref()], &crate::ID)
}

// ───────────────────────────── ladder ─────────────────────────────
//
// The ladder is a list of bands, each spreading `levels` orders evenly over
// [lo_bp, hi_bp] around the opening price, every order the same size in quote
// atoms. Order k is level k counted through the bands in order. The client
// (manifest-app lib/stonks/ladder.ts) computes the same orders with the same
// integer arithmetic to preview and to pick which ones to place.
//
//   header: version u8 (= 2), band count u8, 2 reserved bytes
//   band:   lo_bp i16, hi_bp i16, levels u16, spread_e1 u16, size_quote_atoms u64

pub const LADDER_VERSION: u8 = 2;
const LADDER_HEADER_BYTES: usize = 4;
const BAND_BYTES: usize = 16;

/// Scale of `rel_e8`, an order's distance from the opening price.
const REL_ONE: i128 = 100_000_000;
/// Scale of `price_e9 * (REL_ONE + rel_e8)`, the price in quote tokens per
/// base token.
const PRICE_SCALE_EXP: i32 = 17;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Band {
    pub lo_bp: i16,
    pub hi_bp: i16,
    pub levels: u16,
    /// Reverse spread in tenths of a basis point.
    pub spread_e1: u16,
    pub size_quote_atoms: u64,
}

pub struct Ladder<'a> {
    bytes: &'a [u8],
}

impl<'a> Ladder<'a> {
    /// Checks the encoding and every band, so later reads cannot fail.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ProgramError> {
        if bytes.len() < LADDER_HEADER_BYTES || bytes[0] != LADDER_VERSION {
            return Err(ProgramError::InvalidInstructionData);
        }
        let n = bytes[1] as usize;
        if bytes.len() != LADDER_HEADER_BYTES + n * BAND_BYTES || n == 0 {
            return Err(ProgramError::InvalidInstructionData);
        }
        let ladder = Ladder { bytes };
        let mut total: usize = 0;
        for i in 0..n {
            let b = ladder.band(i);
            // Every price stays above zero: lo > -100%.
            if b.levels == 0 || b.hi_bp <= b.lo_bp || b.lo_bp <= -10_000 || b.spread_e1 == 0 {
                return Err(ProgramError::InvalidInstructionData);
            }
            total += b.levels as usize;
        }
        if total > MAX_TOTAL_ORDERS {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(ladder)
    }

    /// A ladder that already passed `parse`, as a position stores it.
    pub fn checked(bytes: &'a [u8]) -> Self {
        Ladder { bytes }
    }

    pub fn band_count(&self) -> usize {
        self.bytes[1] as usize
    }

    pub fn band(&self, i: usize) -> Band {
        let b = &self.bytes[LADDER_HEADER_BYTES + i * BAND_BYTES..][..BAND_BYTES];
        Band {
            lo_bp: i16::from_le_bytes([b[0], b[1]]),
            hi_bp: i16::from_le_bytes([b[2], b[3]]),
            levels: u16::from_le_bytes([b[4], b[5]]),
            spread_e1: u16::from_le_bytes([b[6], b[7]]),
            size_quote_atoms: u64::from_le_bytes(b[8..16].try_into().unwrap()),
        }
    }

    pub fn total_orders(&self) -> usize {
        (0..self.band_count())
            .map(|i| self.band(i).levels as usize)
            .sum()
    }

    /// Fails when any order around `price_e9` is one Manifest cannot take.
    /// Within a band prices rise with the level, and the price exponent with
    /// them, while the base amount falls; so each band's lowest and highest
    /// level are its extremes.
    pub fn check_orders(
        &self,
        price_e9: u64,
        base_decimals: u8,
        quote_decimals: u8,
    ) -> Result<(), ProgramError> {
        for i in 0..self.band_count() {
            let b = self.band(i);
            level_order(&b, 0, price_e9, base_decimals, quote_decimals)?;
            level_order(
                &b,
                b.levels as i128 - 1,
                price_e9,
                base_decimals,
                quote_decimals,
            )?;
        }
        Ok(())
    }

    /// Order `k` around `price_e9`, None when it rounds to nothing (no
    /// distance from the price, or too small for one base atom).
    pub fn order(
        &self,
        k: usize,
        price_e9: u64,
        base_decimals: u8,
        quote_decimals: u8,
    ) -> Result<Option<LadderOrder>, ProgramError> {
        let mut k = k;
        for i in 0..self.band_count() {
            let b = self.band(i);
            if k >= b.levels as usize {
                k -= b.levels as usize;
                continue;
            }
            return level_order(&b, k as i128, price_e9, base_decimals, quote_decimals);
        }
        Err(ProgramError::InvalidArgument)
    }
}

/// One order as Manifest takes it: quote atoms per base atom is
/// `price_mantissa * 10^price_exponent`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LadderOrder {
    pub is_bid: bool,
    pub base_atoms: u64,
    pub price_mantissa: u32,
    pub price_exponent: i8,
    /// Quote atoms a bid locks: base × price, rounded up as Manifest does.
    pub quote_atoms: u64,
    pub spread_e1: u16,
}

fn pow10(e: u32) -> Result<i128, ProgramError> {
    10i128
        .checked_pow(e)
        .ok_or(ProgramError::ArithmeticOverflow)
}

fn level_order(
    b: &Band,
    i: i128,
    price_e9: u64,
    base_decimals: u8,
    quote_decimals: u8,
) -> Result<Option<LadderOrder>, ProgramError> {
    let overflow = || ProgramError::ArithmeticOverflow;
    let lo = b.lo_bp as i128 * 10_000;
    let width = (b.hi_bp as i128 - b.lo_bp as i128) * 10_000;
    // Level centres: lo + (i + ½) · width / levels, in 1e-8.
    let rel_e8 = lo + (2 * i + 1) * width / (2 * b.levels as i128);
    // Quote tokens per base token, scaled by 1e17; above zero as lo > -100%.
    let price = price_e9 as i128 * (REL_ONE + rel_e8);
    if price <= 0 {
        return Err(ProgramError::InvalidArgument);
    }
    // Quote atoms per base atom = price · 10^(qd − bd − 17). Keep the ten
    // leading digits that fit a u32 mantissa; the rest round down.
    let mut mantissa = price;
    let mut exponent = quote_decimals as i32 - base_decimals as i32 - PRICE_SCALE_EXP;
    while mantissa > u32::MAX as i128 {
        mantissa /= 10;
        exponent += 1;
    }
    // Reverse orders take exponents up to 0 (ReverseTight) and Manifest's
    // prices go down to 10^-18.
    if !(-18..=0).contains(&exponent) {
        return Err(ProgramError::InvalidArgument);
    }
    // Base atoms worth the band's size at the level's price.
    let base_atoms = (b.size_quote_atoms as i128)
        .checked_mul(pow10(base_decimals as u32 + PRICE_SCALE_EXP as u32)?)
        .ok_or_else(overflow)?
        / price
            .checked_mul(pow10(quote_decimals as u32)?)
            .ok_or_else(overflow)?;
    let base_atoms: u64 = base_atoms.try_into().map_err(|_| overflow())?;
    let scale = pow10((-exponent) as u32)?;
    let quote_atoms: u64 = ((base_atoms as i128 * mantissa + scale - 1) / scale)
        .try_into()
        .map_err(|_| overflow())?;
    // Checked as if it were an order even when it rounds to nothing, so that
    // a band's extreme levels still bound the ones next to them.
    if rel_e8 == 0 || base_atoms == 0 {
        return Ok(None);
    }
    Ok(Some(LadderOrder {
        is_bid: rel_e8 < 0,
        base_atoms: base_atoms as u64,
        price_mantissa: mantissa as u32,
        price_exponent: exponent as i8,
        quote_atoms: quote_atoms as u64,
        spread_e1: b.spread_e1,
    }))
}
