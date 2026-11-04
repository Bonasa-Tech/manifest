use bytemuck::{Pod, Zeroable};
use pinocchio::error::ProgramError;
use solana_program::pubkey::Pubkey;
use static_assertions::const_assert_eq;

/// First 8 bytes of every position account.
pub const POSITION_DISCRIMINANT: u64 = u64::from_le_bytes(*b"mfxposn3");

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

/// A price in quote tokens per base token, `mantissa · 10^exponent`, in
/// Manifest's own format: it keeps its digits however cheap or dear the
/// token, and does not depend on either mint's decimals. Zero (a zero
/// mantissa) only where none is set.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct Price {
    pub mantissa: u32,
    pub exponent: i8,
    pub _padding: [u8; 3],
}

/// Exponents a price may carry: far beyond any token's, and small enough
/// that the ladder arithmetic cannot overflow on them.
pub const PRICE_EXPONENTS: core::ops::RangeInclusive<i8> = -40..=20;
/// A price in instruction data: mantissa u32, exponent i8, little-endian.
pub const PRICE_BYTES: usize = 5;

impl Price {
    pub const fn new(mantissa: u32, exponent: i8) -> Self {
        Price {
            mantissa,
            exponent,
            _padding: [0; 3],
        }
    }

    pub fn is_set(&self) -> bool {
        self.mantissa != 0
    }

    /// A price from instruction data; it must be set and its exponent in
    /// range.
    pub fn read(bytes: &[u8]) -> Result<Self, ProgramError> {
        let price = Price::new(
            u32::from_le_bytes(bytes[..4].try_into().unwrap()),
            bytes[4] as i8,
        );
        if !price.is_set() || !PRICE_EXPONENTS.contains(&price.exponent) {
            return Err(ProgramError::InvalidArgument);
        }
        Ok(price)
    }

    pub fn to_bytes(&self) -> [u8; PRICE_BYTES] {
        let mut out = [0; PRICE_BYTES];
        out[..4].copy_from_slice(&self.mantissa.to_le_bytes());
        out[4] = self.exponent as u8;
        out
    }
}

/// One balance snapshot.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct Snapshot {
    pub base_atoms: u64,
    pub quote_atoms: u64,
    pub price: Price,
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
    /// closing began). Grows with every ClosePosition after the first.
    pub closed_with: Snapshot,
    pub total_orders: u16,
    /// Was the placed count; the bitmap after the ladder holds it.
    pub _reserved: u16,
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
        self.closed_with.price.is_set()
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
// (manifest-app lib/spreads/ladder.ts) computes the same orders with the same
// integer arithmetic to preview and to pick which ones to place.
//
//   header: version u8 (= 2), band count u8, 2 reserved bytes
//   band:   lo_bp i16, hi_bp i16, levels u16, spread_e1 u16, size_quote_atoms u64

pub const LADDER_VERSION: u8 = 2;
const LADDER_HEADER_BYTES: usize = 4;
const BAND_BYTES: usize = 16;

/// Scale of `rel_e8`, an order's distance from the opening price.
const REL_ONE: i128 = 100_000_000;
const REL_ONE_EXP: i32 = 8;
/// Largest power of ten a band's size (a u64) is scaled up by on its way to
/// base atoms, so that the product stays inside an i128.
const MAX_SIZE_SCALE_EXP: i32 = 18;

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
    /// Checks the encoding and every band, so later reads cannot fail; the
    /// ladder and how many orders it describes.
    pub fn parse(bytes: &'a [u8]) -> Result<(Self, usize), ProgramError> {
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
        Ok((ladder, total))
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

    /// Fails when any order around `price` is one Manifest cannot take.
    /// Within a band prices rise with the level, and the price exponent with
    /// them, while the base amount falls; so each band's lowest and highest
    /// level are its extremes.
    pub fn check_orders(
        &self,
        price: Price,
        base_decimals: u8,
        quote_decimals: u8,
    ) -> Result<(), ProgramError> {
        for i in 0..self.band_count() {
            let b = self.band(i);
            level_order(&b, 0, price, base_decimals, quote_decimals)?;
            level_order(
                &b,
                b.levels as i128 - 1,
                price,
                base_decimals,
                quote_decimals,
            )?;
        }
        Ok(())
    }

    /// Order `k` around `price`, None when it rounds to nothing (no
    /// distance from the price, or too small for one base atom).
    pub fn order(
        &self,
        k: usize,
        price: Price,
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
            return level_order(&b, k as i128, price, base_decimals, quote_decimals);
        }
        Err(ProgramError::InvalidArgument)
    }

    /// Fails unless, on each side of the opening price, no placed order is
    /// farther from it than an unplaced one. Placing nearest first keeps the
    /// position from trading with itself: an order that fills flips to the
    /// other side by its spread, so it stays between the position's
    /// outermost bid and ask, and every later order goes out beyond those.
    /// Levels at the opening price, which place nothing, are on neither side.
    pub fn check_nearest_first(&self, placed: &[u8]) -> Result<(), ProgramError> {
        let is_placed = |k: usize| placed[k / 8] & (1 << (k % 8)) != 0;
        // Per side, the farthest placed and the nearest unplaced distance.
        let mut bids = (0u64, u64::MAX);
        let mut asks = (0u64, u64::MAX);
        let mut first = 0;
        for i in 0..self.band_count() {
            let b = self.band(i);
            let levels = b.levels as usize;
            // Levels rise in price: bids, perhaps one at the opening price,
            // then asks.
            let bid_levels = partition_point(levels, |l| level_rel_e8(&b, l) < 0);
            let ask_from = if bid_levels < levels && level_rel_e8(&b, bid_levels) == 0 {
                bid_levels + 1
            } else {
                bid_levels
            };
            // Bids nearest first from the top, asks from the bottom.
            let placed_here = |l: usize| is_placed(first + l);
            for (side, (last_placed, first_unplaced)) in [
                (&mut bids, frontier((0..bid_levels).rev(), placed_here)?),
                (&mut asks, frontier(ask_from..levels, placed_here)?),
            ] {
                if let Some(l) = last_placed {
                    side.0 = side.0.max(level_rel_e8(&b, l).unsigned_abs());
                }
                if let Some(l) = first_unplaced {
                    side.1 = side.1.min(level_rel_e8(&b, l).unsigned_abs());
                }
            }
            first += levels;
        }
        if bids.0 > bids.1 || asks.0 > asks.1 {
            return Err(ProgramError::Custom(ERROR_NOT_NEAREST_FIRST));
        }
        Ok(())
    }
}

