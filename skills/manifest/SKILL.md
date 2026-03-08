---
name: manifest
description: Use this skill when building, debugging, or integrating with the Manifest DEX on Solana, especially for TypeScript SDK usage, transaction construction with `ManifestClient`, market state reads via `Market`, order/account-model decisions, and frontend patterns derived from the `manifest.trade` web app.
---

# Manifest Skill

## Use This Skill When

- A task touches Manifest orderbook integrations, trading flows, or market reads.
- You need to add or update TypeScript code using the Manifest SDK.
- You need to build or adapt a frontend based on patterns used by the `manifest.trade` web app.
- You need guidance on order types, wrapper/global accounts, or setup flows.
- You need Bonasa-Tech Manifest repo commands for validation or implementation details.

## First Steps

Target surfaces:

- Primary guidance in this skill folder: `references/manifest-actions.md` and `references/manifest-sdk.md`
- Frontend architecture patterns: `references/manifest-ui.md`

If working inside the Bonasa-Tech `manifest` repo:

- SDK client changes: `client/ts/src`
- SDK tests/examples: `client/ts/tests`
- Rust AMM interface: `client/rust/src`
- Program/runtime behavior: `programs/`

Reference files:

- Action selection: `references/manifest-actions.md`
- API quick map: `references/manifest-sdk.md`
- Frontend integration map: `references/manifest-ui.md`

## Standard Workflow

1. Identify whether the change is read-only market access (`Market`) or transaction-building (`ManifestClient`).
2. For transaction changes, confirm setup path (`getSetupIxs`, seat/wrapper assumptions) before placing/canceling/withdrawing.
3. If the task touches a frontend, follow the UI split described in `references/manifest-ui.md`.
4. Prefer composing instructions and returning them to caller boundaries (UI/bot layers decide send/sign).
5. Validate with the smallest relevant test or command before broad test runs.

## UI Conventions

- Keep market metadata and ticker caching centralized in a single shared provider/context.
- Use `ManifestClient.getClientReadOnly(...)` for anonymous or pre-setup UI reads.
- In wallet-connected flows, check `ManifestClient.getSetupIxs(...)` before assuming wrapper/seat state, then use `getClientForMarketNoPrivateKey(...)` when setup is complete.
- For orderbook display and price discovery in the UI, prefer `bidsL2()` / `asksL2()` after `market.reload(connection)` rather than reconstructing prices manually.
- Reuse shared browser caches before adding new fetch loops. The current web app caches market tickers, metadata, and notional volume in `sessionStorage`.
- Reuse a shared transaction submission helper instead of open-coding wallet send behavior, especially if mobile wallet adapters are in scope.
- For live fills/history in the frontend, combine a REST backfill path with an optional websocket feed instead of assuming websocket-only delivery.

## Validation Commands

- If working in the Bonasa-Tech `manifest` repo, TypeScript client tests (local validator flow):
```bash
sh local-validator-test.sh
```

- If working in the Bonasa-Tech `manifest` repo, program tests:
```bash
cargo test-sbf
```

- If working in the Bonasa-Tech `manifest` repo, build program:
```bash
cargo build-sbf
```

## Output Expectations

- Reference exact files changed.
- Keep behavior notes explicit for deposits, order placement, cancels, and withdrawals.
- If assumptions are required (market address, token mints, signer model), state them clearly.
