#!/usr/bin/env bash
# fuzz.sh -- talk to FuzzCorp for the manifest crucible harness.
#
# `run_fuzzcorp` rebuilds the program (platform-tools v1.57, sBPF v3, DWARF kept) and the harness
# (static linux/amd64), assembles the bundle (BUILD=1 ./build-bundle.sh), re-checks it with
# ./bundle-guard.sh, and uploads it. BUILD=0 bundles the already-staged artifacts as they are.
#
#   ./fuzz.sh run_fuzzcorp [--validate-only] [--dry-run]
#                                   rebuild program + harness, bundle, guard, [dry-run,] then
#                                   upload (or only validate) ./bundle on FuzzCorp
#   ./fuzz.sh dry_run [bundle-dir]  execute the bundled harness once (FUZZ_DRY_RUN=1) the way the
#                                   worker does -- setup() must complete against the program in
#                                   the bundle. Off a linux/amd64 host this needs docker (the
#                                   harness is a linux/amd64 binary). No upload.
#   ./fuzz.sh doctor                check the setup: FUZZ_* env, the fuzz-up binary, then
#                                   `fuzz-up version` (server reachable) and `fuzz-up user login`
#                                   (key valid; lists the org/projects it grants)
#   ./fuzz.sh seed_corpus [dir]     upload ./corpus to the lineage (coverage needs a corpus)
#   ./fuzz.sh local [seconds]       fuzz locally against the staged program, no server involved
#   ./fuzz.sh coverage              replay ./corpus with coverage into ./coverage/coverage.lcov
#
# Uploads go through fuzz-up, the public uploader
# (https://github.com/asymmetric-research/fuzz-up), the same tool the CI workflow uses via
# fuzz-upload-action. Install a release binary:
#   curl -sfL -o ~/.local/bin/fuzz-up \
#     https://github.com/asymmetric-research/fuzz-up/releases/latest/download/fuzz-up_darwin_arm64
#   chmod +x ~/.local/bin/fuzz-up      # fuzz-up_{linux,darwin}_{amd64,arm64}
#
# Environment -- provided by the caller, never written to disk by this script:
#   FUZZ_API_KEY        required by run_fuzzcorp / seed_corpus
#   FUZZ_ORGANIZATION   required by run_fuzzcorp / seed_corpus
#   FUZZ_PROJECT        required by run_fuzzcorp / seed_corpus
#   FUZZ_API_ORIGIN     optional: non-default cluster
#   FUZZ_UP_BIN         optional: the fuzz-up binary to use (default: fuzz-up on PATH)
#   BUNDLE_DIR          optional: where build-bundle.sh stages the bundle (default ./bundle)
#   DRY_RUN_IMAGE       optional: linux/amd64 image for dry_run off a linux/amd64 host (default:
#                       the FuzzCorp worker image if pulled, else debian:bookworm-slim -- the
#                       harness is static)
#   BUILD               1 (default): rebuild program + harness first; 0: bundle what is staged
#   BUILD_PROGRAM, BUILD_HARNESS, PLATFORM_TOOLS_VERSION, SBF_ARCH, FUZZ_REVISION, PROGRAM_DIR,
#   HARNESS_BIN, SOURCES_ORIGINAL_PATH, MUTE
#                       passed through to build-bundle.sh (see its header)
set -euo pipefail

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUNDLE_DIR="${BUNDLE_DIR:-$HARNESS_DIR/bundle}"
# Must match the lineage name build-bundle.sh writes into manifest.fc.json.
LINEAGE="manifest_fuzz__invariant_test"

log() { echo "[$(date +'%Y-%m-%d %H:%M:%S')] $*"; }
die() { echo "error: $*" >&2; exit 1; }

# ---------------------------------------------------------------------------------------------
# Upload. fuzz-up reads the target from FUZZ_ORGANIZATION / FUZZ_PROJECT and the key from
# FUZZ_API_KEY; it has no config file, so nothing on the machine can redirect an upload. (The full
# `fuzz` CLI does have one, and an active profile silently overrides those env vars -- which is
# exactly why this script uses fuzz-up.)
# ---------------------------------------------------------------------------------------------
resolve_uploader() {
    if [[ -n "${FUZZ_UP_BIN:-}" ]]; then echo "$FUZZ_UP_BIN"
    elif command -v fuzz-up >/dev/null; then echo fuzz-up
    else return 1
    fi
}

