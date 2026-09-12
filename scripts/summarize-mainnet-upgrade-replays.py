#!/usr/bin/env python3
"""Aggregate a directory of manifest-replay report.json files."""

from __future__ import annotations

import argparse
import json
from collections import Counter, defaultdict
from pathlib import Path


ALL_INSTRUCTIONS = (
    "CreateMarket",
    "ClaimSeat",
    "Deposit",
    "Withdraw",
    "Swap",
    "Expand",
    "BatchUpdate",
    "GlobalCreate",
    "GlobalAddTrader",
    "GlobalDeposit",
    "GlobalWithdraw",
    "GlobalEvict",
    "GlobalClean",
    "SwapV2",
)
MARKET_INSTRUCTIONS = (
    "CreateMarket",
    "ClaimSeat",
    "Deposit",
    "Withdraw",
    "Swap",
    "Expand",
    "BatchUpdate",
    "GlobalClean",
    "SwapV2",
)
EXISTING_MARKET_INSTRUCTIONS = tuple(
    name for name in MARKET_INSTRUCTIONS if name != "CreateMarket"
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=Path)
    parser.add_argument("--json", type=Path)
    parser.add_argument("--markdown", type=Path)
    return parser.parse_args()


def cache_only(report: dict) -> bool:
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


def classify_difference(report: dict) -> str:
    if report["oldVsNew"]["equal"]:
        return "identical"
    if not cache_only(report):
        return "unexpected"
    summary = report["new"]["marketSummary"]
    sides = [
        name
        for name, field in (("base", "baseGlobal"), ("quote", "quoteGlobal"))
        if summary[field] is not None
    ]
    return f"global-cache:{'+'.join(sides) or 'raw'}"


