#![cfg_attr(
    all(feature = "opt-asm", target_arch = "sbf"),
    feature(asm_experimental_arch)
)]

pub use free_list::*;
pub use hypertree::*;
pub use llrb::*;
pub use red_black_tree::*;
pub use utils::*;

pub mod free_list;
pub mod hypertree;
pub mod llrb;
pub mod red_black_tree;
pub mod utils;

// Optimized implementations (feature-gated)
// These modules provide unsafe, optimized versions of the tree operations
// that skip bounds checking for improved compute unit performance.
#[cfg(feature = "opt-unsafe")]
pub mod utils_opt;

#[cfg(feature = "opt-unsafe")]
pub mod red_black_tree_opt;

#[cfg(feature = "opt-unsafe")]
pub mod llrb_opt;

// Re-export optimized helpers at crate level for convenience
#[cfg(feature = "opt-unsafe")]
pub use utils_opt::{get_helper_unchecked, get_mut_helper_unchecked};

// Re-export optimized traits - pub when certora is enabled, pub(crate) otherwise
#[cfg(all(feature = "opt-unsafe", feature = "certora"))]
pub use red_black_tree_opt::{
    RedBlackTreeReadOperationsHelpersOpt, RedBlackTreeWriteOperationsHelpersOpt,
};
#[cfg(all(feature = "opt-unsafe", not(feature = "certora")))]
pub(crate) use red_black_tree_opt::{
    RedBlackTreeReadOperationsHelpersOpt, RedBlackTreeWriteOperationsHelpersOpt,
};

#[cfg(all(feature = "opt-unsafe", feature = "certora"))]
pub use llrb_opt::LLRBOptOperations;

// sBPF assembly FFI bindings (feature-gated)
#[cfg(feature = "opt-asm")]
pub mod asm;
