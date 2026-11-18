use std::path::PathBuf;

use super::{load_initial_state, run_transactions, TestFixture};

const RUST_LOG_DEFAULT: &str = "solana_rbpf::vm=info,\
             solana_program_runtime::stable_log=debug,\
             solana_runtime::message_processor=debug,\
             solana_runtime::system_instruction_processor=info,\
             solana_program_test=info,\
             solana_bpf_loader_program=debug";

pub async fn run_test(initial_data_path: String, transaction_data_path: String) {
    let mut initial_test_data_path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    initial_test_data_path.push(initial_data_path);
    let test_fixture: TestFixture =
        load_initial_state(initial_test_data_path.display().to_string()).await;

    solana_logger::setup_with_default(RUST_LOG_DEFAULT);

    // Run through all transactions
    let mut tx_test_data_path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    tx_test_data_path.push(transaction_data_path);
    run_transactions(tx_test_data_path.display().to_string(), test_fixture).await;
}
