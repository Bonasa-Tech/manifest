#!/usr/bin/env python3
"""Select distinct, recently active Manifest markets for replay sampling."""

from __future__ import annotations

import argparse
import base64
import json
import os
import struct
import urllib.request
from pathlib import Path


PROGRAM = "MNFSTqtC93rEfYHB6hF82sKdZpUDFWkViLByLd1k1Ms"
MARKET_DISCRIMINANT = 4859840929024028656
BASE58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--rpc-url",
        default=os.environ.get("SOLANA_RPC_URL", "https://api.mainnet-beta.solana.com"),
    )
    parser.add_argument("--count", type=int, default=10)
    parser.add_argument("--candidate-pool", type=int, default=250)
    parser.add_argument("--signature-limit", type=int, default=100)
    parser.add_argument("--recent-slots", type=int, default=100_000)
    parser.add_argument("--minimum-successes", type=int, default=5)
    parser.add_argument("--allow-duplicate-pairs", action="store_true")
    parser.add_argument("--output", type=Path)
    return parser.parse_args()


def base58_encode(value: bytes) -> str:
    number = int.from_bytes(value, "big")
    encoded = ""
    while number:
        number, remainder = divmod(number, 58)
        encoded = BASE58[remainder] + encoded
    zeroes = len(value) - len(value.lstrip(b"\0"))
    return "1" * zeroes + (encoded or ("" if zeroes else "1"))


def request(url: str, payload: object) -> object:
    req = urllib.request.Request(
        url,
        json.dumps(payload).encode(),
        {"content-type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=120) as response:
        return json.load(response)


def rpc(url: str, method: str, params: list[object]) -> object:
    body = request(
        url,
        {"jsonrpc": "2.0", "id": 1, "method": method, "params": params},
    )
    if "error" in body:
        raise RuntimeError(f"RPC {method} failed: {body['error']}")
    return body["result"]


def market_headers(url: str) -> list[dict]:
    discriminator = base58_encode(struct.pack("<Q", MARKET_DISCRIMINANT))
    accounts = rpc(
        url,
        "getProgramAccounts",
        [
            PROGRAM,
            {
                "commitment": "finalized",
                "encoding": "base64",
                "filters": [{"memcmp": {"offset": 0, "bytes": discriminator}}],
                "dataSlice": {"offset": 0, "length": 192},
            },
        ],
    )
    rows = []
    for account in accounts:
        data = base64.b64decode(account["account"]["data"][0])
        rows.append(
            {
                "market": account["pubkey"],
                "baseMint": base58_encode(data[16:48]),
                "quoteMint": base58_encode(data[48:80]),
                "allocatedBytes": struct.unpack_from("<I", data, 152)[0],
                "quoteVolumeAtoms": struct.unpack_from("<Q", data, 184)[0],
            }
        )
    return rows


def candidate_markets(markets: list[dict], pool_size: int) -> list[dict]:
    selected = {
        row["market"]: row
        for row in sorted(
            markets, key=lambda row: row["allocatedBytes"], reverse=True
        )[:pool_size]
    }
    for row in sorted(
        markets, key=lambda row: row["quoteVolumeAtoms"], reverse=True
    )[:pool_size]:
        selected[row["market"]] = row
    return list(selected.values())


def add_activity(url: str, markets: list[dict], limit: int) -> None:
    for begin in range(0, len(markets), 50):
        batch = [
            {
                "jsonrpc": "2.0",
                "id": index,
                "method": "getSignaturesForAddress",
                "params": [
                    row["market"],
                    {"commitment": "finalized", "limit": limit},
                ],
            }
            for index, row in enumerate(markets[begin : begin + 50], begin)
        ]
        response = request(url, batch)
        if not isinstance(response, list):
            raise RuntimeError("RPC endpoint did not return a JSON batch response")
        by_id = {item["id"]: item for item in response}
        for index, row in enumerate(markets[begin : begin + 50], begin):
            item = by_id.get(index, {})
            if "error" in item:
                raise RuntimeError(
                    f"signature lookup for {row['market']} failed: {item['error']}"
                )
            signatures = item.get("result", [])
            successful_slots = [
                value["slot"] for value in signatures if value["err"] is None
            ]
            row["sampledSignatures"] = len(signatures)
            row["successfulSignatures"] = len(successful_slots)
            row["failedSignatures"] = len(signatures) - len(successful_slots)
            row["latestSuccessfulSlot"] = max(successful_slots, default=0)
            row["successfulSlotSpan"] = (
                max(successful_slots) - min(successful_slots)
                if len(successful_slots) > 1
                else None
            )
            divisor = max(1, row["successfulSlotSpan"] or 1)
            row["successfulTransactionsPerThousandSlots"] = (
                1000.0 * len(successful_slots) / divisor
            )


def select(args: argparse.Namespace) -> dict:
    current_slot = rpc(args.rpc_url, "getSlot", [{"commitment": "finalized"}])
    all_markets = market_headers(args.rpc_url)
    candidates = candidate_markets(all_markets, args.candidate_pool)
    add_activity(args.rpc_url, candidates, args.signature_limit)
    eligible = [
        row
        for row in candidates
        if row["successfulSignatures"] >= args.minimum_successes
        and row["latestSuccessfulSlot"] >= current_slot - args.recent_slots
    ]
    eligible.sort(
        key=lambda row: (
            -row["successfulTransactionsPerThousandSlots"],
            -row["successfulSignatures"],
            -row["latestSuccessfulSlot"],
            row["market"],
        )
    )
    chosen = []
    pairs = set()
    for row in eligible:
        pair = tuple(sorted((row["baseMint"], row["quoteMint"])))
        if not args.allow_duplicate_pairs and pair in pairs:
            continue
        chosen.append(row)
        pairs.add(pair)
        if len(chosen) == args.count:
            break
    if len(chosen) < args.count:
        raise RuntimeError(
            f"only {len(chosen)} markets met the activity and pair-diversity filters"
        )
    return {
        "selectionSlot": current_slot,
        "program": PROGRAM,
        "criteria": {
            "count": args.count,
            "candidatePoolPerSignal": args.candidate_pool,
            "signatureLimit": args.signature_limit,
            "recentSlots": args.recent_slots,
            "minimumSuccessfulSignatures": args.minimum_successes,
            "distinctUnorderedMintPairs": not args.allow_duplicate_pairs,
            "ranking": "successful transactions per 1000 slots in the sampled signatures",
        },
        "marketAccountCount": len(all_markets),
        "candidateCount": len(candidates),
        "markets": chosen,
    }


def main() -> None:
    args = parse_args()
    if args.count < 1 or args.candidate_pool < 1 or not 1 <= args.signature_limit <= 1000:
        raise SystemExit("count and candidate-pool must be positive; signature-limit is 1..1000")
    result = select(args)
    rendered = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered)
    else:
        print(rendered, end="")
    for row in result["markets"]:
        print(
            row["market"],
            row["baseMint"],
            row["quoteMint"],
            f"success={row['successfulSignatures']}/{row['sampledSignatures']}",
            f"success/1000-slots={row['successfulTransactionsPerThousandSlots']:.1f}",
        )


if __name__ == "__main__":
    main()
