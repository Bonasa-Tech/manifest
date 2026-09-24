# Release notes

## Security hardening (`cyr/security`)

### Wrapper behavior changes

- The wrapper retains the deployed PostOnly behavior: it skips expired makers
  while checking the top of book and silently removes a crossing PostOnly
  replacement before invoking the core. This keeps a stale replacement quote
  from rolling back cancellations and other valid work in the same batch. The
  core remains authoritative for every order the wrapper forwards.
- `cancel_all` cancels wrapper-tracked orders and restores the deployed
  wrapper's full core-book search for the trader's orders placed directly
  through the core. This preserves replacement-order funding behavior, while
  very large shared books can consume substantial CU. `cancelAllScanCursor`
  remains reserved for byte-layout compatibility.

### Solana transaction v1 readiness

- All offchain transaction reads now opt in to transaction v1
  (`maxSupportedTransactionVersion: 1`) and the pinned `@solana/web3.js` is
  raised to 1.99.0, the first release that can decode a v1 message. Without
  both changes every v1 transaction fails with RPC error -32015, and a single
  v1 transaction fails an entire `getBlock` response.
- The fill feed now skips a transaction it cannot decode instead of letting the
  error unwind the polling loop. The previous behavior resumed from the newest
  signature, silently dropping every fill in the skipped window.

### Stats API behavior changes

- Wallet-only `/completeFills` requests without an explicit `fromSlot` are
  bounded to one day. Responses now include `effectiveSlotRange` so callers
  can see the exact bounds and paginate historical ranges explicitly.
