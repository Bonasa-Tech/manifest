use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use std::collections::BTreeMap;

use crate::types::{
    AccountDiff, ByteDiff, ByteDiffRange, ComparisonReport, ComputeUnitComparison, Fixture,
    ReplayResult,
};

pub fn build_report(
    fixture_path: String,
    fixture: &Fixture,
    old: ReplayResult,
    new: ReplayResult,
) -> Result<ComparisonReport> {
    let chain = BASE64.decode(&fixture.chain_final_market.data_base64)?;
    let old_bytes = BASE64.decode(&old.final_market_data_base64)?;
    let new_bytes = BASE64.decode(&new.final_market_data_base64)?;
    let mut instruction_counts = BTreeMap::new();
    for ix in &fixture.instructions {
        *instruction_counts.entry(ix.name.clone()).or_insert(0) += 1;
    }
    let mut account_snapshot_counts = BTreeMap::new();
    for account in &fixture.accounts {
        *account_snapshot_counts
            .entry(format!("{:?}", account.source))
            .or_insert(0) += 1;
    }
    let (identical_writable_accounts, changed_writable_accounts) = account_diffs(&old, &new);
    let compute_unit_comparison = compute_unit_comparison(&old, &new)?;
    Ok(ComparisonReport {
        fixture: fixture_path,
        market: fixture.market.clone(),
        start_slot: fixture.start_slot,
        end_slot: fixture.end_slot,
        transactions_touching_market: fixture.transactions_touching_market,
        failed_transactions_skipped: fixture.failed_transactions_skipped,
        successful_transactions_without_manifest: fixture.successful_transactions_without_manifest,
        account_snapshot_counts,
        baseline_missing_accounts: fixture.baseline_missing_accounts.clone(),
        instruction_counts,
        compute_unit_comparison,
        identical_writable_accounts,
        changed_writable_accounts,
        old_vs_new: byte_diff(&old_bytes, &new_bytes),
        old_vs_chain: byte_diff(&old_bytes, &chain),
        new_vs_chain: byte_diff(&new_bytes, &chain),
        old,
        new,
    })
}