require_env() {
    local missing=()
    for v in FUZZ_API_KEY FUZZ_ORGANIZATION FUZZ_PROJECT; do
        [[ -n "${!v:-}" ]] || missing+=("$v")
    done
    (( ${#missing[@]} == 0 )) || die "missing environment variables: ${missing[*]}"
}

# Preflight for every upload: the FUZZ_* env, a fuzz-up binary, then two server round trips --
# `fuzz-up version` (the API server answers; prints cluster and version) and `fuzz-up user login`
# (the key authenticates; prints the organizations and projects it grants, stores nothing on disk).
# A FUZZ_PROJECT the key does not list is reported, because an upload aimed at the wrong project is
# otherwise only noticed on the dashboard. API keys are per-cluster: a key for one cluster 401s
# against another.
doctor() {
    require_env
    UPLOADER="$(resolve_uploader)" \
      || die "fuzz-up is not on PATH: install it (see this script's header) or set FUZZ_UP_BIN"
    log "uploader: $UPLOADER  organization: $FUZZ_ORGANIZATION  project: $FUZZ_PROJECT${FUZZ_API_ORIGIN:+  origin: $FUZZ_API_ORIGIN}"
    "$UPLOADER" version \
      || die "fuzz-up cannot reach the API server (FUZZ_API_ORIGIN=${FUZZ_API_ORIGIN:-<default>})"
    local login
    login="$("$UPLOADER" user login 2>&1)" \
      || { printf '%s\n' "$login" >&2; die "FUZZ_API_KEY does not authenticate against the API server"; }
    printf '%s\n' "$login"
    grep -qE "^[[:space:]]*-[[:space:]]*${FUZZ_PROJECT}[[:space:]]*$" <<<"$login" \
        || log "warning: FUZZ_PROJECT='$FUZZ_PROJECT' is not among the projects this key grants (listed above)"
}

# A fresh lineage has no corpus, and the cover task fails on an empty one ("Found 0 input files"),
# so coverage shows nothing until explore has produced inputs. Seeding with the harness's own
# corpus makes it render right away. The directory must be flat.
seed_corpus() {
    local dir="${1:-$HARNESS_DIR/corpus}"
    [[ -d "$dir" ]] || die "no corpus directory at $dir"
    doctor
    log "seeding lineage $LINEAGE (kind ${CORPUS_KIND:-main}) from $dir with $UPLOADER"
    "$UPLOADER" upload corpus --lineage "$LINEAGE" --kind "${CORPUS_KIND:-main}" "$dir"
}

# ---------------------------------------------------------------------------------------------
# Dry run. Executes the bundle's own harness once (FUZZ_DRY_RUN=1) from <bundle>/harness, the way
# the worker launches it: setup() must complete against the program in the bundle and one
# iteration must run. It is the only local check that sees a program built with other
# platform-tools (faults at entry) or a harness built before the IDL changed ("invalid instruction
# data"); both otherwise surface only in the FuzzCorp error log after the upload, as every task on
# the fleet dying in setup.
# ---------------------------------------------------------------------------------------------
dry_run_exec() {
    local dir="$1"
    if [[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]]; then
        log "dry run: native (FUZZ_DRY_RUN=1, cwd $dir/harness)"
        (cd "$dir/harness" && FUZZ_DRY_RUN=1 ./invariant_test)
    else
        # A missing or stopped docker is "could not run the check", NOT "the check failed" --
        # conflating the two turns an environment gap into a false report that the bundle is bad.
        command -v docker >/dev/null && docker info >/dev/null 2>&1 \
          || { echo "warning: the bundled harness is a linux/amd64 binary and docker is not available here," >&2
               echo "         so its dry run was SKIPPED, not passed. CI runs it natively on a linux/amd64" >&2
               echo "         runner before every upload. To check the same code path locally, run the host" >&2
               echo "         build from the bundle directory:" >&2
               echo "           (cd $dir/harness && FUZZ_DRY_RUN=1 $HARNESS_DIR/target/release/invariant_test)" >&2
               return 2; }
        local image="${DRY_RUN_IMAGE:-}"
        if [[ -z "$image" ]]; then
            if docker image inspect ghcr.io/asymmetric-research/fuzzcorp/worker-runtime:dev >/dev/null 2>&1; then
                image=ghcr.io/asymmetric-research/fuzzcorp/worker-runtime:dev
            else
                image=debian:bookworm-slim
            fi
        fi
        log "dry run: docker --platform linux/amd64 $image (FUZZ_DRY_RUN=1, cwd harness/)"
        docker run --rm --platform linux/amd64 -v "$dir:/bundle" -w /bundle/harness \
            -e FUZZ_DRY_RUN=1 --entrypoint /bundle/harness/invariant_test "$image"
    fi
}

dry_run() {
    local dir="${1:-$BUNDLE_DIR}"
    [[ -f "$dir/manifest.fc.json" && -x "$dir/harness/invariant_test" ]] \
        || die "no bundle at $dir -- build one first (./build-bundle.sh, or run_fuzzcorp)"
    dir="$(cd "$dir" && pwd)"
    local rc=0
    dry_run_exec "$dir" || rc=$?
    if [[ $rc -eq 2 ]]; then
        log "dry run SKIPPED (see the warning above); the bundle was not proven to run here"
        return 0
    fi
    [[ $rc -eq 0 ]] || die "the bundled harness FAILED its dry run -- not a bundle to upload. A fault at entry means the program was built with other platform-tools or another sBPF arch (BUILD_PROGRAM=1 rebuilds it); 'invalid instruction data' means the harness is older than the IDL (BUILD_HARNESS=1 rebuilds it)."
    log "dry run: setup and one iteration succeeded against $(basename "$dir")/harness/programs/manifest.so"
}

upload_bundle() {
    [[ -f "$BUNDLE_DIR/manifest.fc.json" ]] \
      || die "no bundle at $BUNDLE_DIR -- run ./build-bundle.sh first"
    log "uploading $BUNDLE_DIR with $UPLOADER $*"
    # Flags go BEFORE the positional path: the CLI parser treats anything after it as an extra
    # argument ("exactly one argument required, please provide a bundle path").
    "$UPLOADER" upload bundle "$@" "$BUNDLE_DIR"
}

run_fuzzcorp() {
    local extra=() do_dry_run=0
    for arg in "$@"; do
        case "$arg" in
            --validate-only) extra+=("$arg") ;;
            --dry-run)       do_dry_run=1 ;;
            *) die "unknown option: $arg" ;;
        esac
    done
    doctor
    local s; s=$(date +%s)
    # build-bundle.sh has one path that exits 0 WITHOUT assembling a bundle (BUILD_PROGRAM=1 on
    # its own just stages the program for local runs). With a previous bundle lying around, the
    # upload below would then ship that stale one. Mark the moment before the build and require
    # the manifest to be newer than the mark.
    local marker
    marker="$(mktemp)"
    BUILD="${BUILD:-1}" "$HARNESS_DIR/build-bundle.sh" "$BUNDLE_DIR"
    [[ -f "$BUNDLE_DIR/manifest.fc.json" ]] \
        || { rm -f "$marker"; die "build-bundle.sh produced no bundle at $BUNDLE_DIR"; }
    if [[ ! "$BUNDLE_DIR/manifest.fc.json" -nt "$marker" ]]; then
        rm -f "$marker"
        die "the bundle at $BUNDLE_DIR was not written by this build -- refusing to upload a stale bundle"
    fi
    rm -f "$marker"
    # Independent re-check of the finished artifact with the same gates CI runs, so nothing that
    # would only surface as an empty dashboard days later gets uploaded. The repo-root argument is
    # what lets it resolve DWARF source paths and check the deploy workflow's wiring.
    CHECK_ONLY=1 "$HARNESS_DIR/bundle-guard.sh" "$BUNDLE_DIR" "$HARNESS_DIR/../.."
    (( do_dry_run == 0 )) || dry_run "$BUNDLE_DIR"
    upload_bundle ${extra[@]+"${extra[@]}"}
    log "done in $(( ($(date +%s) - s) / 60 ))m $(( ($(date +%s) - s) % 60 ))s"
}

