# Fee collection

`scripts/collect-wrapper-fees.ts` is the fee collector's entry point. It covers
four distinct jobs, all signed by the authorized collector
(`B6dmr2UAn2wgjdm3T4N1Vjd8oPYRRTguByW7AEngkeL6`):

| Job                    | Flag                          | What it recovers                                                           |
| ---------------------- | ----------------------------- | -------------------------------------------------------------------------- |
| Wrapper fee collection | `--all-public` / `--wrapper=` | Fees accrued on wrapper state accounts                                     |
| Wrapper compaction     | `--defrag`                    | Rent freed by packing a wrapper account                                    |
| Core market sweep      | `--markets`                   | Advanced-order fees accrued on market accounts, plus any already-free tail |
| Expired order cleaning | `--global-clean`              | One gas deposit (5,000 lamports) per expired order                         |

Everything is read-only without `--execute`. Every transaction is simulated
before submission, and transactions are batched as v1 (4,096-byte, 64-address)
messages.

```
yarn collect:wrapper-fees --help
yarn test:collect-wrapper-fees      # wrapper batching/CU unit tests
yarn test:collector-core            # core sweep + clean unit tests
```

## What each job is actually worth

Measured on mainnet, 2026-10-22. These numbers drive the defaults, so they are
worth knowing before choosing flags.

**Accrued fees: ~0.7 SOL/day program-wide.** Sampled four times over 1.5 hours
from a freshly zeroed baseline: 0.029, 0.030, 0.031 SOL/hour. This accrues
whether or not anyone sweeps, so **running more often does not earn more** — it
only shortens the lag between accrual and collection.

**Tail rent: scales with frequency, but mostly comes from traders.** Truncating
a market's free tail returns rent, and running hourly harvests more than
running daily. However, of 80 markets truncated in one sweep, **69 regrew
within two hours and traders re-paid 87% of the rent taken**: `batch_update`
replenishes the free reserve at one block per placement batch, funded by
whoever is placing the order. On an active market this is closer to a transfer
from traders than to recovered waste. It is unambiguously good on abandoned
markets, where the space never regrows.

That asymmetry is why `--markets` collects fees by default and tail tightening
is a separate opt-in via `--tail-tighten`.

**Expired order cleaning: tiny but free.** Each clean pays 5,000 lamports and a
transaction costs 5,000 in signature fees, so a single clean nets nothing and
`--global-clean` is only permitted alongside `--markets`, where the signature is
already paid for. A typical pass finds ~180 cleanable orders, worth ~0.0009 SOL.

## Flags

### Target selection

- `--wrapper=ADDRESS` — exactly one wrapper. Use this before any public sweep.
- `--all-public` — every initialized public wrapper with collectible fees.
- `--markets` — core market sweep. Can run alone (no wrapper target needed).
- `--global-clean` — also clean expired orders. **Requires `--markets`**; a
  clean-only run barely covers its own signature.

One of `--wrapper`, `--all-public`, or `--markets` is required.

### Scope and filtering

- `--defrag` — prepend a wrapper Defrag to each Collect in the same
  transaction, so freed rent is swept with the fees. Requires the wrapper
  program at v3.0.25 or later.
- `--tail-tighten` — include markets whose free tail can be truncated, not just
  those holding fees. Implies a minimum shrink of 1 block; see `--min-shrink`.
  Safe to run hourly — a sweep relocates nothing — but read the note above
  about who funds the regrowth.
- `--min-shrink=N` (env `MIN_SHRINK_BLOCKS`) — with `--tail-tighten`, require
  the sweep to truncate at least N blocks. Higher values target fewer, more
  worthwhile markets. `--min-shrink=3` typically selects ~90 markets.
- `--max-markets=N` (env `MAX_MARKETS`) — process at most N markets, highest
  recovery first. Useful for a paced first run.

### Batching and pacing

- `--max-wrappers-per-tx=N` (env `MAX_WRAPPERS_PER_TX`) — safety cap below the
  v1 maximum of 61 wrappers (32 with `--defrag`, where the 64-instruction limit
  binds first).
- `--delay-ms=N` (env `COLLECT_DELAY_MS`) — delay between transactions,
  default 10,000ms.

Core-side batching is automatic and measured, not estimated: **19 market sweeps
per transaction** (the 64-address limit binds at three addresses per market) and
up to 24 cleans. Candidate batches are serialized to check the real wire size
before being accepted.

### Execution

- `--execute` — submit. Without it the command is read-only.
- `--help` — usage.

### Environment

- `RPC_URL` — required.
- `KEYPAIR_PATH` — authorized collector keypair; required with `--execute`.
  Defaults to `~/.config/solana/id.json`.

## Recipes

```bash
# What is there to collect right now? (read-only)
yarn collect:wrapper-fees --markets --global-clean

# Hourly: fees + expired orders + tail tightening on worthwhile markets
yarn collect:wrapper-fees --markets --global-clean --tail-tighten --min-shrink=3 --execute

# Daily: fees only, every market, no tail disturbance
yarn collect:wrapper-fees --markets --global-clean --execute

# Wrapper side, unchanged
yarn collect:wrapper-fees --defrag --all-public --execute

# First run on a new deployment: pace it
yarn collect:wrapper-fees --markets --max-markets=50 --execute
```

## Safety properties

