//! Program entrypoint tuned for compute units.
//!
//! `solana_program::entrypoint!` decodes the runtime's serialized input buffer
//! into a heap allocated `Vec<AccountInfo>`, reading every account field
//! through a running byte offset and dropping the whole `Vec` (and every `Rc`
//! in it) again when the instruction returns. This module produces the very
//! same `solana_program::account_info::AccountInfo` values, so nothing else in
//! the program changes, but:
//!
//! * accounts are written straight into a fixed-size stack array (no `Vec`,
//!   no drop at the end),
//! * each account header is read as one `#[repr(C)]` struct at a known
//!   address instead of field-by-field offset arithmetic,
//! * the only work that is left per account is the two `Rc<RefCell<_>>`
//!   allocations that the `AccountInfo` type itself requires, and those go
//!   through [`BumpAllocator`], a bump allocator whose allocation path is
//!   about half the instructions of the `solana_program` default.
//!
//! Those two allocations are the floor for this account type, and they are
//! most of what is left. Measured with the account sweep in
//! `tests/cases/cu.rs`, this entrypoint costs 185 CU plus 72 per account; a
//! variant that writes one raw pointer per account instead of the two
//! `Rc<RefCell<_>>` costs 14 per account. So about 58 CU per account, four
//! fifths of the per-account cost, is the `AccountInfo` type rather than the
//! parsing.
//!
//! Reaching that floor needs a zero-copy account type such as pinocchio's
//! `AccountInfo`, which is a single pointer into this same input buffer with
//! the fields read through it on demand. That is not a drop-in replacement:
//! it is a different type, so every processor, every wrapper in `validation/`,
//! every CPI helper and the test fixtures would have to be written against it.
//! The usual way around that is to port only the hottest instructions: keep a
//! parallel zero-copy implementation of those handlers, their state and their
//! CPIs, and dispatch on the instruction discriminant before deserializing, so
//! everything else falls through to the ordinary path.
//!
//! That trade pays off when the hot instructions carry a dozen accounts each
//! and what it deletes is a framework's per-account deserialization and
//! validation, which costs far more than the 58 CU per account here. Neither
//! holds for manifest: its instructions carry 3 to 6 accounts and the loaders
//! in `validation/` are already thin. The saving would be about 290 CU on a
//! five account wrapper batch update and 175 on the three account core batch
//! update it calls, so under 500 CU on a transaction that costs about 27,000,
//! in exchange for a second implementation of every ported instruction to keep
//! correct and audited. If one instruction ever dominates the budget, porting
//! only that one behind a discriminant check is the shape to reach for.
//!
//! Instructions with more than [`MAX_ACCOUNTS`] accounts fall back to the
//! stock `solana_program` deserializer, so behavior is identical for every
//! input.

use solana_program::{
    account_info::AccountInfo, entrypoint::MAX_PERMITTED_DATA_INCREASE, pubkey::Pubkey,
};
use std::{
    alloc::{GlobalAlloc, Layout},
    cell::RefCell,
    mem::{size_of, MaybeUninit},
    ptr::{addr_of_mut, null_mut},
    rc::Rc,
    slice::{from_raw_parts, from_raw_parts_mut},
};

/// Most accounts that are deserialized onto the stack. Manifest instructions
/// take at most 14 accounts, the wrapper 15 and the ui wrapper 20. Same bound
/// as `solana_program::entrypoint_no_alloc!`. Larger instructions still work,
/// they just take the heap allocating path.
pub const MAX_ACCOUNTS: usize = 64;

/// Program instruction processor signature, same as `solana_program`'s.
pub type ProcessInstruction =
    fn(&Pubkey, &[AccountInfo], &[u8]) -> solana_program::entrypoint::ProgramResult;

/// Value of `RuntimeAccount::dup_info` when the account is not a duplicate.
const NON_DUP_MARKER: u8 = u8::MAX;

/// The runtime aligns the start of every account (and the trailing rent epoch)
/// to this many bytes.
const BPF_ALIGN_OF_U128: usize = 8;

/// Fixed-size header the runtime serializes ahead of each non-duplicate
/// account's data. Layout is defined by the SBF ABI (see
/// `solana_program_entrypoint::deserialize`).
#[repr(C)]
struct RuntimeAccount {
    /// [`NON_DUP_MARKER`], or the index of the earlier account this duplicates.
    dup_info: u8,
    is_signer: u8,
    is_writable: u8,
    executable: u8,
    /// Runtime padding that the entrypoint fills with the account's original
    /// data length. `AccountInfo::realloc` reads it back from the four bytes
    /// immediately before `key`.
    original_data_len: u32,
    key: Pubkey,
    owner: Pubkey,
    lamports: u64,
    data_len: u64,
}

