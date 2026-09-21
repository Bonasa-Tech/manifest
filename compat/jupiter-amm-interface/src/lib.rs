use anyhow::{anyhow, Context, Error, Result};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use solana_account_decoder::{encode_ui_account, UiAccount, UiAccountEncoding};
use solana_sdk::clock::Clock;
use std::collections::HashSet;

use std::sync::atomic::{AtomicI64, AtomicU64};
use std::sync::Arc;
use std::{collections::HashMap, convert::TryFrom, str::FromStr};
mod custom_serde;
mod swap;
use custom_serde::field_as_string;
pub use swap::{RemainingAccountsInfo, Side, Swap};

/// An abstraction in order to share reserve mints and necessary data
use solana_sdk::{account::Account, instruction::AccountMeta, pubkey::Pubkey};

#[derive(Serialize, Deserialize, PartialEq, Clone, Copy, Default, Debug)]
pub enum SwapMode {
    #[default]
    ExactIn,
    ExactOut,
}

impl FromStr for SwapMode {
    type Err = Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "ExactIn" => Ok(SwapMode::ExactIn),
            "ExactOut" => Ok(SwapMode::ExactOut),
            _ => Err(anyhow!("{} is not a valid SwapMode", s)),
        }
    }
}

#[derive(Debug)]
pub struct QuoteParams {
    pub amount: u64,
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub swap_mode: SwapMode,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Quote {
    pub in_amount: u64,
    pub out_amount: u64,
    pub fee_amount: u64,
    pub fee_mint: Pubkey,
    pub fee_pct: Decimal,
}

pub type QuoteMintToReferrer = HashMap<Pubkey, Pubkey, ahash::RandomState>;

pub struct SwapParams<'a, 'b> {
    pub swap_mode: SwapMode,
    pub in_amount: u64,
    pub out_amount: u64,
    pub source_mint: Pubkey,
    pub destination_mint: Pubkey,
    pub source_token_account: Pubkey,
    pub destination_token_account: Pubkey,
    /// This can be the user or the program authority over the source_token_account.
    pub token_transfer_authority: Pubkey,
    pub quote_mint_to_referrer: Option<&'a QuoteMintToReferrer>,
    pub jupiter_program_id: &'b Pubkey,
    /// Instead of returning the relevant Err, replace dynamic accounts with the default Pubkey
    /// This is useful for crawling market with no tick array
    pub missing_dynamic_accounts_as_default: bool,
}

impl SwapParams<'_, '_> {
    /// A placeholder to indicate an optional account or used as a terminator when consuming remaining accounts
    /// Using the jupiter program id
    pub fn placeholder_account_meta(&self) -> AccountMeta {
        AccountMeta::new_readonly(*self.jupiter_program_id, false)
    }
}

pub struct SwapAndAccountMetas {
    pub swap: Swap,
    pub account_metas: Vec<AccountMeta>,
}

pub type AccountMap = HashMap<Pubkey, Account, ahash::RandomState>;

pub fn try_get_account_data<'a>(account_map: &'a AccountMap, address: &Pubkey) -> Result<&'a [u8]> {
    account_map
        .get(address)
        .map(|account| account.data.as_slice())
        .with_context(|| format!("Could not find address: {address}"))
}

pub fn try_get_account_data_and_owner<'a>(
    account_map: &'a AccountMap,
    address: &Pubkey,
) -> Result<(&'a [u8], &'a Pubkey)> {
    let account = account_map
        .get(address)
        .with_context(|| format!("Could not find address: {address}"))?;
    Ok((account.data.as_slice(), &account.owner))
}

pub struct AmmContext {
    pub clock_ref: ClockRef,
}

pub trait Amm {
    fn from_keyed_account(keyed_account: &KeyedAccount, amm_context: &AmmContext) -> Result<Self>
    where
        Self: Sized;
    /// A human readable label of the underlying DEX
    fn label(&self) -> String;
    fn program_id(&self) -> Pubkey;
    /// The pool state or market state address
    fn key(&self) -> Pubkey;
    /// The mints that can be traded
    fn get_reserve_mints(&self) -> Vec<Pubkey>;
    /// The accounts necessary to produce a quote
    fn get_accounts_to_update(&self) -> Vec<Pubkey>;
    /// Picks necessary accounts to update it's internal state
    /// Heavy deserialization and precomputation caching should be done in this function
    fn update(&mut self, account_map: &AccountMap) -> Result<()>;

    fn quote(&self, quote_params: &QuoteParams) -> Result<Quote>;

    /// Indicates which Swap has to be performed along with all the necessary account metas
    fn get_swap_and_account_metas(&self, swap_params: &SwapParams) -> Result<SwapAndAccountMetas>;

    /// Indicates if get_accounts_to_update might return a non constant vec
    fn has_dynamic_accounts(&self) -> bool {
        false
    }

    /// Indicates whether `update` needs to be called before `get_reserve_mints`
    fn requires_update_for_reserve_mints(&self) -> bool {
        false
    }

    // Indicates that whether ExactOut mode is supported
    fn supports_exact_out(&self) -> bool {
        false
    }

    fn clone_amm(&self) -> Box<dyn Amm + Send + Sync>;

    /// It can only trade in one direction from its first mint to second mint, assuming it is a two mint AMM
    fn unidirectional(&self) -> bool {
        false
    }

    /// For testing purposes, provide a mapping of dependency programs to function
    fn program_dependencies(&self) -> Vec<(Pubkey, String)> {
        vec![]
    }

