pub mod batch_update_checks;
pub mod cancel_order_checks;
pub mod deposit_checks;
pub mod funds_checks;
pub mod global_checks;
pub mod market_checks;
pub mod matching_checks;
pub mod no_funds_loss_util;
pub mod no_revert_checks;
pub mod order_type_checks;
pub mod place_order_checks;
pub mod rbtree_checks;
pub mod seat_pubkey_checks;
pub mod swap_checks;
pub mod withdraw_checks;

/// Utility functions for verification.
pub(crate) mod verification_utils {
    /// Initialises the static (i.e., global) mutable variables to the initial
    /// value that they have in the system. This function should be called from
    /// rules that assume to start from a freshly instantiated system. If this
    /// function is not called, the prover will assume that static mutable
    /// variables have `nondet` value.
    pub(crate) fn init_static() {
        crate::state::init_mock();
        crate::certora::hooks::initialize_hooks();
    }
}

#[macro_export]
macro_rules! cvt_static_initializer {
    () => {
        crate::certora::spec::verification_utils::init_static();
    };
}
