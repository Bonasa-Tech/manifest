use nondet::*;
use solana_program::clock::Slot;

static mut CVT_CLOCK_SLOT: Option<Slot> = None;

// The prover by default does not inline functions that start with
// prefix "cvt_".  We could tell the prover to inline these functions
// but we tell the rustc to inline them.

#[allow(non_snake_case)]
#[inline(always)]
pub fn cvt_get_next_clock_slot() -> Slot {
    unsafe {
        let new_slot = Slot::from(nondet::<u64>());
        match CVT_CLOCK_SLOT {
            Some(old_slot) => {
                cvt::CVT_assume(new_slot > old_slot);
            }
            _ => {}
        }
        CVT_CLOCK_SLOT = Some(new_slot);
        return new_slot;
    }
}

#[allow(non_snake_case, static_mut_refs)]
#[inline(always)]
pub fn cvt_get_clock_slot() -> Slot {
    // need to call at least once cvt_get_next_clock_slot before calling this function
    cvt::CVT_assert(unsafe { CVT_CLOCK_SLOT.is_some() });
    return unsafe { CVT_CLOCK_SLOT.unwrap() };
}
