use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use hypertree::HyperTreeValueIteratorTrait;
use manifest::{
    quantities::WrapperU64,
    state::{claimed_seat::ClaimedSeat, MarketFixed, MarketValue, RestingOrder, MARKET_FIXED_SIZE},
};
use sha2::{Digest, Sha256};
use solana_account::Account;
use solana_clock::Clock;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_instruction::{AccountMeta, Instruction};
use solana_program::{program_pack::Pack, rent::Rent};
use solana_program_test::ProgramTest;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::Transaction;
use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::Path,
    str::FromStr,
};

use crate::types::{
    FinalAccountState, Fixture, InstructionResult, MarketSummary, ReplayResult, RestingOrderResult,
};

const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const TOKEN_2022_PROGRAM: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";

pub async fn replay(fixture: &Fixture, label: &str, program_path: &Path) -> Result<ReplayResult> {
    let program_bytes = fs::read(program_path).with_context(|| {
        format!(
            "could not read {} program {}",
            label,
            program_path.display()
        )
    })?;
    if !program_bytes.starts_with(b"\x7fELF") {
        return Err(anyhow!(
            "{} is not an ELF shared object",
            program_path.display()
        ));
    }
    let program_sha256 = hex(&Sha256::digest(&program_bytes));
    let temp = tempfile::tempdir()?;
    fs::write(temp.path().join("manifest_replay.so"), &program_bytes)?;
    std::env::set_var("BPF_OUT_DIR", temp.path());

    let manifest_id = Pubkey::from_str(&fixture.manifest_program)?;
    let market_id = Pubkey::from_str(&fixture.market)?;
    let mut test = ProgramTest::new("manifest_replay", manifest_id, None);
    // ProgramTest 4 preloads runtime-matched SBF Token/Token-2022 programs.
    // Native SPL processors use a different solana-sysvar generation and
    // cannot safely share this runtime's syscall stubs.
    test.set_compute_max_units(1_400_000);
    test.set_transaction_account_lock_limit(128);
    let program_test_rent = Rent::default();
    let mut program_test_rent_top_ups = std::collections::BTreeMap::new();
    for snapshot in &fixture.accounts {
        let address = Pubkey::from_str(&snapshot.address)?;
        if address == manifest_id
            || address == spl_token::id()
            || address == spl_token_2022::id()
            || snapshot.address.starts_with("Sysvar")
        {
            continue;
        }
        let data = BASE64.decode(&snapshot.data_base64)?;
        let minimum_balance = program_test_rent.minimum_balance(data.len());
        let lamports =
            if !snapshot.executable && snapshot.lamports > 0 && snapshot.lamports < minimum_balance
            {
                program_test_rent_top_ups.insert(
                    snapshot.address.clone(),
                    minimum_balance - snapshot.lamports,
                );
                minimum_balance
            } else {
                snapshot.lamports
            };
        test.add_account(
            address,
            Account {
                lamports,
                data,
                owner: Pubkey::from_str(&snapshot.owner)?,
                executable: snapshot.executable,
                rent_epoch: snapshot.rent_epoch,
            },
        );
    }

    let mut context = test.start_with_context().await;
    let exact_token_accounts: BTreeSet<Pubkey> = fixture
        .accounts
        .iter()
        .filter(|account| matches!(account.source, crate::types::SnapshotSource::Baseline))
        .filter_map(|account| Pubkey::from_str(&account.address).ok())
        .collect();
    let mut instruction_results = Vec::with_capacity(fixture.instructions.len());

    for (ordinal, captured) in fixture.instructions.iter().enumerate() {
        context.set_sysvar(&Clock {
            slot: captured.slot,
            epoch_start_timestamp: 0,
            epoch: 0,
            leader_schedule_epoch: 0,
            unix_timestamp: 0,
        });
        let data = bs58::decode(&captured.data_base58).into_vec()?;
        let metas = captured
            .accounts
            .iter()
            .map(|meta| {
                let address = Pubkey::from_str(&meta.address)?;
                Ok(if meta.is_writable {
                    AccountMeta::new(address, meta.is_signer)
                } else {
                    AccountMeta::new_readonly(address, meta.is_signer)
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let ix = Instruction {
            program_id: manifest_id,
            accounts: metas,
            data,
        };

        let before_market = context
            .banks_client
            .get_account(market_id)
            .await?
            .ok_or_else(|| anyhow!("market missing before replay instruction {ordinal}"))?;
        let (before_summary, before_orders) = inspect_market(&before_market.data)?;
        let before_tokens = token_balances(&mut context, captured).await?;

        let uniqueness_ix = ComputeBudgetInstruction::set_compute_unit_price((ordinal + 1) as u64);
        let blockhash = context.get_new_latest_blockhash().await?;
        let mut transaction =
            Transaction::new_with_payer(&[uniqueness_ix, ix], Some(&context.payer.pubkey()));
        transaction.partial_sign(&[&context.payer], blockhash);
        let outcome = context
            .banks_client
            .process_transaction_with_metadata(transaction)
            .await?;
        let success = outcome.result.is_ok();
        let error = outcome.result.err().map(|error| format!("{error:?}"));
        let metadata = outcome.metadata;
        let transaction_compute_units = metadata.as_ref().map_or(0, |m| m.compute_units_consumed);
        let logs = metadata.map_or_else(Vec::new, |m| m.log_messages);
        let compute_units = manifest_compute_units(&logs, &fixture.manifest_program)
            .unwrap_or(transaction_compute_units);

        let after_market = context
            .banks_client
            .get_account(market_id)
            .await?
            .ok_or_else(|| anyhow!("market missing after replay instruction {ordinal}"))?;
        let (after_summary, after_orders) = inspect_market(&after_market.data)?;
        let after_tokens = token_balances(&mut context, captured).await?;
        let (base_delta, quote_delta) = if captured.name == "Swap" || captured.name == "SwapV2" {
            token_deltas(
                &before_tokens,
                &after_tokens,
                &exact_token_accounts,
                &before_summary,
            )
        } else {
            (None, None)
        };
        let new_resting_orders = after_orders
            .into_iter()
            .filter(|(sequence, _)| !before_orders.contains_key(sequence))
            .map(|(_, order)| order)
            .collect();
        instruction_results.push(InstructionResult {
            signature: captured.signature.clone(),
            slot: captured.slot,
            name: captured.name.clone(),
            success,
            compute_units,
            error,
            logs,
            base_token_delta: base_delta,
            quote_token_delta: quote_delta,
            order_sequence_delta: after_summary
                .order_sequence_number
                .saturating_sub(before_summary.order_sequence_number),
            resting_bid_delta: after_summary.resting_bids as i64
                - before_summary.resting_bids as i64,
            resting_ask_delta: after_summary.resting_asks as i64
                - before_summary.resting_asks as i64,
            new_resting_orders,
        });
    }

    let final_market = context
        .banks_client
        .get_account(market_id)
        .await?
        .ok_or_else(|| anyhow!("market missing after replay"))?;
    let final_market_sha256 = hex(&Sha256::digest(&final_market.data));
    let market_summary = summarize_market(&final_market.data)?;
    let mut writable_addresses: BTreeSet<String> = fixture
        .instructions
        .iter()
        .flat_map(|ix| ix.accounts.iter())
        .filter(|meta| meta.is_writable)
        .map(|meta| meta.address.clone())
        .collect();
    writable_addresses.insert(fixture.market.clone());
    let mut final_accounts = std::collections::BTreeMap::new();
    for address in writable_addresses {
        let pubkey = Pubkey::from_str(&address)?;
        if let Some(account) = context.banks_client.get_account(pubkey).await? {
            final_accounts.insert(
                address,
                FinalAccountState {
                    lamports: account.lamports,
                    owner: account.owner.to_string(),
                    executable: account.executable,
                    data_len: account.data.len(),
                    data_sha256: hex(&Sha256::digest(&account.data)),
                    data: account.data,
                },
            );
        }
    }
    Ok(ReplayResult {
        label: label.to_owned(),
        program_path: program_path.display().to_string(),
        program_sha256,
        instruction_results,
        final_market_data_base64: BASE64.encode(&final_market.data),
        final_market_sha256,
        market_summary,
        program_test_rent_top_ups,
        final_accounts,
    })
}

fn manifest_compute_units(logs: &[String], program: &str) -> Option<u64> {
    let prefix = format!("Program {program} consumed ");
    logs.iter().rev().find_map(|line| {
        line.strip_prefix(&prefix)?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    })
}

async fn token_balances(
    context: &mut solana_program_test::ProgramTestContext,
    captured: &crate::types::CapturedInstruction,
) -> Result<HashMap<Pubkey, (Pubkey, u64)>> {
    let mut result = HashMap::new();
    for meta in &captured.accounts {
        let address = Pubkey::from_str(&meta.address)?;
        if result.contains_key(&address) {
            continue;
        }
        let Some(account) = context.banks_client.get_account(address).await? else {
            continue;
        };
        let owner = account.owner.to_string();
        if let Some((mint, amount)) = decode_token_account(&owner, &account.data)? {
            result.insert(address, (mint, amount));
        }
    }
    Ok(result)
}

fn decode_token_account(owner: &str, data: &[u8]) -> Result<Option<(Pubkey, u64)>> {
    let is_account = if owner == TOKEN_PROGRAM {
        spl_token::state::Account::unpack(data).is_ok()
    } else if owner == TOKEN_2022_PROGRAM {
        spl_token_2022_interface::extension::StateWithExtensions::<
            spl_token_2022_interface::state::Account,
        >::unpack(data)
        .is_ok()
    } else {
        false
    };
    if !is_account {
        return Ok(None);
    }
    Ok(Some((
        Pubkey::new_from_array(data[0..32].try_into()?),
        u64::from_le_bytes(data[64..72].try_into()?),
    )))
}

fn token_deltas(
    before: &HashMap<Pubkey, (Pubkey, u64)>,
    after: &HashMap<Pubkey, (Pubkey, u64)>,
    exact: &BTreeSet<Pubkey>,
    market: &MarketSummary,
) -> (Option<i128>, Option<i128>) {
    let base = Pubkey::from_str(&market.base_mint).ok();
    let quote = Pubkey::from_str(&market.quote_mint).ok();
    let mut base_delta = 0i128;
    let mut quote_delta = 0i128;
    let mut saw_base = false;
    let mut saw_quote = false;
    for (address, (mint, before_amount)) in before {
        if exact.contains(address) {
            continue;
        }
        let Some((_, after_amount)) = after.get(address) else {
            continue;
        };
        let delta = *after_amount as i128 - *before_amount as i128;
        if Some(*mint) == base {
            base_delta += delta;
            saw_base = true;
        }
        if Some(*mint) == quote {
            quote_delta += delta;
            saw_quote = true;
        }
    }
    (
        saw_base.then_some(base_delta),
        saw_quote.then_some(quote_delta),
    )
}

pub fn summarize_market(data: &[u8]) -> Result<MarketSummary> {
    Ok(inspect_market(data)?.0)
}

fn inspect_market(
    data: &[u8],
) -> Result<(
    MarketSummary,
    std::collections::BTreeMap<u64, RestingOrderResult>,
)> {
    if data.len() < MARKET_FIXED_SIZE {
        return Err(anyhow!("market account is only {} bytes", data.len()));
    }
    let fixed: MarketFixed = bytemuck::pod_read_unaligned(&data[..MARKET_FIXED_SIZE]);
    let market = MarketValue {
        fixed,
        dynamic: data[MARKET_FIXED_SIZE..].to_vec(),
    };
    let seats: HashMap<_, _> = market
        .get_claimed_seats()
        .iter::<ClaimedSeat>()
        .map(|(index, seat)| (index, seat.trader.to_string()))
        .collect();
    let mut orders = std::collections::BTreeMap::new();
    for (side, tree) in [("bid", market.get_bids()), ("ask", market.get_asks())] {
        for (_, order) in tree.iter::<RestingOrder>() {
            let price_bytes: [u8; 16] = bytemuck::bytes_of(&order.get_price()).try_into()?;
            orders.insert(
                order.get_sequence_number(),
                RestingOrderResult {
                    sequence_number: order.get_sequence_number(),
                    trader: seats
                        .get(&order.get_trader_index())
                        .cloned()
                        .unwrap_or_else(|| "unknown".to_owned()),
                    side: side.to_owned(),
                    num_base_atoms: order.get_num_base_atoms().as_u64(),
                    price_raw: u128::from_le_bytes(price_bytes).to_string(),
                    order_type: order.get_order_type().as_u8(),
                    order_type_name: order_type_name(order.get_order_type().as_u8()).to_owned(),
                },
            );
        }
    }
    let summary = MarketSummary {
        data_len: data.len(),
        order_sequence_number: market.fixed.get_order_sequence_number(),
        quote_volume_atoms: market.fixed.get_quote_volume().as_u64(),
        resting_bids: market.get_bids().iter::<RestingOrder>().count(),
        resting_asks: market.get_asks().iter::<RestingOrder>().count(),
        claimed_seats: market.get_claimed_seats().iter::<ClaimedSeat>().count(),
        base_mint: market.fixed.get_base_mint().to_string(),
        quote_mint: market.fixed.get_quote_mint().to_string(),
        base_global: (*market.fixed.get_base_global() != Pubkey::default())
            .then(|| market.fixed.get_base_global().to_string()),
        quote_global: (*market.fixed.get_quote_global() != Pubkey::default())
            .then(|| market.fixed.get_quote_global().to_string()),
    };
    Ok((summary, orders))
}

fn order_type_name(value: u8) -> &'static str {
    match value {
        0 => "Limit",
        1 => "ImmediateOrCancel",
        2 => "PostOnly",
        3 => "Global",
        4 => "Reverse",
        5 => "ReverseTight",
        _ => "Unknown",
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0xf) as usize] as char);
    }
    output
}
