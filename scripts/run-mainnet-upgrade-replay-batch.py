#!/usr/bin/env python3
"""Run resumable replay windows across a selected set of Manifest markets."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import threading
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path


OUTCOME_FIELDS = (
    "name",
    "success",
    "baseTokenDelta",
    "quoteTokenDelta",
    "orderSequenceDelta",
    "restingBidDelta",
    "restingAskDelta",
    "newRestingOrders",
)


def parse_args() -> argparse.Namespace:
    root = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser()
    parser.add_argument("--selection", type=Path, required=True)
    parser.add_argument("--new-program", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--rpc-url", default=os.environ.get("SOLANA_RPC_URL"))
    parser.add_argument("--runs-per-market", type=int, default=50)
    parser.add_argument("--slots", type=int, default=10)
    parser.add_argument("--concurrency", type=int, default=8)
    parser.add_argument("--attempts", type=int, default=3)
    parser.add_argument(
        "--binary", type=Path, default=root / "target/debug/manifest-replay"
    )
    return parser.parse_args()


def cache_only(report: dict) -> bool:
    if report["oldVsNew"]["equal"]:
        return True
    if any(
        account["address"] != report["market"]
        for account in report["changedWritableAccounts"]
    ):
        return False
    return all(
        192 <= byte_range["start"]
        and byte_range["endExclusive"] <= 256
        for byte_range in report["oldVsNew"]["ranges"]
    )


def outcome_mismatches(report: dict) -> int:
    old = report["old"]["instructionResults"]
    new = report["new"]["instructionResults"]
    if len(old) != len(new):
        return max(len(old), len(new))
    return sum(
        any(left[field] != right[field] for field in OUTCOME_FIELDS)
        for left, right in zip(old, new)
    )


def main() -> None:
    args = parse_args()
    if not args.rpc_url:
        raise SystemExit("--rpc-url or SOLANA_RPC_URL is required")
    for name in ("runs_per_market", "slots", "concurrency", "attempts"):
        if getattr(args, name) < 1:
            raise SystemExit(f"--{name.replace('_', '-')} must be positive")
    selection = json.loads(args.selection.read_text())
    markets = [row["market"] for row in selection["markets"]]
    if len(markets) != len(set(markets)):
        raise SystemExit("selection contains duplicate markets")
    if len({market[:8] for market in markets}) != len(markets):
        raise SystemExit("selection contains markets with colliding 8-character prefixes")
    binary = args.binary.resolve()
    candidate = args.new_program.resolve()
    if not binary.is_file() or not candidate.is_file():
        raise SystemExit("replay binary and candidate program must exist")

    args.output.mkdir(parents=True, exist_ok=True)
    logs = args.output / "logs"
    logs.mkdir(exist_ok=True)
    progress_path = args.output / "batch-progress.json"
    lock = threading.Lock()
    results: dict[str, dict] = {}

    def save_progress() -> None:
        summary = {
            "selection": str(args.selection),
            "newProgram": str(candidate),
            "marketCount": len(markets),
            "runsPerMarket": args.runs_per_market,
            "slotsPerRun": args.slots,
            "attempted": len(results),
            "completed": sum(
                row["status"] == "complete" for row in results.values()
            ),
            "failed": sum(row["status"] == "failed" for row in results.values()),
            "unexpected": sum(
                row.get("status") == "complete"
                and (not row["cacheOnly"] or row["outcomeMismatches"] != 0)
                for row in results.values()
            ),
            "runs": dict(sorted(results.items())),
        }
        temporary = progress_path.with_suffix(".tmp")
        temporary.write_text(json.dumps(summary, indent=2) + "\n")
        temporary.replace(progress_path)

    def run_market(market: str) -> None:
        short = market[:8]
        for run_number in range(1, args.runs_per_market + 1):
            key = f"{short}/{run_number:03d}"
            output = args.output / short / f"run-{run_number:03d}"
            report_path = output / "report.json"
            log_path = logs / f"{short}-{run_number:03d}.log"
            if report_path.is_file():
                report = json.loads(report_path.read_text())
                row = {
                    "status": "complete",
                    "report": str(report_path),
                    "cacheOnly": cache_only(report),
                    "outcomeMismatches": outcome_mismatches(report),
                }
            else:
                row = {"status": "failed", "report": str(report_path)}
                child_env = os.environ.copy()
                child_env["SOLANA_RPC_URL"] = args.rpc_url
                command = [
                    str(binary),
                    "run",
                    "--market",
                    market,
                    "--slots",
                    str(args.slots),
                    "--new-program",
                    str(candidate),
                    "--output",
                    str(output),
                ]
                for attempt in range(1, args.attempts + 1):
                    with log_path.open("w") as log:
                        log.write(f"attempt {attempt}/{args.attempts}\n")
                        completed = subprocess.run(
                            command,
                            env=child_env,
                            stdout=log,
                            stderr=subprocess.STDOUT,
                            check=False,
                        )
                    if completed.returncode == 0 and report_path.is_file():
                        report = json.loads(report_path.read_text())
                        row = {
                            "status": "complete",
                            "report": str(report_path),
                            "cacheOnly": cache_only(report),
                            "outcomeMismatches": outcome_mismatches(report),
                        }
                        break
                if row["status"] == "failed":
                    row["log"] = str(log_path)
            with lock:
                results[key] = row
                save_progress()
                if (
                    row["status"] == "failed"
                    or not row.get("cacheOnly", False)
                    or row.get("outcomeMismatches", 0) != 0
                ):
                    print(key, row, flush=True)
                elif run_number % 10 == 0:
                    print(f"{short}: {run_number}/{args.runs_per_market}", flush=True)

    with ThreadPoolExecutor(max_workers=min(args.concurrency, len(markets))) as executor:
        list(executor.map(run_market, markets))

    with lock:
        save_progress()
    progress = json.loads(progress_path.read_text())
    print(
        f"completed={progress['completed']} failed={progress['failed']} "
        f"unexpected={progress['unexpected']}",
        flush=True,
    )
    if progress["failed"] or progress["unexpected"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
