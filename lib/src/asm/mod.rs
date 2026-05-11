//! FFI bindings for sBPF assembly optimized tree operations.
//!
//! This module provides Rust bindings to assembly-optimized versions of
//! the core Red-Black tree operations. For SBF builds, the assembly sources
//! are included with `global_asm!` and linked into the program as native
//! symbols.
//!
//! # Feature Flags
//!
//! This module is only compiled when the `opt-asm` feature is enabled.
//!
//! # Build Requirements
//!
//! To use this module:
//! Build for SBF with `opt-asm` or `opt-full` enabled.
//!
//! # Safety
//!
//! All functions in this module are unsafe because they:
//! - Call into external assembly code
//! - Perform raw memory manipulation
//! - Assume caller has validated all indices and pointers

use crate::DataIndex;

#[cfg(all(feature = "opt-asm", target_arch = "sbf"))]
core::arch::global_asm!(include_str!("../../asm/src/rotate_left.s"));
#[cfg(all(feature = "opt-asm", target_arch = "sbf"))]
core::arch::global_asm!(include_str!("../../asm/src/rotate_right.s"));
#[cfg(all(feature = "opt-asm", target_arch = "sbf"))]
core::arch::global_asm!(include_str!("../../asm/src/insert_fix.s"));
#[cfg(all(feature = "opt-asm", target_arch = "sbf"))]
core::arch::global_asm!(include_str!("../../asm/src/remove_fix.s"));

// External sBPF assembly functions for tree operations.
//
// These functions are implemented in lib/asm/src/*.s and compiled into SBF
// builds via global_asm! above.
//
// Function signatures follow the sBPF calling convention:
// - r1-r5: Arguments
// - r0: Return value
// - r6-r9: Callee-saved
//
// Memory layout for RBNode (matches Rust struct):
// - Offset 0x00: left (u32)
// - Offset 0x04: right (u32)
// - Offset 0x08: parent (u32)
// - Offset 0x0C: color (u8)
// - Offset 0x0D: payload_type (u8)
// - Offset 0x0E: _unused_padding (u16)
// - Offset 0x10: value (variable size)
#[cfg(feature = "opt-asm")]
extern "C" {
    /// Left rotation in assembly.
    ///
    /// # Arguments
    /// - data: Pointer to the backing byte array
    /// - index: Index of node G to rotate around
    /// - root: Pointer to root index (may be updated)
    ///
    /// # Safety
    /// Caller must ensure:
    /// - data points to valid memory of sufficient size
    /// - index points to a valid node with a right child
    /// - root points to valid u32 storage
    pub fn rotate_left_asm(data: *mut u8, index: u32, root: *mut u32);

    /// Right rotation in assembly.
    ///
    /// # Arguments
    /// - data: Pointer to the backing byte array
    /// - index: Index of node G to rotate around
    /// - root: Pointer to root index (may be updated)
    ///
    /// # Safety
    /// Caller must ensure:
    /// - data points to valid memory of sufficient size
    /// - index points to a valid node with a left child
    /// - root points to valid u32 storage
    pub fn rotate_right_asm(data: *mut u8, index: u32, root: *mut u32);

    /// Insert fix operation in assembly.
    ///
    /// # Arguments
    /// - data: Pointer to the backing byte array
    /// - index: Index of newly inserted node to fix
    /// - root: Pointer to root index (may be updated)
    ///
    /// # Returns
    /// Index of next node to fix, or NIL if done.
    ///
    /// # Safety
    /// Caller must ensure all indices point to valid nodes.
    pub fn insert_fix_asm(data: *mut u8, index: u32, root: *mut u32) -> u32;

    /// Remove fix operation in assembly.
    ///
    /// # Arguments
    /// - data: Pointer to the backing byte array
    /// - current: Current node index (may be NIL)
    /// - parent: Parent node index
    /// - root: Pointer to root index (may be updated)
    /// - out: Output pointer for next current index followed by next parent index
    ///
    /// # Safety
    /// Caller must ensure parent points to a valid node.
    pub fn remove_fix_asm(data: *mut u8, current: u32, parent: u32, root: *mut u32, out: *mut u32);
}

