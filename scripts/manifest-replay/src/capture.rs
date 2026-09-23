use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde_json::Value;
use solana_program::program_pack::Pack;
use solana_pubkey::Pubkey;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    str::FromStr,
    time::Duration,
};

use crate::{
    rpc::{Rpc, RpcAccount, SignatureInfo, MANIFEST_PROGRAM},
    types::{
        instruction_name, AccountSnapshot, CapturedAccountMeta, CapturedInstruction, Fixture,
        SnapshotSource,
    },
};

const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const TOKEN_2022_PROGRAM: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";
const RENT_SYSVAR: &str = "SysvarRent111111111111111111111111111111111";
const GENEROUS_TOKEN_BALANCE: u64 = 1_000_000_000_000_000_000;
const GENEROUS_LAMPORT_BALANCE: u64 = 1_000_000_000_000;

#[derive(Clone, Debug)]
struct TokenHint {
    mint: String,
    owner: String,
}

#[derive(Eq, PartialEq)]
struct DeployedProgram {
    elf: Vec<u8>,
    last_deployed_slot: Option<u64>,
}

pub async fn capture(
    rpc_url: String,
    commitment: String,
    market: String,
    slots: u64,
) -> Result<(Fixture, Vec<u8>)> {
    Pubkey::from_str(&market).context("invalid market public key")?;
    let recorded_rpc_url = redact_rpc_url(&rpc_url);
    let rpc = Rpc::new(rpc_url.clone(), commitment.clone());
    // Capture before the market baseline, then reject upgrades during capture.
    // A crate version is not the version of the program deployed at its ID.
    let mut token_programs = BTreeMap::new();
    for address in [TOKEN_PROGRAM, TOKEN_2022_PROGRAM] {
        token_programs.insert(
            address.to_owned(),
            fetch_deployed_program(&rpc, address, None).await?,
        );
    }

    let discovery = rpc.account(&market, None).await?;
    let discovery_account = discovery
        .value
        .ok_or_else(|| anyhow!("market {market} does not exist"))?;
    if discovery_account.owner != MANIFEST_PROGRAM {
        bail!(
            "market {market} is owned by {}, expected {MANIFEST_PROGRAM}",
            discovery_account.owner
        );
    }
    let market_data = BASE64.decode(&discovery_account.data.0)?;
    let (base_mint, quote_mint, base_vault, quote_vault) = market_addresses(&market_data)?;
    let manifest_id = Pubkey::from_str(MANIFEST_PROGRAM)?;
    let base_mint_pk = Pubkey::from_str(&base_mint)?;
    let quote_mint_pk = Pubkey::from_str(&quote_mint)?;
    let (base_global, _) =
        Pubkey::find_program_address(&[b"global", base_mint_pk.as_ref()], &manifest_id);
    let (quote_global, _) =
        Pubkey::find_program_address(&[b"global", quote_mint_pk.as_ref()], &manifest_id);
    let (base_global_vault, _) =
        Pubkey::find_program_address(&[b"global-vault", base_mint_pk.as_ref()], &manifest_id);
    let (quote_global_vault, _) =
        Pubkey::find_program_address(&[b"global-vault", quote_mint_pk.as_ref()], &manifest_id);

    let baseline_addresses = vec![
        market.clone(),
        base_mint.clone(),
        quote_mint.clone(),
        base_vault,
        quote_vault,
        base_global.to_string(),
        quote_global.to_string(),
        base_global_vault.to_string(),
        quote_global_vault.to_string(),
        RENT_SYSVAR.to_owned(),
    ];
    let baseline_response = rpc
        .accounts(&baseline_addresses, Some(discovery.context.slot))
        .await?;
    let start_slot = baseline_response.context.slot;
    let mut snapshots = BTreeMap::<String, AccountSnapshot>::new();
    let exact_accounts: BTreeSet<String> = baseline_addresses.iter().cloned().collect();
    let mut baseline_missing_accounts = Vec::new();
    for (address, account) in baseline_addresses
        .iter()
        .zip(baseline_response.value.iter())
    {
        if let Some(account) = account {
            snapshots.insert(
                address.clone(),
                account.snapshot(address.clone(), SnapshotSource::Baseline)?,
            );
        } else {
            baseline_missing_accounts.push(address.clone());
        }
    }
    if !snapshots.contains_key(&market) {
        bail!("market disappeared while taking the baseline snapshot");
    }

    let target_slot = start_slot
        .checked_add(slots.max(1))
        .ok_or_else(|| anyhow!("slot overflow"))?;
    while rpc.slot().await? < target_slot {
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    let final_market_response = rpc.account(&market, Some(target_slot)).await?;
    let end_slot = final_market_response.context.slot;
    let chain_final_market = final_market_response
        .value
        .ok_or_else(|| anyhow!("market disappeared at end slot {end_slot}"))?
        .snapshot(market.clone(), SnapshotSource::EndSlot)?;

    let signatures = signatures_in_range(&rpc, &market, start_slot, end_slot).await?;
    let transactions_touching_market = signatures.len();
    let failed_transactions_skipped = signatures
        .iter()
        .filter(|signature| signature.err.is_some())
        .count();
    let mut successful_transactions_without_manifest = 0usize;
    let mut instructions = Vec::new();
    let mut token_hints = HashMap::<String, TokenHint>::new();
    let mut transaction_position = 0usize;
    for signature in signatures {
        if signature.err.is_some() {
            continue;
        }
        let tx = rpc
            .transaction(&signature.signature)
            .await?
            .ok_or_else(|| anyhow!("transaction {} was unavailable", signature.signature))?;
        collect_token_hints(&tx, &mut token_hints)?;
        let mut extracted =
            extract_manifest_instructions(&tx, &signature, transaction_position, &market)?;
        if extracted.is_empty() {
            successful_transactions_without_manifest += 1;
        }
        instructions.append(&mut extracted);
        transaction_position += 1;
    }

    add_abi_token_hints(&instructions, &base_mint, &quote_mint, &mut token_hints);

    let referenced_addresses: BTreeSet<String> = instructions
        .iter()
        .flat_map(|ix| ix.accounts.iter().map(|meta| meta.address.clone()))
        .collect();
    token_hints.retain(|address, _| referenced_addresses.contains(address));

    let mut requested = BTreeSet::new();
    for ix in &instructions {
        for meta in &ix.accounts {
            if !is_runtime_account(&meta.address) {
                requested.insert(meta.address.clone());
            }
        }
    }
    for hint in token_hints.values() {
        requested.insert(hint.mint.clone());
    }
    let requested: Vec<String> = requested
        .into_iter()
        .filter(|a| !exact_accounts.contains(a))
        .collect();
    let mut fetched = HashMap::<String, RpcAccount>::new();
    for chunk in requested.chunks(100) {
        let response = rpc.accounts(chunk, Some(end_slot)).await?;
        for (address, account) in chunk.iter().zip(response.value) {
            if let Some(account) = account {
                fetched.insert(address.clone(), account);
            }
        }
    }
    let mint_programs: HashMap<String, String> = token_hints
        .values()
        .filter_map(|hint| {
            fetched
                .get(&hint.mint)
                .map(|account| account.owner.clone())
                .or_else(|| {
                    snapshots
                        .get(&hint.mint)
                        .map(|account| account.owner.clone())
                })
                .map(|owner| (hint.mint.clone(), owner))
        })
        .collect();

    for address in requested {
        if snapshots.contains_key(&address) || is_runtime_account(&address) {
            continue;
        }
        if let Some(account) = fetched.get(&address) {
            let mut snapshot = account.snapshot(address.clone(), SnapshotSource::EndSlot)?;
            if !exact_accounts.contains(&address)
                && is_token_program(&snapshot.owner)
                && fund_token_snapshot(&mut snapshot)?
            {
                snapshot.source = SnapshotSource::FundedToken;
            }
            snapshots.insert(address, snapshot);
        } else if let Some(hint) = token_hints.get(&address) {
            let token_program = mint_programs
                .get(&hint.mint)
                .cloned()
                .unwrap_or_else(|| TOKEN_PROGRAM.to_owned());
            snapshots.insert(
                address.clone(),
                synthesize_token_account(address, hint, token_program)?,
            );
        } else {
            snapshots.insert(
                address.clone(),
                AccountSnapshot {
                    address,
                    lamports: GENEROUS_LAMPORT_BALANCE,
                    owner: SYSTEM_PROGRAM.to_owned(),
                    executable: false,
                    rent_epoch: 0,
                    data_base64: String::new(),
                    source: SnapshotSource::SynthesizedSystem,
                },
            );
        }
    }

    let signer_addresses: BTreeSet<&str> = instructions
        .iter()
        .flat_map(|ix| ix.accounts.iter())
        .filter(|meta| meta.is_signer)
        .map(|meta| meta.address.as_str())
        .collect();
    for (address, snapshot) in &mut snapshots {
        if signer_addresses.contains(address.as_str()) && snapshot.owner == SYSTEM_PROGRAM {
            snapshot.lamports = snapshot.lamports.max(GENEROUS_LAMPORT_BALANCE);
            if snapshot.source == SnapshotSource::EndSlot {
                snapshot.source = SnapshotSource::FundedSystem;
            }
        }
    }

    for (address, before) in &token_programs {
        let after = fetch_deployed_program(&rpc, address, Some(end_slot)).await?;
        if after != *before
            || after
                .last_deployed_slot
                .is_some_and(|slot| slot > start_slot)
        {
            bail!("token program {address} changed during capture; retry the capture");
        }
    }
    let deployed_program = fetch_deployed_program(&rpc, MANIFEST_PROGRAM, Some(end_slot)).await?;
    Ok((
        Fixture {
            version: 2,
            rpc_url: recorded_rpc_url,
            commitment,
            market,
            manifest_program: MANIFEST_PROGRAM.to_owned(),
            token_programs: token_programs
                .into_iter()
                .map(|(address, program)| (address, BASE64.encode(program.elf)))
                .collect(),
            start_slot,
            end_slot,
            transactions_touching_market,
            failed_transactions_skipped,
            successful_transactions_without_manifest,
            baseline_missing_accounts,
            accounts: snapshots.into_values().collect(),
            instructions,
            chain_final_market,
        },
        deployed_program.elf,
    ))
}

fn redact_rpc_url(url: &str) -> String {
    url.split_once('?')
        .map_or_else(|| url.to_owned(), |(base, _)| format!("{base}?<redacted>"))
}

async fn signatures_in_range(
    rpc: &Rpc,
    market: &str,
    start_slot: u64,
    end_slot: u64,
) -> Result<Vec<SignatureInfo>> {
    let mut newest_first = Vec::new();
    let mut before: Option<String> = None;
    loop {
        let page = rpc.signatures(market, before.as_deref()).await?;
        if page.is_empty() {
            break;
        }
        let reached_start = page.iter().any(|item| item.slot <= start_slot);
        before = page.last().map(|item| item.signature.clone());
        newest_first.extend(
            page.into_iter()
                .filter(|item| item.slot > start_slot && item.slot <= end_slot),
        );
        if reached_start {
            break;
        }
    }
    // getSignaturesForAddress is newest-first, but its ordering for multiple
    // transactions in one slot is not an API guarantee. Matching can depend on
    // that order, so recover the canonical transaction position from getBlock.
    let mut per_slot = BTreeMap::<u64, Vec<String>>::new();
    for signature in &newest_first {
        per_slot
            .entry(signature.slot)
            .or_default()
            .push(signature.signature.clone());
    }
    let mut block_positions = HashMap::<String, usize>::new();
    for (slot, slot_signatures) in per_slot {
        if slot_signatures.len() == 1 {
            block_positions.insert(slot_signatures[0].clone(), 0);
            continue;
        }
        let ordered = rpc.block_signatures(slot).await?;
        let positions: HashMap<&str, usize> = ordered
            .iter()
            .enumerate()
            .map(|(position, signature)| (signature.as_str(), position))
            .collect();
        for signature in slot_signatures {
            let position = positions.get(signature.as_str()).copied().ok_or_else(|| {
                anyhow!("transaction {signature} was absent from finalized block {slot}")
            })?;
            block_positions.insert(signature, position);
        }
    }
    sort_signatures_in_ledger_order(&mut newest_first, &block_positions);
    Ok(newest_first)
}

fn sort_signatures_in_ledger_order(
    signatures: &mut [SignatureInfo],
    block_positions: &HashMap<String, usize>,
) {
    signatures.sort_by_key(|signature| {
        (
            signature.slot,
            block_positions
                .get(&signature.signature)
                .copied()
                .unwrap_or_default(),
        )
    });
}

fn extract_manifest_instructions(
    tx: &Value,
    signature: &SignatureInfo,
    transaction_position: usize,
    market: &str,
) -> Result<Vec<CapturedInstruction>> {
    let message = &tx["transaction"]["message"];
    let mut keys: Vec<String> = message["accountKeys"]
        .as_array()
        .ok_or_else(|| anyhow!("missing accountKeys"))?
        .iter()
        .map(|v| v.as_str().unwrap_or_default().to_owned())
        .collect();
    for group in ["writable", "readonly"] {
        if let Some(loaded) = tx["meta"]["loadedAddresses"][group].as_array() {
            keys.extend(
                loaded
                    .iter()
                    .map(|v| v.as_str().unwrap_or_default().to_owned()),
            );
        }
    }
    let header = &message["header"];
    let required = header["numRequiredSignatures"].as_u64().unwrap_or(0) as usize;
    let ro_signed = header["numReadonlySignedAccounts"].as_u64().unwrap_or(0) as usize;
    let ro_unsigned = header["numReadonlyUnsignedAccounts"].as_u64().unwrap_or(0) as usize;
    let static_len = message["accountKeys"].as_array().map_or(0, Vec::len);
    let loaded_writable = tx["meta"]["loadedAddresses"]["writable"]
        .as_array()
        .map_or(0, Vec::len);
    let privilege = |index: usize| -> (bool, bool) {
        if index < static_len {
            let signer = index < required;
            let writable = if signer {
                index < required.saturating_sub(ro_signed)
            } else {
                index < static_len.saturating_sub(ro_unsigned)
            };
            (signer, writable)
        } else {
            (false, index < static_len + loaded_writable)
        }
    };
    let mut inner_by_outer: HashMap<usize, &Vec<Value>> = HashMap::new();
    if let Some(groups) = tx["meta"]["innerInstructions"].as_array() {
        for group in groups {
            if let (Some(index), Some(ixs)) =
                (group["index"].as_u64(), group["instructions"].as_array())
            {
                inner_by_outer.insert(index as usize, ixs);
            }
        }
    }
    let outers = message["instructions"]
        .as_array()
        .ok_or_else(|| anyhow!("missing instructions"))?;
    let mut result = Vec::new();
    for (outer_index, outer) in outers.iter().enumerate() {
        if compiled_program(outer, &keys) == Some(MANIFEST_PROGRAM) {
            let captured = capture_ix(
                outer,
                &keys,
                &privilege,
                signature,
                transaction_position,
                outer_index,
                None,
            )?;
            if captured.accounts.iter().any(|meta| meta.address == market) {
                result.push(captured);
            }
        }
        if let Some(inner) = inner_by_outer.get(&outer_index) {
            for (inner_index, ix) in inner.iter().enumerate() {
                if compiled_program(ix, &keys) == Some(MANIFEST_PROGRAM) {
                    let captured = capture_ix(
                        ix,
                        &keys,
                        &privilege,
                        signature,
                        transaction_position,
                        outer_index,
                        Some(inner_index),
                    )?;
                    if captured.accounts.iter().any(|meta| meta.address == market) {
                        result.push(captured);
                    }
                }
            }
        }
    }
    Ok(result)
}

fn compiled_program<'a>(ix: &Value, keys: &'a [String]) -> Option<&'a str> {
    ix["programIdIndex"]
        .as_u64()
        .and_then(|i| keys.get(i as usize))
        .map(String::as_str)
}

