#!/usr/bin/env bash
#
# Assembles a FuzzCorp bundle for the crucible manifest invariant harness.
#
# Usage: ./build-bundle.sh [output-dir]    (default: ./bundle)
#        ./build-bundle.sh --dirty-inputs  list uncommitted changes to the bundle's inputs, exit
#
# The output directory is deleted and recreated, so it must be absent, empty or a previous bundle
# (it holds manifest.fc.json); the repository, this harness, $HOME and / are always refused.
#
# With no knobs it bundles what is already built: the harness binary (see HARNESS_BIN) and the
# staged programs/manifest.so + programs/manifest_symbols.so. `./fuzz.sh run_fuzzcorp` runs it
# with BUILD=1, which rebuilds both halves first so a stale artifact can never be uploaded.
#
#   BUILD=1               BUILD_HARNESS=1 + BUILD_PROGRAM=1
#   BUILD_HARNESS=1       regenerate idls/manifest.json from the program source (./gen-idl.py),
#                         then build the harness static for the linux/amd64 workers: `cargo build`
#                         with musl-gcc on a linux/amd64 host, `cargo zigbuild` anywhere else
#                         (needs cargo-zigbuild + zig). The IDL is compiled in
#                         (declare_fuzz_program!), so the harness must be rebuilt after any change
#                         to the program's instruction interface -- a stale binary fails setup on
#                         the worker with an instruction-data or account error.
#   BUILD_PROGRAM=1       rebuild manifest FROM SOURCE with cargo build-sbf (DWARF kept, SBPF arch
#                         and platform-tools pinned) and stage BOTH outputs of that one build into
#                         programs/: the stripped deploy .so the harness executes and its
#                         unstripped twin for coverage.
#   PLATFORM_TOOLS_VERSION platform-tools for BUILD_PROGRAM (default v1.57: what
#                         workspace.metadata.solana pins, i.e. the compiler that produces the
#                         shipped program). The build goes to <repo>/target/sbf-<version>/, one
#                         directory per version, because a target/ populated by another version
#                         passes cargo's freshness check and would ship the other compiler's
#                         binary.
#   SBF_ARCH              --arch for cargo build-sbf (default v3, matching SVM_ARCH in
#                         .github/workflows/ci-verifiable-build.yml, i.e. what is deployed). The
#                         resulting ELF is checked for sBPF v3 (e_flags 0x3) before it is staged.
#   HARNESS_BIN           harness binary to bundle when not building it (default: the musl target
#                         build, else target/release). Workers are linux/amd64; a glibc build can
#                         die there with a bare `status 1`, so the musl target is preferred.
#   PROGRAM_DIR           the repository root containing programs/manifest (default: the git
#                         checkout this script lives in, else ../..)
#   FUZZ_REVISION         commit recorded in the manifest (default: git HEAD when the bundle's
#                         inputs have no uncommitted changes; otherwise a content hash of HEAD plus
#                         those changes, with a warning)
#   SOURCES_ORIGINAL_PATH override the derived coverage prefix (only when derivation is ambiguous)
#   MUTE                  comma-separated property ids to silence via SCOUT_CHECK_MUTE, for
#                         findings already triaged and written up
#
# The bundle uses FuzzCorp's native `crucible` driver: it drives the single harness binary through
# FUZZ_* env vars and parses the [FUZZ_*] stderr protocol. One lineage,
# manifest_fuzz__invariant_test, runs the harness in crucible's default action-sequence mode.
set -euo pipefail

