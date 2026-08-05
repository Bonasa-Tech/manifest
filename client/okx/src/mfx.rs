use async_trait::async_trait;
use hypertree::{
    hypertree::{HyperTreeReadOperations, HyperTreeValueIteratorTrait},
    validate_red_black_tree,
};
use log::error;
use manifest::{
    quantities::WrapperU64,
    state::{MarketValue, RestingOrder, MARKET_FIXED_DISCRIMINANT},
};
use solana_account_decoder::UiAccountEncoding;
use solana_client::{
    rpc_client::RpcClient,
    rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig},
    rpc_filter::{Memcmp, RpcFilterType},
};
use solana_program::pubkey::Pubkey;
use std::{collections::HashMap, error::Error, str::FromStr};
use tokio::sync::mpsc::Sender;

use super::{get_extra, Dex, PoolMetadata, PoolMetadataValue};

pub struct Manifest;

#[async_trait]
impl Dex for Manifest {
    fn dex_name(&self) -> String {
        "Manifest".to_string()
    }

    fn dex_program_id(&self) -> Pubkey {
        manifest::ID
    }

    fn quote(&self, amount_in: f64, metadata: &PoolMetadata) -> f64 {
        // Assumes asks only because the interface does not have a way to differentiate direction.
        if amount_in <= 0.0 {
            return 0.0;
        }

        let mut remaining_in = amount_in;
        let mut total_out = 0.0;

        let asks = get_extra!(metadata, "asks", PoolMetadataValue::Array).unwrap_or(vec![]);
        let base_decimals =
            get_extra!(metadata, "base_decimals", PoolMetadataValue::Number).unwrap_or(6.0_f64);
        let quote_decimals =
            get_extra!(metadata, "quote_decimals", PoolMetadataValue::Number).unwrap_or(6.0_f64);
        // Metadata is untrusted: bound exponents before integer pow and reject
        // NaN/fractional values that would otherwise truncate during casting.
        let decimal_scale = |decimals: f64| -> Option<f64> {
            if !decimals.is_finite() || decimals.fract() != 0.0 || !(0.0..=19.0).contains(&decimals)
            {
                return None;
            }
            Some(10_u64.checked_pow(decimals as u32)? as f64)
        };
        let Some(base_scale) = decimal_scale(base_decimals) else {
            return 0.0;
        };
        let Some(quote_scale) = decimal_scale(quote_decimals) else {
            return 0.0;
        };

        for ask in asks {
            match ask {
                PoolMetadataValue::Array(ask) => {
                    if ask.len() < 2 {
                        continue;
                    }
                    let base_atoms = match ask[0] {
                        PoolMetadataValue::Number(base_atoms) => base_atoms,
                        _ => 0.0_f64,
                    };
                    let quote_atoms = match ask[1] {
                        PoolMetadataValue::Number(base_atoms) => base_atoms,
                        _ => 0.0_f64,
                    };
                    let base_tokens = base_atoms / base_scale;
                    let quote_tokens = quote_atoms / quote_scale;
                    if !base_tokens.is_finite()
                        || !quote_tokens.is_finite()
                        || base_tokens <= 0.0
                        || quote_tokens < 0.0
                    {
                        continue;
                    }

                    if base_tokens <= remaining_in {
                        total_out += quote_tokens;
                        remaining_in -= base_tokens;
                        continue;
                    }
                    total_out += remaining_in / base_tokens * quote_tokens;
                    return total_out;
                }
                _ => {}
            }
        }

        // Fall through in case we exhaust the book and do not fully fill.
        total_out
    }

    fn fetch_pool_addresses(&self, client: &RpcClient) -> Vec<String> {
        let accounts = match client.get_program_accounts_with_config(
            &manifest::id().to_bytes().into(),
            RpcProgramAccountsConfig {
                filters: Some(vec![RpcFilterType::Memcmp(Memcmp::new_raw_bytes(
                    0,
                    MARKET_FIXED_DISCRIMINANT.to_le_bytes().into(),
                ))]),
                account_config: RpcAccountInfoConfig {
                    encoding: Some(UiAccountEncoding::Base64),
                    ..RpcAccountInfoConfig::default()
                },
                ..RpcProgramAccountsConfig::default()
            },
        ) {
            Ok(accs) => accs,
            Err(e) => {
                error!(
                    "Failed to fetch {} market addresses: {}",
                    self.dex_name(),
                    e
                );
                return Vec::new();
            }
        };
        accounts
            .into_iter()
            .map(|(pk, _acct)| pk.to_string())
            .collect()
    }

