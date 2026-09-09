use std::mem::size_of;

pub type DataIndex = u32;

/// Marker for zero-copy account layouts accepted by [`get_helper`].
///
/// # Safety
/// Every initialized byte pattern must be valid for the type. Unlike
/// `bytemuck::Pod`, this contract permits inert C-layout padding because these
/// helpers never expose the value as bytes.
pub unsafe trait Get: Copy {}

/// Read a struct of type T in an array of data at a given index.
///
/// This is a safe function: it range-checks `index` on every target, so an
/// out-of-range index is a clean panic rather than an out-of-bounds read. That
/// makes it sound for callers that supply arbitrary slices or indices, such as
/// native RPC buffers and instruction-data index hints.
///
/// Alignment: native builds still assert the effective address is aligned,
/// because a caller may pass a shifted slice. On Solana the base is the
/// runtime's eight-byte-aligned account data and every offset is formed from
/// eight-byte-aligned fixed headers and block sizes, and every `Get` type used
/// by the program has alignment at most eight, so alignment is only
/// debug-asserted there.
///
/// The hot data-structure walks, where the range check is measurable, use
/// [`get_helper_unchecked`] instead, under the allocator/tree invariants
/// documented at those call sites.
#[inline(always)]
pub fn get_helper<T: Get>(data: &[u8], index: DataIndex) -> &T {
    let index_usize: usize = index as usize;
    let bytes: &[u8] = &data[index_usize..index_usize + size_of::<T>()];
    #[cfg(not(target_os = "solana"))]
    assert_eq!((bytes.as_ptr() as usize) % std::mem::align_of::<T>(), 0);
    #[cfg(target_os = "solana")]
    debug_assert_eq!((bytes.as_ptr() as usize) % std::mem::align_of::<T>(), 0);
    // SAFETY: `Get` supplies the validity contract, the slice range is checked
    // just above, alignment is checked/asserted, and the returned reference is
    // tied to `data`.
    unsafe { &*bytes.as_ptr().cast::<T>() }
}

/// Mutable counterpart of [`get_helper`]. Range-checked on every target; the
/// same soundness notes apply.
#[inline(always)]
pub fn get_mut_helper<T: Get>(data: &mut [u8], index: DataIndex) -> &mut T {
    let index_usize: usize = index as usize;
    let bytes: &mut [u8] = &mut data[index_usize..index_usize + size_of::<T>()];
    #[cfg(not(target_os = "solana"))]
    assert_eq!((bytes.as_ptr() as usize) % std::mem::align_of::<T>(), 0);
    #[cfg(target_os = "solana")]
    debug_assert_eq!((bytes.as_ptr() as usize) % std::mem::align_of::<T>(), 0);
    // SAFETY: As above, with exclusive access inherited from `data`.
    unsafe { &mut *bytes.as_mut_ptr().cast::<T>() }
}

/// Unchecked counterpart of [`get_helper`] for the hot data-structure walks,
/// where the range check is measurable CU. On Solana it indexes without the
/// bound check; on native it stays range-checked (native callers may pass
/// arbitrary slices). The bound and alignment are debug-asserted on both, so
/// the tests still trap a bad index.
///
/// # Safety
/// `index` must be the start of an in-bounds, correctly aligned `T` in `data`:
/// `index as usize + size_of::<T>() <= data.len()`, and the effective address
/// must be `align_of::<T>()`-aligned. Callers in this crate satisfy this by
/// only passing node handles the free list allocated, read back out of node
/// links or the free list; see the module-level invariants in
/// `red_black_tree`, `linked_list` and `free_list`. Passing an arbitrary or
/// out-of-range index is undefined behavior — range-check it first (or use the
/// safe [`get_helper`]).
#[inline(always)]
pub unsafe fn get_helper_unchecked<T: Get>(data: &[u8], index: DataIndex) -> &T {
    let index_usize: usize = index as usize;
    let end: usize = index_usize + size_of::<T>();
    debug_assert!(
        end <= data.len(),
        "get_helper_unchecked index out of bounds"
    );
    #[cfg(not(target_os = "solana"))]
    let bytes: &[u8] = &data[index_usize..end];
    #[cfg(target_os = "solana")]
    // SAFETY: the caller guarantees `end <= data.len()` (see the contract
    // above); debug-asserted just above.
    let bytes: &[u8] = unsafe { data.get_unchecked(index_usize..end) };
    debug_assert_eq!((bytes.as_ptr() as usize) % std::mem::align_of::<T>(), 0);
    // SAFETY: `Get` supplies the validity contract, the caller guarantees the
    // range and alignment, and the returned reference is tied to `data`.
    unsafe { &*bytes.as_ptr().cast::<T>() }
}