def aggregate(paths: list[Path]) -> dict:
    instruction_counts: Counter[str] = Counter()
    difference_classes: Counter[str] = Counter()
    snapshot_counts: Counter[str] = Counter()
    cu = defaultdict(lambda: {
        "count": 0,
        "oldSuccesses": 0,
        "newSuccesses": 0,
        "oldTotal": 0,
        "newTotal": 0,
    })
    markets = defaultdict(lambda: {
        "reports": 0,
        "activeReports": 0,
        "transactionsTouchingMarket": 0,
        "instructions": 0,
        "oldReplayVsChainMismatches": 0,
        "unexpectedDifferenceReports": 0,
        "oldBaseTokenDelta": 0,
        "newBaseTokenDelta": 0,
        "oldQuoteTokenDelta": 0,
        "newQuoteTokenDelta": 0,
        "oldNewRestingOrders": 0,
        "newNewRestingOrders": 0,
        "oldOrderSequenceDelta": 0,
        "newOrderSequenceDelta": 0,
        "oldRestingBidDelta": 0,
        "newRestingBidDelta": 0,
        "oldRestingAskDelta": 0,
        "newRestingAskDelta": 0,
    })
    reports = []
    instruction_outcome_mismatch_reports = 0
    for path in paths:
        report = json.loads(path.read_text())
        reports.append(report)
        market_summary = report["old"]["marketSummary"]
        market_key = (
            report["market"],
            market_summary["baseMint"],
            market_summary["quoteMint"],
        )
        market_row = markets[market_key]
        market_row["reports"] += 1
        market_row["activeReports"] += bool(report["instructionCounts"])
        market_row["transactionsTouchingMarket"] += report["transactionsTouchingMarket"]
        market_row["instructions"] += sum(report["instructionCounts"].values())
        market_row["oldReplayVsChainMismatches"] += not report["oldVsChain"]["equal"]
        market_row["unexpectedDifferenceReports"] += (
            classify_difference(report) == "unexpected"
        )
        for label in ("old", "new"):
            for result in report[label]["instructionResults"]:
                if result["baseTokenDelta"] is not None:
                    market_row[f"{label}BaseTokenDelta"] += result["baseTokenDelta"]
                if result["quoteTokenDelta"] is not None:
                    market_row[f"{label}QuoteTokenDelta"] += result["quoteTokenDelta"]
                market_row[f"{label}NewRestingOrders"] += len(result["newRestingOrders"])
                market_row[f"{label}OrderSequenceDelta"] += result["orderSequenceDelta"]
                market_row[f"{label}RestingBidDelta"] += result["restingBidDelta"]
                market_row[f"{label}RestingAskDelta"] += result["restingAskDelta"]
        instruction_counts.update(report["instructionCounts"])
        snapshot_counts.update(report["accountSnapshotCounts"])
        difference_classes[classify_difference(report)] += 1
        comparable_fields = (
            "name",
            "success",
            "baseTokenDelta",
            "quoteTokenDelta",
            "orderSequenceDelta",
            "restingBidDelta",
            "restingAskDelta",
            "newRestingOrders",
        )
        if len(report["old"]["instructionResults"]) != len(report["new"]["instructionResults"]):
            instruction_outcome_mismatch_reports += 1
        elif any(
            any(old[field] != new[field] for field in comparable_fields)
            for old, new in zip(
                report["old"]["instructionResults"],
                report["new"]["instructionResults"],
            )
        ):
            instruction_outcome_mismatch_reports += 1
        for name, comparison in report["computeUnitComparison"].items():
            row = cu[name]
            row["count"] += comparison["instructionCount"]
            row["oldSuccesses"] += comparison["oldSuccesses"]
            row["newSuccesses"] += comparison["newSuccesses"]
            row["oldTotal"] += comparison["oldTotalComputeUnits"]
            row["newTotal"] += comparison["newTotalComputeUnits"]

    for row in cu.values():
        row["delta"] = row["newTotal"] - row["oldTotal"]
        row["percentChange"] = (
            100.0 * row["delta"] / row["oldTotal"] if row["oldTotal"] else None
        )
        row["oldAverage"] = row["oldTotal"] / row["count"]
        row["newAverage"] = row["newTotal"] / row["count"]

    observed = set(instruction_counts)
    market_missing = [name for name in MARKET_INSTRUCTIONS if name not in observed]
    all_missing = [name for name in ALL_INSTRUCTIONS if name not in observed]
    active = sum(bool(report["instructionCounts"]) for report in reports)
    old_chain_mismatches = sum(not report["oldVsChain"]["equal"] for report in reports)
    unexpected = difference_classes["unexpected"]
    total_cu = {
        "count": sum(row["count"] for row in cu.values()),
        "oldSuccesses": sum(row["oldSuccesses"] for row in cu.values()),
        "newSuccesses": sum(row["newSuccesses"] for row in cu.values()),
        "oldTotal": sum(row["oldTotal"] for row in cu.values()),
        "newTotal": sum(row["newTotal"] for row in cu.values()),
    }
    total_cu["delta"] = total_cu["newTotal"] - total_cu["oldTotal"]
    total_cu["percentChange"] = (
        100.0 * total_cu["delta"] / total_cu["oldTotal"]
        if total_cu["oldTotal"]
        else None
    )
    total_cu["oldAverage"] = (
        total_cu["oldTotal"] / total_cu["count"] if total_cu["count"] else None
    )
    total_cu["newAverage"] = (
        total_cu["newTotal"] / total_cu["count"] if total_cu["count"] else None
    )
    market_rows = [
        {
            "market": market,
            "baseMint": base,
            "quoteMint": quote,
            **counts,
        }
        for (market, base, quote), counts in markets.items()
    ]
    market_rows.sort(key=lambda row: (-row["instructions"], row["market"]))
    return {
        "reports": len(reports),
        "activeReports": active,
        "idleReports": len(reports) - active,
        "uniqueMarkets": len(markets),
        "uniqueMintPairs": len({(base, quote) for _, base, quote in markets}),
        "markets": market_rows,
        "firstSlot": min(report["startSlot"] for report in reports),
        "lastSlot": max(report["endSlot"] for report in reports),
        "capturedSlotSpan": sum(
            report["endSlot"] - report["startSlot"] for report in reports
        ),
        "transactionsTouchingMarket": sum(
            report["transactionsTouchingMarket"] for report in reports
        ),
        "failedTransactionsSkipped": sum(
            report["failedTransactionsSkipped"] for report in reports
        ),
        "successfulTransactionsWithoutManifest": sum(
            report["successfulTransactionsWithoutManifest"] for report in reports
        ),
        "instructionCounts": dict(sorted(instruction_counts.items())),
        "computeUnitsByInstruction": dict(sorted(cu.items())),
        "computeUnitsTotal": total_cu,
        "differenceClasses": dict(sorted(difference_classes.items())),
        "oldReplayVsChainMismatches": old_chain_mismatches,
        "unexpectedDifferenceReports": unexpected,
        "instructionOutcomeMismatchReports": instruction_outcome_mismatch_reports,
        "accountSnapshotCounts": dict(sorted(snapshot_counts.items())),
        "coverage": {
            "allManifest": {
                "covered": len(observed.intersection(ALL_INSTRUCTIONS)),
                "total": len(ALL_INSTRUCTIONS),
                "missing": all_missing,
            },
            "marketTouching": {
                "covered": len(observed.intersection(MARKET_INSTRUCTIONS)),
                "total": len(MARKET_INSTRUCTIONS),
                "missing": market_missing,
            },
            "existingMarketTouching": {
                "covered": len(observed.intersection(EXISTING_MARKET_INSTRUCTIONS)),
                "total": len(EXISTING_MARKET_INSTRUCTIONS),
                "missing": [
                    name for name in EXISTING_MARKET_INSTRUCTIONS if name not in observed
                ],
            },
            "globalOnlyExcludedByMarketFilter": [
                name for name in ALL_INSTRUCTIONS if name not in MARKET_INSTRUCTIONS
            ],
        },
        "oldProgramSha256": sorted({report["old"]["programSha256"] for report in reports}),
        "newProgramSha256": sorted({report["new"]["programSha256"] for report in reports}),
    }