# ---------------------------------------------------------------------------------------------
# Local runs. No server, no bundle: the host build of the harness against the staged program.
# ---------------------------------------------------------------------------------------------
local_fuzz() {
    local bin="$HARNESS_DIR/target/release/invariant_test"
    [[ -x "$bin" ]] \
      || die "no host build at $bin -- (cd $HARNESS_DIR && cargo build --release --features invariant_test)"
    mkdir -p "$HARNESS_DIR/corpus" "$HARNESS_DIR/crashes"
    log "fuzzing locally; corpus and crashes under $HARNESS_DIR. Ctrl-C to stop."
    (cd "$HARNESS_DIR" && FUZZ_CORPUS_IN=corpus FUZZ_CORPUS_OUT=corpus FUZZ_CRASHES_DIR=crashes "$bin")
}

# LCOV over the existing corpus. The symbols path must not contain a "target/" component (the
# harness splits FUZZ_SYMBOLS there to infer the source root), which is why the symbols live in
# programs/ rather than being referenced in place.
coverage() {
    local bin="$HARNESS_DIR/target/release/invariant_test"
    [[ -x "$bin" ]] || die "no host build at $bin"
    [[ -f "$HARNESS_DIR/programs/manifest_symbols.so" ]] \
      || die "no programs/manifest_symbols.so -- BUILD_PROGRAM=1 ./build-bundle.sh stages it"
    [[ -d "$HARNESS_DIR/corpus" ]] && [[ -n "$(ls -A "$HARNESS_DIR/corpus" 2>/dev/null)" ]] \
      || die "no corpus to replay -- run ./fuzz.sh local first"
    mkdir -p "$HARNESS_DIR/coverage"
    (cd "$HARNESS_DIR" && FUZZ_COVERAGE_ONLY=1 FUZZ_CORPUS_IN=corpus \
        FUZZ_COVERAGE_OUT=coverage/coverage.lcov \
        FUZZ_SYMBOLS="$HARNESS_DIR/programs/manifest_symbols.so" "$bin")
    log "wrote $HARNESS_DIR/coverage/coverage.lcov"
}

usage() {
    sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed '$d' | sed 's/^# \{0,1\}//'
    exit 1
}

[[ $# -eq 0 || ${1:-} == --help || ${1:-} == -h ]] && usage
cmd=$1; shift
case "$cmd" in
    run_fuzzcorp)    run_fuzzcorp "$@" ;;
    dry_run|dry-run) dry_run "$@" ;;
    doctor)          doctor ;;
    seed_corpus)     seed_corpus "$@" ;;
    local)           local_fuzz "$@" ;;
    coverage)        coverage ;;
    *)               usage ;;
esac