**Market sweeps relocate nothing.** Both defrag budgets are zero, which was
verified on mainnet to leave every resting-order and claimed-seat index
byte-identical across 4,148 markets. Clients holding cached indexes are
unaffected.

**Relocation is deliberately not exposed.** Compacting a market by moving live
orders broke two third-party integrators on 2026-10-22 — one stopped quoting
within the same 2,000-slot bucket, another panicked at 737 failures per 1,000
transactions reading a stale block offset. A single relocated order was enough.
That work needs integrator coordination and advance notice, not a flag.

**Markets that would cost money are never selected.** The processor tops a
market up to two free blocks _before_ compacting
(`free_blocks_short_of_n(2)` → `batch_expand_market`), funded by the collector.
On a market with no reclaimable tail that is a pure loss of ~406,400 lamports.
73 such markets cost 0.0385 SOL across three sweeps before this was caught, so
any market with negative net recovery is now excluded.

**Clean failures do not abort a run.** An expired order can be cancelled by
anyone between the read and the send, which invalidates its index. A clean
batch that fails simulation is skipped with a warning; a _sweep_ batch that
fails stops the run, because that indicates a real problem.

**Interrupts are honoured between transactions.** Ctrl-C stops before the next
transaction; a second Ctrl-C exits immediately.

## Why a sweep sometimes leaves exactly 2 free blocks

Defrag sizes an account to `frontier.max(keep_count + 2)` — never fewer than
two spare nodes, which the paths that allocate twice depend on.

- A market with **interior holes** keeps them: `frontier` dominates the floor,
  so only the tail above the highest live block is truncated. This is why
  sweeping a large, churned market is nearly pure fee collection — markets
  holding 78–280 free blocks lose none of them.
- A **densely packed** market with a free tail hits the floor and lands on
  exactly 2 free blocks. A market with 5 free blocks at the tail gives back 3.

Only relocation could gather interior holes, which is exactly the operation
that is not exposed here.

## DefiLlama reporting

The stats server indexes finalized transactions for the authorized collector
and serves `GET /fees?startTimestamp=SECONDS&endTimestamp=SECONDS`. Intervals
are half-open (`start <= blockTime < end`) and limited to 31 days. All amounts
are decimal **strings of lamports**, not SOL or USD.

Reporting includes wrapper collections, core market/vault defrag (including
rent released by resizing or closure), and expired global-order cleaning.
Amounts are recognized when collected, not when originally charged. This is
collector revenue only: separate UI/referral token destinations are outside
this ledger. Collection gas and collector-funded account expansion are added
back to the collector's balance change to recover gross collection receipts;
`fundingLamports` separately identifies the latter. Ordinary wallet deposits,
outgoing transfers, and failed transactions are excluded. A transaction mixing
collections with unrelated instructions stops indexing for review rather than
silently contaminating the total.

The response contains `feesLamports`, `revenueLamports`, `fundingLamports`,
`breakdown`, `collector`, the requested timestamps, `indexedThrough`,
`methodology: "collection-basis-v1"`, and `complete: true`. HTTP 400 means an
invalid interval. HTTP 503 means the interval is outside the current process's
indexed coverage; it must never be interpreted as zero fees. Empty intervals
return zero only after indexing has established complete coverage.
`coverageStartTimestamp` identifies the first full second after startup.

Fee reporting is entirely in memory. It starts from stats-server startup time,
with no configured start date, SQL tables, migrations, or persistent cursor.
The first scan runs at startup and subsequent scans run hourly, independently
of the hourly collector and DefiLlama adapter. Each successful scan commits its
collections and coverage together. Signature deduplication prevents counting
retries twice. Failed scans leave the last completed results intact.

Restarting the server discards its fee history and starts a new coverage
window. Requests spanning or preceding that startup return HTTP 503, including
requests for older intervals DefiLlama has already fetched. This design cannot
backfill across restarts. A partial startup hour is unavailable; the adapter
can report subsequent fully indexed hours. Each server replica has independent
coverage, including `READ_ONLY` instances, since fee indexing makes no database
writes. Use a stable instance for the fees endpoint if replicas restart at
different times.

The only optional fee-specific setting is `FEES_RPC_URL`, which overrides
`RPC_URL`. The RPC must support transaction version 1, finalized signature
history and transaction metadata (including inner instructions) since startup.
No historical backfill before startup is performed.

Rollout:

1. Deploy the stats server and monitor `Fee collection indexing failed` logs.
2. After a complete hour has been indexed, query an interval entirely within
   the current process lifetime. Check `complete`, `coverageStartTimestamp`
   and `indexedThrough`, and reconcile receipts against collector transactions.
3. Copy `integrations/defillama/fees/manifest-trade/index.ts` into
   `DefiLlama/dimension-adapters/fees/manifest-trade/index.ts`. The adapter has
   no fixed start date and propagates unavailable intervals as errors. The
   existing volume adapter remains unchanged.
4. Test an indexed interval with DefiLlama's runner before submission. Make
   the collection-basis methodology, resizing proceeds, and restart-limited
   history explicit to reviewers. Request attachment of `fees: manifest-trade`
   to the existing Manifest Trade listing.

Local checks: `yarn test:fee-collections`. The fixture is a public mainnet
collection transaction with directly reconciled wallet balances. The prepared
adapter is not automatically published or deployed by this repository.