const _: () = assert!(size_of::<RuntimeAccount>() == 88);

/// Declares the program `entrypoint` symbol, the heap allocator and the
/// default panic handler, like `solana_program::entrypoint!`, but using
/// [`process_entrypoint`] for the deserialization and [`BumpAllocator`] as the
/// heap. As with `solana_program`, a program that enables its own
/// `custom-heap` feature keeps its own allocator.
#[macro_export]
macro_rules! entrypoint {
    ($process_instruction:ident) => {
        /// # Safety
        ///
        /// Only called by the Solana runtime with its serialized input buffer.
        #[no_mangle]
        pub unsafe extern "C" fn entrypoint(input: *mut u8) -> u64 {
            #[cfg(all(not(feature = "custom-heap"), target_os = "solana"))]
            $crate::entrypoint::BumpAllocator::init();
            $crate::entrypoint::process_entrypoint(input, $process_instruction)
        }
        #[cfg(all(not(feature = "custom-heap"), target_os = "solana"))]
        #[global_allocator]
        static A: $crate::entrypoint::BumpAllocator = $crate::entrypoint::BumpAllocator;
        solana_program::custom_panic_default!();
    };
}

/// Bump allocator over the program heap, like `solana_program`'s default
/// allocator: memory is never freed, `realloc` is allocate-and-copy. The
/// differences are that the cursor is initialized once by the entrypoint
/// instead of being checked on every allocation, and that it grows upward so
/// an allocation is an align, an add and a bounds check.
///
/// The cursor lives in the first word of the heap, so the usable heap is
/// `HEAP_LENGTH - 8` bytes, the same as `solana_program`'s allocator.
pub struct BumpAllocator;

/// Address of the allocation cursor: the first word of the heap.
const HEAP_CURSOR: *mut usize = solana_program::entrypoint::HEAP_START_ADDRESS as *mut usize;
/// First usable heap byte, just after the cursor.
const HEAP_BOTTOM: usize =
    solana_program::entrypoint::HEAP_START_ADDRESS as usize + size_of::<usize>();
/// One past the last usable heap byte.
const HEAP_TOP: usize = solana_program::entrypoint::HEAP_START_ADDRESS as usize
    + solana_program::entrypoint::HEAP_LENGTH;

impl BumpAllocator {
    /// Resets the allocation cursor. Must run before the first allocation, so
    /// [`entrypoint!`] calls it first thing.
    ///
    /// # Safety
    ///
    /// Only valid on the Solana target where the heap region exists.
    #[inline(always)]
    pub unsafe fn init() {
        *HEAP_CURSOR = HEAP_BOTTOM;
    }
}

unsafe impl GlobalAlloc for BumpAllocator {
    #[inline(always)]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let start: usize = (*HEAP_CURSOR).wrapping_add(layout.align() - 1) & !(layout.align() - 1);
        // `start` is a heap address (< 2^35) and `Layout` sizes are at most
        // `isize::MAX`, so this cannot wrap.
        let end: usize = start.wrapping_add(layout.size());
        if end > HEAP_TOP {
            return null_mut();
        }
        *HEAP_CURSOR = end;
        start as *mut u8
    }

    #[inline(always)]
    unsafe fn dealloc(&self, _: *mut u8, _: Layout) {
        // Bump allocator: never frees.
    }
}

/// Deserializes the runtime input buffer at `input` and runs
/// `process_instruction` on it, returning the runtime status code.
///
/// # Safety
///
/// `input` must be the buffer the runtime hands to the program `entrypoint`.
#[inline(always)]
pub unsafe fn process_entrypoint(input: *mut u8, process_instruction: ProcessInstruction) -> u64 {
    let num_accounts: usize = *(input as *const u64) as usize;
    if num_accounts > MAX_ACCOUNTS {
        return process_entrypoint_on_heap(input, process_instruction);
    }

    let mut accounts: [MaybeUninit<AccountInfo>; MAX_ACCOUNTS] =
        [const { MaybeUninit::<AccountInfo>::uninit() }; MAX_ACCOUNTS];
    let (program_id, instruction_data) =
        deserialize(input.add(size_of::<u64>()), num_accounts, &mut accounts);
    let accounts: &[AccountInfo] =
        from_raw_parts(accounts.as_ptr() as *const AccountInfo, num_accounts);

    call_process_instruction(program_id, accounts, instruction_data, process_instruction)
}

/// Kept out of line so the account array above does not share a stack frame
/// with the instruction processors.
#[inline(never)]
fn call_process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
    process_instruction: ProcessInstruction,
) -> u64 {
    match process_instruction(program_id, accounts, instruction_data) {
        Ok(()) => solana_program::entrypoint::SUCCESS,
        Err(error) => error.into(),
    }
}

