---
name: manifest
description: Use this skill when building, debugging, or integrating with the Manifest DEX on Solana, especially for TypeScript SDK usage in `manifest/client/ts`, frontend patterns derived from the `manifest.trade` web app, transaction construction with `ManifestClient`, market state reads via `Market`, and repo-specific validation/testing workflows.
---

# Manifest Skill

## Use This Skill When

- A task touches Manifest orderbook integrations, trading flows, or market reads.
- You need to add or update TypeScript code using the Manifest SDK.
- You need to build or adapt a frontend based on patterns used by the `manifest.trade` web app.
- You need repo-accurate commands for testing Manifest client or program changes.

## First Steps

Target surfaces:

- SDK client changes: `client/ts/src`
- SDK tests/examples: `client/ts/tests`
- Rust AMM interface: `client/rust/src`
- Program/runtime behavior: `programs/`
- Frontend architecture patterns: `references/manifest-ui.md`

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

- TypeScript client tests (local validator flow):
```bash
sh local-validator-test.sh
```

- Program tests:
```bash
cargo test-sbf
```

- Build program:
```bash
cargo build-sbf
```

## Output Expectations

- Reference exact files changed.
- Keep behavior notes explicit for deposits, order placement, cancels, and withdrawals.
- If assumptions are required (market address, token mints, signer model), state them clearly.
