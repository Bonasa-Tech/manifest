# Manifest mainnet rent reclamation assessment

**Updated scope:** see [the expanded assessment](EXPANDED.md) for markets, globals, vaults, and global gas prepayments. It finds **45.73 SOL** with one spare node per market, or **53.61 SOL** if eligible empty markets/globals and their vaults are retired. The original market-only snapshot follows below.

**Spare-node correction:** eight was only a sensitivity scenario. The current code maintains at least one spare node; two avoid expansion for a temporary swap seat plus a reverse-order node. Zero is a storage lower bound requiring changed program assumptions, not a compatible cleanup target. Keeping one in the original snapshot releases **27.285959697 SOL including excess**; two release **26.471127697 SOL**.

**28.796548497 SOL is the gross potential under the full scenario: compact market storage, remove every seat with exactly zero base and quote balances and no resting orders, trim all remaining free nodes, and collect the resulting excess lamports.** Of this, **17.975478400 SOL** is newly released by shrinking and **10.821070097 SOL** is already above the current rent minimum. This is an assessment, not an executed reclamation or an immediately available withdrawal instruction.

The finalized snapshot covers all **3,717 Manifest markets** returned by a discriminator-filtered `getProgramAccounts` call at slot **451,669,723**, September 29, 2026, **14:32:48 UTC**. The RPC genesis hash was verified as mainnet. Program: `MNFSTqtC93rEfYHB6hF82sKdZpUDFWkViLByLd1k1Ms`. Repository baseline: `212b453c`; analysis branch: `analysis/mainnet-rent-reclamation`.

| Scenario | Bytes removed | SOL released by resizing | SOL including existing excess |
| --- | ---: | ---: | ---: |
| Collect existing excess only | 0 | 0 | 10.821070097 |
| Trim existing trailing free nodes; keep live indices | 537,600 | 2.731008000 | 13.552078097 |
| Compact and remove existing free nodes; keep every seat | 1,264,720 | 6.424777600 | 17.245847697 |
| Compact, remove empty seats, trim all free nodes | 3,538,480 | **17.975478400** | **28.796548497** |
| Same, retaining up to 2 spare nodes per market | 3,080,720 | 15.650057600 | 26.471127697 |
| Same, retaining up to 8 spare nodes per market | 2,608,480 | 13.251078400 | 24.072148497 |
| Same, retaining up to 32 spare nodes per market | 2,019,120 | 10.257129600 | 21.078199697 |

The rows are alternative scenarios, not additive. Spare-node scenarios never enlarge an account: they retain up to the specified number of available nodes. Full trimming leaves the 256-byte market header and all retained seats and orders. No market accounts, token vaults, global accounts, or wrapper accounts are closed or included in the recovery amounts.

The current accounts hold **47.285960337 SOL**. Their current rent requirement is **36.464890240 SOL**; the requirement after maximal compaction is **18.489411840 SOL**. The difference between the current balance and the final requirement is the 28.796548497 SOL headline.

The node inventory reconciles exactly:

| Node category | Count |
| --- | ---: |
| Existing free nodes | 15,809 |
| Seats with zero balances and no orders | 28,422 |
| Seats retained | 6,148 |
| Bid orders retained | 11,384 |
| Ask orders retained | 10,122 |
| Total allocated nodes | 71,885 |

There are 34,570 seats in total. Removing the 28,422 eligible seats contributes **11.550700800 SOL** beyond existing free-list compaction. A further **3,592 zero-balance seats have orders** and are retained. Withdrawable seat balances exclude funds committed to orders, so testing balances alone would overestimate eligibility. Every node reachable from either order tree counts as an order, including expired, global, reverse, and zero-quantity orders; this assessment does not cancel or clean orders. Historical quote volume does not disqualify an otherwise empty seat. After eligible seats are removed, 1,327 markets have no remaining seats or orders, but their headers and vaults remain allocated.

The live Rent sysvar reported **5,080 lamports per byte-year and exemption threshold 1**, making each removed 80-byte node worth **406,400 lamports = 0.0004064 SOL**. The calculation is `minimum_balance(bytes) = (bytes + 128) × 5,080`. It was checked against `getMinimumBalanceForRentExemption` at 0, 256, 336, and 958,736 bytes. The Rent sysvar was fetched at finalized slot 451,669,734. The familiar historical 6,960-lamport-per-byte assumption would overstate this snapshot's resize savings. Existing excess is measured from actual balances and current rent; its historical cause was not investigated. [Solana account storage documentation](https://solana.com/docs/core/accounts) describes the storage-balance model; the live RPC measurements in this report take precedence over example rates.

