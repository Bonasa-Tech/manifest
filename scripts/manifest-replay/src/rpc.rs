use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::types::{AccountSnapshot, SnapshotSource};

pub const MANIFEST_PROGRAM: &str = "MNFSTqtC93rEfYHB6hF82sKdZpUDFWkViLByLd1k1Ms";

pub struct Rpc {
    client: Client,
    url: String,
    commitment: String,
    next_id: AtomicU64,
}

#[derive(Debug, Deserialize)]
pub struct ContextValue<T> {
    pub context: RpcContext,
    pub value: T,
}

#[derive(Debug, Deserialize)]
pub struct RpcContext {
    pub slot: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcAccount {
    pub lamports: u64,
    pub owner: String,
    pub executable: bool,
    pub rent_epoch: u64,
    pub data: (String, String),
}

#[derive(Clone, Debug, Deserialize)]
pub struct SignatureInfo {
    pub signature: String,
    pub slot: u64,
    pub err: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct BlockSignatures {
    pub signatures: Vec<String>,
}

impl RpcAccount {
    pub fn snapshot(&self, address: String, source: SnapshotSource) -> Result<AccountSnapshot> {
        if self.data.1 != "base64" {
            bail!("RPC returned unsupported account encoding {}", self.data.1);
        }
        BASE64
            .decode(&self.data.0)
            .with_context(|| format!("invalid base64 account data for {address}"))?;
        Ok(AccountSnapshot {
            address,
            lamports: self.lamports,
            owner: self.owner.clone(),
            executable: self.executable,
            rent_epoch: self.rent_epoch,
            data_base64: self.data.0.clone(),
            source,
        })
    }
}

impl Rpc {
    pub fn new(url: String, commitment: String) -> Self {
        Self {
            client: Client::new(),
            url,
            commitment,
            next_id: AtomicU64::new(1),
        }
    }

    async fn call<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let response = self
            .client
            .post(&self.url)
            .json(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .send()
            .await
            .with_context(|| format!("RPC {method} request failed"))?
            .error_for_status()
            .with_context(|| format!("RPC {method} returned an HTTP error"))?;
        let body: Value = response
            .json()
            .await
            .with_context(|| format!("RPC {method} returned invalid JSON"))?;
        if let Some(error) = body.get("error") {
            return Err(anyhow!("RPC {method} error: {error}"));
        }
        serde_json::from_value(
            body.get("result")
                .cloned()
                .ok_or_else(|| anyhow!("RPC {method} response has no result"))?,
        )
        .with_context(|| format!("could not decode RPC {method} result"))
    }

    pub async fn account(
        &self,
        address: &str,
        min_slot: Option<u64>,
    ) -> Result<ContextValue<Option<RpcAccount>>> {
        let mut config = json!({"encoding": "base64", "commitment": self.commitment});
        if let Some(slot) = min_slot {
            config["minContextSlot"] = json!(slot);
        }
        self.call("getAccountInfo", json!([address, config])).await
    }

    pub async fn accounts(
        &self,
        addresses: &[String],
        min_slot: Option<u64>,
    ) -> Result<ContextValue<Vec<Option<RpcAccount>>>> {
        let mut config = json!({"encoding": "base64", "commitment": self.commitment});
        if let Some(slot) = min_slot {
            config["minContextSlot"] = json!(slot);
        }
        self.call("getMultipleAccounts", json!([addresses, config]))
            .await
    }

    pub async fn slot(&self) -> Result<u64> {
        self.call("getSlot", json!([{"commitment": self.commitment}]))
            .await
    }

    pub async fn signatures(
        &self,
        address: &str,
        before: Option<&str>,
    ) -> Result<Vec<SignatureInfo>> {
        let mut config = json!({"limit": 1000, "commitment": self.commitment});
        if let Some(signature) = before {
            config["before"] = json!(signature);
        }
        self.call("getSignaturesForAddress", json!([address, config]))
            .await
    }

    pub async fn block_signatures(&self, slot: u64) -> Result<Vec<String>> {
        let block: Option<BlockSignatures> = self
            .call(
                "getBlock",
                json!([slot, {
                    "commitment": self.commitment,
                    "encoding": "json",
                    "transactionDetails": "signatures",
                    "rewards": false,
                    "maxSupportedTransactionVersion": 0
                }]),
            )
            .await?;
        block
            .map(|value| value.signatures)
            .ok_or_else(|| anyhow!("finalized block {slot} was unavailable"))
    }

    pub async fn transaction(&self, signature: &str) -> Result<Option<Value>> {
        self.call(
            "getTransaction",
            json!([signature, {
                "commitment": self.commitment,
                "encoding": "json",
                "maxSupportedTransactionVersion": 0
            }]),
        )
        .await
    }
}
