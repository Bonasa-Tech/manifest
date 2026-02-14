use crate::{
    logs::{emit_stack, FundingCrankLog},
    program::{get_mut_dynamic_account, ManifestError},
    quantities::{BaseAtoms, QuoteAtoms, QuoteAtomsPerBaseAtom, WrapperU64},
    require,
    state::{claimed_seat::ClaimedSeat, ClaimedSeatTreeReadOnly, MarketRefMut},
    validation::loaders::CrankFundingContext,
};
use borsh::{BorshDeserialize, BorshSerialize};
use hypertree::{get_mut_helper, DataIndex, HyperTreeValueIteratorTrait, RBNode};
use solana_program::{
    account_info::AccountInfo, clock::Clock, entrypoint::ProgramResult,
    program_error::ProgramError, pubkey::Pubkey, sysvar::Sysvar,
};
use std::cell::RefMut;

/// Pyth V2 price account magic number
const PYTH_MAGIC: u32 = 0xa1b2c3d4;
/// Offset of exponent (i32) in Pyth V2 price account
const PYTH_EXPO_OFFSET: usize = 20;
/// Offset of aggregate price (i64) in Pyth V2 price account
const PYTH_AGG_PRICE_OFFSET: usize = 208;
/// Offset of aggregate confidence (u64) in Pyth V2 price account
const PYTH_AGG_CONF_OFFSET: usize = 216;
/// Offset of aggregate status (u32) in Pyth V2 price account
const PYTH_AGG_STATUS_OFFSET: usize = 224;
/// Pyth status value for "Trading"
const PYTH_STATUS_TRADING: u32 = 1;
/// Minimum Pyth price account data length
const PYTH_MIN_DATA_LEN: usize = 240;

/// Funding period in seconds (1 hour)
const FUNDING_PERIOD_SECS: i64 = 3600;
/// Funding rate scaling factor (1e9)
const FUNDING_SCALE: i64 = 1_000_000_000;

#[derive(BorshDeserialize, BorshSerialize)]
pub struct CrankFundingParams {}

impl CrankFundingParams {
    pub fn new() -> Self {
        CrankFundingParams {}
    }
}

/// Read Pyth V2 price from account data.
/// Returns (price: i64, expo: i32, confidence: u64)
fn read_pyth_price(data: &[u8]) -> Result<(i64, i32, u64), ProgramError> {
    if data.len() < PYTH_MIN_DATA_LEN {
        solana_program::msg!("Pyth account data too small: {}", data.len());
        return Err(ManifestError::InvalidPerpsOperation.into());
    }

    let magic = u32::from_le_bytes(data[0..4].try_into().unwrap());
    if magic != PYTH_MAGIC {
        solana_program::msg!("Pyth magic mismatch: expected {:#x}, got {:#x}", PYTH_MAGIC, magic);
        return Err(ManifestError::InvalidPerpsOperation.into());
    }

    let expo = i32::from_le_bytes(
        data[PYTH_EXPO_OFFSET..PYTH_EXPO_OFFSET + 4]
            .try_into()
            .unwrap(),
    );
    let price = i64::from_le_bytes(
        data[PYTH_AGG_PRICE_OFFSET..PYTH_AGG_PRICE_OFFSET + 8]
            .try_into()
            .unwrap(),
    );
    let conf = u64::from_le_bytes(
        data[PYTH_AGG_CONF_OFFSET..PYTH_AGG_CONF_OFFSET + 8]
            .try_into()
            .unwrap(),
    );
    let status = u32::from_le_bytes(
        data[PYTH_AGG_STATUS_OFFSET..PYTH_AGG_STATUS_OFFSET + 4]
            .try_into()
            .unwrap(),
    );

    if status != PYTH_STATUS_TRADING {
        solana_program::msg!("Pyth price not trading: status={}", status);
        return Err(ManifestError::InvalidPerpsOperation.into());
    }

    if price <= 0 {
        solana_program::msg!("Pyth price not positive: {}", price);
        return Err(ManifestError::InvalidPerpsOperation.into());
    }

    Ok((price, expo, conf))
}