    fn get_accounts_len(&self) -> usize {
        32 // Default to a near whole legacy transaction to penalize no implementation
    }

    /// The identifier of the underlying liquidity
    ///
    /// Example:
    /// For RaydiumAmm uses Openbook market A this will return Some(A)
    /// For Openbook market A, it will also return Some(A)
    fn underlying_liquidities(&self) -> Option<HashSet<Pubkey>> {
        None
    }

    /// Provides a shortcut to establish if the AMM can be used for trading
    /// If the market is active at all
    fn is_active(&self) -> bool {
        true
    }
}

impl Clone for Box<dyn Amm + Send + Sync> {
    fn clone(&self) -> Box<dyn Amm + Send + Sync> {
        self.clone_amm()
    }
}

pub type AmmLabel = &'static str;

pub trait AmmProgramIdToLabel {
    const PROGRAM_ID_TO_LABELS: &[(Pubkey, AmmLabel)];
}

pub trait SingleProgramAmm {
    const PROGRAM_ID: Pubkey;
    const LABEL: AmmLabel;
}

impl<T: SingleProgramAmm> AmmProgramIdToLabel for T {
    const PROGRAM_ID_TO_LABELS: &[(Pubkey, AmmLabel)] = &[(Self::PROGRAM_ID, Self::LABEL)];
}

#[macro_export]
macro_rules! single_program_amm {
    ($amm_struct:ty, $program_id:expr, $label:expr) => {
        impl SingleProgramAmm for $amm_struct {
            const PROGRAM_ID: Pubkey = $program_id;
            const LABEL: &'static str = $label;
        }
    };
}

#[derive(Clone, Deserialize, Serialize)]
pub struct KeyedAccount {
    pub key: Pubkey,
    pub account: Account,
    pub params: Option<Value>,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Market {
    #[serde(with = "field_as_string")]
    pub pubkey: Pubkey,
    #[serde(with = "field_as_string")]
    pub owner: Pubkey,
    /// Additional data an Amm requires, Amm dependent and decoded in the Amm implementation
    pub params: Option<Value>,
}

impl From<KeyedAccount> for Market {
    fn from(
        KeyedAccount {
            key,
            account,
            params,
        }: KeyedAccount,
    ) -> Self {
        Market {
            pubkey: key,
            owner: account.owner,
            params,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct KeyedUiAccount {
    pub pubkey: String,
    #[serde(flatten)]
    pub ui_account: UiAccount,
    /// Additional data an Amm requires, Amm dependent and decoded in the Amm implementation
    pub params: Option<Value>,
}

impl From<KeyedAccount> for KeyedUiAccount {
    fn from(keyed_account: KeyedAccount) -> Self {
        let KeyedAccount {
            key,
            account,
            params,
        } = keyed_account;

        let ui_account = encode_ui_account(&key, &account, UiAccountEncoding::Base64, None, None);

        KeyedUiAccount {
            pubkey: key.to_string(),
            ui_account,
            params,
        }
    }
}

impl TryFrom<KeyedUiAccount> for KeyedAccount {
    type Error = Error;

    fn try_from(keyed_ui_account: KeyedUiAccount) -> Result<Self, Self::Error> {
        let KeyedUiAccount {
            pubkey,
            ui_account,
            params,
        } = keyed_ui_account;
        let account = ui_account
            .decode()
            .unwrap_or_else(|| panic!("Failed to decode ui_account for {}", pubkey));

        Ok(KeyedAccount {
            key: Pubkey::from_str(&pubkey)?,
            account,
            params,
        })
    }
}

#[derive(Default, Clone)]
pub struct ClockRef {
    pub slot: Arc<AtomicU64>,
    /// The timestamp of the first `Slot` in this `Epoch`.
    pub epoch_start_timestamp: Arc<AtomicI64>,
    /// The current `Epoch`.
    pub epoch: Arc<AtomicU64>,
    pub leader_schedule_epoch: Arc<AtomicU64>,
    pub unix_timestamp: Arc<AtomicI64>,
}

impl ClockRef {
    pub fn update(&self, clock: Clock) {
        self.epoch
            .store(clock.epoch, std::sync::atomic::Ordering::Relaxed);
        self.slot
            .store(clock.slot, std::sync::atomic::Ordering::Relaxed);
        self.unix_timestamp
            .store(clock.unix_timestamp, std::sync::atomic::Ordering::Relaxed);
        self.epoch_start_timestamp.store(
            clock.epoch_start_timestamp,
            std::sync::atomic::Ordering::Relaxed,
        );
        self.leader_schedule_epoch.store(
            clock.leader_schedule_epoch,
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

impl From<Clock> for ClockRef {
    fn from(clock: Clock) -> Self {
        ClockRef {
            epoch: Arc::new(AtomicU64::new(clock.epoch)),
            epoch_start_timestamp: Arc::new(AtomicI64::new(clock.epoch_start_timestamp)),
            leader_schedule_epoch: Arc::new(AtomicU64::new(clock.leader_schedule_epoch)),
            slot: Arc::new(AtomicU64::new(clock.slot)),
            unix_timestamp: Arc::new(AtomicI64::new(clock.unix_timestamp)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey;

    #[test]
    fn test_market_deserialization() {
        let json = r#"
        {
            "lamports": 1000,
            "owner": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            "pubkey": "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263",
            "executable": false,
            "rentEpoch": 0
        }
        "#;
        let market: Market = serde_json::from_str(json).unwrap();
        assert_eq!(
            market.owner,
            pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")
        );
        assert_eq!(
            market.pubkey,
            pubkey!("DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263")
        );
    }
}
