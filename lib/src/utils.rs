use std::mem::size_of;

pub type DataIndex = u32;

/// Marker for zero-copy account layouts accepted by [`get_helper`].
///
/// # Safety
/// Every initialized byte pattern must be valid for the type, and callers must
/// provide storage aligned for the type. Unlike `bytemuck::Pod`, this contract
/// permits inert C-layout padding because these helpers never expose the value
/// as bytes.
pub unsafe trait Get: Copy {}

/// Read a struct of type T in an array of data at a given index.
pub fn get_helper<T: Get>(data: &[u8], index: DataIndex) -> &T {
    let index_usize: usize = index as usize;
    let bytes: &[u8] = &data[index_usize..index_usize + size_of::<T>()];
    assert_eq!((bytes.as_ptr() as usize) % std::mem::align_of::<T>(), 0);
    // SAFETY: `Get` supplies the validity contract and the alignment and range
    // are checked above. The returned reference is tied to `data`.
    unsafe { &*bytes.as_ptr().cast::<T>() }
}

/// Read a struct of type T in an array of data at a given index.
pub fn get_mut_helper<T: Get>(data: &mut [u8], index: DataIndex) -> &mut T {
    let index_usize: usize = index as usize;
    let bytes: &mut [u8] = &mut data[index_usize..index_usize + size_of::<T>()];
    assert_eq!((bytes.as_ptr() as usize) % std::mem::align_of::<T>(), 0);
    // SAFETY: `Get` supplies the validity contract and the alignment and range
    // are checked above. The exclusive reference is tied to `data`.
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
