# Release notes

## Security hardening (`cyr/security`)

### Wrapper behavior changes

- PostOnly crossing validation is now exclusively authoritative in the core
  program. If any PostOnly order crosses when a wrapper batch lands, the core
  returns `PostOnlyCrosses` and the entire transaction rolls back, including
  cancellations and other replacement orders in that batch. Market makers
  that require cancellations to make progress independently of replacement
  quotes should submit cancellation-only and placement transactions
  separately.
- The wrapper market-info property formerly exposed as `lastUpdatedSlot` is
  now `cancelAllScanCursor`. Its on-chain bytes have represented a bounded
  cancel-all scan cursor since that feature was introduced, not a freshness
  slot. Removing the old source-level name makes stale integrations fail at
  compile time instead of silently interpreting a byte offset as a slot.

### Stats API behavior changes

- Wallet-only `/completeFills` requests without an explicit `fromSlot` are
  bounded to one day. Responses now include `effectiveSlotRange` so callers
  can see the exact bounds and paginate historical ranges explicitly.
