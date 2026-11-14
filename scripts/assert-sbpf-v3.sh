#!/usr/bin/env bash

set -euo pipefail

if (( $# == 0 )); then
  echo "usage: $0 <program.so> [...]" >&2
  exit 2
fi

# readelf or llvm-readelf on PATH; failing both (macOS without LLVM), the
# llvm-readelf of the platform-tools the workspace pins, which cargo-build-sbf
# has installed.
if command -v readelf >/dev/null 2>&1; then
  elf_reader=readelf
elif command -v llvm-readelf >/dev/null 2>&1; then
  elf_reader=llvm-readelf
else
  root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  tools_version="$(awk -F'"' '/^tools-version/{print $2}' "$root/Cargo.toml")"
  elf_reader="$HOME/.cache/solana/$tools_version/platform-tools/llvm/bin/llvm-readelf"
  if [[ -z "$tools_version" || ! -x "$elf_reader" ]]; then
    echo "neither readelf nor llvm-readelf is on PATH, and platform-tools ${tools_version:-?} has no llvm-readelf at $elf_reader; install binutils or LLVM, or run cargo build-sbf first" >&2
    exit 1
  fi
fi

for program in "$@"; do
  if [[ ! -f "$program" ]]; then
    echo "missing SBF program: $program" >&2
    exit 1
  fi

  header="$(LC_ALL=C "$elf_reader" --file-header "$program")"
  flags="$(awk -F: '/Flags:/{gsub(/[[:space:]]/, "", $2); split($2, parts, ","); print parts[1]}' <<< "$header")"
  if [[ "$flags" != "0x3" ]]; then
    echo "$program is not an sBPF v3 ELF (flags: ${flags:-unreadable})" >&2
    exit 1
  fi

  echo "$program: sBPF v3"
done
