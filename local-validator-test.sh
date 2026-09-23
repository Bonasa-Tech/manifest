#!/bin/sh

set -eu

cd "$(CDPATH= cd "$(dirname "$0")" && pwd)"

(cd programs/manifest && cargo build-sbf --arch v3 --tools-version v1.57)
(cd programs/wrapper && cargo build-sbf --arch v3 --tools-version v1.57)
(cd programs/ui-wrapper && cargo build-sbf --arch v3 --tools-version v1.57)
./scripts/assert-sbpf-v3.sh \
  target/deploy/manifest.so \
  target/deploy/wrapper.so \
  target/deploy/ui_wrapper.so
echo "Rebuilt program"

# Never run against an existing validator, which may contain an older build.
rpc_url=http://127.0.0.1:8899
if solana --url "$rpc_url" cluster-version >/dev/null 2>&1; then
  echo "An RPC server is already running at $rpc_url; stop it before testing" >&2
  exit 1
fi

# Own only this invocation's process and temporary ledger, not test-ledger or
# the user's Solana configuration. Clean up on test failure and interruption.
validator_ledger=$(mktemp -d "${TMPDIR:-/tmp}/manifest-validator.XXXXXX")
validator_pid=
cleanup() {
  result=$?
  trap - 0 INT TERM HUP
  if [ -n "$validator_pid" ]; then
    kill "$validator_pid" 2>/dev/null || true
    attempts=0
    while kill -0 "$validator_pid" 2>/dev/null && [ "$attempts" -lt 10 ]; do
      sleep 1
      attempts=$((attempts + 1))
    done
    if kill -0 "$validator_pid" 2>/dev/null; then
      kill -9 "$validator_pid" 2>/dev/null || true
    fi
    wait "$validator_pid" 2>/dev/null || true
  fi
  # This exact directory was created by mktemp above and is never user-supplied.
  rm -rf -- "$validator_ledger"
  exit "$result"
}
trap cleanup 0
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

solana-test-validator --ledger "$validator_ledger" --bpf-program MNFSTqtC93rEfYHB6hF82sKdZpUDFWkViLByLd1k1Ms target/deploy/manifest.so --bpf-program UMnFStVeG1ecZFc2gc5K3vFy3sMpotq8C91mXBQDGwh target/deploy/ui_wrapper.so --bpf-program wMNFSTkir3HgyZTsB7uqu3i7FA73grFCptPXgrZjksL target/deploy/wrapper.so >"$validator_ledger/validator.log" 2>&1 &
validator_pid=$!

attempts=0
while :; do
  if ! kill -0 "$validator_pid" 2>/dev/null || [ "$attempts" -ge 60 ]; then
    echo 'Test validator failed to become ready' >&2
    tail -n 40 "$validator_ledger/validator.log" >&2
    exit 1
  fi
  if solana --url "$rpc_url" cluster-version >/dev/null 2>&1 && kill -0 "$validator_pid" 2>/dev/null; then
    break
  fi
  sleep 1
  attempts=$((attempts + 1))
done

yarn test
echo "Done with client tests"