pub(crate) fn process_crank_funding(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let _params = CrankFundingParams::try_from_slice(data)?;
    let crank_context: CrankFundingContext = CrankFundingContext::load(accounts)?;

    let CrankFundingContext {
        market,
        payer,
        pyth_price_feed,
    } = crank_context;

    // Read Pyth price from the oracle account
    let pyth_data = pyth_price_feed.try_borrow_data()?;
    let (oracle_price, oracle_expo, _confidence) = read_pyth_price(&pyth_data)?;
    drop(pyth_data);

    // Get current timestamp
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;

    let market_data: &mut RefMut<&mut [u8]> = &mut market.try_borrow_mut_data()?;
    let mut dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);

    // Update cached oracle price
    dynamic_account
        .fixed
        .set_oracle_price(oracle_price as u64, oracle_expo);

    let last_funding_ts = dynamic_account.fixed.get_last_funding_timestamp();

    // If first crank ever, just set the timestamp and return
    if last_funding_ts == 0 {
        dynamic_account.fixed.set_last_funding_timestamp(now);
        return Ok(());
    }

    let time_elapsed = now.saturating_sub(last_funding_ts);
    if time_elapsed <= 0 {
        return Ok(());
    }

    // Compute orderbook mark price for funding rate calculation
    // Use the oracle price as the "index price" and compute mark from orderbook
    let mark_price_result =
        super::liquidate::compute_mark_price(&dynamic_account);

    // If we can't compute mark price (empty book), use oracle as both
    let mark_price: QuoteAtomsPerBaseAtom = match mark_price_result {
        Ok(p) => p,
        Err(_) => {
            // Orderbook empty — no funding rate needed, just update timestamp
            dynamic_account.fixed.set_last_funding_timestamp(now);
            return Ok(());
        }
    };

    // Convert oracle price to quote atoms for a reference amount of base atoms.
    // Oracle price = price * 10^expo (USD per unit)
    // quote_atoms_for_ref_base = oracle_price * 10^expo * ref_base / 10^base_decimals * 10^quote_decimals
    // = oracle_price * 10^(expo + quote_decimals - base_decimals + 9)  [ref_base = 1e9]
    let base_decimals = dynamic_account.fixed.get_base_mint_decimals() as i64;
    let quote_decimals = dynamic_account.fixed.get_quote_mint_decimals() as i64;

    let reference_base = BaseAtoms::new(1_000_000_000); // 1e9 base atoms for precision
    let mark_quote = mark_price
        .checked_quote_for_base(reference_base, false)?
        .as_u64() as i128;

    let oracle_quote_i128: i128 = {
        let adjusted_expo = oracle_expo as i64 + quote_decimals - base_decimals + 9;
        if adjusted_expo >= 0 {
            (oracle_price as i128) * 10i128.pow(adjusted_expo as u32)
        } else {
            let divisor = 10i128.pow((-adjusted_expo) as u32);
            (oracle_price as i128) / divisor
        }
    };

    if oracle_quote_i128 <= 0 {
        dynamic_account.fixed.set_last_funding_timestamp(now);
        return Ok(());
    }

    // Funding rate = (mark - oracle) / oracle * time_elapsed / FUNDING_PERIOD * FUNDING_SCALE
    let price_diff = mark_quote - oracle_quote_i128;
    let funding_rate_scaled: i64 = ((price_diff * FUNDING_SCALE as i128 * time_elapsed as i128)
        / (oracle_quote_i128 * FUNDING_PERIOD_SECS as i128)) as i64;

    // Update cumulative funding
    let prev_cumulative = dynamic_account.fixed.get_cumulative_funding();
    let new_cumulative = prev_cumulative.saturating_add(funding_rate_scaled);
    dynamic_account.fixed.set_cumulative_funding(new_cumulative);
    dynamic_account.fixed.set_last_funding_timestamp(now);

    // Apply funding to all traders with open positions
    // Collect trader indices first to avoid borrow issues
    let trader_indices: Vec<DataIndex> = {
        let claimed_seats_tree: ClaimedSeatTreeReadOnly = ClaimedSeatTreeReadOnly::new(
            &dynamic_account.dynamic,
            dynamic_account.fixed.get_claimed_seats_root_index(),
            hypertree::NIL,
        );
        claimed_seats_tree
            .iter::<ClaimedSeat>()
            .map(|(index, _)| index)
            .collect()
    };

    for trader_index in trader_indices {
        let claimed_seat: &mut ClaimedSeat =
            get_mut_helper::<RBNode<ClaimedSeat>>(&mut dynamic_account.dynamic, trader_index)
                .get_mut_value();

        let position_size = claimed_seat.get_position_size();
        if position_size == 0 {
            continue;
        }

        // funding_payment = position_size * funding_rate_scaled / FUNDING_SCALE
        // Longs pay when mark > oracle (positive funding_rate), receive when mark < oracle
        // Shorts are opposite
        let funding_payment: i64 = ((position_size as i128 * funding_rate_scaled as i128)
            / FUNDING_SCALE as i128) as i64;

        let current_margin = claimed_seat.quote_withdrawable_balance.as_u64();
        // Longs pay positive funding, shorts receive positive funding
        // funding_payment is already signed correctly:
        // long (pos > 0) * positive rate = positive payment (pay)
        // short (pos < 0) * positive rate = negative payment (receive)
        let new_margin = if funding_payment >= 0 {
            current_margin.saturating_sub(funding_payment as u64)
        } else {
            current_margin.saturating_add(funding_payment.unsigned_abs())
        };
        claimed_seat.quote_withdrawable_balance = QuoteAtoms::new(new_margin);
    }

    emit_stack(FundingCrankLog {
        market: *market.info.key,
        cranker: *payer.key,
        oracle_price: oracle_price as u64,
        funding_rate: funding_rate_scaled as u64,
        timestamp: now as u64,
        _padding: [0; 8],
    })?;

    Ok(())
}
