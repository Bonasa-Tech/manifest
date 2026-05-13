use async_trait::async_trait;
use solana_client::rpc_client::RpcClient;
use solana_program::pubkey::Pubkey;
use std::{collections::HashMap, error::Error};
use tokio::sync::mpsc::Sender;

pub mod mfx;

// Trait from: https://web3.okx.com/build/docs/waas/dex-integration
#[async_trait]
pub trait Dex: Send + Sync {
    // Return DEX Name
    fn dex_name(&self) -> String;

    // Return DEX Program ID
    fn dex_program_id(&self) -> Pubkey;

    // Quote function
    fn quote(&self, amount_in: f64, metadata: &PoolMetadata) -> f64;

    // fetch all the pool address
    fn fetch_pool_addresses(&self, client: &RpcClient) -> Vec<String>;

    // monitor new pools and event changes
    async fn listen_new_pool_addresses(
        &self,
        client: &RpcClient,
        address_tx: Sender<String>,
    ) -> Result<(), Box<dyn Error>>;

    // To export quote parameters from the pool address
    fn fetch_pool_metadata(&self, client: &RpcClient, pool_address: &str) -> Option<PoolMetadata>;
}

// abstract design for pool metadata
#[derive(Clone)]
pub struct PoolMetadata {
    pub pool_address: String,
    pub base_mint: String,
    pub quote_mint: String,
    pub base_reserve: Option<f64>,
    pub quote_reserve: Option<f64>,
    pub trade_fee: Option<f64>,
    pub extra: HashMap<String, PoolMetadataValue>,
}

// Extended value types for pool
#[derive(Clone)]
pub enum PoolMetadataValue {
    String(String),
    Number(f64),
    Bool(bool),
    Array(Vec<PoolMetadataValue>),
    Map(HashMap<String, PoolMetadataValue>),
}

// Generic simplify HashMap access
macro_rules! get_extra {
    ($metadata:expr, $key:expr, $variant:path) => {
        $metadata.extra.get($key).and_then(|v| match v {
            $variant(val) => Some(val.clone()),
            _ => None,
        })
    };
}

pub(crate) use get_extra;
