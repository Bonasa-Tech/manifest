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
