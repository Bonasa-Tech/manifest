use std::fmt::Display;

use bytemuck::{Pod, Zeroable};

use crate::{get_helper, DataIndex, RBNode};

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
// All of them store their entries in RBNode blocks, a 16 byte header of links
// followed by the payload, so that the same free list can serve any of them.
// Read and write operations can be separated. Read only iterator is required.
// It is a separate trait because it wasnt possible to get the rust traits to
// work with it in the same trait.
pub trait HyperTreeReadOperations<'a> {
    fn lookup_index<V: Payload>(&'a self, value: &V) -> DataIndex;
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

/// The bytes a HyperTree lives in, for the value iterator that every
/// HyperTree shares.
pub trait HyperTreeData<'a> {
    fn hypertree_data(&self) -> &[u8];
}

impl<'a, T> HyperTreeValueIteratorTrait<'a, T> for T
where
    T: HyperTreeData<'a> + HyperTreeReadOperations<'a>,
{
    /// Iterates from the max downwards, in the order the structure defines.
    fn iter<V: Payload>(&'a self) -> HyperTreeValueReadOnlyIterator<'a, T, V> {
        let mut index = self.get_max_index();
        if index == NIL {
            index = self.lookup_max_index::<V>();
        }
        HyperTreeValueReadOnlyIterator {
            tree: self,
            index,
            phantom: std::marker::PhantomData,
        }
    }
}

impl<'a, T: HyperTreeReadOperations<'a> + HyperTreeData<'a>, V: Payload> Iterator
    for HyperTreeValueReadOnlyIterator<'a, T, V>
{
    type Item = (DataIndex, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        let index: DataIndex = self.index;
        let next_index: DataIndex = self.tree.get_next_lower_index::<V>(self.index);
        if index == NIL {
            None
        } else {
            let result: &RBNode<V> = get_helper::<RBNode<V>>(self.tree.hypertree_data(), index);
            self.index = next_index;
            Some((index, result.get_value()))
        }
    }
}

pub trait HyperTreeWriteOperations<'a, V: Payload> {
    fn insert(&mut self, index: DataIndex, value: V);
    fn remove_by_index(&mut self, index: DataIndex);
}
