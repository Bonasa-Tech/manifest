---
name: manifest
description: Use this skill when building, debugging, or integrating with the Manifest DEX on Solana, especially for TypeScript SDK usage, transaction construction with `ManifestClient`, market state reads via `Market`, and order/account-model decisions.
---

# Manifest Skill

## Use This Skill When

- A task touches Manifest orderbook integrations, trading flows, or market reads.
- You need to add or update TypeScript code using the Manifest SDK.
- You need guidance on order types, wrapper/global accounts, or setup flows.
- You need Bonasa-Tech Manifest repo commands for validation or implementation details.

## First Steps

Primary guidance in this skill folder:

- `references/manifest-sdk.md`

If working inside the Bonasa-Tech `manifest` repo:

- SDK client changes: `client/ts/src`
- SDK tests/examples: `client/ts/tests`
- Rust AMM interface: `client/rust/src`
- Program/runtime behavior: `programs/`

Reference files:

- API quick map: `references/manifest-sdk.md`

## Standard Workflow

1. Identify whether the change is read-only market access (`Market`) or transaction-building (`ManifestClient`).
2. For transaction changes, confirm setup path (`getSetupIxs`, seat/wrapper assumptions) before placing/canceling/withdrawing.
3. Prefer composing instructions and returning them to caller boundaries.
4. Validate with the smallest relevant test or command before broad test runs.

## Validation Commands

- If working in the Bonasa-Tech `manifest` repo, TypeScript client tests (local validator flow):
```bash
sh local-validator-test.sh
```

- If working in the Bonasa-Tech `manifest` repo, program tests:
```bash
cargo test-sbf
```

## Output Expectations

- Reference exact files changed.
- Keep behavior notes explicit for deposits, order placement, cancels, and withdrawals.
- If assumptions are required (market address, token mints, signer model), state them clearly.
