# Release notes

## Security hardening (`cyr/security`)

### Wrapper behavior changes

- PostOnly crossing validation is now exclusively authoritative in the core
  program. If any PostOnly order crosses when a wrapper batch lands, the core
  returns `PostOnlyCrosses` and the entire transaction rolls back, including
  cancellations and other replacement orders in that batch. During a fast
  market move this can leave stale
  quotes resting. Market makers that require cancellations to make progress
  independently of replacement quotes should submit cancellation-only and
  placement transactions separately.
- `cancel_all` only cancels orders tracked by the wrapper. It deliberately does
  not scan the entire shared market for orders placed directly through the core
  program; that fallback caused a large tail-compute regression. Direct orders
  remain cancellable by sequence number/index or with `cancelAllOnCoreIx()`.
  The `cancelAllScanCursor` market-info field remains reserved solely to retain
  the existing on-chain byte layout.

### Stats API behavior changes

- Wallet-only `/completeFills` requests without an explicit `fromSlot` are
  bounded to one day. Responses now include `effectiveSlotRange` so callers
  can see the exact bounds and paginate historical ranges explicitly.
