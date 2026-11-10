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

Dependencies are the repo's own (`@solana/web3.js`, `@solana/kit`, `bs58`, `ws`);
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

## Continuously sweeping markets that traded

`--loop=SECONDS` is the existing long-running sweep control. By itself it
repeats ordinary, unfiltered sweep-only passes. Add the explicit
`--filled-since-defrag` flag to open the fill-feed WebSocket, retain each
market's newest fill slot in memory, and filter later passes to markets filled
since their last sweep. There is no second process, fill state file, or shared
watcher state directory.

`--loop=SECONDS` is the start-to-start interval between passes in this mode,
matching the collector's existing fixed-rate loop convention. If a pass takes
longer than the interval, the next pass starts immediately. `--pace=MS` retains
its existing meaning as the delay between transactions within a pass. For
example, an hourly production loop is:

```bash
RPC_URL=<url> KEYPAIR_PATH=<collector.json> \
  node scripts/defrag/defrag.mjs \
    --sweep-only --all --v1 --execute \
    --loop=3600 --filled-since-defrag
```

`--loop` requires `--sweep-only` and keeps the process alive.
`--filled-since-defrag` is opt-in and requires `--loop`; it never changes a
one-shot run. Without `--execute`, the loop continuously performs dry runs
until stopped.

How the comparison works:

- A confirmed log entry records the `slot` its defrag landed in. That slot is
  the "since". Entries written before the field existed are resolved from their
  recorded signature on demand. With this filter enabled, a confirmed entry is
  a baseline rather than a permanent "done" marker, so the same log supports
  recurring sweeps without `--ignore-log`.
- A market is kept when the feed's newest fill slot for it is at or after that
  slot. The check is a map lookup, not a scan.
- A market with no logged defrag is **kept** — there is no "since" to measure.
  So a fresh `--state-dir`, or a new `--log=` file, keeps everything and the
  filter appears to do nothing.
- Coverage starts when the process first connects. A market whose last sweep
  predates that is **kept** because the process cannot judge fills it did not
  observe. The first pass after each process start is therefore conservative
  and establishes fresh baselines for the markets it sweeps.
- The process waits for the WebSocket to reconnect before starting another
  pass. Brief disconnects during its lifetime are accepted without resetting
  coverage; a fill broadcast only during such a gap can be missed, and that
  market becomes eligible when another fill is observed.

A consequence: the filter does little until the process has been up longer than
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
- `--quiet-only` selects the opposite set and is rejected in combination.
- `--use-cached-plan` is rejected because cached membership can include markets
  already completed by a partial run or omit markets that filled afterward.
  Looped passes rebuild membership from current state and do not overwrite the
  ordinary plan cache.

## State

Logs, the plan cache, and any abort record are written under `--state-dir`
(default `./defrag-state`, created on demand). An ordinary one-shot run is
resumable: a market recorded confirmed is skipped on the next run with the same
parameters. Continuous fill mode instead uses each confirmation slot as that
market's next fill baseline; an unfiltered loop re-evaluates every market each
pass. State is keyed by operation, so a sweep and a relocation of the same
market don't shadow each other.

## Safety rails built in

- Every batch is simulated immediately before it is sent; a batch that fails
  simulation is split in half and retried rather than abandoned.
- `--health-every=N` samples program-wide health during a run and aborts if
  manifest-attributed failures rise above the pre-run baseline on two
  consecutive checks.
- Transaction expiry retries on a fresh blockhash; no priority fees.
