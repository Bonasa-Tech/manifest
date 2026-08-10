use std::mem::size_of;

use bytemuck::{Pod, Zeroable};
use hypertree::{DataIndex, Get, NIL};
use manifest::state::DynamicAccount;
use solana_program::pubkey::Pubkey;
use static_assertions::const_assert_eq;

use crate::processors::shared::WRAPPER_USER_DISCRIMINANT;

#[repr(C)]
#[derive(Default, Debug, Copy, Clone, Pod, Zeroable)]
pub struct ManifestWrapperUserFixed {
    pub discriminant: u64,

    // This is the same signer for the core program.
    pub trader: Pubkey,

    pub num_bytes_allocated: u32,
    pub free_list_head_index: DataIndex,

    // Market infos is a tree that points to roots of trees.
    pub market_infos_root_index: DataIndex,

    pub _padding: [u32; 3],
}
const_assert_eq!(
    size_of::<ManifestWrapperUserFixed>(),
    8 +   // discriminant
    32 +  // trader
    4 +   // num_bytes_allocated
    4 +   // free_list_head_index
    4 +   // market_infos_root_index
    12 // padding
);
pub const WRAPPER_FIXED_SIZE: usize = 64;
const_assert_eq!(size_of::<ManifestWrapperUserFixed>(), WRAPPER_FIXED_SIZE);
const_assert_eq!(size_of::<ManifestWrapperUserFixed>() % 8, 0);
unsafe impl Get for ManifestWrapperUserFixed {}

impl ManifestWrapperUserFixed {
    pub fn new_empty(trader: &Pubkey) -> ManifestWrapperUserFixed {
        ManifestWrapperUserFixed {
            discriminant: WRAPPER_USER_DISCRIMINANT,
            trader: *trader,
            num_bytes_allocated: 0,
            free_list_head_index: NIL,
            market_infos_root_index: NIL,
            _padding: [0; 3],
        }
    }
}

/// Fully owned Wrapper User account, used in clients that can copy.
pub type WrapperUserValue = DynamicAccount<ManifestWrapperUserFixed, Vec<u8>>;
/// Full wrapper reference type.
pub type WrapperUserRef<'a> = DynamicAccount<&'a ManifestWrapperUserFixed, &'a [u8]>;
/// Mutable wrapper reference type.
pub type WrapperUserRefMut<'a> = DynamicAccount<&'a mut ManifestWrapperUserFixed, &'a mut [u8]>;
