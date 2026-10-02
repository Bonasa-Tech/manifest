# manifest crucible fuzz harness

Stateful invariant fuzzer for the `manifest` program, built on
[crucible](https://github.com/asymmetric-research/crucible) and run on FuzzCorp. It replays
randomized instruction sequences against the real program in an in-process LiteSVM and checks
protocol invariants after every step.

Self-contained crate: it has its own `[workspace]`, so it is **not** part of the manifest build,
and it depends on the program only as a built artifact (`programs/manifest.so`) rather than as a
crate. Nothing here can change the program, its lockfile or its own build.

## Layout

| path | what |
| --- | --- |
| `src/main.rs` | the harness: fixture, one action per instruction, ten invariants, and the test suite CI gates on |
| `gen-idl.py` | derives `idls/manifest.json` from the program source; `--check` fails when the two disagree |
| `build-bundle.sh` | builds program + harness and assembles the FuzzCorp bundle |
| `bundle-guard.sh` | fail-closed checks on a finished bundle; run before any upload |
| `derive-sources-prefix.sh` | derives the coverage `sources_original_path` from the symbols' DWARF |
| `fuzz.sh` | operator front door: local fuzzing, coverage, dry run, upload |

## The IDL is derived, not committed by hand

`declare_fuzz_program!` reads `idls/manifest.json` at **compile** time, so an IDL from another tree
means the harness has never once been built against the program it claims to fuzz. `gen-idl.py`
removes that risk: it reads the `ManifestInstruction` enum, the dispatch match in `lib.rs` and each
processor's borsh params struct, and **reconciles every instruction's account count against the
loader that actually reads it**. CI runs `./gen-idl.py --check`, so a change to the program's
interface fails the build instead of producing a harness that silently cannot call it.

## Build and bundle

Three artifacts go into the bundle: the program, its unstripped twin (the coverage symbols) and the
harness. One command rebuilds all three, assembles the bundle, guards it and uploads:

```bash
cd fuzz/crucible && ./fuzz.sh run_fuzzcorp          # --validate-only stops before the upload
```

That is `BUILD=1 ./build-bundle.sh` plus the guard and the upload. The pieces, for doing them by
hand — the same steps `.github/workflows/fuzzcorp.yml` runs:

```bash
# 1. The program, from the repo root, with DWARF kept, at the pinned platform-tools and sBPF arch.
#    Both come from the repository itself: workspace.metadata.solana tools-version, and SVM_ARCH in
#    ci-verifiable-build.yml. A v0 build of this source is a DIFFERENT binary than the deployed v3
#    one, so build-bundle.sh checks the ELF and refuses a mismatch.
CARGO_TARGET_DIR="$PWD/target/sbf-v1.57-v3" CARGO_PROFILE_RELEASE_DEBUG=2 CARGO_PROFILE_RELEASE_STRIP=false \
  cargo build-sbf --tools-version v1.57 --arch v3 --manifest-path programs/manifest/Cargo.toml

# 2. Stage BOTH outputs of that ONE build: target/.../deploy/manifest.so is the stripped program
#    the harness executes, and target/sbpfv3-solana-solana/release/manifest.so is its unstripped
#    twin carrying the DWARF. They must come from the same invocation -- their .text is
#    byte-identical, and bundle-guard.sh checks exactly that (GATE H), because symbols from another
#    build map coverage onto the wrong lines with no error anywhere.

# 3. The harness, cross-built static for the linux/amd64 workers, then the bundle and the guard.
cd fuzz/crucible
cargo zigbuild --release --locked --features invariant_test --target x86_64-unknown-linux-musl
#   (on linux/amd64: plain `cargo build` with musl-tools installed)
./build-bundle.sh                  # -> ./bundle
./bundle-guard.sh bundle ../..     # fail-closed; pass the repo root or GATE I is skipped
```

`build-bundle.sh`'s header documents every knob (`BUILD`, `BUILD_PROGRAM`, `BUILD_HARNESS`,
`PLATFORM_TOOLS_VERSION`, `SBF_ARCH`, `HARNESS_BIN`, `PROGRAM_DIR`, `FUZZ_REVISION`,
`SOURCES_ORIGINAL_PATH`, `MUTE`, `ALLOW_NO_COVERAGE`).

Prerequisites for the scripts:

```bash
python3 -m pip install 'tree-sitter==0.25.2' 'tree-sitter-rust==0.24.2'   # gen-idl.py parses Rust
```

plus an **LLVM** `dwarfdump` — on macOS `xcrun -f dwarfdump` from the Xcode CLT (homebrew's
`dwarfdump` is libdwarf's, a different tool whose output they cannot read), on Debian/Ubuntu
`apt-get install llvm`. tree-sitter-rust must be **0.24.2 or newer**: 0.24.0 mis-parses `&raw` as
the Rust 2024 raw-borrow operator and refuses any crate with an ordinary local named `raw`.

`bundle-guard.sh` reports one expected warning: only ~37% of the DWARF line-table addresses fall
inside `.text`, so "coverage may be partial". That is the price of fuzzing the binary the project
actually deploys. The program's `[profile.release]` is `opt-level = 3` with `lto = "fat"`, and
crucible's own documentation measures source coverage at 24% under those settings versus 43% at
`opt-level = 1`. Lowering it would read better and fuzz a binary nobody runs, so the profile is
left alone; where inlining leaves a handler with no line records that is a **measurement** gap, not
a coverage gap, and should be diagnosed as one rather than compiled around.

`./fuzz.sh dry_run` executes the bundled harness once before an upload. It is the only check that
sees a program built with the wrong platform-tools or arch (faults at entry) or a harness built
before the IDL changed (`invalid instruction data`); both otherwise show up only in the FuzzCorp
error log after the upload, as every task on the fleet dying in setup.

## Running it locally

```bash
./fuzz.sh local        # fuzz against the staged program; corpus/ and crashes/ fill up
./fuzz.sh coverage     # replay corpus/ with coverage -> coverage/coverage.lcov
cargo test --features invariant_test
```

## CI

`.github/workflows/fuzzcorp.yml` runs the steps above on every push to `main` and every pull
request touching the program, `lib/`, or this harness: check the IDL against the program source,
check formatting, build the program from source with DWARF, cross-build the harness, run the harness
tests against that build, assemble the bundle, guard it, dry-run it, then upload. Pull requests
build and guard without uploading; a manual run can pass `validate_only` to have `fuzz-up` validate
the bundle instead of uploading it.

Every toolchain pin is **read from the repository** rather than duplicated in the workflow
(platform-tools and the Agave CLI from `Cargo.toml`, the sBPF arch from `ci-verifiable-build.yml`,
the Rust channel from `rust-toolchain.toml` here), so the workflow cannot drift from the program.

Required configuration (Settings > Secrets and variables > Actions):

- secret `FUZZ_API_KEY` — FuzzCorp API key. Keys are per-cluster. This is the only secret: crucible
  is public, so cargo fetches it anonymously.
- variables `FUZZ_ORGANIZATION`, `FUZZ_PROJECT`, and optionally `FUZZ_API_ORIGIN` for a non-default
  cluster.

Coverage on the dashboard additionally needs a corpus: a fresh lineage has none and the cover task
fails on an empty one, so seed it once (`./fuzz.sh seed_corpus`) or wait for explore to produce
inputs. After the first real run, the definition of done is `fuzz list errors` clean **and**
`fuzz list cover` showing `lines_found > 0` for `manifest_fuzz__invariant_test` by name — execs
climbing proves the campaign runs, not that anyone can see what it covered.