fn capture_ix<F: Fn(usize) -> (bool, bool)>(
    ix: &Value,
    keys: &[String],
    privilege: &F,
    signature: &SignatureInfo,
    transaction_position: usize,
    outer_index: usize,
    inner_index: Option<usize>,
) -> Result<CapturedInstruction> {
    let data_base58 = ix["data"]
        .as_str()
        .ok_or_else(|| anyhow!("compiled instruction has no data"))?
        .to_owned();
    let data = bs58::decode(&data_base58).into_vec()?;
    let account_indices = ix["accounts"]
        .as_array()
        .ok_or_else(|| anyhow!("compiled instruction has no accounts"))?;
    let discriminator = data.first().copied();
    // Both swap discriminators are accepted by the shared loader. Routers can
    // therefore use the separate-owner layout with the legacy discriminator.
    // Inner-instruction records omit invoke_signed signer privileges, so infer
    // that layout from the system program occupying account position three.
    let has_separate_swap_owner = matches!(discriminator, Some(4 | 13))
        && account_indices
            .get(3)
            .and_then(Value::as_u64)
            .and_then(|index| keys.get(index as usize))
            .is_some_and(|address| address == SYSTEM_PROGRAM);
    let accounts = account_indices
        .iter()
        .enumerate()
        .map(|(position, value)| {
            let index = value
                .as_u64()
                .ok_or_else(|| anyhow!("invalid account index"))? as usize;
            let (transaction_signer, is_writable) = privilege(index);
            // Inner-instruction records omit invoke_signed privileges. Restore
            // signer requirements from Manifest's instruction ABI.
            let abi_signer = position == 0 || (has_separate_swap_owner && position == 1);
            Ok(CapturedAccountMeta {
                address: keys
                    .get(index)
                    .ok_or_else(|| anyhow!("account index out of range"))?
                    .clone(),
                is_signer: transaction_signer || abi_signer,
                is_writable,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(CapturedInstruction {
        signature: signature.signature.clone(),
        slot: signature.slot,
        transaction_position,
        outer_instruction_index: outer_index,
        inner_instruction_index: inner_index,
        name: instruction_name(&data),
        data_base58,
        accounts,
    })
}

fn collect_token_hints(tx: &Value, hints: &mut HashMap<String, TokenHint>) -> Result<()> {
    let message = &tx["transaction"]["message"];
    let mut keys: Vec<String> = message["accountKeys"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    for group in ["writable", "readonly"] {
        if let Some(loaded) = tx["meta"]["loadedAddresses"][group].as_array() {
            keys.extend(loaded.iter().filter_map(Value::as_str).map(str::to_owned));
        }
    }
    for field in ["preTokenBalances", "postTokenBalances"] {
        if let Some(balances) = tx["meta"][field].as_array() {
            for balance in balances {
                let Some(index) = balance["accountIndex"].as_u64() else {
                    continue;
                };
                let Some(address) = keys.get(index as usize) else {
                    continue;
                };
                let Some(mint) = balance["mint"].as_str() else {
                    continue;
                };
                let owner = balance["owner"].as_str().unwrap_or(SYSTEM_PROGRAM);
                hints.entry(address.clone()).or_insert(TokenHint {
                    mint: mint.to_owned(),
                    owner: owner.to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn add_abi_token_hints(
    instructions: &[CapturedInstruction],
    base_mint: &str,
    quote_mint: &str,
    hints: &mut HashMap<String, TokenHint>,
) {
    for ix in instructions {
        let token_accounts = match ix.name.as_str() {
            "Swap" => Some((0, [(3, base_mint), (4, quote_mint)])),
            "SwapV2" => Some((1, [(4, base_mint), (5, quote_mint)])),
            _ => None,
        };
        if let Some((owner_index, token_accounts)) = token_accounts {
            let Some(owner) = ix
                .accounts
                .get(owner_index)
                .map(|meta| meta.address.clone())
            else {
                continue;
            };
            for (account_index, mint) in token_accounts {
                if let Some(account) = ix.accounts.get(account_index) {
                    hints.entry(account.address.clone()).or_insert(TokenHint {
                        mint: mint.to_owned(),
                        owner: owner.clone(),
                    });
                }
            }
            continue;
        }
        if matches!(ix.name.as_str(), "Deposit" | "Withdraw") {
            let (Some(owner), Some(token), Some(mint)) =
                (ix.accounts.first(), ix.accounts.get(2), ix.accounts.get(5))
            else {
                continue;
            };
            hints.entry(token.address.clone()).or_insert(TokenHint {
                mint: mint.address.clone(),
                owner: owner.address.clone(),
            });
        }
    }
}

fn market_addresses(data: &[u8]) -> Result<(String, String, String, String)> {
    if data.len() < 144 {
        bail!("market data is only {} bytes", data.len());
    }
    let key = |range: std::ops::Range<usize>| -> String {
        Pubkey::new_from_array(data[range].try_into().expect("32-byte market field")).to_string()
    };
    Ok((key(16..48), key(48..80), key(80..112), key(112..144)))
}

fn fund_token_snapshot(snapshot: &mut AccountSnapshot) -> Result<bool> {
    let mut data = BASE64.decode(&snapshot.data_base64)?;
    let is_account = if snapshot.owner == TOKEN_PROGRAM {
        spl_token::state::Account::unpack(&data).is_ok()
    } else if snapshot.owner == TOKEN_2022_PROGRAM {
        spl_token_2022::extension::StateWithExtensions::<spl_token_2022::state::Account>::unpack(
            &data,
        )
        .is_ok()
    } else {
        false
    };
    if is_account {
        data[64..72].copy_from_slice(&GENEROUS_TOKEN_BALANCE.to_le_bytes());
        snapshot.lamports = snapshot.lamports.max(GENEROUS_LAMPORT_BALANCE);
        // Native wrapped-SOL accounts move lamports alongside token amounts.
        if u32::from_le_bytes(data[109..113].try_into()?) == 1 {
            let reserve = u64::from_le_bytes(data[113..121].try_into()?);
            snapshot.lamports = GENEROUS_TOKEN_BALANCE.saturating_add(reserve);
        }
        snapshot.data_base64 = BASE64.encode(data);
        return Ok(true);
    }
    Ok(false)
}

fn synthesize_token_account(
    address: String,
    hint: &TokenHint,
    token_program: String,
) -> Result<AccountSnapshot> {
    let mut data = vec![0u8; spl_token::state::Account::LEN];
    data[0..32].copy_from_slice(Pubkey::from_str(&hint.mint)?.as_ref());
    data[32..64].copy_from_slice(Pubkey::from_str(&hint.owner)?.as_ref());
    data[64..72].copy_from_slice(&GENEROUS_TOKEN_BALANCE.to_le_bytes());
    data[108] = 1;
    Ok(AccountSnapshot {
        address,
        lamports: GENEROUS_LAMPORT_BALANCE,
        owner: token_program,
        executable: false,
        rent_epoch: 0,
        data_base64: BASE64.encode(data),
        source: SnapshotSource::SynthesizedToken,
    })
}

fn is_token_program(owner: &str) -> bool {
    owner == TOKEN_PROGRAM || owner == TOKEN_2022_PROGRAM
}
fn is_runtime_account(address: &str) -> bool {
    matches!(
        address,
        MANIFEST_PROGRAM | TOKEN_PROGRAM | TOKEN_2022_PROGRAM | SYSTEM_PROGRAM
    ) || address.starts_with("Sysvar")
}

async fn fetch_deployed_program(
    rpc: &Rpc,
    address: &str,
    min_slot: Option<u64>,
) -> Result<DeployedProgram> {
    let program = rpc
        .account(address, min_slot)
        .await?
        .value
        .ok_or_else(|| anyhow!("deployed program {address} is missing"))?;
    if !program.executable {
        bail!("program {address} is not executable");
    }
    let data = BASE64.decode(&program.data.0)?;
    // UpgradeableLoaderState::Program is bincode enum tag 2 followed by the ProgramData pubkey.
    let (elf, last_deployed_slot) =
        if program.owner == solana_sdk_ids::bpf_loader_upgradeable::id().to_string() {
            if data.len() != 36 || u32::from_le_bytes(data[0..4].try_into()?) != 2 {
                bail!("invalid upgradeable program account {address}");
            }
            let programdata = Pubkey::new_from_array(data[4..36].try_into()?).to_string();
            let account = rpc
                .account(&programdata, min_slot)
                .await?
                .value
                .ok_or_else(|| anyhow!("program-data account {programdata} is missing"))?;
            let bytes = BASE64.decode(&account.data.0)?;
            if account.owner != program.owner
                || bytes.len() <= 45
                || u32::from_le_bytes(bytes[0..4].try_into()?) != 3
            {
                bail!("invalid program-data account {programdata}");
            }
            (
                bytes[45..].to_vec(),
                Some(u64::from_le_bytes(bytes[4..12].try_into()?)),
            )
        } else if program.owner == solana_sdk_ids::bpf_loader::id().to_string()
            || program.owner == solana_sdk_ids::bpf_loader_deprecated::id().to_string()
        {
            (data, None)
        } else {
            bail!("unsupported loader {} for program {address}", program.owner);
        };
    if !elf.starts_with(b"\x7fELF") {
        bail!("deployed program {address} does not contain an ELF");
    }
    Ok(DeployedProgram {
        elf,
        last_deployed_slot,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_direct_and_inner_manifest_in_ledger_order() {
        let data = bs58::encode([4u8]).into_string();
        let tx = json!({
            "transaction": {"message": {
                "accountKeys": [
                    "11111111111111111111111111111111",
                    "SysvarC1ock11111111111111111111111111111111",
                    MANIFEST_PROGRAM,
                    "ComputeBudget111111111111111111111111111111"
                ],
                "header": {
                    "numRequiredSignatures": 1,
                    "numReadonlySignedAccounts": 0,
                    "numReadonlyUnsignedAccounts": 2
                },
                "instructions": [
                    {"programIdIndex": 2, "accounts": [0, 1], "data": data},
                    {"programIdIndex": 3, "accounts": [], "data": ""},
                    {"programIdIndex": 2, "accounts": [1], "data": data}
                ]
            }},
            "meta": {
                "loadedAddresses": {"writable": [], "readonly": []},
                "innerInstructions": [{
                    "index": 1,
                    "instructions": [{"programIdIndex": 2, "accounts": [1, 0], "data": data}]
                }]
            }
        });
        let signature = SignatureInfo {
            signature: "sig".to_owned(),
            slot: 42,
            err: None,
        };
        let result =
            extract_manifest_instructions(&tx, &signature, 0, "11111111111111111111111111111111")
                .unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "Swap");
        assert_eq!(result[0].inner_instruction_index, None);
        assert!(result[0].accounts[0].is_signer);
        assert!(result[0].accounts[1].is_writable);
        assert_eq!(result[1].inner_instruction_index, Some(0));
    }

    #[test]
    fn restores_separate_owner_signer_for_legacy_swap_discriminator() {
        let data = bs58::encode([4u8]).into_string();
        let market = "SysvarC1ock11111111111111111111111111111111";
        let tx = json!({
            "transaction": {"message": {
                "accountKeys": [
                    "payer",
                    "owner",
                    market,
                    SYSTEM_PROGRAM,
                    MANIFEST_PROGRAM,
                    "outer-program"
                ],
                "header": {
                    "numRequiredSignatures": 1,
                    "numReadonlySignedAccounts": 0,
                    "numReadonlyUnsignedAccounts": 2
                },
                "instructions": [
                    {"programIdIndex": 5, "accounts": [], "data": ""}
                ]
            }},
            "meta": {
                "loadedAddresses": {"writable": [], "readonly": []},
                "innerInstructions": [{
                    "index": 0,
                    "instructions": [{
                        "programIdIndex": 4,
                        "accounts": [0, 1, 2, 3],
                        "data": data
                    }]
                }]
            }
        });
        let signature = SignatureInfo {
            signature: "sig".to_owned(),
            slot: 42,
            err: None,
        };
        let result = extract_manifest_instructions(&tx, &signature, 0, market).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "Swap");
        assert!(result[0].accounts[1].is_signer);
    }

    #[test]
    fn orders_same_slot_transactions_by_block_position() {
        let mut signatures = vec![
            SignatureInfo {
                signature: "later-slot".to_owned(),
                slot: 43,
                err: None,
            },
            SignatureInfo {
                signature: "second".to_owned(),
                slot: 42,
                err: None,
            },
            SignatureInfo {
                signature: "first".to_owned(),
                slot: 42,
                err: None,
            },
        ];
        let positions = HashMap::from([
            ("later-slot".to_owned(), 1),
            ("second".to_owned(), 19),
            ("first".to_owned(), 3),
        ]);
        sort_signatures_in_ledger_order(&mut signatures, &positions);
        assert_eq!(
            signatures
                .iter()
                .map(|value| value.signature.as_str())
                .collect::<Vec<_>>(),
            ["first", "second", "later-slot"]
        );
    }
}