pub fn print_report(report: &ComparisonReport) {
    println!("\nManifest mainnet upgrade replay");
    println!("market: {}", report.market);
    println!(
        "slots: {}..={} ({} slot span)",
        report.start_slot,
        report.end_slot,
        report.end_slot - report.start_slot
    );
    println!(
        "Manifest instructions: {}",
        report.instruction_counts.values().sum::<usize>()
    );
    let replayed_transactions = report
        .old
        .instruction_results
        .iter()
        .map(|result| result.signature.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    println!(
        "transactions: {} touched market, {} represented in replay, {} failed and {} had no Manifest invocation",
        report.transactions_touching_market,
        replayed_transactions,
        report.failed_transactions_skipped,
        report.successful_transactions_without_manifest,
    );
    println!(
        "account snapshots: {}",
        report
            .account_snapshot_counts
            .iter()
            .map(|(source, count)| format!("{source}={count}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    if !report.baseline_missing_accounts.is_empty() {
        println!(
            "baseline accounts absent on chain: {}",
            report.baseline_missing_accounts.join(", ")
        );
    }
    for (name, count) in &report.instruction_counts {
        println!("  {name}: {count}");
    }
    print_run("old", &report.old);
    print_run("new", &report.new);
    println!("\nCU difference by instruction type (new - old)");
    for (name, comparison) in &report.compute_unit_comparison {
        let percent = comparison
            .percent_change
            .map_or_else(|| "n/a".to_owned(), |value| format!("{value:+.1}%"));
        println!(
            "  {name}: count={} old total/avg={}/{:.1} new total/avg={}/{:.1} delta={:+} ({percent}) successes={}/{} -> {}/{}",
            comparison.instruction_count,
            comparison.old_total_compute_units,
            comparison.old_average_compute_units,
            comparison.new_total_compute_units,
            comparison.new_average_compute_units,
            comparison.compute_unit_delta,
            comparison.old_successes,
            comparison.instruction_count,
            comparison.new_successes,
            comparison.instruction_count,
        );
        if comparison.both_successful_count != comparison.instruction_count {
            let both_percent = comparison
                .both_successful_percent_change
                .map_or_else(|| "n/a".to_owned(), |value| format!("{value:+.1}%"));
            println!(
                "    both-successful only: count={} old total={} new total={} delta={:+} ({both_percent})",
                comparison.both_successful_count,
                comparison.both_successful_old_total_compute_units,
                comparison.both_successful_new_total_compute_units,
                comparison.both_successful_compute_unit_delta,
            );
        }
    }
    println!(
        "\nwritable accounts, old vs new: {} identical, {} changed",
        report.identical_writable_accounts,
        report.changed_writable_accounts.len()
    );
    for account in &report.changed_writable_accounts {
        println!(
            "  {}: lamports {:?} -> {:?}, data differences={} ({} vs {} bytes)",
            account.address,
            account.old_lamports,
            account.new_lamports,
            account.data.differing_bytes,
            account.data.left_len,
            account.data.right_len,
        );
    }
    print_diff("old vs new", &report.old_vs_new);
    print_diff("old replay vs captured chain", &report.old_vs_chain);
    print_diff("new replay vs captured chain", &report.new_vs_chain);
}

fn compute_unit_comparison(
    old: &ReplayResult,
    new: &ReplayResult,
) -> Result<BTreeMap<String, ComputeUnitComparison>> {
    anyhow::ensure!(
        old.instruction_results.len() == new.instruction_results.len(),
        "old and new replay result counts differ"
    );
    let mut comparisons = BTreeMap::<String, ComputeUnitComparison>::new();
    for (old_result, new_result) in old.instruction_results.iter().zip(&new.instruction_results) {
        anyhow::ensure!(
            old_result.name == new_result.name,
            "old and new instruction types differ at slot {}",
            old_result.slot
        );
        anyhow::ensure!(
            old_result.slot == new_result.slot && old_result.signature == new_result.signature,
            "old and new replay results are not aligned"
        );
        let comparison = comparisons.entry(old_result.name.clone()).or_default();
        comparison.instruction_count += 1;
        comparison.old_successes += usize::from(old_result.success);
        comparison.new_successes += usize::from(new_result.success);
        comparison.old_total_compute_units += old_result.compute_units;
        comparison.new_total_compute_units += new_result.compute_units;
        if old_result.success && new_result.success {
            comparison.both_successful_count += 1;
            comparison.both_successful_old_total_compute_units += old_result.compute_units;
            comparison.both_successful_new_total_compute_units += new_result.compute_units;
        }
    }
    for comparison in comparisons.values_mut() {
        let count = comparison.instruction_count as f64;
        comparison.compute_unit_delta =
            comparison.new_total_compute_units as i128 - comparison.old_total_compute_units as i128;
        comparison.percent_change = (comparison.old_total_compute_units != 0).then(|| {
            comparison.compute_unit_delta as f64 * 100.0 / comparison.old_total_compute_units as f64
        });
        comparison.old_average_compute_units = comparison.old_total_compute_units as f64 / count;
        comparison.new_average_compute_units = comparison.new_total_compute_units as f64 / count;
        comparison.both_successful_compute_unit_delta =
            comparison.both_successful_new_total_compute_units as i128
                - comparison.both_successful_old_total_compute_units as i128;
        comparison.both_successful_percent_change =
            (comparison.both_successful_old_total_compute_units != 0).then(|| {
                comparison.both_successful_compute_unit_delta as f64 * 100.0
                    / comparison.both_successful_old_total_compute_units as f64
            });
    }
    Ok(comparisons)
}

fn account_diffs(old: &ReplayResult, new: &ReplayResult) -> (usize, Vec<AccountDiff>) {
    let addresses: std::collections::BTreeSet<_> = old
        .final_accounts
        .keys()
        .chain(new.final_accounts.keys())
        .cloned()
        .collect();
    let mut identical = 0;
    let mut changed = Vec::new();
    for address in addresses {
        let old_account = old.final_accounts.get(&address);
        let new_account = new.final_accounts.get(&address);
        let data = byte_diff(
            old_account.map_or(&[], |account| account.data.as_slice()),
            new_account.map_or(&[], |account| account.data.as_slice()),
        );
        let is_equal = old_account.zip(new_account).is_some_and(|(left, right)| {
            left.lamports == right.lamports
                && left.owner == right.owner
                && left.executable == right.executable
                && data.equal
        });
        if is_equal {
            identical += 1;
            continue;
        }
        changed.push(AccountDiff {
            address,
            old_exists: old_account.is_some(),
            new_exists: new_account.is_some(),
            old_lamports: old_account.map(|account| account.lamports),
            new_lamports: new_account.map(|account| account.lamports),
            old_owner: old_account.map(|account| account.owner.clone()),
            new_owner: new_account.map(|account| account.owner.clone()),
            old_data_sha256: old_account.map(|account| account.data_sha256.clone()),
            new_data_sha256: new_account.map(|account| account.data_sha256.clone()),
            data,
        });
    }
    (identical, changed)
}

fn print_run(label: &str, run: &ReplayResult) {
    println!("\n{label} program: {}", run.program_path);
    println!("  ELF sha256: {}", run.program_sha256);
    if !run.program_test_rent_top_ups.is_empty() {
        println!(
            "  ProgramTest rent top-ups: {}",
            run.program_test_rent_top_ups
                .iter()
                .map(|(address, lamports)| format!("{address}=+{lamports}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let succeeded = run
        .instruction_results
        .iter()
        .filter(|result| result.success)
        .count();
    println!(
        "  results: {succeeded}/{} succeeded",
        run.instruction_results.len()
    );
    let mut by_name: BTreeMap<&str, Vec<&crate::types::InstructionResult>> = BTreeMap::new();
    for result in &run.instruction_results {
        by_name.entry(&result.name).or_default().push(result);
    }
    for (name, results) in by_name {
        let total: u64 = results.iter().map(|r| r.compute_units).sum();
        let min = results.iter().map(|r| r.compute_units).min().unwrap_or(0);
        let max = results.iter().map(|r| r.compute_units).max().unwrap_or(0);
        let avg = if results.is_empty() {
            0
        } else {
            total / results.len() as u64
        };
        let failures = results.iter().filter(|r| !r.success).count();
        println!(
            "  {name}: count={} CU total={} min/avg/max={}/{}/{} failures={}",
            results.len(),
            total,
            min,
            avg,
            max,
            failures
        );
    }
    let swaps: Vec<_> = run
        .instruction_results
        .iter()
        .filter(|r| r.name == "Swap" || r.name == "SwapV2")
        .collect();
    if !swaps.is_empty() {
        let base: i128 = swaps.iter().filter_map(|r| r.base_token_delta).sum();
        let quote: i128 = swaps.iter().filter_map(|r| r.quote_token_delta).sum();
        println!("  net swap trader-token deltas: base={base:+} atoms quote={quote:+} atoms");
        for result in swaps {
            println!(
                "    slot={} sig={} base={:+} quote={:+} CU={} {}",
                result.slot,
                result.signature,
                result.base_token_delta.unwrap_or(0),
                result.quote_token_delta.unwrap_or(0),
                result.compute_units,
                if result.success { "ok" } else { "FAILED" }
            );
        }
    }
    let order_sequence_delta: u64 = run
        .instruction_results
        .iter()
        .map(|r| r.order_sequence_delta)
        .sum();
    let bid_delta: i64 = run
        .instruction_results
        .iter()
        .map(|r| r.resting_bid_delta)
        .sum();
    let ask_delta: i64 = run
        .instruction_results
        .iter()
        .map(|r| r.resting_ask_delta)
        .sum();
    let new_resting_orders: Vec<_> = run
        .instruction_results
        .iter()
        .flat_map(|result| result.new_resting_orders.iter())
        .collect();
    println!(
        "  orders: sequence +{order_sequence_delta}, {} newly resting, resting bid delta {bid_delta:+}, resting ask delta {ask_delta:+}",
        new_resting_orders.len()
    );
    for order in new_resting_orders {
        println!(
            "    seq={} {} base-atoms={} raw-price={} type={} trader={}",
            order.sequence_number,
            order.side,
            order.num_base_atoms,
            order.price_raw,
            order.order_type_name,
            order.trader,
        );
    }
    println!(
        "  final market: sequence={} bids={} asks={} seats={} quote-volume={} sha256={}",
        run.market_summary.order_sequence_number,
        run.market_summary.resting_bids,
        run.market_summary.resting_asks,
        run.market_summary.claimed_seats,
        run.market_summary.quote_volume_atoms,
        run.final_market_sha256
    );
    println!(
        "  global cache: base={} quote={}",
        run.market_summary.base_global.as_deref().unwrap_or("empty"),
        run.market_summary
            .quote_global
            .as_deref()
            .unwrap_or("empty"),
    );
    for result in run.instruction_results.iter().filter(|r| !r.success) {
        println!(
            "  FAILED slot={} sig={} {}: {}",
            result.slot,
            result.signature,
            result.name,
            result.error.as_deref().unwrap_or("unknown error")
        );
    }
}

fn print_diff(label: &str, diff: &ByteDiff) {
    if diff.equal {
        println!("\n{label}: identical ({} bytes)", diff.left_len);
    } else {
        println!(
            "\n{label}: {} differing byte positions (length {} vs {})",
            diff.differing_bytes, diff.left_len, diff.right_len
        );
        for range in diff.ranges.iter().take(8) {
            println!(
                "  bytes {}..{}: {} -> {}",
                range.start, range.end_exclusive, range.left_hex, range.right_hex
            );
        }
        if diff.ranges.len() > 8 {
            println!("  ... {} more ranges in report.json", diff.ranges.len() - 8);
        }
    }
}

fn byte_diff(left: &[u8], right: &[u8]) -> ByteDiff {
    let max_len = left.len().max(right.len());
    let differing_bytes = (0..max_len)
        .filter(|&i| left.get(i) != right.get(i))
        .count();
    let mut ranges = Vec::new();
    let mut index = 0;
    while index < max_len {
        if left.get(index) == right.get(index) {
            index += 1;
            continue;
        }
        let start = index;
        while index < max_len && left.get(index) != right.get(index) && index - start < 32 {
            index += 1;
        }
        ranges.push(ByteDiffRange {
            start,
            end_exclusive: index,
            left_hex: hex(&left[start.min(left.len())..index.min(left.len())]),
            right_hex: hex(&right[start.min(right.len())..index.min(right.len())]),
        });
    }
    ByteDiff {
        equal: differing_bytes == 0,
        left_len: left.len(),
        right_len: right.len(),
        differing_bytes,
        ranges,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_diff_groups_changes_and_accounts_for_length() {
        let diff = byte_diff(&[0, 1, 2, 3], &[0, 9, 8, 3, 4]);
        assert!(!diff.equal);
        assert_eq!(diff.differing_bytes, 3);
        assert_eq!(diff.ranges.len(), 2);
        assert_eq!(diff.ranges[0].start, 1);
        assert_eq!(diff.ranges[0].end_exclusive, 3);
        assert_eq!(diff.ranges[1].start, 4);
    }
}