# llvm-dwarfdump only: libdwarf's `dwarfdump` (homebrew) prints another format, and reading it as
# LLVM's would count 0 compile units and silently drop coverage from the manifest.
find_llvm_dwarfdump() {
  local c
  for c in llvm-dwarfdump /opt/homebrew/opt/llvm/bin/llvm-dwarfdump \
           /usr/lib/llvm-*/bin/llvm-dwarfdump /usr/bin/llvm-dwarfdump-*; do
    command -v "$c" >/dev/null 2>&1 && { echo "$c"; return 0; }
  done
  if c="$(xcrun -f dwarfdump 2>/dev/null)" && "$c" --version 2>/dev/null | grep -q LLVM; then
    echo "$c"; return 0
  fi
  if command -v dwarfdump >/dev/null 2>&1 && dwarfdump --version 2>/dev/null | grep -q LLVM; then
    echo dwarfdump; return 0
  fi
  return 1
}

# A symbols file with no DWARF still satisfies `[ -f ]`, then renders EMPTY coverage on the server
# while every CI step reports success. Require real compile units instead.
has_dwarf() {
  local so="$1" dd="" n
  dd="$(find_llvm_dwarfdump)" || true
  if [ -z "$dd" ]; then
    echo "warning: no llvm-dwarfdump available; cannot verify DWARF in $so" >&2
    return 0
  fi
  n=$($dd --debug-info "$so" 2>/dev/null | grep -c DW_TAG_compile_unit || true)
  if [ "${n:-0}" -eq 0 ]; then
    echo "warning: $so carries NO DWARF (0 compile units) -- coverage would render empty." >&2
    echo "         Build the program with CARGO_PROFILE_RELEASE_DEBUG=2 and" >&2
    echo "         CARGO_PROFILE_RELEASE_STRIP=false." >&2
    return 1
  fi
  echo "coverage: $(basename "$so") carries $n compile units"
  return 0
}

# The deployed program is sBPF v3 (SVM_ARCH in ci-verifiable-build.yml). A v0 or v1 build is a
# DIFFERENT binary for the same source, so shipping one would fuzz bytecode nobody runs. The ELF
# header's first e_flags field carries the version, which is what the repository's own
# scripts/assert-sbpf-v3.sh checks; this re-checks it here so the bundle cannot be assembled
# around a mismatched artifact.
assert_sbf_arch() {
  local so="$1" want="$2" reader="" flags
  for c in readelf llvm-readelf; do command -v "$c" >/dev/null 2>&1 && { reader="$c"; break; }; done
  if [ -z "$reader" ]; then
    echo "warning: no readelf/llvm-readelf; cannot verify the sBPF version of $so" >&2
    return 0
  fi
  flags="$(LC_ALL=C "$reader" --file-header "$so" \
    | awk -F: '/Flags:/{gsub(/[[:space:]]/, "", $2); split($2, p, ","); print p[1]}')"
  local expected
  case "$want" in
    v0) expected=0x0 ;; v1) expected=0x1 ;; v2) expected=0x2 ;; v3) expected=0x3 ;;
    *) echo "warning: unknown SBF_ARCH '$want'; not checking the ELF" >&2; return 0 ;;
  esac
  if [ "$flags" != "$expected" ]; then
    echo "error: $so has e_flags ${flags:-unreadable}, expected $expected for sBPF $want." >&2
    echo "       That is a different binary than the one deployed; refusing to bundle it." >&2
    exit 1
  fi
  echo "program: $(basename "$so") is sBPF $want (e_flags $flags)"
}

