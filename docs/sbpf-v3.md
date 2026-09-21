# sBPF v3 build policy

Deployable artifacts are built with platform-tools v1.57 and `--arch v3`.
CI rejects an artifact unless its ELF flags report CPU version 3. The Solana
CLI, validator, and verifiable-build image are pinned to Agave 4.2.2.

The private replay benchmark receives both `SVM_ARCH=v3` and `SBF_ARCH=v3`,
and the public workflow validates every Manifest and wrapper artifact produced
by the harness before accepting its result. The first v3 replay should become
the new CU baseline; v2 and v3 CU results must not be compared as if they used
the same VM cost model.

Local release-size measurements on this branch, compared with the previous v2
artifacts, are:

| Program | v2 bytes | v3 bytes | Change |
| --- | ---: | ---: | ---: |
| Manifest | 351,576 | 340,792 | -3.1% |
| Wrapper | 146,672 | 143,128 | -2.4% |
| UI wrapper | 250,872 | 240,704 | -4.1% |

ProgramTest 3.x cannot parse platform-tools v1.57's canonical v3 ELF layout.
Agave 4.x can, but its public test types have moved ahead of the Program SDK,
SPL Token, and Jupiter interface types used here. Therefore the Rust
in-process regression and coverage jobs use isolated v2 test artifacts. The
Agave 4.2.2 validator/TypeScript suite is the behavioral v3 integration gate.
Certora also remains on its legacy SBF target because its writable-global
mocks do not link for v3. Neither exception produces a deployable artifact.
