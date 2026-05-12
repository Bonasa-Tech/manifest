#!/bin/bash
set -euo pipefail

TEST_VALIDATOR_PID=""

cleanup() {
  if [ -n "$TEST_VALIDATOR_PID" ]; then
    kill "$TEST_VALIDATOR_PID" || true
    wait "$TEST_VALIDATOR_PID" 2>/dev/null || true
    TEST_VALIDATOR_PID=""
  fi

  echo "Cleaning up ledger"
  rm -rf test-ledger
}

trap cleanup EXIT

run_validator_tests() {
  local optimization="$1"

  if [ "$optimization" = "opt-full" ]; then
    cargo build-sbf -- -p manifest-dex -p wrapper -p ui-wrapper --features test,opt-full
    cargo build-sbf -- -p manifest-dex --features test,opt-full
  else
    cargo build-sbf -- -p manifest-dex -p wrapper -p ui-wrapper --features test
    cargo build-sbf -- -p manifest-dex --features test
  fi
  echo "Rebuilt programs for $optimization"

  solana-test-validator --reset --bpf-program MNFSTqtC93rEfYHB6hF82sKdZpUDFWkViLByLd1k1Ms target/deploy/manifest.so --bpf-program UMnFStVeG1ecZFc2gc5K3vFy3sMpotq8C91mXBQDGwh target/deploy/ui_wrapper.so --bpf-program wMNFSTkir3HgyZTsB7uqu3i7FA73grFCptPXgrZjksL target/deploy/wrapper.so  >/dev/null 2>&1 &
  TEST_VALIDATOR_PID=$!

  echo "Started $optimization test validator, sleeping for 15 seconds before starting"
  sleep 15

  echo "Setting solana config to localnet"
  solana config set --url l

  yarn test
  echo "Done with $optimization client tests"

  cleanup
}

run_validator_tests default
run_validator_tests opt-full
