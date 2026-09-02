/// A trait for giving a type a non-deterministic value
pub trait Nondet: Sized {
    fn nondet() -> Self;

    #[cfg(feature = "std")]
    fn nondet_ref() -> &'static mut Self {
        std::boxed::Box::leak(std::boxed::Box::new(Self::nondet()))
    }

    /// i32::nondet_with(|x| x > 0)
    fn nondet_with<F>(func: F) -> Self
    where
        F: FnOnce(&Self) -> bool,
    {
        let val = Self::nondet();
        cvt::CVT_assume(func(&val));
        val
    }
}

/// Return a nondet value of type according tot the Nondet trait
pub fn nondet<T: Nondet>() -> T {
    Nondet::nondet()
}

#[cfg(feature = "std")]
pub fn nondet_ref<T: Nondet>() -> &'static mut T {
    Nondet::nondet_ref()
}

pub fn nondet_with<T: Nondet, F>(func: F) -> T
where
    F: FnOnce(&T) -> bool,
{
    Nondet::nondet_with(func)
}

#[macro_export]
macro_rules! nondet_impl {
    ($t:ty, $v:expr, $doc:tt) => {
        impl Nondet for $t {
            #[inline]
            #[doc = $doc]
            fn nondet() -> $t {
                $v
            }
        }
    };
}