/// PlaceOrders fails with this when an order would go out before a nearer
/// one of its side, see `Ladder::check_nearest_first`.
pub const ERROR_NOT_NEAREST_FIRST: u32 = 1002;

/// Along one side's levels of a band, nearest first: the last placed and the
/// first unplaced level. Fails when a placed level follows an unplaced one.
fn frontier(
    mut near: impl Iterator<Item = usize>,
    is_placed: impl Fn(usize) -> bool,
) -> Result<(Option<usize>, Option<usize>), ProgramError> {
    let mut last_placed = None;
    for l in near.by_ref() {
        if !is_placed(l) {
            if near.any(&is_placed) {
                return Err(ProgramError::Custom(ERROR_NOT_NEAREST_FIRST));
            }
            return Ok((last_placed, Some(l)));
        }
        last_placed = Some(l);
    }
    Ok((last_placed, None))
}

/// The first of `0..n` for which `pred` is false, `pred` true on a prefix.
fn partition_point(n: usize, pred: impl Fn(usize) -> bool) -> usize {
    let (mut lo, mut hi) = (0, n);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if pred(mid) {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

/// Level `i`'s centre relative to the opening price, in 1e-8: lo + (i + ½) ·
/// width / levels. Fits an i64 (at most about 3.3e8 · 2^17), and rises with
/// the level.
fn level_rel_e8(b: &Band, i: usize) -> i64 {
    let lo = b.lo_bp as i64 * 10_000;
    let width = (b.hi_bp as i64 - b.lo_bp as i64) * 10_000;
    lo + (2 * i as i64 + 1) * width / (2 * b.levels as i64)
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
    /// Distance from the opening price, in 1e-8 of it.
    pub distance_e8: u64,
}

fn pow10(e: u32) -> Result<i128, ProgramError> {
    10i128
        .checked_pow(e)
        .ok_or(ProgramError::ArithmeticOverflow)
}

fn level_order(
    b: &Band,
    i: i128,
    price: Price,
    base_decimals: u8,
    quote_decimals: u8,
) -> Result<Option<LadderOrder>, ProgramError> {
    let overflow = || ProgramError::ArithmeticOverflow;
    let rel_e8 = level_rel_e8(b, i as usize) as i128;
    // Quote atoms per base atom at the level: p · 10^x, with p = mantissa ·
    // (1 + rel) · 1e8, above zero as lo > -100%.
    let mut p = price.mantissa as i128 * (REL_ONE + rel_e8);
    if p <= 0 {
        return Err(ProgramError::InvalidArgument);
    }
    let mut x = price.exponent as i32 - REL_ONE_EXP + quote_decimals as i32 - base_decimals as i32;
    // Base atoms are size · 10^-x / p: a price far below one quote atom per
    // base atom gives up digits of p, never more than it has.
    while x < -MAX_SIZE_SCALE_EXP {
        p /= 10;
        x += 1;
    }
    if p == 0 {
        return Err(ProgramError::InvalidArgument);
    }
    // Base atoms worth the band's size at the level's price.
    let size = b.size_quote_atoms as i128;
    let base_atoms = if x <= 0 {
        size.checked_mul(pow10((-x) as u32)?).ok_or_else(overflow)? / p
    } else {
        size / p.checked_mul(pow10(x as u32)?).ok_or_else(overflow)?
    };
    // Manifest takes a u32 mantissa and exponents from -18 (its smallest
    // price) to 0 (ReverseTight's largest); the digits beyond round down.
    let (mut mantissa, mut exponent) = (p, x);
    while mantissa > u32::MAX as i128 || exponent < -18 {
        mantissa /= 10;
        exponent += 1;
    }
    if exponent > 0 || mantissa == 0 {
        return Err(ProgramError::InvalidArgument);
    }
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
        base_atoms,
        price_mantissa: mantissa as u32,
        price_exponent: exponent as i8,
        quote_atoms,
        spread_e1: b.spread_e1,
        distance_e8: rel_e8.unsigned_abs() as u64,
    }))
}