/// Stock `Vec` based deserialization, used only when an instruction carries
/// more than [`MAX_ACCOUNTS`] accounts.
#[cold]
#[inline(never)]
unsafe fn process_entrypoint_on_heap(
    input: *mut u8,
    process_instruction: ProcessInstruction,
) -> u64 {
    let (program_id, accounts, instruction_data) = solana_program::entrypoint::deserialize(input);
    match process_instruction(program_id, &accounts, instruction_data) {
        Ok(()) => solana_program::entrypoint::SUCCESS,
        Err(error) => error.into(),
    }
}

/// Decodes `num_accounts` accounts starting at `input` (which must point just
/// past the leading account count) into `accounts`, then returns the program
/// id and instruction data that follow them.
///
/// # Safety
///
/// `input` must point into the runtime input buffer at the first account and
/// `num_accounts` must not exceed `accounts.len()`.
#[inline(always)]
unsafe fn deserialize<'a>(
    mut input: *mut u8,
    num_accounts: usize,
    accounts: &mut [MaybeUninit<AccountInfo<'a>>; MAX_ACCOUNTS],
) -> (&'a Pubkey, &'a [u8]) {
    // Three shapes taken from zero-copy deserializers were measured here and
    // not kept, so that nobody has to run the experiment again:
    //
    // * Peeling the first account out of the loop, which is sound because a
    //   duplicate marker always names an earlier account, is worth 2 CU per
    //   instruction. That does not pay for duplicating the account reading
    //   body, and doing it with a flag tested in the loop instead costs 1.5
    //   CU per account.
    // * Replacing `align_offset` below with `(ptr + 7) & !7` costs 7 CU per
    //   account: the integer round trip generates worse code than the pointer
    //   intrinsic on SBPF.
    // * Unrolling the loop cannot be worth much either, since peeling showed
    //   the whole per-iteration bookkeeping is about 2 CU.
    let first: *mut AccountInfo<'a> = accounts.as_mut_ptr() as *mut AccountInfo<'a>;
    let end: *mut AccountInfo<'a> = first.add(num_accounts);
    let mut dst: *mut AccountInfo<'a> = first;

    while dst != end {
        let account: *mut RuntimeAccount = input as *mut RuntimeAccount;
        let dup_info: u8 = (*account).dup_info;
        if dup_info == NON_DUP_MARKER {
            let data_len: usize = (*account).data_len as usize;
            (*account).original_data_len = data_len as u32;
            let data: *mut u8 = input.add(size_of::<RuntimeAccount>());

            // Each field is written as soon as it is decoded so that few
            // values are live at once (the SBF target has 10 registers). The
            // runtime serializes the three flags as 0 or 1; masking keeps the
            // conversion branchless, which is 9 CU per account cheaper than
            // `!= 0`.
            addr_of_mut!((*dst).key).write(&(*account).key);
            addr_of_mut!((*dst).owner).write(&(*account).owner);
            addr_of_mut!((*dst).is_signer).write(((*account).is_signer & 1) != 0);
            addr_of_mut!((*dst).is_writable).write(((*account).is_writable & 1) != 0);
            // Nothing in these programs reads `executable`, and writing a
            // constant `false` here instead of reading it measures 2 CU per
            // account cheaper. Not taken: the flag would then be wrong in
            // every `AccountInfo` the program hands out, including the array
            // the runtime translates when one of them is passed to a CPI, and
            // 2 CU per account does not buy that.
            addr_of_mut!((*dst).executable).write(((*account).executable & 1) != 0);
            addr_of_mut!((*dst).lamports).write(Rc::new(RefCell::new(&mut *addr_of_mut!(
                (*account).lamports
            ))));
            addr_of_mut!((*dst).data)
                .write(Rc::new(RefCell::new(from_raw_parts_mut(data, data_len))));

            // Skip the data, the realloc padding, alignment and the rent epoch.
            input = data.add(data_len).add(MAX_PERMITTED_DATA_INCREASE);
            input = input.add(input.align_offset(BPF_ALIGN_OF_U128));
            addr_of_mut!((*dst).rent_epoch).write(*(input as *const u64));
            input = input.add(size_of::<u64>());
        } else {
            // Duplicate: one marker byte plus seven bytes of padding, and the
            // `AccountInfo` shares the `Rc`s of the account it duplicates.
            input = input.add(size_of::<u64>());
            dst.write((*first.add(dup_info as usize)).clone());
        }
        dst = dst.add(1);
    }

    let instruction_data_len: usize = *(input as *const u64) as usize;
    input = input.add(size_of::<u64>());
    let instruction_data: &[u8] = from_raw_parts(input, instruction_data_len);
    let program_id: &Pubkey = &*(input.add(instruction_data_len) as *const Pubkey);

    (program_id, instruction_data)
}
