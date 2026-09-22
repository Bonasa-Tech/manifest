#!/usr/bin/env bash

set -euo pipefail

if (( $# == 0 )); then
  echo "usage: $0 <program.so> [...]" >&2
  exit 2
fi

# LLVM's reader is available on macOS via LLVM; GNU binutils is not required.
if command -v readelf >/dev/null 2>&1; then
  elf_reader=readelf
elif command -v llvm-readelf >/dev/null 2>&1; then
  elf_reader=llvm-readelf
else
  echo 'Install GNU binutils or LLVM and put readelf or llvm-readelf on PATH' >&2
  exit 1
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
