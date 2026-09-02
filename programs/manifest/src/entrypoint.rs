//! How many accounts an instruction is deserialized onto the stack for.
//!
//! This module used to hold a hand written entrypoint: `solana_program`'s
//! macro decoded the runtime's input into a heap allocated `Vec<AccountInfo>`,
//! reading every field through a running offset, and this replaced it with a
//! fixed size stack array and a bump allocator. It cost 185 CU plus 72 per
//! account, of which about 58 per account was the `Rc<RefCell<_>>` pairs the
//! `solana_program` account type itself requires, and the module's own notes
//! said reaching the floor below that needed a zero copy account type.
//!
//! pinocchio is that type, so the entrypoint here is now pinocchio's and all
//! that survives is the bound: 126 CU plus 47 per account, measured by
//! `entrypoint_only` in `tests/cases/cu.rs`.

/// Most accounts an instruction is deserialized for.
///
/// Manifest instructions take at most 14 accounts, the wrapper 15 and the ui
/// wrapper 20. Instructions carrying more than this are rejected by the
/// runtime rather than silently truncated.
pub const MAX_ACCOUNTS: usize = 64;
