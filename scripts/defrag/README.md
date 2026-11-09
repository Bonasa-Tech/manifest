# Core-market defrag runner

`defrag.mjs` is a single, self-contained, collector-only tool for compacting
Manifest core **market** accounts and sweeping the fees that accrue on them. It
has no machine-specific references — configuration is entirely flags and
environment — so it runs from any checkout on any machine.

This is the **core-market** side. Wrapper fee collection and wrapper defrag live
in `../collect-wrapper-fees.ts`; the two touch different programs.

## Run it

```bash
# read-only dry run (always do this first)
RPC_URL=<url> node scripts/defrag/defrag.mjs --sweep-only --all --v1

# execute: collect fees + truncate free tails across every market, batched
RPC_URL=<url> KEYPAIR_PATH=<collector.json> \
  node scripts/defrag/defrag.mjs --sweep-only --all --v1 --execute

node scripts/defrag/defrag.mjs --help    # full flag reference
```

Dependencies are the repo's own (`@solana/web3.js`, `@solana/kit`, `bs58`);
nothing extra to install.

## What it does, and what's safe

Defrag has two very different modes, and the safety difference is the whole
story:

- **Sweep-only** (`--sweep-only`, both budgets zero) collects accrued fees and
  truncates the already-free tail of a market. It **relocates nothing** — every
  resting-order and claimed-seat index stays byte-identical — so it is safe on
  any market, including those fronted by third-party integrators, and safe to
  run on a schedule.

- **Relocation** (`--budget=N`, `--seats-only`, or unbounded full compaction)
  moves live orders to reclaim trapped interior space. That changes the indexes
  a client may have cached. A well-behaved client re-reads or passes a hint the
  program tolerates; a client that dereferences a stale byte offset will fail.
  Relocate one market at a time, after confirming the integrators on that market
  handle it, never as an unattended sweep.

Markets the program would have to *expand* before compacting (fewer than two
free blocks) are never selected — that would cost the collector rent for no
gain. Market closure is double-gated (`--allow-close` **and**
`--yes-destroy-market-addresses`) and off by default.

## Limiting a sweep to markets that traded

`--filled-since-defrag` keeps only markets the fill feed saw trade since the
last defrag recorded for them, so a pass touches books that are actually
trading.

Fills come from the feed, which is a live broadcaster — no history, no replay,
no per-market query — so something has to be listening between sweeps.
`fill-watcher.mjs` does that, and must be running wherever the sweep is
scheduled, pointed at the same `--state-dir`:

```bash
node scripts/defrag/fill-watcher.mjs --state-dir=./defrag-state
```

It keeps `fill-state.json` (market → newest fill slot) written atomically, and
reloads it on start, so a restart only loses fills that happened while it was
down.

How the comparison works:

- A confirmed log entry records the `slot` its defrag landed in. That slot is
  the "since". Entries written before the field existed are resolved from their
  recorded signature on demand.
- A market is kept when the feed's newest fill slot for it is at or after that
  slot. The check is a map lookup, not a scan.
- A market with no logged defrag is **kept** — there is no "since" to measure.
  So a fresh `--state-dir`, or a new `--log=` file, keeps everything and the
  filter appears to do nothing.
- If the watcher's recorded slot is more than ~5,000 slots behind the chain, the
  run **fails** rather than filtering. A dead watcher makes every market look
  unfilled, which is the one failure mode here that quietly shrinks the sweep,
  so it is made loud.
- The state records `coverageFrom`, the slot from which observation has been
  continuous. A market whose last sweep predates that is **kept**: the watcher
  was not listening then, so its silence says nothing. Restarting after more
  than ~150 slots of downtime resets the window, and a state file written before
  this field existed is treated as covering nothing.

A consequence: the filter does little until the watcher has been up longer than
the usual gap since a market was last swept. Early on most markets come back
unobserved and are kept; it tightens as coverage grows. That is deliberate — the
alternative is dropping markets the watcher simply never saw.

Skipping is mostly deferral, not forfeiture: the baseline only advances when a
sweep actually lands, so a skipped market's window widens and the next fill
collects everything accrued since. Only a market that never fills again is
permanently passed over.

Two things worth knowing before using it:

- It runs *after* simulation, because the fill scan is the most expensive check
  in the run. It shortens the execution phase, not planning.
- `--quiet-only` selects the opposite set and is rejected in combination. A plan
  cache built with a different setting for this flag is also rejected.

## State

Logs, the plan cache, and any abort record are written under `--state-dir`
(default `./defrag-state`, created on demand). The run is resumable: a market
recorded confirmed is skipped on the next run with the same parameters, so a
paced rollout is just the same command again. State is keyed by operation, so a
sweep and a relocation of the same market don't shadow each other.

## Safety rails built in

- Every batch is simulated immediately before it is sent; a batch that fails
  simulation is split in half and retried rather than abandoned.
- `--health-every=N` samples program-wide health during a run and aborts if
  manifest-attributed failures rise above the pre-run baseline on two
  consecutive checks.
- Transaction expiry retries on a fresh blockhash; no priority fees.
