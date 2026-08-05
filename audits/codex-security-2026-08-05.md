# Codex Security Scan — 2026-08-05

- Repository revision: `8402c1658179bc3956cb0f7ba40af8ebc861485e`
- Scan ID: `89e16242-77ab-470a-8a34-c937745001e4`
- Mode: standard whole-repository scan (563 files)
- Result: complete
- Findings: 9 (2 medium, 7 low; all high confidence)

## Findings

1. **Medium** — Public analytics routes allow unbounded database and application work.
2. **Medium** — The OKX adapter reverses full-fill and partial-fill quote math.
3. **Low** — Malformed global-account bytes can panic the Jupiter quote adapter.
4. **Low** — The TypeScript SDK accepts non-Market accounts as markets.
5. **Low** — Credentialed CI jobs execute supplier-mutable action references.
6. **Low** — The slim market parser accepts unvalidated best-order indices.
7. **Low** — CI executes the Solana installer without artifact verification.
8. **Low** — Release workflows publish artifacts built by mutable dependencies.
9. **Low** — The trade verifier trusts unbound logs and suppresses failed markets.

The full local scan artifacts (report, SARIF, JSON findings, coverage, and scan
manifest) were produced under `../manifest-codex-security` when this record was
created. The security-hardening changes are included in the same audit commit.