# Witness which compiler ACTUALLY produced the artifact, rather than which one was requested.
#
# The expected rustc is read out of the installed platform-tools itself, so there is no hardcoded
# version mapping to drift: whatever `cargo-build-sbf --tools-version <v>` installed under
# ~/.cache/solana/<v> is the compiler that build should have used, and DW_AT_producer in the
# unstripped artifact says which one did. A mismatch means the requested toolchain was not the one
# that ran -- a different binary than the deployed program, which every other gate here accepts.
assert_compiler() {
  local so="$1" tools="$2" dd="" expected="" produced="" rustc=""
  dd="$(find_llvm_dwarfdump)" || true
  if [ -z "$dd" ]; then
    echo "warning: no llvm-dwarfdump; cannot witness the compiler that built $so" >&2
    return 0
  fi
  for cand in "${HOME}/.cache/solana/$tools/platform-tools/rust/bin/rustc" \
              "${CARGO_HOME:-$HOME/.cargo}/.cache/solana/$tools/platform-tools/rust/bin/rustc"; do
    [ -x "$cand" ] && rustc="$cand" && break
  done
  if [ -z "$rustc" ]; then
    echo "warning: no rustc found for platform-tools $tools; cannot witness the compiler" >&2
    return 0
  fi
  # "rustc 1.95.0-dev (ae660768a 2026-08-17)" -> "1.95.0-dev (ae660768a 2026-08-17)"
  expected="$("$rustc" --version 2>/dev/null | sed 's/^rustc //')"
  produced="$($dd --debug-info "$so" 2>/dev/null \
    | grep -oE 'rustc version [^)]*\)' | head -1 | sed 's/^rustc version //')"
  if [ -z "$expected" ] || [ -z "$produced" ]; then
    echo "warning: could not compare compilers (expected='${expected}' produced='${produced}')" >&2
    return 0
  fi
  if [ "$expected" != "$produced" ]; then
    echo "error: $so was built by rustc $produced, but platform-tools $tools ships rustc $expected." >&2
    echo "       cargo-build-sbf used a different toolchain than the one requested, so this is not" >&2
    echo "       the binary the project deploys. Refusing to bundle it." >&2
    exit 1
  fi
  echo "compiler: built by rustc $produced, matching platform-tools $tools"
}

# Keep idls/manifest.json identical to what the program source derives. The harness compiles the
# IDL in (declare_fuzz_program!), so an IDL from another tree means the harness has never once
# been built against the program it claims to fuzz. gen-idl.py reads the instruction enum, the
# dispatch match and the processors' borsh params, and reconciles every instruction's account
# count against what its loader actually reads -- so this both refreshes the file and fails closed
# when the program's interface moves.
sync_idl() {
  echo "idl: deriving idls/manifest.json from $PROGRAM_DIR/programs/manifest"
  "$HERE/gen-idl.py" --crate "$PROGRAM_DIR/programs/manifest"
}

# What the bundle is built from, relative to the repository root.
BUNDLE_INPUTS=(programs lib Cargo.toml Cargo.lock rust-toolchain.toml fuzz/crucible
               .github/workflows/fuzzcorp.yml)

# Uncommitted changes to the bundle's inputs (modified, staged or untracked; ignored files such as
# target/, corpus/ and bundle/ excepted), one `git status --porcelain` line each.
dirty_inputs() {
  git -C "$PROGRAM_DIR" rev-parse --git-dir >/dev/null 2>&1 || return 0
  git -C "$PROGRAM_DIR" status --porcelain --untracked-files=all -- "${BUNDLE_INPUTS[@]}"
}

# Commit of the source under test, recorded in the manifest (CI passes the SHA of its clean
# checkout). HEAD describes the bundle only while none of its inputs has uncommitted changes;
# otherwise the manifest would credit uncommitted code to that commit, so the revision becomes a
# content hash of HEAD plus the pending changes.
revision() {
  local head dirty
  if [ -n "${FUZZ_REVISION:-}" ]; then echo "$FUZZ_REVISION"; return 0; fi
  if ! head="$(git -C "$PROGRAM_DIR" rev-parse HEAD 2>/dev/null)"; then
    cat "$HERE/src/main.rs" "$PROGRAM_SO" \
      | python3 -c 'import hashlib,sys; print(hashlib.sha1(sys.stdin.buffer.read()).hexdigest())'
    return 0
  fi
  dirty="$(dirty_inputs)"
  if [ -z "$dirty" ]; then echo "$head"; return 0; fi
  echo "warning: the bundle's inputs have uncommitted changes; the manifest records a content" >&2
  echo "         hash of HEAD ${head:0:12} plus these changes, not the commit:" >&2
  printf '%s\n' "$dirty" | head -20 | sed 's/^/           /' >&2
  (cd "$PROGRAM_DIR" && python3 - "$head" "${BUNDLE_INPUTS[@]}" <<'PY'
import hashlib, subprocess, sys
head, paths = sys.argv[1], sys.argv[2:]
h = hashlib.sha1(head.encode())
h.update(subprocess.run(["git", "diff", "HEAD", "--binary", "--", *paths],
                        capture_output=True, check=True).stdout)
listed = subprocess.run(["git", "ls-files", "--others", "--exclude-standard", "-z", "--", *paths],
                        capture_output=True, check=True).stdout
for path in sorted(p for p in listed.split(b"\0") if p):
    h.update(path + b"\0")
    with open(path, "rb") as fh:
        h.update(fh.read())
print(h.hexdigest())
PY
  )
}

