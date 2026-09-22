#!/usr/bin/env python3
"""One pinned build and layout check for CLI proofs and local just recipes."""

import argparse
import contextlib
import json
import subprocess
import sys
from pathlib import Path

from check_elf import check


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--json', action='store_true')
    parser.add_argument('-l', action='store_true')  # Certora build-script protocol
    parser.add_argument('--arch', choices=['v3'], default='v3')
    parser.add_argument('--cargo_features', default='')
    args = parser.parse_args()
    command = ['cargo', 'certora-sbf', '--json', '--tools-version', 'v1.53', '--arch', args.arch]
    if args.cargo_features.strip():
        command.extend(['--features', args.cargo_features])
    result = subprocess.run(command, cwd=Path(__file__).resolve().parents[2], capture_output=True, text=True)
    sys.stderr.write(result.stderr)
    if result.returncode:
        return result.returncode
    summary = json.loads(result.stdout)
    if not summary.get('success'):
        raise ValueError('Certora compiler did not report a successful build')
    artifact = Path(summary['project_directory']) / summary['executables']
    with contextlib.redirect_stdout(sys.stderr):
        check(artifact, finalize=True)
    # Preserve the compiler's source/inlining/summary metadata for the prover.
    print(result.stdout, end='')
    return 0


if __name__ == '__main__':
    sys.exit(main())
