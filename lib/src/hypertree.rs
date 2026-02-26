use std::fmt::Display;

use bytemuck::{Pod, Zeroable};

use crate::{DataIndex, RedBlackTreeRangeIterator};

// Set to less than DataIndex::MAX because formal verification required it. It
// would be better to set it fully to DataIndex::MAX, but not a major concern
// because it is just set to an unreacahable data index and SVM limits the
// account size to 10MB.
#[cfg(feature = "certora")]
pub const NIL: DataIndex = 0x7F_FF_FF_FF;
#[cfg(not(feature = "certora"))]
pub const NIL: DataIndex = DataIndex::MAX;

#[cfg(feature = "certora")]
#[macro_export]
macro_rules! is_not_nil {
    ($v: expr) => {
        $v < NIL
    };
}

#[cfg(feature = "certora")]
#[macro_export]
macro_rules! is_nil {
    ($v: expr) => {
        $v >= NIL
    };
}

#[cfg(not(feature = "certora"))]
#[macro_export]
macro_rules! is_not_nil {
    ($v: expr) => {
        $v != NIL
    };
}

#[cfg(not(feature = "certora"))]
#[macro_export]
macro_rules! is_nil {
    ($v: expr) => {
        $v == NIL
    };
}

#[macro_export]
macro_rules! eq_nil {
    ($v: expr) => {
        $v == NIL
    };
}

pub trait Payload: Zeroable + Pod + PartialOrd + Ord + PartialEq + Eq + Display {}
impl<T: Zeroable + Pod + PartialOrd + Ord + PartialEq + Eq + Display> Payload for T {}

// A HyperTree is any datastructure that does not require contiguous memory and
// implements max, insert, delete, lookup, iterator, successor, predecessor.
// Read and write operations can be separated. Read only iterator is required.
// It is a separate trait because it wasnt possible to get the rust traits to
// work with it in the same trait.
pub trait HyperTreeReadOperations<'a> {
    fn lookup_index<V: Payload>(&'a self, value: &V) -> DataIndex;
    fn range<V: Payload>(&'a self, min: &V, max: &V) -> RedBlackTreeRangeIterator<'a, Self, V>;
    fn lookup_max_index<V: Payload>(&'a self) -> DataIndex;
    fn get_max_index(&self) -> DataIndex;
    fn get_root_index(&self) -> DataIndex;
    fn get_next_lower_index<V: Payload>(&'a self, index: DataIndex) -> DataIndex;
    fn get_next_higher_index<V: Payload>(&'a self, index: DataIndex) -> DataIndex;
}

pub struct HyperTreeValueReadOnlyIterator<'a, T: HyperTreeReadOperations<'a>, V: Payload> {
    pub(crate) tree: &'a T,
    pub(crate) index: DataIndex,
    pub(crate) phantom: std::marker::PhantomData<&'a V>,
}

pub trait HyperTreeValueIteratorTrait<'a, T: HyperTreeReadOperations<'a>> {
    fn iter<V: Payload>(&'a self) -> HyperTreeValueReadOnlyIterator<'a, T, V>;
}

pub trait HyperTreeWriteOperations<'a, V: Payload> {
    fn insert(&mut self, index: DataIndex, value: V);
    fn remove_by_index(&mut self, index: DataIndex);
}