# The output directory is deleted and recreated: refuse anything that is not plainly a bundle
# directory -- /, $HOME, the repository, this harness or any directory containing one of them, a
# symlink or non-directory, and a non-empty directory holding no manifest.fc.json.
guard_out() {
  python3 - "$1" "$HERE" "$PROGRAM_DIR" "${HOME:-/}" <<'PY'
import os, sys
out, *protected = sys.argv[1:]
if not out.strip():
    sys.exit("error: empty output directory")
if os.path.islink(out):
    sys.exit(f"error: output directory {out} is a symlink; pass the real directory")
real = os.path.realpath(out)
for keep in ["/", *protected]:
    kept = os.path.realpath(keep)
    if real == kept or kept.startswith(real.rstrip("/") + "/"):
        sys.exit(f"error: refusing to delete {real}: it is or contains {kept}")
if os.path.exists(real):
    if not os.path.isdir(real):
        sys.exit(f"error: output path {real} exists and is not a directory")
    if os.listdir(real) and not os.path.isfile(os.path.join(real, "manifest.fc.json")):
        sys.exit(f"error: {real} is not empty and holds no manifest.fc.json -- not a bundle; "
                 f"refusing to delete it")
PY
}

# Build the program from source (BUILD_PROGRAM=1), with DWARF kept so the line table keys the
# program's sources as "programs/manifest/src/..." and "lib/src/...", which the coverage prefix
# relies on. SBF bytecode is not a host-arch concern, so this runs natively on any host whose
# cargo-build-sbf knows the pinned platform-tools.
#
# Two artifacts come out of ONE invocation so their PCs match: the stripped deploy .so the harness
# executes, and the unstripped intermediate under target/<triple>/release/ carrying the DWARF. The
# triple directory is globbed -- which one cargo-build-sbf writes depends on the arch and CLI
# (sbpf-solana-solana for v0, sbpfv3-solana-solana for v3).
#
# The program's own [profile.release] is used unchanged (opt-level 3, lto fat, overflow-checks on).
# Lowering opt-level would improve the DWARF line mapping but would fuzz a binary nobody deploys;
# where inlining leaves a handler with no LCOV symbol that is a MEASUREMENT gap, not a coverage
# gap, and is diagnosed as such rather than compiled around.
build_program() {
  command -v cargo-build-sbf >/dev/null \
    || { echo "error: cargo-build-sbf not on PATH (install the Agave release that ships platform-tools ${PLATFORM_TOOLS_VERSION:-v1.57})" >&2; exit 1; }
  [ -f "$PROGRAM_DIR/programs/manifest/Cargo.toml" ] \
    || { echo "error: no program at $PROGRAM_DIR/programs/manifest (set PROGRAM_DIR)" >&2; exit 1; }
  local tools="${PLATFORM_TOOLS_VERSION:-v1.57}" arch="${SBF_ARCH:-v3}"
  # One target directory PER platform-tools version: --tools-version alone does not force a
  # rebuild when the bundled rustc reports the same version, so a target/ populated by another
  # toolchain can be reused as "fresh" and ship the other compiler's binary.
  local tdir="$PROGRAM_DIR/target/sbf-$tools-$arch"
  echo "building manifest from source (platform-tools $tools, arch $arch, via $(cargo build-sbf --version 2>/dev/null | head -1); target dir $tdir)"
  local build_log
  build_log="$(mktemp)"
  set +e
  (cd "$PROGRAM_DIR/programs/manifest" \
    && CARGO_TARGET_DIR="$tdir" CARGO_PROFILE_RELEASE_DEBUG=2 CARGO_PROFILE_RELEASE_STRIP=false \
       cargo build-sbf --tools-version "$tools" --arch "$arch") 2>&1 | tee "$build_log"
  local status="${PIPESTATUS[0]}"
  set -e
  [ "$status" -eq 0 ] || { echo "error: cargo build-sbf failed" >&2; exit 1; }
  # cargo build-sbf reports frame-size violations on stdout and may still exit 0. A binary built
  # past the limit faults where the real program does not, so whole instruction families read as
  # unreachable and any crash on an affected path is an artefact. "overwrites values in the frame"
  # is a toolchain mismatch and is never shipped; an "exceeded max offset" only counts inside
  # manifest's own code -- dependency crates are compiled alongside, and a frame in one of them is
  # platform-tools noise.
  if grep -q "overwrites values in the frame" "$build_log" \
     || grep "exceeded max offset" "$build_log" | grep -q "manifest"; then
    echo "error: SBF stack-frame violation in manifest; refusing to ship a binary that faults where the real program does not" >&2
    grep -E "exceeded max offset|overwrites values in the frame" "$build_log" >&2
    exit 1
  elif grep -q "exceeded max offset" "$build_log"; then
    echo "warning: stack-frame diagnostics in dependency code (not manifest):" >&2
    grep "exceeded max offset" "$build_log" >&2
  fi
  # cargo-build-sbf returns early only when the REQUESTED platform-tools equals its own built-in
  # one; otherwise it consults platform-tools/releases/latest, which reports the most recently
  # published release rather than the highest, and can silently DOWNGRADE with only a warning.
  # The repository documents this and asserts against it for release builds
  # (.github/workflows/ci-verifiable-build.yml). Every other gate here would still pass on a
  # downgraded binary: it is still sBPF v3, still carries DWARF, still matches its own symbols.
  if grep -qiE 'is not valid, latest version is|using the built-in version' "$build_log"; then
    echo "error: cargo-build-sbf did not use the requested platform-tools $tools:" >&2
    grep -iE 'is not valid, latest version is|using the built-in version' "$build_log" >&2
    exit 1
  fi
  rm -f "$build_log"

  local deploy="$tdir/deploy/manifest.so"
  [ -f "$deploy" ] || { echo "error: deploy artifact missing: $deploy" >&2; exit 1; }
  local unstripped
  unstripped="$(find "$tdir" -path '*/release/manifest.so' -not -path '*/deps/*' 2>/dev/null \
                | head -1 || true)"
  [ -n "$unstripped" ] \
    || { echo "error: no unstripped manifest.so under $tdir/*/release/ -- coverage would render EMPTY" >&2; exit 1; }
  assert_sbf_arch "$deploy" "$arch"
  assert_compiler "$unstripped" "$tools"
  # Stage both outputs of this ONE build next to each other, so a later run without BUILD_PROGRAM
  # bundles the pair that belongs together.
  mkdir -p "$HERE/programs"
  cp "$deploy"     "$HERE/programs/manifest.so"
  cp "$unstripped" "$HERE/programs/manifest_symbols.so"
  PROGRAM_SO="$HERE/programs/manifest.so"
  SYMBOLS="$HERE/programs/manifest_symbols.so"
  echo "program from source: programs/manifest.so ($(wc -c < "$PROGRAM_SO") bytes) <- $deploy"
  echo "                     programs/manifest_symbols.so ($(wc -c < "$SYMBOLS") bytes) <- $unstripped"
}

