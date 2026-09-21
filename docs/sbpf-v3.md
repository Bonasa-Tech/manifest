# sBPF v3 build policy

Deployable artifacts are built with platform-tools v1.57 and `--arch v3`.
CI rejects an artifact unless its ELF flags report CPU version 3. The Solana
CLI, validator, and verifiable-build image are pinned to Agave 4.2.2.

The private replay benchmark selects `SVM_ARCH=v3`, and the public workflow
validates every Manifest and wrapper artifact produced by the harness before
accepting its result. The original v2 replay remains the performance target.
Compare identical source, platform tools, runtime, and recorded transactions
when isolating the effect of the architecture change.

## Performance recovery

The original v2 private-replay budget is met without raising any limit. The
controlled replay uses the same 4,158 recorded rows and Agave 4.2.2 runtime:

| CU per order | Original v2 target | Initial v3 | Optimized v3 | Optimized v3 vs target |
| --- | ---: | ---: | ---: | ---: |
| Manifest p50 | 1,438 | 1,875 | 1,385 | -3.7% |
| Manifest p95 | 2,424 | 3,236 | 2,357 | -2.8% |
| Manifest p99 | 2,651 | 3,504 | 2,558 | -3.5% |

Three final release-artifact runs returned those same percentiles and passed
the original CU targets. Every run retained all 4,158 signatures and the same
replay accounting. One zero-order row is excluded from
per-order percentiles. See [the aggregate results and ELF hashes](benchmarks/manifest-sbpf-2026-09-21.json);
the private recorded transaction corpus is not copied into this repository.

The initial roughly 30–34% Manifest increase came from generated instruction
changes, including removal of v2 PQR wide arithmetic. The same host runtime was
used on both sides; this is not a new VM accounting baseline. Guest instruction
metering still counts the executed instructions.

The source-level mitigations are:

- Cancel exact decimal factors of 10^12 or 10^9 before multiplying prices and
  sizes when possible, so common conversions fit native u64 arithmetic.
- Use checked 32-bit-limb products and normalized long division for the exact
  full-width fallback, retaining a remainder for ceiling rounding. Preserve
  the original overflow errors and precision; no floating-point approximation,
  price quantization or global disabling of overflow checks.
- Construct mantissa/exponent prices with native limb products, and avoid
  constructing prices for wrapper asks or global orders that do not need them
  during balance preparation. Invalid exponents are still rejected.
- Simplify tree insertion descent and inline iterator/predecessor work while
  preserving equal-price FIFO behavior.
- Retain validated order indices in TypeScript market snapshots and use them
  in the three core-cancel convenience methods, avoiding linear book searches.

Arithmetic tests compare boundaries, random inputs, both decimal fast paths,
fallbacks and rounding against full-width Rust arithmetic. Certora retains
the original full-width expressions because its arithmetic helper summaries
are not bit-precise; the optimized arithmetic is **not** independently proved
by those summaries.

The reference targets above are fixed measurements, not automatically updated
from benchmark history. The measured runs were checked against those targets.

### v2 portability and remaining differences

The same optimizations also build and run on v2:

| CU per order | Optimized v2 | Optimized v3 | v3 vs optimized v2 |
| --- | ---: | ---: | ---: |
| p50 | 1,324 | 1,385 | +4.6% |
| p95 | 2,238 | 2,357 | +5.3% |
| p99 | 2,432 | 2,558 | +5.2% |

Recovering the original budget does **not** eliminate the ISA's opportunity
cost, or guarantee every individual instruction is cheaper. These source
changes are portable; enabling unsupported v2 opcodes in a v3 ELF is not a
compatible backport. All CI and deployable artifacts still target v3.

Selected deterministic microbenchmarks make the remaining differences explicit:

| Transaction | Original v2 | Optimized v3 |
| --- | ---: | ---: |
| Core place 1 | 3,218 | 3,350 |
| Wrapper cancel 5 and place 5, 20 resting | 16,645 | 16,929 |
| Wrapper replace 10, 64 resting | 35,741 | 35,343 |
| Wrapper replace 10, 256 resting | 60,874 | 61,046 |
| Core cancel 10 with hints, 256 resting | 6,178 | 6,324 |
| Core cancel 10 without hints, 256 resting | 99,950 | 124,007 |

The last two rows are the same cancellation workload with and without indices.
The updated SDK changes that path from an unhinted 99,950-CU v2 transaction to
a hinted 6,324-CU v3 transaction in this fixture. Hints are validated on chain;
if intervening fills/cancels invalidate the snapshot, reload and retry. Passing
`false` to `cancelAllOnCoreIx`, `cancelBidsOnCoreIx` or `cancelAsksOnCoreIx`
retains the old sequence-number scan. Both paths reject missing orders; the
opt-out does not make a filled/cancelled order succeed. Legacy manually
constructed orders without indices also fall back to unhinted cancels.

## Release size

The measured replay-control and final release artifacts have these sizes:

| Program | v2 bytes | v3 bytes | Change |
| --- | ---: | ---: | ---: |
| Manifest | 353,432 | 342,320 | -3.1% |
| Wrapper | 152,888 | 143,752 | -6.0% |

The final UI wrapper is 239,648 bytes. Size reductions alone do not predict CU
improvements: a smaller instruction set can require more executed instructions.

## Runtime and formal tooling

Rust regression and coverage use Agave 4.2.2 ProgramTest and canonical v3
artifacts. This requires Rust 1.93 for the host test runtime and coordinated
Solana Program SDK, RPC client, and Jupiter compatibility-crate updates.
Certora also builds v3: its mutable verification-only globals are assigned to
the writable `.rodata.certora` section instead of the v3-discarded `.bss`.
Its supported Certora compiler is still platform-tools v1.53, explicitly
selected in the formal workflow; this is a compiler-tooling difference, not a
v2 architecture exception. The local Certora build succeeds. This work did not
submit a new remote proof run or treat a successful build as a proof result.

There are no v2 build exceptions in CI. Release, verifiable, ProgramTest,
coverage, validator, TypeScript integration, Certora, and replay benchmark
artifacts all target sBPF v3.

## Validation of this optimization set

- Three complete private replays, each passing the original v2 budget.
- 179 SBF integration tests: 139 core, 30 wrapper and 10 UI wrapper, using
  v3 test-feature artifacts under Agave 4.2.2 ProgramTest.
- Native arithmetic/tree tests (83 each), wrapper/UI unit tests, TypeScript
  typechecking and nine targeted SDK tests, including cancellation hints.
- Successful v3 ELF checks for all release,
  test-feature and local Certora artifacts.

These are local checks; no new remote CI, verifiable-container build or Certora
cloud proof execution is claimed for this optimization set.
