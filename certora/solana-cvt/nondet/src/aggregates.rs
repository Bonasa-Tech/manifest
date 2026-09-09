use crate::{nondet, nondet_impl, Nondet};
use cvt::CVT_assume;
use std::alloc::{alloc, Layout};

/// The returned pointer is a raw pointer and we won't free it.
/// That's okay since this function is only for verification purposes.
/// That's why we alloc a raw pointer to avoid including all the ownership/reference counting
/// stuff from Box and Rc
pub fn nondet_pointer<T: Nondet>() -> *mut T {
    unsafe {
        let layout = Layout::new::<T>();
        let ptr = alloc(layout) as *mut T;
        CVT_assume(!ptr.is_null());
        *ptr = nondet();
        ptr
    }
}

nondet_impl! { [u8; 32], cvt::CVT_nondet_array_of_32_bytes(), "Nondet for 32-byte array" }

impl<T: Nondet> Nondet for Option<T> {
    #[inline]
    fn nondet() -> Option<T> {
        if nondet::<bool>() {
            Some(nondet::<T>())
        } else {
            None
        }
    }
}