# Build the harness for the linux/amd64 workers (BUILD_HARNESS=1). Static musl: cc-rs finds
# musl-gcc for the C dependencies on a linux host; elsewhere zig provides the cross toolchain.
# rust-toolchain.toml in this directory pins the compiler and installs the musl target.
build_harness() {
  local target=x86_64-unknown-linux-musl
  sync_idl
  if [ "$(uname -s)" = Linux ] && [ "$(uname -m)" = x86_64 ]; then
    echo "building harness for $target (cargo build, musl-gcc)"
    (cd "$HERE" && cargo build --release --locked --features invariant_test --target "$target")
  else
    command -v cargo-zigbuild >/dev/null \
      || { echo "error: cargo-zigbuild not on PATH (cargo install cargo-zigbuild)" >&2; exit 1; }
    command -v zig >/dev/null || { echo "error: zig not on PATH (brew install zig)" >&2; exit 1; }
    echo "cross-building harness for $target (cargo zigbuild)"
    (cd "$HERE" && cargo zigbuild --release --locked --features invariant_test --target "$target")
  fi
  BIN="$HERE/target/$target/release/invariant_test"
}

HERE="$(cd "$(dirname "$0")" && pwd)"
# The harness lives at <repo>/fuzz/crucible. A handoff archive is not a git checkout, so fall back
# to the fixed relative layout.
PROGRAM_DIR="${PROGRAM_DIR:-$(git -C "$HERE" rev-parse --show-toplevel 2>/dev/null || (cd "$HERE/../.." && pwd))}"
if [ "${1:-}" = --dirty-inputs ]; then
  dirty_inputs
  exit 0
