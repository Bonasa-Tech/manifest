# sBPF v3 follow-up: main rebase and remaining CU gap

This follows the [original migration measurements](sbpf-v3.md). The branch was
rebased onto `main` at `e676ab299f1e1ca6ac46fca6d0aa02e4a288523c`, preserving its
allocator fix, wrapper fee, order-book changes and restored full-book cancel-all
behavior. Earlier measurements must not be relabeled as results for this source.

The rebase also required updating the newly added `manifest-replay` tool to
ProgramTest 4, using its runtime-matched SBF token programs, and updating its
default verifiable-build recipe to v3. CI retains main's preinstallation and
image-version safeguards, with the v1.57 archive checksum and compiler pins.

## Preserve the benchmark, not just its numerical limits

Main restored a wrapper `cancel_all=true` search across the full core order book,
including orders placed directly on the core. That behavior is preserved and
tested. Running the old private harness unchanged against the rebased v3 build
measured **1,267 / 5,545 / 5,895** CU/order at p50/p95/p99. Those tail increases
include the restored full-book scans; they are not an isolated ISA regression.

Main also introduced a private-fixture adapter that expands recorded cancel-all
events into explicit cancels of the fixture's wrapper-tracked orders. This
fixture places its resting orders through that wrapper; its direct swaps are
IOC. The adapter avoids scanning unrelated traders' orders while retaining the
intended cancellations. This measures explicit cancellation, **not** the cost
of the production wrapper's full-book cancel-all API.

The rebased adapter retains the original order-count denominator and zero-order
transactions. It does **not** count the newly expanded cancel entries as extra
orders, drop empty cancel-all events, or change the recorded reference controls.
Counting those expanded entries would make CU/order look cheaper without an
equivalent compute saving. The original numerical targets and reference controls
are unchanged. The measurements were validated using a SHA-256 fingerprint of
the sorted `(signature, recorded reference CU, original order count)` tuples from
the original v2 replay, including zero-order rows. The private corpus itself
is not published.

## Additional source optimization

Skip cancellation bookkeeping when there are no wrapper indices to remove.
Skip allocating/copying the core's return data when no placements were forwarded.
The successful core batch has no placement records in that case. No arithmetic,
rounding, order matching, fee, account layout or bounds checks are changed.

Same-source v3 microbenchmarks before/after these two changes:

| Transaction | Before | After | CU saved |
| --- | ---: | ---: | ---: |
| Wrapper cancel 1 of 20 | 6,076 | 5,831 | 245 |
| Wrapper cancel 5 of 20 | 9,124 | 8,879 | 245 |
| Wrapper cancel 10 of 20 | 12,723 | 12,478 | 245 |
| Wrapper empty batch, 20 resting | 4,748 | 4,480 | 268 |
| Wrapper place 10 | 37,518 | 37,486 | 32 |
| Wrapper replace 10, 256 resting | 58,090 | 58,081 | 9 |

These changes also build for v2; portable improvements do not by themselves
prove that the same-source v2/v3 gap has closed.

In the full replay, the two changes move v3 p50/p95/p99 from
**1,262 / 2,259 / 2,369** to **1,261 / 2,258 / 2,368** CU/order. That is only
one CU at each percentile: useful savings for cancellation-only instructions,
but not a substantial further recovery of the replay tail. Both runs retain
all 4,158 original signatures, order counts and recorded reference controls.
Both remain below the unchanged **1,438 / 2,424 / 2,651** limits. The larger
improvement over the pre-rebase report includes main's source changes and the
disclosed fixture adapter; it must not all be credited to these two guards.

### Matched optimized-v2 comparison

| CU per order | Optimized v2 | Final v3 | v3 penalty |
| --- | ---: | ---: | ---: |
| p50 | 1,202 | 1,261 | +4.9% |
| p95 | 2,148 | 2,258 | +5.1% |
| p99 | 2,250 | 2,368 | +5.2% |

The same updated source, platform-tools v1.57, runtime and adapted fixture were
used for both targets. The approximately 5% ISA gap remains, much as in the
pre-rebase matched-source comparison. The work has **not** eliminated that
opportunity cost or raised the performance baseline. See the
[raw aggregates, accounting fingerprint, artifact hashes and microbenchmarks](benchmarks/manifest-sbpf-followup-2026-09-21.json).

## Further optimization priorities

1. Resolve multiple unhinted core cancels in a single book walk before deleting
   in request order. The current batch loop calls `cancel_order` separately,
   and each call scans both complete book sides. In the rebased v3 fixture,
   canceling 10 of 256 orders costs **131,303 CU without hints versus 6,316 with
   hints**. A batch resolver could avoid repeated searches without requiring a
   fresh SDK snapshot. It must preserve missing-order errors, ownership checks,
   duplicate-sequence detection, repeated-cancel behavior and original error
   ordering, including batches mixing hinted and unhinted cancels. This needs
   dedicated differential tests and formal-model review before implementation.
2. Fuse wrapper cancellation counting, list unlinking and free-list insertion
   where safe, eliminating repeated node reads and passes. Preserve neighboring
   links before recycling each node, global-order counts, and deduplication.
3. Reduce repeated checked node access and u32-to-pointer conversions in tree
   traversal. Profile this on each architecture while keeping account
   bounds/alignment checks and equal-price FIFO semantics intact.
4. Profile the remaining full-width arithmetic and compiler-generated checked
   multiplications before changing another hot path. Existing exact decimal
   fast paths already remove much of Manifest's original wide-arithmetic cost.
   Do not disable overflow checks globally or enable v2-only opcodes on v3.

These are next experiments, not claimed additional savings. Benchmark
each against both architectures with identical source, fixtures and accounting;
a portable speedup can improve production CU while leaving the ISA gap intact.

## Validation after rebasing

- 179 SBF integration tests: 139 core, 30 wrapper and 10 UI wrapper. These
  include the restored cancel-all behavior and the final wrapper fast paths.
- 82 Manifest, 83 tree, five wrapper and nine UI native unit tests; four tests
  of the newly added mainnet replay tool.
- All-target workspace compilation, CI's strict Clippy command, nightly Rust
  formatting, TypeScript typechecking, targeted formatting and nine SDK tests.
- All three controlled private replays pass the unchanged numerical budget
  and the original per-row accounting fingerprint.
- Fresh release, test-feature and Certora artifacts report ELF version 3.
  The Certora build succeeds with its writable mock section; this is not a
  new cloud proof or a proof of the optimized arithmetic.

The final branch is based on the fetched main revision above and passes a local
merge check. No remote CI or force-push is claimed. A local verifiable-container
build was not run because this session cannot access the Docker daemon; CI
retains its pinned-image compiler-version check.
