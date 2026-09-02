# Solana Certora Verification Tool (CVT) #

This workspace defines a set of functions and macros that can be
called from Solana programs to create non-deterministic environments,
add assumptions, check properties, call summaries, etc.

It includes the following crates:

- `vectors`: Define simpler implementions of `Vec` to be used during
  verification. Probably the most useful implementation is
  `NoResizableVec`.

- `cvt`: convenient macros (`cvt_assert!`, `cvt_assume!`,
  `cvt_satisfy!`, `cvt_vacuity_check!`, etc) to be used for writing
  specs.

 
- `cvt_macro`: definition of macro attribute `#[rule]`. This attribute
  tells the Solana prover that the function is a rule.
 
- `early_panic`: definition of macro attribute `#[early_panic]`. This
  macro replaces `?` with `unwrap`. It helps the prover by removing
  error paths.

- `hook_macro` - Defines macros for inserting hooks at the beginning and end of functions.

- `nondet`: Definition of `Nondet` trait for Rust types such as
  numbers, pointers, etc and also for common Solana specific types
  such as `AccountInfo`, `Pubkey`, `spl_token::state::Mint`, etc. You
  can also add `#[derive(Nondet)]` for simple user-defined Rust
  `struct`. The `Nondet` traits provides the function `nondet()`.

- `solana_cvt`: summaries for SPL Token instructions such as
  `transfer`, `mint_to`, and `burn`. It also contains mocks for
  `solana_program::Clock`.

- `calltrace` - Defines macros to print values in the Solana's prover
  calltrace. 