fi
OUT="${1:-$HERE/bundle}"
guard_out "$OUT"
PROGRAM_SO="$HERE/programs/manifest.so"
SYMBOLS="$HERE/programs/manifest_symbols.so"
if [ -n "${HARNESS_BIN:-}" ]; then
  BIN="$HARNESS_BIN"
elif [ -f "$HERE/target/x86_64-unknown-linux-musl/release/invariant_test" ]; then
  BIN="$HERE/target/x86_64-unknown-linux-musl/release/invariant_test"
else
  BIN="$HERE/target/release/invariant_test"
fi

if [ "${BUILD:-0}" = 1 ] || [ "${BUILD_HARNESS:-0}" = 1 ]; then build_harness; fi

if [ "${BUILD:-0}" = 1 ] || [ "${BUILD_PROGRAM:-0}" = 1 ]; then
  build_program
elif [ ! -f "$PROGRAM_SO" ]; then
  echo "error: target program not found at $PROGRAM_SO" >&2
  echo "       build it from source first: BUILD_PROGRAM=1 $0" >&2
  exit 1
fi

if [ ! -f "$BIN" ]; then
  # BUILD_PROGRAM=1 alone on a fresh checkout: the program is staged for local runs (cargo test /
  # crucible run); a bundle additionally needs the linux/amd64 harness.
  if [ "${BUILD_PROGRAM:-0}" = 1 ] && [ "${BUILD:-0}" != 1 ]; then
    echo "program staged in programs/; no harness binary at $BIN, so no bundle was assembled (BUILD=1 builds both)"
    exit 0
  fi
  echo "error: harness binary not found at $BIN" >&2
  echo "       build it first: (cd $HERE && cargo build --release --features invariant_test --target x86_64-unknown-linux-musl)" >&2
  echo "       (cargo zigbuild instead of cargo build on a non-linux host), or run with BUILD_HARNESS=1" >&2
  exit 1
fi

# amd64 on a GitHub ubuntu-latest runner. Read from the artifact, not from `uname -m`: on a Mac
# the harness is cross-built, and a native Mach-O would upload, validate, and then never be picked
# up by a worker.
DESC="$(file -b "$BIN")"
case "$DESC" in
  *"ELF 64-bit"*"x86-64"*)  ARCH=amd64 ;;
  *"ELF 64-bit"*"aarch64"*) ARCH=arm64 ;;
  *) echo "error: $BIN is not a linux ELF executable (workers cannot run it): $DESC" >&2; exit 1 ;;
