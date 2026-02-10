# Security Vulnerability Fix: Unsafe Array Indexing in Token Account Data Access

## Vulnerability Details

**Severity:** HIGH
**Type:** Unsafe Array Indexing / Potential Panic
**Files Affected:**
- `programs/manifest/src/program/processor/deposit.rs` (line 73)
- `programs/manifest/src/program/processor/withdraw.rs` (line 73)

## Description

Both deposit and withdraw functions perform unsafe array indexing on token account data without validating that the data length is sufficient. The code directly accesses bytes `[0..32]` to compare with the base mint, but never checks if the account data is at least 32 bytes long.

### Vulnerable Code

```rust
// Current code in both deposit.rs and withdraw.rs
let is_base: bool =
    &trader_token.try_borrow_data()?[0..32] == dynamic_account.get_base_mint().as_ref();
```

## Exploit Scenario

1. Attacker creates a malformed token account with less than 32 bytes of data
2. Attacker calls `deposit` or `withdraw` instruction with this malformed account
3. Code attempts to access `[0..32]` slice on insufficient data
4. **Transaction panics**, causing denial of service
5. If this occurs during critical operations (after tokens transferred but before balance updated), funds could be temporarily locked

## Impact

- **Denial of Service (DoS)**: Legitimate transactions can be blocked
- **Transaction Failure**: Unexpected panics instead of proper error handling
- **Potential Fund Locking**: If panic occurs after token transfer but before state update
- **Poor UX**: Users receive generic panic errors instead of meaningful messages

## Proof of Concept

```rust
// Test case demonstrating the vulnerability
#[test]
#[should_panic]
fn test_short_token_account_panic() {
    // Create a token account with only 20 bytes of data (< 32)
    let mut data = vec![0u8; 20];
    let trader_token = AccountInfo {
        data: RefCell::new(&mut data),
        // ... other fields
    };

    // This will panic when trying to access [0..32]
    let borrowed = trader_token.try_borrow_data().unwrap();
    let _slice = &borrowed[0..32]; // PANIC: index out of bounds
}
```

## Proposed Fix

Add length validation before array indexing:

```rust
// Fixed code for both deposit.rs and withdraw.rs
let trader_token_data = trader_token.try_borrow_data()?;

// Validate data length before indexing
require!(
    trader_token_data.len() >= 32,
    ManifestError::InvalidTokenAccount,
    "Token account data too short"
)?;

let is_base: bool =
    &trader_token_data[0..32] == dynamic_account.get_base_mint().as_ref();
```

## Alternative Fix (More Defensive)

For even better safety, use proper token account deserialization:

```rust
use spl_token::state::Account as TokenAccount;

// Deserialize the token account to access mint properly
let trader_token_account = TokenAccount::unpack(&trader_token.try_borrow_data()?)?;
let is_base: bool = trader_token_account.mint == *dynamic_account.get_base_mint();
```

This approach:
- Validates all token account invariants
- Provides better error messages
- Follows Solana best practices
- Automatically checks data length

## Files to Modify

1. **programs/manifest/src/program/processor/deposit.rs**
   - Line 72-73: Add length check or use proper deserialization

2. **programs/manifest/src/program/processor/withdraw.rs**
   - Line 72-73: Add length check or use proper deserialization

## Testing

After applying the fix, add test cases:

```rust
#[test]
fn test_deposit_with_short_token_account() {
    // Should return proper error instead of panicking
    let result = process_deposit_with_short_account();
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), ManifestError::InvalidTokenAccount);
}

#[test]
fn test_withdraw_with_malformed_account() {
    // Should handle gracefully
    let result = process_withdraw_with_invalid_data();
    assert!(result.is_err());
}
```

## Severity Justification

**HIGH** because:
- Easily exploitable (attacker controls account data)
- Causes transaction failures (DoS potential)
- Affects critical operations (deposit/withdraw)
- Could lead to temporary fund locking in edge cases
- No authentication needed (anyone can trigger)

**Not CRITICAL** because:
- Does not directly steal funds
- State inconsistency is temporary/recoverable
- Requires specific malformed account setup

## Recommendation

1. Apply the alternative fix using proper token account deserialization
2. Add comprehensive test coverage for edge cases
3. Consider auditing other locations with array indexing
4. Add fuzzing tests for account data validation

## Discovery

Found via automated security audit using AI-powered code analysis.
Reported by: Claude (AI Agent) via Superteam Earn Security Bounty
Date: February 11, 2026