def markdown(summary: dict) -> str:
    lines = [
        "# Manifest replay aggregate",
        "",
        f"- Reports: {summary['reports']} ({summary['activeReports']} active, {summary['idleReports']} idle)",
        f"- Markets: {summary['uniqueMarkets']} across {summary['uniqueMintPairs']} ordered mint pairs",
        f"- Slots: {summary['firstSlot']}..{summary['lastSlot']} ({summary['capturedSlotSpan']} captured slot span)",
        f"- Market-touching transactions: {summary['transactionsTouchingMarket']}",
        f"- Failed transactions skipped: {summary['failedTransactionsSkipped']}",
        f"- Successful transactions with no selected-market Manifest instruction: {summary['successfulTransactionsWithoutManifest']}",
        f"- Deployed replay/chain mismatches: {summary['oldReplayVsChainMismatches']}",
        f"- Unexpected old/new difference reports: {summary['unexpectedDifferenceReports']}",
        f"- Per-instruction outcome mismatch reports: {summary['instructionOutcomeMismatchReports']}",
        "",
        "## Market distribution",
        "",
        "| Market | Base mint | Quote mint | Reports | Active | Transactions | Instructions | Old/chain mismatch | Unexpected old/new |",
        "| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]
    for row in summary["markets"]:
        lines.append(
            f"| {row['market']} | {row['baseMint']} | {row['quoteMint']} "
            f"| {row['reports']} | {row['activeReports']} "
            f"| {row['transactionsTouchingMarket']} | {row['instructions']} "
            f"| {row['oldReplayVsChainMismatches']} "
            f"| {row['unexpectedDifferenceReports']} |"
        )
    lines.extend([
        "",
        "## Market outcomes",
        "",
        "Token deltas are trader-account atoms. They are kept per market because different mint atoms cannot be summed meaningfully.",
        "",
        "| Market | Old base delta | New base delta | Old quote delta | New quote delta | Old/new new orders | Old/new sequence delta | Old/new bid delta | Old/new ask delta |",
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
    ])
    for row in summary["markets"]:
        lines.append(
            f"| {row['market']} | {row['oldBaseTokenDelta']} | {row['newBaseTokenDelta']} "
            f"| {row['oldQuoteTokenDelta']} | {row['newQuoteTokenDelta']} "
            f"| {row['oldNewRestingOrders']}/{row['newNewRestingOrders']} "
            f"| {row['oldOrderSequenceDelta']}/{row['newOrderSequenceDelta']} "
            f"| {row['oldRestingBidDelta']}/{row['newRestingBidDelta']} "
            f"| {row['oldRestingAskDelta']}/{row['newRestingAskDelta']} |"
        )
    lines.extend([
        "",
        "## Instruction and CU coverage",
        "",
        "| Instruction | Count | Old success | New success | Old CU | New CU | Delta | Change |",
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
    ])
    for name, row in summary["computeUnitsByInstruction"].items():
        percent = "n/a" if row["percentChange"] is None else f"{row['percentChange']:+.1f}%"
        lines.append(
            f"| {name} | {row['count']} | {row['oldSuccesses']} | {row['newSuccesses']} "
            f"| {row['oldTotal']} | {row['newTotal']} | {row['delta']:+d} | {percent} |"
        )
    row = summary["computeUnitsTotal"]
    total_percent = (
        "n/a" if row["percentChange"] is None else f"{row['percentChange']:+.1f}%"
    )
    lines.append(
        f"| **Total** | **{row['count']}** | **{row['oldSuccesses']}** | **{row['newSuccesses']}** "
        f"| **{row['oldTotal']}** | **{row['newTotal']}** | **{row['delta']:+d}** "
        f"| **{total_percent}** |"
    )
    market = summary["coverage"]["marketTouching"]
    existing = summary["coverage"]["existingMarketTouching"]
    all_types = summary["coverage"]["allManifest"]
    lines.extend([
        "",
        f"Market-touching coverage: **{market['covered']}/{market['total']}**. Missing: {', '.join(market['missing'])}.",
        "",
        f"Existing-market coverage: **{existing['covered']}/{existing['total']}**. Missing: {', '.join(existing['missing'])}.",
        "",
        f"All-enum coverage: **{all_types['covered']}/{all_types['total']}**. Missing: {', '.join(all_types['missing'])}.",
        "",
        "The global-only instructions excluded by the selected-market filter are: "
        + ", ".join(summary["coverage"]["globalOnlyExcludedByMarketFilter"])
        + ".",
        "",
        "## Final-state difference classes",
        "",
        "| Class | Reports |",
        "| --- | ---: |",
    ])
    for name, count in summary["differenceClasses"].items():
        lines.append(f"| {name} | {count} |")
    lines.append("")
    return "\n".join(lines)


def main() -> None:
    args = parse_args()
    paths = sorted(args.root.rglob("report.json"))
    if not paths:
        raise SystemExit(f"no report.json files found under {args.root}")
    summary = aggregate(paths)
    rendered_json = json.dumps(summary, indent=2) + "\n"
    rendered_markdown = markdown(summary)
    if args.json:
        args.json.write_text(rendered_json)
    else:
        print(rendered_json, end="")
    if args.markdown:
        args.markdown.write_text(rendered_markdown)


if __name__ == "__main__":
    main()
