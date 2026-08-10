use std::mem::size_of;

use bytemuck::{Pod, Zeroable};
use hypertree::{DataIndex, Get, NIL};
use solana_program::pubkey::Pubkey;
use static_assertions::const_assert_eq;

use crate::loader::WRAPPER_STATE_DISCRIMINANT;

#[repr(C)]
#[derive(Default, Debug, Copy, Clone, Pod, Zeroable)]
pub struct ManifestWrapperStateFixed {
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
    size_of::<ManifestWrapperStateFixed>(),
    8 +   // discriminant
    32 +  // trader
    4 +   // num_bytes_allocated
    4 +   // free_list_head_index
    4 +   // market_infos_root_index
    12 // padding
);
pub const WRAPPER_FIXED_SIZE: usize = 64;
const_assert_eq!(size_of::<ManifestWrapperStateFixed>(), WRAPPER_FIXED_SIZE);
const_assert_eq!(size_of::<ManifestWrapperStateFixed>() % 8, 0);
unsafe impl Get for ManifestWrapperStateFixed {}

impl ManifestWrapperStateFixed {
    pub fn new_empty(trader: &Pubkey) -> ManifestWrapperStateFixed {
        ManifestWrapperStateFixed {
            discriminant: WRAPPER_STATE_DISCRIMINANT,
            trader: *trader,
            num_bytes_allocated: 0,
            free_list_head_index: NIL,
            market_infos_root_index: NIL,
            _padding: [0; 3],
        }
    }
}