Recovery is concentrated. The ten markets with the largest resize savings account for **48.03% of resize savings**, or **11.904143514 SOL including those markets' existing excess**. The two largest alone yield **10.233439248 SOL including excess**:

| Market | Existing free nodes | Eligible empty seats | Resize savings (SOL) | Existing excess (SOL) |
| --- | ---: | ---: | ---: | ---: |
| `ENhU8LsaR7vDD2G1CsWcsuSGNrih9Cv5WZEk7q9kPapQ` | 4 | 10,887 | 4.426102400 | 1.664579728 |
| `GWBWsmmWFZrg9QFFKcSneiMqjemUbQL9Lq3zWFwsgHS7` | 2 | 7,437 | 3.023209600 | 1.119547520 |
| `8sjV1AqBFvFuADBCQHhotaRq5DFFYSjjg1jMyVWMqXvZ` | 168 | 713 | 0.358038400 | 0.165763786 |

My assessment: this is a bounded opportunity of roughly **24–29 SOL**, rather than hundreds of SOL. Collecting the **10.82 SOL of existing excess**, or pairing that with trailing-only trimming for **13.55 SOL**, offers a simpler first scope because live node indices remain unchanged. Whether full compaction is worthwhile depends on the engineering and review cost of the required program changes; the additional benefit of maximal compaction and seat removal over trailing-only trimming is **15.244470400 SOL**. Growth after cleanup will require users or another payer to fund rent again. Gross recovery is not recurring income.

The checked-out core instruction enum has no general market shrink, defragmentation, empty-seat eviction, or market excess-collection instruction. Its internal `release_seat` helper is used for temporary swap seats. These results therefore require new program support and an explicit decision about who receives the released lamports. This assessment does not establish ownership of historical rent deposits and does not verify deployed program bytecode against this checkout.

Full compaction must preserve tree links, roots, best-order indices, and order-to-seat references. Wrappers and clients also cache seat and order indices; they need compatible migration or refresh behavior before indices can move or seats can disappear. Returning an empty node to a free list alone releases **no SOL**: the account must shrink and the program must transfer the surplus while retaining the new minimum balance. Retaining a buffer reduces immediate regrowth, and zero-buffer behavior needs validation for all supported trading paths. Snapshot seat eligibility must be checked again during any eventual on-chain operation. Transaction fees, priority fees, deployment, engineering, and review costs have not been deducted; no transaction plan was simulated or submitted.

The analyzer validated owner, discriminator, layout version, allocation boundaries, tree parent links, cycles/overlap, order-to-seat references, and full node accounting. Every allocated block belonged to exactly one of the three trees or the free list; there were no unallocated trailing bytes and no excluded markets. An independent pass through the existing TypeScript SDK matched bids, asks, seats, and eligible-seat counts for **all 3,717 markets**. Four synthetic tests cover eligibility, dust balances, expired/global orders, and invalid structures.

Files: [summary.json](summary.json) contains exact aggregate values and rent RPC checks; [markets.json](markets.json) contains per-market figures, mints, and snapshot-only eligible seat indices, sorted by resize savings. These indices are evidence, not durable transaction inputs. The local read-only analyzer (`scripts/assess-market-rent.mjs`) loads `RPC_URL` from the existing environment or `.env` and never loads a wallet or submits transactions.

The assessment scripts are retained locally and are not included in this branch.
The following commands require those local copies.

```bash
# Fetch a new finalized mainnet snapshot and regenerate the assessment data.
node scripts/assess-market-rent.mjs

# Reproduce this assessment offline using the locally preserved raw snapshot.
node scripts/assess-market-rent.mjs \
  --snapshot .git/assessment-snapshots/mainnet-markets-451669723.json.gz \
  --out /tmp/manifest-rent-replay

node --test scripts/assess-market-rent.test.mjs
```

The raw compressed RPC snapshot is retained locally under `.git/assessment-snapshots/` and is not tracked. Its decompressed canonical JSON SHA-256 is `957cd687acc6aeecbfcd0cf5e4d23889be311325bab0a771afedd7a66985bc10`. A fresh live run will produce different figures as markets change.
