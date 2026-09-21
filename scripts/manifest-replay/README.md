# Manifest mainnet upgrade replay

This tool captures the state and successful Manifest instructions for a market over a finalized mainnet slot range, then replays the extracted Manifest invocations against the currently deployed program and a candidate program build.

For an end-to-end check, run:

```sh
./scripts/verify-mainnet-upgrade.sh run \
  --market <MARKET_PUBKEY> \
  --rpc-url "$SOLANA_RPC_URL" \
  --slots 5
```

When `--new-program` is omitted, the wrapper builds the current checkout with the repository's pinned verifiable-build recipe: `solana-verify 0.5.1`, sBPF v3, platform-tools v1.57, and the Docker image digest used by CI. If the pinned `solana-verify` is unavailable, the wrapper installs it under `target/manifest-replay-tools/`. It verifies the ELF version and uses `target/deploy/manifest.so` as the candidate. Pass `--new-program <ELF>` to use an already-built candidate and skip that build; this also allows controlled comparisons with historical v2 artifacts.

The replay runs on Agave 4.2.2 ProgramTest (host Rust 1.93), including its runtime-matched SBF Token and Token-2022 programs. It does not register native SPL processors from a different SDK generation. Re-run both old and new artifacts on this runtime when comparing CU; historical results from an older runtime are not directly interchangeable.

`run` writes a reusable `fixture.json`, the downloaded deployed program, the captured chain-final market, both replay-final markets, every final writable account under `old-final-accounts/` and `new-final-accounts/`, and `report.json`. The terminal summary shows instruction counts, success and failure results, CU totals and min/average/max by instruction type, the old-to-new CU delta and percentage by instruction type, per-swap net trader token deltas (positive means received by the trader accounts), new resting-order details, order and resting-book changes, decoded final market summaries including cached global keys, writable-account differences, and byte ranges that differ.

For repeated candidate builds, capture once and replay without RPC access:

```sh
./scripts/verify-mainnet-upgrade.sh capture \
  --market <MARKET_PUBKEY> \
  --rpc-url "$SOLANA_RPC_URL" \
  --slots 20 \
  --output replay-fixture

./scripts/verify-mainnet-upgrade.sh replay \
  --fixture replay-fixture/fixture.json
```

Aggregate any directory containing multiple replay reports with:

```sh
./scripts/summarize-mainnet-upgrade-replays.py replay-runs/my-batch \
  --json replay-runs/my-batch/aggregate.json \
  --markdown replay-runs/my-batch/aggregate.md
```

Select distinct high-activity mainnet markets before a repeated run with:

```sh
./scripts/select-mainnet-manifest-markets.py \
  --rpc-url "$SOLANA_RPC_URL" \
  --count 10 \
  --output replay-runs/market-selection.json
```

The selector discovers Manifest market accounts, forms a bounded candidate pool from the largest books and lifetime quote-volume counters, ranks that pool by successful transactions per slot in recent signatures, and chooses distinct unordered mint pairs by default. The JSON output records the selection slot and every ranking input so a batch can be audited later. Use `--allow-duplicate-pairs` when separate market accounts for the same pair are desired.

Run a resumable multi-market campaign from that selection with:

```sh
SOLANA_RPC_URL="$SOLANA_RPC_URL" \
./scripts/run-mainnet-upgrade-replay-batch.py \
  --selection replay-runs/market-selection.json \
  --new-program target/deploy/manifest.so \
  --output replay-runs/candidate-1000 \
  --runs-per-market 50 \
  --slots 10 \
  --concurrency 8
```

Each market is captured sequentially so its windows do not overlap; independent markets run concurrently. Existing reports are reused when the command is resumed. `batch-progress.json` records failed runs, outcome mismatches, and any writable-account or market difference outside cache bytes `192..256`.

The aggregate separates idle and active windows, shows the market and mint-pair distribution, reports per-market net token and order outcomes, reports CU totals and deltas by instruction type, checks deployed replay results against captured chain state, classifies cache-only and unexpected final-state differences, compares per-instruction token and order outcomes, and reports coverage against both the full instruction enum and the subset that can touch a market.

The baseline snapshot contains the market, its base and quote mints, market vaults, canonical globals, global vaults, and the mainnet Rent sysvar. Solana 2.2 ProgramTest fixes its bank to the SDK's default rent schedule and cannot install mainnet's current schedule. If that default requires more lamports than an exact captured account has, the replay tops up the in-memory copy to ProgramTest's minimum and records the adjustment in `programTestRentTopUps`; fixture values remain exact. This prevents ProgramTest's post-transaction rent check from rejecting an expansion that is rent-exempt on mainnet. Every account passed to an extracted direct or inner Manifest invocation is included in the fixture. Non-vault user token accounts preserve their mint, authority, token program, native-token marker, and extensions, while their token amount and rent funding are raised to generous balances so sampled swaps and deposits do not fail because of wallet funding. Missing temporary token accounts are synthesized from transaction token-balance metadata or the Manifest instruction ABI. External system signers are also funded for rent expansion. Market and global state and all market/global vault balances remain exact in the fixture.

Only Manifest invocations that include the selected market are replayed. Setup and cleanup instructions owned by wrappers, aggregators, the system program, and token programs are excluded, as are Manifest instructions for other markets in the same routed transaction. Writable privileges for an inner invocation come from the enclosing transaction, while signer requirements missing from RPC inner-instruction records are restored from the Manifest ABI. Swap account layout is inspected separately because Manifest accepts the separate-owner layout with either swap discriminator. The captured currently deployed program provides the control run; `old replay vs captured chain` in the report makes any environmental mismatch visible.

Transactions are ordered by their canonical position in each finalized block. `getSignaturesForAddress` provides the transaction set but does not guarantee ordering within one slot, so capture asks `getBlock` for signature positions whenever a slot contains more than one selected transaction.

Globals are snapshotted exactly at the baseline. A global can also be changed during the window by a transaction for another market that does not include the selected market account; such a transaction is deliberately outside this market-filtered replay. If that shared-state change affects a later selected-market fill, `old replay vs captured chain` will show the mismatch. The old/new comparison remains controlled because both program versions receive the same baseline globals and extracted instructions. Use shorter windows when reproducing chain state itself is important, and treat a nonzero deployed/chain mismatch as a dependency warning rather than a candidate-program difference.
