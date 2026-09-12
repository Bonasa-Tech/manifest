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
          --arch=v2 \
          --cargo-build-sbf-args="--tools-version v1.57" \
          --base-image="solanafoundation/solana-verifiable-build@sha256:a1c0d5899ee0ffc81412428760662d9ba4643c2003ec3a92ab6f75a6e2e52a1b" \
          --library-name=manifest
      )
      args+=(--new-program "$repo_dir/target/deploy/manifest.so")
    fi
    ;;
esac

exec cargo run --manifest-path "$repo_dir/Cargo.toml" -p manifest-replay -- "${args[@]}"
