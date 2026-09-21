#!/usr/bin/env bash

set -euo pipefail

if (( $# == 0 )); then
  echo "usage: $0 <program.so> [...]" >&2
  exit 2
fi

for program in "$@"; do
  if [[ ! -f "$program" ]]; then
    echo "missing SBF program: $program" >&2
    exit 1
  fi

  flags="$({ readelf --file-header "$program" || true; } | awk -F: '/Flags:/{gsub(/[[:space:]]/, "", $2); split($2, parts, ","); print parts[1]}')"
  if [[ "$flags" != "0x3" ]]; then
    echo "$program is not an sBPF v3 ELF (flags: ${flags:-unreadable})" >&2
    exit 1
  fi

  echo "$program: sBPF v3"
done