/// Node field offsets for assembly code.
/// These must match the RBNode struct layout exactly.
pub mod offsets {
    pub const NODE_LEFT: usize = 0x00;
    pub const NODE_RIGHT: usize = 0x04;
    pub const NODE_PARENT: usize = 0x08;
    pub const NODE_COLOR: usize = 0x0C;
    pub const NODE_PAYLOAD_TYPE: usize = 0x0D;
    pub const NODE_PADDING: usize = 0x0E;
    pub const NODE_VALUE: usize = 0x10;
}

/// Color values for assembly code.
pub mod colors {
    pub const COLOR_BLACK: u8 = 0;
    pub const COLOR_RED: u8 = 1;
}

/// Safe wrapper for rotate_left_asm.
///
/// # Safety
///
/// Caller must ensure:
/// - data has sufficient length for all node accesses
/// - index points to a valid node with a non-NIL right child
/// - root_index is a valid mutable reference
#[cfg(feature = "opt-asm")]
#[inline]
pub unsafe fn rotate_left_asm_wrapper(
    data: &mut [u8],
    index: DataIndex,
    root_index: &mut DataIndex,
) {
    rotate_left_asm(data.as_mut_ptr(), index, root_index as *mut DataIndex);
}

/// Safe wrapper for rotate_right_asm.
///
/// # Safety
///
/// Caller must ensure:
/// - data has sufficient length for all node accesses
/// - index points to a valid node with a non-NIL left child
/// - root_index is a valid mutable reference
#[cfg(feature = "opt-asm")]
#[inline]
pub unsafe fn rotate_right_asm_wrapper(
    data: &mut [u8],
    index: DataIndex,
    root_index: &mut DataIndex,
) {
    rotate_right_asm(data.as_mut_ptr(), index, root_index as *mut DataIndex);
}

/// Safe wrapper for insert_fix_asm.
///
/// # Safety
///
/// Caller must ensure:
/// - data has sufficient length for all node accesses
/// - index points to a valid newly inserted node
/// - root_index is a valid mutable reference
#[cfg(feature = "opt-asm")]
#[inline]
pub unsafe fn insert_fix_asm_wrapper(
    data: &mut [u8],
    index: DataIndex,
    root_index: &mut DataIndex,
) -> DataIndex {
    insert_fix_asm(data.as_mut_ptr(), index, root_index as *mut DataIndex)
}

/// Safe wrapper for remove_fix_asm.
///
/// # Safety
///
/// Caller must ensure:
/// - data has sufficient length for all node accesses
/// - parent_index points to a valid node
/// - root_index is a valid mutable reference
#[cfg(feature = "opt-asm")]
#[inline]
pub unsafe fn remove_fix_asm_wrapper(
    data: &mut [u8],
    current_index: DataIndex,
    parent_index: DataIndex,
    root_index: &mut DataIndex,
) -> (DataIndex, DataIndex) {
    let mut out: [DataIndex; 2] = [0, 0];
    remove_fix_asm(
        data.as_mut_ptr(),
        current_index,
        parent_index,
        root_index as *mut DataIndex,
        out.as_mut_ptr(),
    );
    (out[0], out[1])
}

#[cfg(test)]
mod tests {
    use super::offsets::*;

    #[test]
    fn test_offset_constants() {
        // Verify our offset constants match expected RBNode layout
        assert_eq!(NODE_LEFT, 0);
        assert_eq!(NODE_RIGHT, 4);
        assert_eq!(NODE_PARENT, 8);
        assert_eq!(NODE_COLOR, 12);
        assert_eq!(NODE_VALUE, 16);
    }

    #[test]
    fn test_color_constants() {
        use super::colors::*;
        assert_eq!(COLOR_BLACK, 0);
        assert_eq!(COLOR_RED, 1);
    }
}
