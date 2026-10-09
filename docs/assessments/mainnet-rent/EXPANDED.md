# Expanded Manifest SOL recovery assessment

Implementation correction: the historical scenarios below include classic-token
vault excess that deployed Tokenkeg cannot withdraw while a vault remains open.
They are scenario upper bounds, not directly executable recovery estimates.
The current market-only implementation estimate is **42.182076620 SOL**; see
[implemented recovery and exclusions](../../market-defrag.md#snapshot-estimate-for-this-implementation).

**45.734913797 SOL is the estimated gross recovery while keeping the markets and vaults open and retaining one spare node per market.** Keeping two spare nodes gives **44.920488197 SOL**. Retiring eligible empty markets and globals with their vaults increases these figures; the one-spare case becomes **53.606932573 SOL**. These totals include rent reductions and collectable surplus, including protocol fees; they are not all newly released storage rent.

| Component | SOL, one spare market node |
| --- | ---: |
| Market compaction, empty-seat removal, and existing excess | 27.293815057 |
| Global compaction and surplus, after gas and seat-fee buffers | 8.629304332 |
| Non-native vault excess lamports; keep vaults open | 9.811794408 |
| **Total** | **45.734913797** |

The fresh scan covers **3,721 markets, 431 globals, and 7,873 distinct vaults** on September 29, 2026. Markets were read at finalized slot **451678319**, globals at **451678322**, and vault batches at **451678334–451678356**. The core slot's block time is **15:11:11 UTC**. This is a short multi-slot census, not an atomic cross-account snapshot. Counts and eligibility must be revalidated for execution. Four markets were created between this scan and the original market-only scan, and other market state changed.

One spare 80-byte node is the current market invariant: seat claims and order placements consume a node before restoring free capacity. Two spare nodes cover a temporary swap seat plus an extra reverse-order node without immediate expansion. Eight is not required. Fully trimming to zero spare nodes raises the storage-bound estimate to **47.247128197 SOL**, but needs program changes to entry paths and free-node assumptions. All supported trading paths would need validation before shipping any compactor.

Globals hold **733 seats**; **339** have zero deposits and no global order referencing that trader/mint anywhere in the market census. A global seat uses two 64-byte nodes, so removing these seats releases **0.220431360 SOL**. Existing global free lists contain **zero nodes**. A 96-byte header and all noneligible seats are retained. Global insertion currently expands before adding a trader, and the allocator expects an empty free list when expanding; a reclamation implementation needs to keep that behavior consistent with any retained free blocks.

After reserving rent at the current size and every outstanding global order's cleanup prepayment, globals have **9.561253412 SOL** of surplus. That includes admission/eviction fees, stranded gas prepayments, old rent excess, and possibly other transfers. It cannot all be labeled abandoned gas. For the main estimate, each global additionally retains as much available surplus as possible up to `remaining seats × 2 × rent(165)`, reflecting the current seat-fee policy. This retains **1.152380440 SOL** of available surplus against a desired buffer of **1.172890720 SOL**; some accounts do not have enough surplus to fill the whole buffer. No money is withdrawn from those accounts' insufficient surplus. Together with resizing, the result is **8.629304332 SOL** from globals. The buffer is an explicit conservative policy assumption, not a separately tracked refund liability. If governance determines no such buffer must remain, globals' upper amount is **9.781684772 SOL**, increasing the one-spare total to **46.887294237 SOL**.

Global gas prepayments were counted by walking both order trees of every market, retaining expired and underfunded orders in the liability count. Global bids map to the quote-mint global; global asks map to the base-mint global. Every global order reserves 5,000 lamports.

| Gas-prepayment finding | Result |
| --- | ---: |
| Resting global orders | 8,246 |
| Markets containing those orders | 1,895 |
| Globals with outstanding orders | 267 |
| Globals with no outstanding orders | 164 |
| **Required reserve for every remaining order** | **0.041230000 SOL** |
| Expired global orders | 0 |
| Individually underfunded global orders | 13 |
| Existing `GlobalClean` refund potential | 0.000065000 SOL |

The 13 candidates follow the current `GlobalClean` balance test: compare the trader's current global balance against the full order's required funding, using base atoms for asks and rounded-up quote atoms for bids. This is not an aggregate funding guarantee across a trader's orders. The candidate count is provisional because market/global reads differ by three slots. The full **0.04123 SOL remains reserved** in the main totals; the cleanup refund is not added to them. Cleanup transaction costs can consume that very small reward.

**A new instruction can reclaim abandoned gas prepayments as part of global surplus.** The current `remove_from_global` explicitly permits order removal without a refund-capable global account bundle, leaving its prepayment in the global. Once the order is gone, that prepayment no longer needs to be reserved for a future cleanup. The state layout does not separately record the historical sources of the global's lamports, however. The precise historical abandoned-gas total cannot be derived from this account snapshot; it would require archival transaction accounting. In particular, **9.561253412 SOL is a mixed surplus figure, not a measurement of abandoned gas alone**. The 164 globals with no current global orders contain **0.706791862 SOL above rent**, before applying any seat-fee buffer.

A collector must retain `rent(new size) + 5,000 × outstanding global orders + any chosen policy buffer`. The outstanding count must be authoritative when collection executes. A permissionless instruction cannot trust a caller-supplied market subset or this off-chain census. New support should establish a complete, consistent census during an authorized migration and maintain liability accounting thereafter, including removals that currently omit global accounts; alternatively, use a controlled maintenance process that prevents the census from becoming stale. The global layout has no existing durable outstanding-order counter. These are implementation requirements, not an implemented or simulated instruction. The decision about the recipient of forfeited prepayments and accumulated fees remains a protocol policy decision.

Token vaults do **not** have market-style free lists. The scan found sizes of 165, 170, 171, 175, 178, 183, and 187 bytes, with **no unused trailing allocation**. Larger accounts hold Token-2022 extensions (transfer fees, transfer hooks, and pausable-account state). This assessment credits **zero SOL to shrinking vault data**. Token-2022's current `Reallocate` preserves existing extensions and grows space; it does not provide arbitrary shrinkage. [Token-2022 reallocation source](https://github.com/solana-program/token-2022/blob/main/program/src/extension/reallocate.rs).

There are **5,772 classic-token vaults and 2,101 Token-2022 vaults**, including **1,204 wrapped-SOL vaults**. Non-native vaults hold **9.811794408 SOL above their present rent requirements**, split into **2.146128869 SOL** for classic token and **7.665665539 SOL** for Token-2022. Only the Token-2022 portion can be collected through an excess-lamport instruction using the Manifest vault PDA authority while keeping vaults open. Deployed classic Tokenkeg does not implement that instruction; its excess remains until eligible account closure. The scan verified token owners, mints, initialized states, self-owned vault authorities, and absence of separate close authorities. No CPI Guard extension was present. See the [classic-token processor](https://github.com/solana-program/token/blob/main/program/src/processor.rs) and [Token-2022 processor](https://github.com/solana-program/token-2022/blob/main/program/src/processor.rs).

**6.902499793 SOL** of vault excess is concentrated in CASH vault `2foKAwvuvKfHjuVK6NjFG94cCTPCYWxbKRDjva9CtiJj`, mint `CASHx9KJUStyftLFWGvEVf59SGeG9sh5FfcnZMVPCASH`. It holds 6,904,018,713 lamports against a 1,518,920-lamport rent minimum. Its token amount is a separate 255,603,461,158 atoms. The excess is directly measured SOL, not a valuation of those tokens; the historical source of that SOL was not investigated.

Wrapped SOL is handled separately: lamports backing its token amount are customer assets, not recoverable rent. Native accounts are rejected by `WithdrawExcessLamports`. Their recorded native rent reserves exceed current rent by **0.613635174 SOL** in aggregate, but that amount is excluded from the keep-open totals; recovering it would require separately validated vault reinitialization/migration behavior while preserving all token backing. There are also **0.041337177 SOL** of unsynchronized native-account lamports, which are excluded from keep-open recovery. Neither amount is simply added to the estimate.

If the protocol is willing to **retire empty accounts**, the census identifies **1,324 markets, 151 globals, and their 2,799 vaults** as candidates. A market qualifies only when no seats/orders remain after eligible seat eviction and both vaults have zero token amounts and no withheld transfer fees. A global qualifies only with no retained seats, no global orders anywhere, and an empty closable vault. Required Token-2022 extension state is considered, and no unknown/confidential extension is accepted as closable. These are snapshot conditions, not proof that a market is permanently inactive.

Closing these vaults adds **4.579365976 SOL** beyond the already-counted non-native excess. Closing the associated market and global headers adds **2.754579200 SOL**. In the one-spare scenario, closing those markets also releases the spare nodes otherwise retained. This yields **53.606932573 SOL total**. The zero-spare storage-bound scenario plus retirement reaches **54.581073373 SOL**. Vault closure amounts and existing excess are not double-counted, and no wrapped-SOL token principal is included. Retired markets cannot continue trading without a suitable recreation or migration path. [Token-account closure rules](https://solana.com/docs/tokens/basics/close-account).

All results are before execution fees, engineering, deployment, and review costs. A core-program collector/compactor and authority-signed vault CPIs are not currently implemented here. The collector must recheck balances and obligations at execution. Deployed bytecode was not matched to this checkout, and no transactions were submitted. My assessment is that the **surplus collection opportunities deserve priority**: the 6.90 SOL CASH-vault excess and global surplus are substantial relative to the 0.22 SOL released by shrinking globals, and they avoid moving market nodes.

Validation: existing SDK decoders independently matched **all 3,721 market counts and eligible seat counts, all 431 global seat counts, all 7,873 vault token balances/mints/authorities/native flags, and all 8,246 global orders**. Eight synthetic tests cover market and global eligibility, malformed references, dust balances, native SOL protection, and withheld token fees. Offline replay regenerates the results from the preserved snapshot. Current rent was read from the Rent sysvar and cross-checked against RPC minima at five sizes.

Artifacts: [expanded-summary.json](expanded-summary.json), [expanded-markets.json](expanded-markets.json), [globals.json](globals.json), [vaults.json](vaults.json), and [gas-prepayments.json](gas-prepayments.json). Each global has an individual order count, gas reserve, surplus, and seat-reclamation estimate. Gas rows include the owning market, mint, trader, and snapshot cleanup eligibility. Snapshot indices are evidence, not durable transaction inputs.

The commands below require the locally retained assessment scripts; those scripts
are not included in this branch.

```bash
# New live read-only census; uses RPC_URL from environment/.env.
node scripts/assess-global-vault-rent.mjs

# Replay this census from the local raw snapshot.
node scripts/assess-global-vault-rent.mjs \
  .git/assessment-snapshots/all-rent-451678322.json.gz

node --test scripts/assess-market-rent.test.mjs scripts/assess-global-vault-rent.test.mjs
```

Raw snapshot SHA-256 of decompressed canonical JSON: `9d7fc56c3ed7367767bd8219334723c896886f04076496fc2f45af9ff571bcac`. The compressed snapshot is preserved locally under `.git/assessment-snapshots/` and is not tracked.