esac
case "$DESC" in
  *"statically linked"*|*"static-pie linked"*) ;;
  *) echo "warning: harness is not statically linked; a glibc build may die on the worker: $DESC" >&2 ;;
esac
# Built without --features invariant_test the binary starts, selects no test and exits 0 -- a
# green campaign that fuzzes nothing.
grep -a -q invariant_test "$BIN" \
  || { echo "error: 'invariant_test' does not appear in $BIN -- was --features invariant_test passed?" >&2; exit 1; }
echo "harness: $BIN ($ARCH, $(wc -c < "$BIN") bytes)"

CRC="$(revision)"
# Already-triaged / written-up invariants, muted so any NEW finding stays visible. Each entry is a
# property id defined by a scout_run_property!("P-...", ..) call in src/main.rs.
MUTE="${MUTE:-}"

# Assemble in a fresh sibling directory and swap it in only once complete: a failed build leaves
# the previous bundle untouched, and nothing but a guarded bundle directory is ever deleted.
mkdir -p "$(dirname "$OUT")"
STAGE="$(mktemp -d "$(dirname "$OUT")/.bundle-stage.XXXXXX")"
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/harness/programs" "$STAGE/srcs"

cp "$BIN"        "$STAGE/harness/invariant_test"
# The one program the harness registers, resolved CWD-relative from harness_run_dir_in_bundle as
# "programs/manifest.so". The token programs come preloaded with LiteSVM.
cp "$PROGRAM_SO" "$STAGE/harness/programs/manifest.so"

# ---------------------------------------------------------------------------------- coverage ---
# Coverage maps LCOV lines onto the PROGRAM's source, not the harness's. The LCOV keys each line on
# the path composed from the DWARF line table (comp_dir when the unit has one, then the include
# directory, then the file); the driver strips sources_original_path and looks the remainder up
# under sources_path_in_bundle. The platform-tools builds record NO comp_dir, so the keys are
# relative to the workspace root: "programs/manifest/src/<file>" and "lib/src/<file>". Read the
# prefix from the artifact rather than hardcoding it -- a wrong prefix does not error, it silently
# yields lines_found: 0.
#
# manifest spans TWO first-party source roots: its own crate and lib/ (the hypertree crate, which
# is compiled in and holds the red-black tree the orderbook lives in). One prefix cannot cover
# both, since they share nothing above the repository root. So the matched root goes under srcs/,
# and lib/ is ALSO staged at the bundle root, where a record that does not match the prefix is
# resolved from. Losing hypertree's lines would hide the data structure that carries most of the
# protocol's state.
[ -d "$PROGRAM_DIR/programs/manifest/src" ] \
  || { echo "error: program source not found at $PROGRAM_DIR/programs/manifest/src (set PROGRAM_DIR)" >&2; exit 1; }
cp -R "$PROGRAM_DIR/programs/manifest/src/." "$STAGE/srcs/"
if [ -d "$PROGRAM_DIR/lib/src" ]; then
  mkdir -p "$STAGE/lib/src"
  cp -R "$PROGRAM_DIR/lib/src/." "$STAGE/lib/src/"
  echo "coverage: lib/src staged at the bundle root for hypertree's line records"
else
  echo "warning: $PROGRAM_DIR/lib/src is missing; hypertree coverage will not render" >&2
fi

