use bytemuck::{Pod, Zeroable};

use crate::{get_mut_helper_unchecked, DataIndex, Get, NIL};

// FreeList is a linked list that keeps track of all the available nodes that
// can be filled with ClaimedSeats and RestingOrders.
const END: u32 = u32::MAX;
pub struct FreeList<'a, T: Pod> {
    /// Index in data of the head of the free list.
    head_index: DataIndex,
    /// Mutable data array of bytes in which the free list lives.
    data: &'a mut [u8],

    /// Placeholder for holding the data type.
    phantom: std::marker::PhantomData<&'a T>,
}

#[derive(Default, Copy, Clone, Zeroable)]
#[repr(C)]
pub struct FreeListNode<T> {
    /// Next in the linked list.
    next_index: DataIndex,
    /// Payload. For free lists, this is just unused, zeroed bytes.
    node_inner: T,
}
// SAFETY: `T: Pod` and `DataIndex` accept every initialized bit pattern.
// `Get` permits the C-layout padding and never exposes it as initialized bytes.
unsafe impl<T: Pod> Get for FreeListNode<T> {}
impl<T: Pod> FreeListNode<T> {
    pub fn has_next(&self) -> bool {
        self.next_index != NIL
    }

    pub fn get_next_index(&self) -> DataIndex {
        self.next_index
    }
}

impl<'a, T: Pod> FreeList<'a, T> {
    /// Create a new free list over existing, well formed free list data.
    ///
    /// # Safety
    /// `data` must be a well formed free list for this account and `head_index`
    /// one of its block handles (or [`NIL`]/`END`): an in-bounds, aligned start
    /// for a `FreeListNode<T>`, as must every handle later passed to `add`. The
    /// methods read and write nodes without bounds checks
    /// ([`get_mut_helper_unchecked`]), so an out-of-range handle is undefined
    /// behavior. The program upholds this by only building free lists over its
    /// own account data and only freeing handles the account allocated.
    pub unsafe fn new(data: &'a mut [u8], head_index: DataIndex) -> Self {
        FreeList {
            head_index,
            data,
            phantom: std::marker::PhantomData,
        }
    }

    /// Gets the index of head.
    pub fn get_head(&self) -> DataIndex {
        self.head_index
    }

    /// Free a node to the free list
    pub fn add(&mut self, index: DataIndex) {
        // SAFETY: `index` is a block handle: either a node being freed or the
        // newly grown region during an expansion, both in-bounds, aligned
        // starts for a `FreeListNode<T>` in `self.data`.
        let node: &mut FreeListNode<T> =
            unsafe { get_mut_helper_unchecked::<FreeListNode<T>>(self.data, index) };
        node.node_inner = T::zeroed();
        node.next_index = self.head_index;
        self.head_index = index;
    }

    /// Free the node at index
    pub fn remove(&mut self) -> DataIndex {
        if self.head_index == END {
            return END;
        }

        let free_node_index: DataIndex = self.head_index;
        // SAFETY: `head_index` is the free-list head, a block handle that is an
        // in-bounds, aligned start for a `FreeListNode<T>` in `self.data`.
        let head: &mut FreeListNode<T> =
            unsafe { get_mut_helper_unchecked::<FreeListNode<T>>(self.data, free_node_index) };

        self.head_index = head.next_index;

        // Do not need to zero the bytes because it was zeroes when adding to
        // the free list.
        head.next_index = 0;

        free_node_index
    }
}

mod test {
    use super::*;
    use static_assertions::assert_not_impl_any;

    assert_not_impl_any!(FreeListNode<u64>: Pod);

    #[allow(unused)]
    #[repr(C, packed)]
    #[derive(Default, Copy, Clone, Pod, Zeroable)]
    struct UnusedFreeListPadding1 {
        _padding: [u8; 1],
    }

    #[test]
    fn test_free_list_basic() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut free_list: FreeList<UnusedFreeListPadding1> =
            unsafe { FreeList::new(&mut data, END) };
        free_list.add(64);
        free_list.add(128);

        assert_eq!(128, free_list.remove());
        assert_eq!(64, free_list.remove());
        assert_eq!(END, free_list.remove());
    }
}
