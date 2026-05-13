use std::mem::size_of;

pub type DataIndex = u32;

/// Marker trait to emit warnings when using get_helper on the Value type
/// rather than on Node<Value>
pub trait Get: bytemuck::Pod {}

/// Read a struct of type T in an array of data at a given index.
#[inline(always)]
pub fn get_helper<T: Get>(data: &[u8], index: DataIndex) -> &T {
    let index_usize: usize = index as usize;
    bytemuck::from_bytes(&data[index_usize..index_usize + size_of::<T>()])
}

/// Read a struct of type T in an array of data at a given index.
#[inline(always)]
pub fn get_mut_helper<T: Get>(data: &mut [u8], index: DataIndex) -> &mut T {
    let index_usize: usize = index as usize;
    bytemuck::from_bytes_mut(&mut data[index_usize..index_usize + size_of::<T>()])
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
