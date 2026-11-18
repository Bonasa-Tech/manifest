#!/bin/bash

set -euxo pipefail

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# The programs under test and the harness that loads them must be built by
# the same platform tools, or the compute numbers describe two different
# compilers. The manifest workspace pins this in workspace.metadata.solana
# and cargo picks it up there, but this workspace has no such setting, so the
# version is passed explicitly to every build below, including the test one.
PLATFORM_TOOLS="${PLATFORM_TOOLS:-v1.57}"
SVM_ARCH="${SVM_ARCH:-${SBF_ARCH:-v3}}"
# ProgramTest otherwise emits several million lines of per-instruction debug
# logs, obscuring failures and exhausting CI log storage.
export RUST_LOG="${RUST_LOG:-error}"

# Build manifest & wrapper.
cd "$REPO_ROOT/programs/manifest"
cargo build-sbf --arch="$SVM_ARCH" --tools-version "$PLATFORM_TOOLS"
cd "$REPO_ROOT/programs/wrapper"
cargo build-sbf --arch="$SVM_ARCH" --tools-version "$PLATFORM_TOOLS"

# Copy over the binaries ProgramTest loads.
cd "$SCRIPT_DIR"
mkdir -p target/deploy
cp "$REPO_ROOT/target/deploy/manifest.so" "$REPO_ROOT/target/deploy/wrapper.so" target/deploy/

# Check the copies ProgramTest will load before spending time replaying them.
# Keep explicit v2 builds available for matched-source CU comparisons.
if [[ "$SVM_ARCH" == v3 ]]; then
  "$REPO_ROOT/scripts/assert-sbpf-v3.sh" target/deploy/manifest.so target/deploy/wrapper.so
fi

# Run tests
cargo test-sbf --arch="$SVM_ARCH" --tools-version "$PLATFORM_TOOLS" benchmark_all -p benchmarking -- --include-ignored

# Format the results to what the tool wants. This is intentionally stdlib-only
# so the benchmark does not mutate the runner's Python environment.
cd "$REPO_ROOT"
python3 scripts/benchmarking/format.py | tee output.txt
