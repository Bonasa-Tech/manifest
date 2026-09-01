# Release notes

## Security hardening (`cyr/security`)

### Wrapper behavior changes

- PostOnly crossing validation is now exclusively authoritative in the core
  program. If any PostOnly order crosses when a wrapper batch lands, the core
  returns `PostOnlyCrosses` and the entire transaction rolls back, including
  cancellations, other replacement orders, and any `cancel_all` scan-cursor
  progress in that batch. During a fast market move this can leave stale
  quotes resting. Market makers that require cancellations to make progress
  independently of replacement quotes should submit cancellation-only and
  placement transactions separately.
- The wrapper market-info property formerly exposed as `lastUpdatedSlot` is
  now `cancelAllScanCursor`. This release repurposes the field from the last
  sync slot to a bounded cancel-all cursor without changing its on-chain byte
  layout. Pre-upgrade slot values fail cursor validation and safely restart at
  byte offset zero. Removing the old source-level name makes stale
  integrations fail at compile time instead of silently interpreting the new
  byte offset as a slot.

### Stats API behavior changes

- Wallet-only `/completeFills` requests without an explicit `fromSlot` are
  bounded to one day. Responses now include `effectiveSlotRange` so callers
  can see the exact bounds and paginate historical ranges explicitly.
