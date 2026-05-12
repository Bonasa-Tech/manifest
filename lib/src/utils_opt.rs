//! Optimized utility functions for hypertree operations.
//!
//! This module provides unsafe, unchecked versions of the helper functions
//! that skip bounds checking for improved compute unit performance.
//!
//! # Safety
//!
//! These functions are unsafe because they:
//! - Do not perform bounds checking on array accesses
//! - Do not verify alignment requirements (bytemuck handles alignment)
//! - Assume the caller has validated that `index` points to valid data
//!
//! Only use these functions when:
//! - The index has been validated to be within bounds
//! - The index points to properly initialized data of type T
//! - Performance is critical (e.g., hot paths in tree operations)

use crate::{DataIndex, Get};

/// Read a struct of type T in an array of data at a given index without bounds checking.
///
/// # Safety
///
/// Caller must ensure:
/// - `index + size_of::<T>() <= data.len()`
/// - The data at `index` is properly aligned for type T
/// - The data at `index` contains valid bytes for type T
#[inline(always)]
pub unsafe fn get_helper_unchecked<T: Get>(data: &[u8], index: DataIndex) -> &T {
    let ptr = data.as_ptr().add(index as usize) as *const T;
    &*ptr
}

/// Read a struct of type T in an array of data at a given index without bounds checking.
///
/// # Safety
///
/// Caller must ensure:
/// - `index + size_of::<T>() <= data.len()`
/// - The data at `index` is properly aligned for type T
/// - The data at `index` contains valid bytes for type T
/// - No other references to the same data exist (mutable aliasing)
#[inline(always)]
pub unsafe fn get_mut_helper_unchecked<T: Get>(data: &mut [u8], index: DataIndex) -> &mut T {
    let ptr = data.as_mut_ptr().add(index as usize) as *mut T;
    &mut *ptr
}

/// Unchecked helper that also skips the size_of calculation by using a precomputed stride.
///
/// # Safety
///
/// Same requirements as `get_helper_unchecked`, plus:
/// - `stride` must equal `size_of::<T>()`
#[inline(always)]
pub unsafe fn get_helper_unchecked_with_stride<T: Get>(
    data: &[u8],
    index: DataIndex,
    _stride: usize,
) -> &T {
    let ptr = data.as_ptr().add(index as usize) as *const T;
    &*ptr
}

/// Unchecked mutable helper that also skips the size_of calculation.
///
/// # Safety
///
/// Same requirements as `get_mut_helper_unchecked`, plus:
/// - `stride` must equal `size_of::<T>()`
#[inline(always)]
pub unsafe fn get_mut_helper_unchecked_with_stride<T: Get>(
    data: &mut [u8],
    index: DataIndex,
    _stride: usize,
) -> &mut T {
    let ptr = data.as_mut_ptr().add(index as usize) as *mut T;
    &mut *ptr
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::get_helper;

    #[derive(Copy, Clone, Debug, PartialEq)]
    #[repr(C)]
    struct TestStruct {
        a: u32,
        b: u32,
    }

    unsafe impl bytemuck::Pod for TestStruct {}
    unsafe impl bytemuck::Zeroable for TestStruct {}
    impl Get for TestStruct {}

    #[test]
    fn test_get_helper_unchecked_equivalence() {
        let mut data = [0u8; 64];
        let test_val = TestStruct { a: 42, b: 100 };

        // Write using safe helper
        let offset = 8u32;
        let dest: &mut TestStruct = crate::get_mut_helper(&mut data, offset);
        *dest = test_val;

        // Read using both safe and unsafe helpers
        let safe_result: &TestStruct = get_helper(&data, offset);
        let unsafe_result: &TestStruct = unsafe { get_helper_unchecked(&data, offset) };

        assert_eq!(safe_result, unsafe_result);
        assert_eq!(safe_result.a, 42);
        assert_eq!(safe_result.b, 100);
    }

    #[test]
    fn test_get_mut_helper_unchecked_equivalence() {
        let mut data = [0u8; 64];
        let offset = 16u32;

        // Write using unsafe helper
        unsafe {
            let dest: &mut TestStruct = get_mut_helper_unchecked(&mut data, offset);
            dest.a = 123;
            dest.b = 456;
        }

        // Verify using safe helper
        let result: &TestStruct = get_helper(&data, offset);
        assert_eq!(result.a, 123);
        assert_eq!(result.b, 456);
    }

    #[test]
    fn test_with_stride_helpers() {
        let mut data = [0u8; 64];
        let offset = 0u32;
        let stride = size_of::<TestStruct>();

        // Write using stride helper
        unsafe {
            let dest: &mut TestStruct =
                get_mut_helper_unchecked_with_stride(&mut data, offset, stride);
            dest.a = 789;
            dest.b = 012;
        }

        // Read using stride helper
        let result: &TestStruct =
            unsafe { get_helper_unchecked_with_stride(&data, offset, stride) };
        assert_eq!(result.a, 789);
        assert_eq!(result.b, 012);
    }
}
