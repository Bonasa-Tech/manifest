use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fixture {
    pub version: u32,
    pub rpc_url: String,
    pub commitment: String,
    pub market: String,
    pub manifest_program: String,
    // Program ID -> base64 ELF captured from the same RPC as the market.
    // Old fixtures deserialize so replay can explain that recapture is required.
    #[serde(default)]
    pub token_programs: std::collections::BTreeMap<String, String>,
    pub start_slot: u64,
    pub end_slot: u64,
    pub transactions_touching_market: usize,
    pub failed_transactions_skipped: usize,
    pub successful_transactions_without_manifest: usize,
    pub baseline_missing_accounts: Vec<String>,
    pub accounts: Vec<AccountSnapshot>,
    pub instructions: Vec<CapturedInstruction>,
    pub chain_final_market: AccountSnapshot,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSnapshot {
    pub address: String,
    pub lamports: u64,
    pub owner: String,
    pub executable: bool,
    pub rent_epoch: u64,
    pub data_base64: String,
    pub source: SnapshotSource,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SnapshotSource {
    Baseline,
    EndSlot,
    FundedToken,
    FundedSystem,
    SynthesizedToken,
    SynthesizedSystem,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedInstruction {
    pub signature: String,
    pub slot: u64,
    pub transaction_position: usize,
    pub outer_instruction_index: usize,
    pub inner_instruction_index: Option<usize>,
    pub name: String,
    pub data_base58: String,
    pub accounts: Vec<CapturedAccountMeta>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedAccountMeta {
    pub address: String,
    pub is_signer: bool,
    pub is_writable: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayResult {
    pub label: String,
    pub program_path: String,
    pub program_sha256: String,
    pub token_program_sha256: std::collections::BTreeMap<String, String>,
    pub instruction_results: Vec<InstructionResult>,
    pub final_market_data_base64: String,
    pub final_market_sha256: String,
    pub market_summary: MarketSummary,
    pub program_test_rent_top_ups: std::collections::BTreeMap<String, u64>,
    pub final_accounts: std::collections::BTreeMap<String, FinalAccountState>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalAccountState {
    pub lamports: u64,
    pub owner: String,
    pub executable: bool,
    pub data_len: usize,
    pub data_sha256: String,
    #[serde(skip)]
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstructionResult {
    pub signature: String,
    pub slot: u64,
    pub name: String,
    pub success: bool,
    pub compute_units: u64,
    pub error: Option<String>,
    pub logs: Vec<String>,
    pub base_token_delta: Option<i128>,
    pub quote_token_delta: Option<i128>,
    pub order_sequence_delta: u64,
    pub resting_bid_delta: i64,
    pub resting_ask_delta: i64,
    pub new_resting_orders: Vec<RestingOrderResult>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestingOrderResult {
    pub sequence_number: u64,
    pub trader: String,
    pub side: String,
    pub num_base_atoms: u64,
    pub price_raw: String,
    pub order_type: u8,
    pub order_type_name: String,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketSummary {
    pub data_len: usize,
    pub order_sequence_number: u64,
    pub quote_volume_atoms: u64,
    pub resting_bids: usize,
    pub resting_asks: usize,
    pub claimed_seats: usize,
    pub base_mint: String,
    pub quote_mint: String,
    pub base_global: Option<String>,
    pub quote_global: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputeUnitComparison {
    pub instruction_count: usize,
    pub old_successes: usize,
    pub new_successes: usize,
    pub old_total_compute_units: u64,
    pub new_total_compute_units: u64,
    pub compute_unit_delta: i128,
    pub percent_change: Option<f64>,
    pub old_average_compute_units: f64,
    pub new_average_compute_units: f64,
    pub both_successful_count: usize,
    pub both_successful_old_total_compute_units: u64,
    pub both_successful_new_total_compute_units: u64,
    pub both_successful_compute_unit_delta: i128,
    pub both_successful_percent_change: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonReport {
    pub fixture: String,
    pub market: String,
    pub start_slot: u64,
    pub end_slot: u64,
    pub transactions_touching_market: usize,
    pub failed_transactions_skipped: usize,
    pub successful_transactions_without_manifest: usize,
    pub account_snapshot_counts: std::collections::BTreeMap<String, usize>,
    pub baseline_missing_accounts: Vec<String>,
    pub instruction_counts: std::collections::BTreeMap<String, usize>,
    pub compute_unit_comparison: std::collections::BTreeMap<String, ComputeUnitComparison>,
    pub identical_writable_accounts: usize,
    pub changed_writable_accounts: Vec<AccountDiff>,
    pub old: ReplayResult,
    pub new: ReplayResult,
    pub old_vs_new: ByteDiff,
    pub old_vs_chain: ByteDiff,
    pub new_vs_chain: ByteDiff,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDiff {
    pub address: String,
    pub old_exists: bool,
    pub new_exists: bool,
    pub old_lamports: Option<u64>,
    pub new_lamports: Option<u64>,
    pub old_owner: Option<String>,
    pub new_owner: Option<String>,
    pub old_data_sha256: Option<String>,
    pub new_data_sha256: Option<String>,
    pub data: ByteDiff,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ByteDiff {
    pub equal: bool,
    pub left_len: usize,
    pub right_len: usize,
    pub differing_bytes: usize,
    pub ranges: Vec<ByteDiffRange>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ByteDiffRange {
    pub start: usize,
    pub end_exclusive: usize,
    pub left_hex: String,
    pub right_hex: String,
}

pub fn instruction_name(data: &[u8]) -> String {
    match data.first().copied() {
        Some(0) => "CreateMarket",
        Some(1) => "ClaimSeat",
        Some(2) => "Deposit",
        Some(3) => "Withdraw",
        Some(4) => "Swap",
        Some(5) => "Expand",
        Some(6) => "BatchUpdate",
        Some(7) => "GlobalCreate",
        Some(8) => "GlobalAddTrader",
        Some(9) => "GlobalDeposit",
        Some(10) => "GlobalWithdraw",
        Some(11) => "GlobalEvict",
        Some(12) => "GlobalClean",
        Some(13) => "SwapV2",
        Some(other) => return format!("Unknown({other})"),
        None => "Empty",
    }
    .to_owned()
}