SRC_ORIG=""
if [ -f "$SYMBOLS" ] && has_dwarf "$SYMBOLS"; then
  cp "$SYMBOLS" "$STAGE/harness/programs/manifest_symbols.so"
  if [ -n "${SOURCES_ORIGINAL_PATH:-}" ]; then
    SRC_ORIG="$SOURCES_ORIGINAL_PATH"
    echo "coverage: sources_original_path taken from the environment: $SRC_ORIG"
  elif ! SRC_ORIG="$(DWARFDUMP="$(find_llvm_dwarfdump || true)" \
                     "$HERE/derive-sources-prefix.sh" "$SYMBOLS" 'programs/manifest/src/')"; then
    echo "::error::could not derive a sources_original_path that matches the coverage" >&2
    echo "::error::profile -- the bundle would render EMPTY source coverage." >&2
    exit 1
  fi
  echo "coverage: sources_original_path=${SRC_ORIG}  sources=srcs/ (from $PROGRAM_DIR/programs/manifest/src)"
elif [ "${ALLOW_NO_COVERAGE:-0}" = 1 ]; then
  echo "warning: programs/manifest_symbols.so is missing or carries no DWARF, and" >&2
  echo "         ALLOW_NO_COVERAGE=1 was set -- the bundle will run but render NO source-level" >&2
  echo "         coverage." >&2
else
  # Fail closed. A coverage-less bundle uploads, schedules and runs perfectly happily; the only
  # symptom is lines_found: 0 on a dashboard days later, with every CI step green. That is the
  # most expensive failure mode here, so it is an error rather than a warning.
  echo "::error::programs/manifest_symbols.so is missing or carries no DWARF, so this bundle" >&2
  echo "::error::would render NO source-level coverage. Build it with BUILD_PROGRAM=1, which" >&2
  echo "::error::stages the unstripped twin of the same build. Set ALLOW_NO_COVERAGE=1 to" >&2
  echo "::error::bundle without coverage deliberately." >&2
  exit 1
fi

# One core per conf: the platform scales a lineage by running many single-core replicas, and the
# harness is a single-threaded LiteSVM replay. Memory is what a corpus_merge over tens of
# thousands of inputs needs, not what one iteration needs.
#
# Manifest v3 keys are snake_case. FuzzCorp rejects unknown keys, so the PascalCase spelling (the
# schema before the snake_case switch) fails the upload -- verified against the two other crucible
# harnesses' accepted manifests.
CRC="$CRC" ARCH="$ARCH" MUTE="$MUTE" SRC_ORIG="$SRC_ORIG" \
  python3 - "$STAGE/manifest.fc.json" <<'PY'
import json, os, sys
params = {
    "binary_path_in_bundle": "harness/invariant_test",
    "harness_run_dir_in_bundle": "harness",
}
if os.environ["SRC_ORIG"]:
    params.update({
        # Must NOT contain a "target/" component: crucible splits FUZZ_SYMBOLS at "/target/" to
        # infer the source root, so such a path corrupts DWARF resolution into 0 source files.
        "symbols_path_in_bundle": "harness/programs/manifest_symbols.so",
        "sources_path_in_bundle": "srcs",
        "sources_original_path": os.environ["SRC_ORIG"],
    })
lineages = [{
    "name": "manifest_fuzz__invariant_test",
    "confs": [{
        "name": "invariant_test",
        "driver": {"type": "crucible", "params": {
            **params,
            "extra_env": {"SCOUT_CHECK_MUTE": os.environ["MUTE"]},
        }},
        "architecture": {"name": os.environ["ARCH"]},
        "yield_time_minutes": 120,
        "memory_kib": 4 << 20,
        "cores": 1,
    }],
}]
manifest = {"version": 3, "revision": {"commit": os.environ["CRC"]}, "lineages": lineages}
with open(sys.argv[1], "w") as fh:
    json.dump(manifest, fh, indent=2)
    fh.write("\n")
PY
python3 -c 'import json,sys; json.load(open(sys.argv[1]))' "$STAGE/manifest.fc.json" \
  || { echo "error: manifest.fc.json is not valid JSON" >&2; exit 1; }

guard_out "$OUT"
rm -rf "$OUT"
mv "$STAGE" "$OUT"
trap - EXIT

echo "bundle assembled at: $OUT  (arch=${ARCH}, revision=${CRC:-unknown})"