    async fn listen_new_pool_addresses(
        &self,
        _client: &RpcClient,
        _address_tx: Sender<String>,
    ) -> Result<(), Box<dyn Error>> {
        // To implement this, if you want to watch all ix on chain for Manifest
        // and find new inits, look for the first by of call data to the
        // manifest ix to be 0x0
        // (https://github.com/Bonasa-Tech/manifest/blob/93d78d5ca60ac7d9ea282f7d57ec5ea61f13de48/programs/manifest/src/program/instruction.rs#L20)

        Ok(())
    }

    fn fetch_pool_metadata(&self, client: &RpcClient, pool_address: &str) -> Option<PoolMetadata> {
        let market_account = client
            .get_account(&Pubkey::from_str(pool_address).ok()?)
            .ok()?;
        if market_account.owner != manifest::ID {
            return None;
        }
        let market_data = market_account.data;
        let market: MarketValue =
            manifest::program::get_dynamic_value_or(market_data.as_slice()).ok()?;
        if market.fixed.discriminant != MARKET_FIXED_DISCRIMINANT {
            return None;
        }
        let bids = market.get_bids();
        let asks = market.get_asks();
        // Validate RPC-sourced links before the zero-copy iterator follows them.
        validate_red_black_tree::<RestingOrder>(
            &market.dynamic,
            bids.get_root_index(),
            bids.get_max_index(),
        )
        .ok()?;
        validate_red_black_tree::<RestingOrder>(
            &market.dynamic,
            asks.get_root_index(),
            asks.get_max_index(),
        )
        .ok()?;
        let base_vault: &Pubkey = market.fixed.get_base_vault();
        let quote_vault: &Pubkey = market.fixed.get_quote_vault();

        let base_reserve: Option<f64> = match client.get_token_account_balance(base_vault) {
            Ok(resp) => Some(resp.ui_amount.unwrap_or(0.0)),
            Err(_) => None,
        };
        let quote_reserve: Option<f64> = match client.get_token_account_balance(quote_vault) {
            Ok(resp) => Some(resp.ui_amount.unwrap_or(0.0)),
            Err(_) => None,
        };

        let mut extra = HashMap::new();
        let base_mint_decimals: u8 = market.fixed.get_base_mint_decimals();
        extra.insert(
            "base_decimals".to_string(),
            PoolMetadataValue::Number(base_mint_decimals as f64),
        );
        extra.insert(
            "quote_decimals".to_string(),
            PoolMetadataValue::Number(market.fixed.get_quote_mint_decimals() as f64),
        );

        // Bids is an array of arrays. Top of book is first. Similar for asks.
        // [ [baseAtoms1, quoteAtoms1], [baseAtoms2, quoteAtoms2], [baseAtoms3, quoteAtoms3], ...]
        let bids_vec: Vec<PoolMetadataValue> = bids
            .iter::<RestingOrder>()
            .map(|(_ind, resting_order)| {
                let bid = resting_order;
                PoolMetadataValue::Array(vec![
                    PoolMetadataValue::Number(bid.get_num_base_atoms().as_u64() as f64),
                    PoolMetadataValue::Number(
                        bid.get_price()
                            .checked_quote_for_base(bid.get_num_base_atoms(), true)
                            .unwrap_or_default()
                            .as_u64() as f64,
                    ),
                ])
            })
            .collect::<Vec<PoolMetadataValue>>();
        extra.insert("bids".to_string(), PoolMetadataValue::Array(bids_vec));

        let asks_vec: Vec<PoolMetadataValue> = asks
            .iter::<RestingOrder>()
            .map(|(_ind, resting_order)| {
                let ask = resting_order;
                PoolMetadataValue::Array(vec![
                    PoolMetadataValue::Number(ask.get_num_base_atoms().as_u64() as f64),
                    PoolMetadataValue::Number(
                        ask.get_price()
                            .checked_quote_for_base(ask.get_num_base_atoms(), true)
                            .unwrap_or_default()
                            .as_u64() as f64,
                    ),
                ])
            })
            .collect::<Vec<PoolMetadataValue>>();
        extra.insert("asks".to_string(), PoolMetadataValue::Array(asks_vec));

        Some(PoolMetadata {
            pool_address: pool_address.to_string(),
            base_mint: market.get_base_mint().to_string(),
            quote_mint: market.get_quote_mint().to_string(),
            base_reserve,
            quote_reserve,
            trade_fee: Some(0.0_f64),
            extra,
        })
    }
}