/// Mutable counterpart of [`get_helper_unchecked`]. Same safety contract, with
/// exclusive access inherited from `data`.
#[inline(always)]
pub unsafe fn get_mut_helper_unchecked<T: Get>(data: &mut [u8], index: DataIndex) -> &mut T {
    let index_usize: usize = index as usize;
    let end: usize = index_usize + size_of::<T>();
    debug_assert!(
        end <= data.len(),
        "get_mut_helper_unchecked index out of bounds"
    );
    #[cfg(not(target_os = "solana"))]
    let bytes: &mut [u8] = &mut data[index_usize..end];
    #[cfg(target_os = "solana")]
    // SAFETY: the caller guarantees `end <= data.len()` (see the contract
    // above); debug-asserted just above.
    let bytes: &mut [u8] = unsafe { data.get_unchecked_mut(index_usize..end) };
    debug_assert_eq!((bytes.as_ptr() as usize) % std::mem::align_of::<T>(), 0);
    // SAFETY: As above, with exclusive access inherited from `data`.
    unsafe { &mut *bytes.as_mut_ptr().cast::<T>() }
}

/// Copy a possibly unaligned zero-copy value from a bounded byte slice.
pub fn read_unaligned<T: Get>(data: &[u8]) -> T {
    assert!(data.len() >= size_of::<T>());
    // SAFETY: `Get` guarantees that initialized bytes form a valid value, and
    // `read_unaligned` does not require the source address to be aligned.
    unsafe { data.as_ptr().cast::<T>().read_unaligned() }
}

/// The standard `bool` is not a `Pod`, define a replacement that is
/// https://docs.rs/spl-pod/latest/src/spl_pod/primitives.rs.html#13
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(transparent)]
pub struct PodBool(pub u8);
impl PodBool {
    pub const fn from_bool(b: bool) -> Self {
        Self(if b { 1 } else { 0 })
    }
}

impl From<bool> for PodBool {
    fn from(b: bool) -> Self {
        Self::from_bool(b)
    }
}

#[test]
fn test_pod_bool() {
    assert_eq!(PodBool::from_bool(false).0 == 1, false);
    assert_eq!(PodBool::from(false).0 == 1, false);
}

#[macro_export]
#[cfg(not(feature = "certora"))]
macro_rules! trace {
    ($($arg:tt)*) => {
        #[cfg(feature = "trace")]
        {
            #[cfg(target_os = "solana")]
            {
            solana_program::msg!("[{}:{}] {}", std::file!(), std::line!(), std::format_args!($($arg)*));
            }
            #[cfg(not(target_os = "solana"))]
            {
            std::println!("[{}:{}] {}", std::file!(), std::line!(), std::format_args!($($arg)*));
            }
        }
    };
}

#[macro_export]
#[cfg(feature = "certora")]
macro_rules! trace {
    ($($arg:tt)*) => {};
}

#[cfg(test)]
mod alignment_tests {
    use super::*;

    #[repr(transparent)]
    #[derive(Copy, Clone)]
    struct Word(u64);

    // SAFETY: Every bit pattern is valid for u64, and this transparent wrapper
    // adds no padding.
    unsafe impl Get for Word {}

    #[repr(align(8))]
    struct AlignedBytes([u8; 16]);

    #[test]
    #[should_panic]
    fn safe_read_rejects_misaligned_data_in_release_builds() {
        let data = AlignedBytes([0; 16]);
        let _ = get_helper::<Word>(&data.0[1..], 0);
    }

    #[test]
    #[should_panic]
    fn safe_mut_read_rejects_misaligned_data_in_release_builds() {
        let mut data = AlignedBytes([0; 16]);
        let _ = get_mut_helper::<Word>(&mut data.0[1..], 0);
    }
}
