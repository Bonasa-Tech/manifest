# Codex Security Scan Follow-up — 2026-08-05

- Repository revision scanned: `e2b748e195f8d718ca152c45a30fa555d2813dce`
- Scan ID: `6ef5391e-939d-4b60-bd98-b25400daa3f5`
- Result: complete; 13 findings (2 medium, 11 low)

The complete scan report and machine-readable artifacts are retained outside
this repository in `../manifest-codex-security/`. This record identifies the
findings addressed by the accompanying `cyr/audit` change.

| Finding | Resolution |
| --- | --- |
| Unbounded orderbook RPC work | Refuse unknown markets in the public orderbook path. |
| Large trader responses | Enforce a 1–1,000 request limit. |
| Mutable credentialed CI action | Pin the action to an immutable commit. |
| Global loader lacks identity checks | Require Manifest ownership, minimum length, and discriminant. |
| Wrapper initialization can claim a pre-created account | Require the wrapper-state account signature. |
| Mutable pull-request action | Pin the action to an immutable commit. |
| Slim parser accepts inconsistent trees | Validate red/black invariants, reachability, overlap, and order bytes. |
| Invalid order type bytes | Reject values outside the defined order-type range. |
| Unauthenticated Solana installer | Download over HTTPS with a version-specific SHA-256 check. |
| Unbound wrapper accounts in stats | Validate wrapper owner, discriminant, and length before caching. |
| Mutable release inputs | Use a content-addressed verifier image. |
| Jupiter reads unauthenticated market data | Validate owner, discriminant, length, and orderbook links before traversal. |
| Recursive TypeScript tree validation | Use an explicit iterative validation stack. |

The Certora job also explicitly requests `id-token: write`; this preserves
compatibility with the pinned Certora action's GitHub App/OIDC integration.
