# Collector-only market defragmentation

`Defrag` (core opcode 14) atomically harvests empty seats, compacts market nodes,
retains two spare nodes, resizes the account, and returns released/excess lamports
to the collector. It also withdraws excess lamports from both non-native token
vaults. When no funded seats or orders remain **and both vaults are empty and
closable**, it closes the vaults and market instead.

Production authority is `B6dmr2UAn2wgjdm3T4N1Vjd8oPYRRTguByW7AEngkeL6`, the same
hardcoded collector as the wrapper. The `test` feature substitutes the existing
test collector. Never deploy a build with `test` enabled.

## Instruction and invariants

The instruction takes an optional little endian `u32` after the discriminator:
the number of nodes this run may relocate, with zero or an absent value meaning
unbounded. See [Bounded runs](#bounded-runs). Accounts, in order:

| Account | Access |
| --- | --- |
| Collector | Writable signer; receives refunds and funds any temporary growth |
| Market | Writable |
| Base vault | Writable, validated against the market and mint |
| Quote vault | Writable, validated against the market and mint |
| Base token program | Readonly |
| Quote token program | Readonly |
| System program | Readonly |

A seat survives if either withdrawable balance is nonzero or any resting order
references it, including global, reverse and expired-but-not-yet-removed orders.
Seats with zero balances and no orders are harvested; their historical per-seat
volume disappears. Market lifetime volume is preserved.

Compaction relocates the existing order nodes and rewrites their links and owner
indices without changing tree shape. Ordinary remove/reinsert at equal prices
would change FIFO priority. Surviving seats are inserted into a rebuilt seat tree;
reclaimed seats are simply left out of it, so reclaiming costs no tree work and a
run that drops thousands of seats costs no more than one that drops none.
Order sequence IDs, prices, quantities, expiry, type, reverse spread and global gas
prepayments are unchanged. The next-order sequence counter advances once to
invalidate wrapper quiet-sync caches; gaps in sequence numbers are valid.

A market larger than `MAX_DEFRAG_BLOCKS` (65,536 nodes, a 5.2 MB account) is
rejected with `InvalidAccountData` rather than exhausting the heap or the compute
budget partway through. The live bitmap is one bit per block, so that bound also
caps it at 8 KiB of the 32 KiB heap.

The live account target is `256 + 80 * (surviving seats + orders + 2)` bytes.
Two spare nodes preserve capacity for paths that need two allocations, including
reverse-order handling. Core batch updates finish with **at least five** spare
nodes, growing only the deficit; they do not shrink larger free lists. The batch
payer supplies this rent, including on cancel-only batches. Defrag reduces the
reserve back to two, or closes the account.

The fixed header and its padding stay unchanged at 256 bytes. No free-node count
is stored or maintained. Reserve checks use the existing free list, stopping once
they have found the required number of nodes (five for a core batch update).

Classic Token and Token-2022 use excess-lamport opcode 38 with the same account
layout. The pinned Token-2022 SDK helper only accepts its own program ID, so the
instruction is constructed with that helper and addressed to the validated vault
program. See the [classic token interface](https://github.com/solana-program/token/blob/main/interface/src/instruction.rs).

Native/WSOL vaults are skipped while the market stays open: principal, recorded
native rent reserve and unsynchronized deposits remain untouched. Closure requires
zero token amounts and zero withheld transfer fees. Unknown/confidential token
extensions prevent automatic closure. Supported vault extensions are transfer-fee
amount, transfer-hook account, pausable account and immutable owner. Token CPIs
run before direct market-lamport changes; any failure rolls back the instruction.

## Bounded runs

Passing a relocation budget makes a single market compactable across several
transactions. Each run:

- reclaims every eligible seat, which is free, and relocates at most `limit`
  surviving nodes into the holes beneath them, working from the top of the
  account down because only vacating a high block lets it shrink;
- leaves the market fully consistent — trees valid, balances intact, FIFO
  preserved, at least two spare nodes — and never larger than it started;
- converges: repeating until the size stops changing reaches the same layout a
  single unbounded run would produce.

So an operator who cannot fit a market in one transaction picks a budget,
simulates it against current state, and repeats until the size settles. There is
no resume cursor and no partial state to carry between runs; each one recomputes
everything from the account.

What a budget cannot bound is the single pass over allocated blocks and the seat
tree rebuild over the survivors, which is why `MAX_DEFRAG_BLOCKS` exists. Model
both terms when choosing a budget rather than assuming a small `limit` makes any
market affordable.

`ManifestClient.defragIx(collector, limit?)` and the Rust `defrag_instruction`
builder both take the budget as an optional argument.

## Wrapper compatibility and rollout

Both wrappers resolve cached seats by trader public key and orders by sequence ID
plus owner. Offsets are checked for bounds, alignment and node type. A stale order
hint searches its side and price level, including all equal-price entries after
tree rotations. Ordinary fills and cancels do not scan both books. Updated hints
are stored in the wrapper.

The regular wrapper recreates missing seats before batch updates. Deposit keeps
its original instruction and account list. Callers must claim a seat before an
initial deposit and claim it again after harvesting; ClaimSeat and Deposit can
be composed in the same transaction. ClaimSeat also refreshes moved seat/order
hints and repairs an existing MarketInfo instead of inserting a duplicate.
After defrag, callers can prepend ClaimSeat before depositing to refresh those
hints even when their seat survived.
Zero withdrawals after harvesting are harmless. The UI wrapper refreshes before
cancellation, recreates seats through its existing placement path, and prepays
placement capacity from its designated payer (one order plus five spare nodes).
It can settle
already accrued fees with no core seat. Reset seat volume cannot wrap into a huge
fee, and already accrued unpaid volume is retained. Unobserved historical volume
on a harvested seat cannot be reconstructed by a wrapper.

Upgrade **both wrappers before invoking Defrag**. Direct core clients must reload
market state and refresh index hints after maintenance; old strict hints can fail.
Use `ManifestClient.defragIx(collector, limit?)` or the Rust `defrag_instruction`
builder. Large accounts need an explicit compute budget, up to 1.4 million units;
simulate each transaction against current state before sending. A market that
does not fit in one transaction is compacted with a relocation budget over
several, subject to `MAX_DEFRAG_BLOCKS`. Closure is irreversible; closed market
addresses are no longer valid trading accounts.

No deployment or mainnet maintenance transaction is part of this change.

## Snapshot estimate for this implementation

Based on the saved September 29, 2026 census at core slot 451678322 and its vault
reads, using the observed 5,080 lamports/byte rent schedule. This is an estimate,
not a simulation of every market, and excludes transaction fees and all globals.

| Included component | SOL |
| --- | ---: |
| Market compaction, seat harvesting, excess rent and empty-market closure, net of growth | 29.440501137 |
| Non-native **market** vault excess | 9.613163461 |
| Remaining vault lamports when retiring 1,324 empty markets | 4.349333416 |
| **Total** | **43.402998014** |

This corrects an important difference from the earlier scenario tables: retaining
exactly two spare nodes sometimes requires growth. The net market figure includes
0.481584 SOL of additional retained rent on markets previously too small for two
spares. Temporary expansion before compaction is returned when no longer needed.
Vault excess and closure amounts are not double-counted. Global vaults are excluded.

## Non-global savings left out

- **0.208584356 SOL** of recorded native rent reserve above current rent in market
  vaults that remain open. Recovering it needs a separate, validated WSOL vault
  migration/reinitialization design preserving backing.
- **0.027649032 SOL** of unsynchronized native lamports in those remaining vaults.
  These are excluded because their economic ownership needs a separate policy.
- Wrapper-account compaction, shrinking or closure, and wrapper excess-rent
  collection are separate maintenance work; this instruction does not receive
  wrapper accounts. No additional amount was assessed here.
- Token-vault byte-size reductions: this instruction collects rent using each
  vault's existing length. Removing extensions or rebuilding smaller token accounts
  is separate work; additional savings are unquantified.
- Vault/market retirement blocked by token dust, withheld fees or unsupported
  extensions. This implementation does not sweep tokens or remove extensions;
  those balances may carry claims beyond rent. Additional savings are unquantified.

See [the broader census](assessments/mainnet-rent/EXPANDED.md) for input data and the
separate global analysis.

## Validation

- SBF builds for core, regular wrapper and UI wrapper using the pinned v1.57 tools.
- Eight new core integration cases: collector authorization/empty closure,
  balance and FIFO preservation/five-node replenishment, classic-token and
  Token-2022 excess collection, WSOL preservation, 13,000 seats with 1,000 funded
  survivors, and an account containing 13,002 free nodes.
- The large seat-population case used approximately 954,000 CU under a 1.4M limit
  **before** relocation budgeting was added. That figure has not been re-measured
  since; re-run the SBF integration suite and pin it before deploying.
- Core integration run: 141 passed, with the one old one-spare-node assertion
  subsequently updated to five and passing on rerun. Two unrelated cases were
  excluded. All 86 core library tests passed.
- All 32 regular-wrapper and 11 UI-wrapper integration tests passed, including
  moved-order cancellation, harvested-seat recovery and a sponsored owner with
  no SOL. Existing tests cover ordinary matching, reverse orders and gas refunds.
- Core library tests cover the bounded path directly: that budget-one runs
  converge byte for byte on the unbounded layout, that no bounded run grows the
  account or invalidates intermediate state, that the hardcoded node offsets
  match the typed accessors, and that an oversized market is rejected.
- TypeScript typecheck and formatting passed; 20 SDK cancellation/account-selection,
  token-program and metrics tests passed. Eight census-script tests passed.
- All 12 slim Rust-client parsing, instruction and SBF integration tests passed.
