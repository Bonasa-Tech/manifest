#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
args=("$@")

case "${1:-}" in
  run|replay)
    has_candidate=false
    for arg in "${args[@]}"; do
      if [[ "$arg" == "--new-program" || "$arg" == --new-program=* ]]; then
        has_candidate=true
        break
      fi
    done
    if [[ "$has_candidate" == false ]]; then
      solana_verify="${SOLANA_VERIFY_BIN:-}"
      if [[ -z "$solana_verify" ]] && command -v solana-verify >/dev/null 2>&1 \
        && [[ "$(solana-verify --version)" == "solana-verify 0.5.1" ]]; then
        solana_verify="$(command -v solana-verify)"
      fi
      if [[ -z "$solana_verify" ]]; then
        solana_verify="$repo_dir/target/manifest-replay-tools/bin/solana-verify"
        if [[ ! -x "$solana_verify" ]] \
          || [[ "$($solana_verify --version)" != "solana-verify 0.5.1" ]]; then
          cargo +1.89.0 install solana-verify \
            --version 0.5.1 \
            --locked \
            --root "$repo_dir/target/manifest-replay-tools"
        fi
      elif [[ "$($solana_verify --version)" != "solana-verify 0.5.1" ]]; then
        echo "SOLANA_VERIFY_BIN must point to solana-verify 0.5.1" >&2
        exit 1
      fi
      (
        cd "$repo_dir"
        "$solana_verify" build \
          --arch=v3 \
          --cargo-build-sbf-args="--tools-version v1.57" \
          --base-image="solanafoundation/solana-verifiable-build@sha256:16053d845922e798ab1852d3fe222faf5a23eeb1db3a13d30b70d6b6e82184ae" \
          --library-name=manifest
        ./scripts/assert-sbpf-v3.sh target/deploy/manifest.so
      )
      args+=(--new-program "$repo_dir/target/deploy/manifest.so")
    fi
    ;;
esac

exec cargo run --locked --manifest-path "$repo_dir/Cargo.toml" -p manifest-replay -- "${args[@]}"
