# sBPF v3 build policy

Deployable artifacts are built with platform-tools v1.57 and `--arch v3`.
CI rejects an artifact unless its ELF flags report CPU version 3. The Solana
CLI, validator, and verifiable-build image are pinned to Agave 4.2.2.

The private replay benchmark selects `SVM_ARCH=v3`, and the public workflow
validates every Manifest and wrapper artifact produced by the harness before
accepting its result. The first v3 replay should become the new CU baseline;
v2 and v3 CU results must not be compared as if they used the same VM cost
model.

Local release-size measurements on this branch, compared with the previous v2
artifacts, are:

| Program | v2 bytes | v3 bytes | Change |
| --- | ---: | ---: | ---: |
| Manifest | 351,576 | 340,792 | -3.1% |
| Wrapper | 146,672 | 143,128 | -2.4% |
| UI wrapper | 250,872 | 240,016 | -4.3% |

The private replay was also run twice from the same source and the same 4,158
recorded rows, changing only `SVM_ARCH` from v2 to v3:

| CU per order | v2 | v3 | Change |
| --- | ---: | ---: | ---: |
| Manifest p50 | 1,438 | 1,875 | +30.4% |
| Manifest p95 | 2,424 | 3,236 | +33.5% |
| Manifest p99 | 2,651 | 3,504 | +32.2% |
| Recorded Phoenix p50 | 6,897 | 6,897 | 0.0% |
| Recorded Phoenix p95 | 13,208 | 13,208 | 0.0% |
| Recorded Phoenix p99 | 13,902 | 13,902 | 0.0% |

The Phoenix values come from the recorded source transactions, so they are the
control. The roughly 30--34% Manifest increase is the v3 VM cost-model impact
and establishes a new baseline; it is not comparable to the published v2
history as a code regression.

Rust regression and coverage use Agave 4.2.2 ProgramTest and canonical v3
artifacts. This requires Rust 1.93 for the host test runtime and coordinated
Solana Program SDK, RPC client, and Jupiter compatibility-crate updates.
Certora also builds v3: its mutable verification-only globals are assigned to
the writable `.rodata.certora` section instead of the v3-discarded `.bss`.

There are no v2 build exceptions in CI. Release, verifiable, ProgramTest,
coverage, validator, TypeScript integration, Certora, and replay benchmark
artifacts all target sBPF v3.
