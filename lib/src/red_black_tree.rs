use bytemuck::{Pod, Zeroable};
use std::cmp::Ordering;

use crate::{
    get_helper, get_mut_helper, trace, DataIndex, Get, HyperTreeReadOperations,
    HyperTreeValueIteratorTrait, HyperTreeValueReadOnlyIterator, HyperTreeWriteOperations, Payload,
    NIL,
};

pub const RBTREE_OVERHEAD_BYTES: usize = 16;

// Overview of all the structs and traits in this file. Skips some internal helpers.
//
// Public
//  struct RedBlackTree<'a, V: Payload>
//    fn new(data: &'a mut [u8], root_index: DataIndex, max_index: DataIndex) -> Self
//    GetRedBlackTreeReadOnlyData
//    GetRedBlackTreeData
//    HyperTreeWriteOperations
//  struct RedBlackTreeReadOnly<'a, V: Payload>
//    fn new(data: &'a [u8], root_index: DataIndex, max_index: DataIndex) -> Self
//    GetRedBlackTreeReadOnlyData
//
//  trait GetRedBlackTreeReadOnlyData<'a>
//    fn data(&self) -> &[u8];
//    fn root_index(&self) -> DataIndex;
//    fn max_index(&self) -> DataIndex;
//    RedBlackTreeReadOperationsHelpers
//    HyperTreeReadOperations
//    RedBlackTreeTestHelpers
//    HyperTreeValueIteratorTrait
//  trait GetRedBlackTreeData<'a>
//    fn data(&mut self) -> &mut [u8];
//    fn set_root_index(&mut self, root_index: DataIndex);
//    RedBlackTreeWriteOperationsHelpers
//  struct RBNode<V>
//    Ord
//    PartialOrd
//    PartialEq
//    Eq
//    fn get_payload_type(&self) -> u8
//    fn set_payload_type(&mut self, payload_type: u8)
//    fn get_mut_value(&mut self) -> &mut V
//    fn get_value(&self) -> &V
//
// Internal
//  trait RedBlackTreeReadOperationsHelpers<'a>
//    fn get_value<V: Payload>(&'a self, index: DataIndex) -> &'a V;
//    fn has_left<V: Payload>(&self, index: DataIndex) -> bool;
//    fn has_right<V: Payload>(&self, index: DataIndex) -> bool;
//    fn get_right_index<V: Payload>(&self, index: DataIndex) -> DataIndex;
//    fn get_left_index<V: Payload>(&self, index: DataIndex) -> DataIndex;
//    fn get_color<V: Payload>(&self, index: DataIndex) -> Color;
//    fn get_parent_index<V: Payload>(&self, index: DataIndex) -> DataIndex;
//    fn is_left_child<V: Payload>(&self, index: DataIndex) -> bool;
//    fn is_right_child<V: Payload>(&self, index: DataIndex) -> bool;
//    fn get_node<V: Payload>(&'a self, index: DataIndex) -> &RBNode<V>;
//    fn get_child_index<V: Payload>(&self, index: DataIndex) -> DataIndex;
//    fn is_internal<V: Payload>(&self, index: DataIndex) -> bool;
//    fn get_sibling_index<V: Payload>(&self, index: DataIndex, parent_index: DataIndex)
// trait RedBlackTreeWriteOperationsHelpers<'a>
//    fn set_color<V: Payload>(&mut self, index: DataIndex, color: Color);
//    fn set_parent_index<V: Payload>(&mut self, index: DataIndex, parent_index: DataIndex);
//    fn set_left_index<V: Payload>(&mut self, index: DataIndex, left_index: DataIndex);
//    fn set_right_index<V: Payload>(&mut self, index: DataIndex, right_index: DataIndex);
//    fn rotate_left<V: Payload>(&mut self, index: DataIndex);
//    fn rotate_right<V: Payload>(&mut self, index: DataIndex);
//    fn swap_node_with_successor<V: Payload>(&mut self, index_0: DataIndex, index_1: DataIndex);
//    fn update_parent_child<V: Payload>(&mut self, index: DataIndex);
// trait RedBlackTreeTestHelpers<'a, T: GetRedBlackTreeReadOnlyData<'a>>
//    fn node_iter<V: Payload>(&'a self) -> RedBlackTreeReadOnlyIterator<T, V>;
//    fn debug_print<V: Payload>(&'a self);
//    fn depth<V: Payload>(&'a self, index: DataIndex) -> i32;
//    fn verify_rb_tree<V: Payload>(&'a self);
//    fn num_black_nodes_through_root<V: Payload>(&'a self, index: DataIndex) -> i32;
// enum Color

/// A Red-Black tree which supports random access O(log n), insert O(log n),
/// delete O(log n), and get max O(1)
pub struct RedBlackTree<'a, V: Payload> {
    /// The address within data that the root node starts.
    root_index: DataIndex,
    /// Unowned byte array which contains all the data for this tree and possibly more.
    data: &'a mut [u8],

    /// Max allowing O(1) access for matching top of book.
    /// If this is initialized to NIL on a tree that is not empty, then do not
    /// update and just keep it as NIL. This feature is only useful for usage
    /// patterns that frequently visit the max.
    max_index: DataIndex,

    phantom: std::marker::PhantomData<&'a V>,
}

/// A Red-Black tree which supports random access O(log n) and get max O(1),
/// but does not require the data to be mutable.
pub struct RedBlackTreeReadOnly<'a, V: Payload> {
    /// The address within data that the root node starts.
    root_index: DataIndex,
    /// Unowned byte array which contains all the data for this tree and possibly more.
    data: &'a [u8],

    /// Max allowing O(1) access for matching top of book.
    /// If this is initialized to NIL on a tree that is not empty, then do not
    /// update and just keep it as NIL. This feature is only useful for usage
    /// patterns that frequently visit the max.
    max_index: DataIndex,

    phantom: std::marker::PhantomData<&'a V>,
}

impl<'a, V: Payload> RedBlackTreeReadOnly<'a, V> {
    /// Creates a new RedBlackTree. Does not mutate data yet. Assumes the actual
    /// data in data is already well formed as a red black tree.
    /// It is necessary to persist the root_index to re-initialize a tree, storing
    /// the max_index is recommended when in-order iteration is performance critical.
    ///
    /// Depending on the index args this will behave differently:
    ///
    /// root=NIL: initializes an empty tree, get_max() is defined
    ///
    /// root!=NIL max=NIL: initializes an existing tree, get_max() is undefined &
    ///                    iter() will dynamically lookup the maximum
    ///
    /// root!=NIL max!=NIL: initializes an existing tree, get_max() is defined
    pub fn new(data: &'a [u8], root_index: DataIndex, max_index: DataIndex) -> Self {
        RedBlackTreeReadOnly::<V> {
            root_index,
            data,
            max_index,
            phantom: std::marker::PhantomData,
        }
    }
}

pub struct RedBlackTreeRangeIterator<'a, T: GetRedBlackTreeReadOnlyData<'a>, V: Payload> {
    pub(crate) tree: &'a T,
    pub(crate) min: &'a V,
    pub(crate) max: &'a V,
    pub(crate) current_index: DataIndex,
    pub(crate) phantom: std::marker::PhantomData<&'a V>,
}

impl<'a, T, V> Iterator for RedBlackTreeRangeIterator<'a, T, V>
where
    T: GetRedBlackTreeReadOnlyData<'a> + HyperTreeReadOperations<'a>,
    V: Payload,
{
    type Item = (DataIndex, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        while self.current_index != NIL {
            let current_value = self.tree.get_value::<V>(self.current_index);

            if current_value >= self.min && current_value <= self.max {
                // Store the current index to return
                let result_index = self.current_index;
                let result_value = current_value;

                // Move to the next lower index
                self.current_index = self.tree.get_next_lower_index::<V>(self.current_index);

                return Some((result_index, result_value));
            }

            // If the current value is out of range, move to the next relevant node
            if current_value < self.min {
                self.current_index = self.tree.get_next_higher_index::<V>(self.current_index);
            } else {
                self.current_index = self.tree.get_next_lower_index::<V>(self.current_index);
            }
        }
        None
    }
}

// Specific to red black trees and not all data structures. Implementing this
// gets a lot of other stuff for free.
pub trait GetRedBlackTreeReadOnlyData<'a> {
    fn data(&self) -> &[u8];
    fn root_index(&self) -> DataIndex;
    fn max_index(&self) -> DataIndex;
}

impl<'a, V: Payload> GetRedBlackTreeReadOnlyData<'a> for RedBlackTreeReadOnly<'a, V> {
    fn data(&self) -> &[u8] {
        self.data
    }
    fn root_index(&self) -> DataIndex {
        self.root_index
    }
    fn max_index(&self) -> DataIndex {
        self.max_index
    }
}

impl<'a, V: Payload> GetRedBlackTreeReadOnlyData<'a> for RedBlackTree<'a, V> {
    fn data(&self) -> &[u8] {
        self.data
    }
    fn root_index(&self) -> DataIndex {
        self.root_index
    }
    fn max_index(&self) -> DataIndex {
        self.max_index
    }
}
pub trait GetRedBlackTreeData<'a> {
    fn data(&mut self) -> &mut [u8];
    fn set_root_index(&mut self, root_index: DataIndex);
}
impl<'a, V: Payload> GetRedBlackTreeData<'a> for RedBlackTree<'a, V> {
    fn data(&mut self) -> &mut [u8] {
        self.data
    }
    fn set_root_index(&mut self, root_index: DataIndex) {
        self.root_index = root_index;
    }
}

// Public just for certora.
#[cfg(feature = "certora")]
pub trait RedBlackTreeReadOperationsHelpers<'a> {
    fn get_value<V: Payload>(&'a self, index: DataIndex) -> &'a V;
    fn has_left<V: Payload>(&self, index: DataIndex) -> bool;
    fn has_right<V: Payload>(&self, index: DataIndex) -> bool;
    fn get_right_index<V: Payload>(&self, index: DataIndex) -> DataIndex;
    fn get_left_index<V: Payload>(&self, index: DataIndex) -> DataIndex;
    fn get_color<V: Payload>(&self, index: DataIndex) -> Color;
    fn get_parent_index<V: Payload>(&self, index: DataIndex) -> DataIndex;
    fn is_left_child<V: Payload>(&self, index: DataIndex) -> bool;
    fn is_right_child<V: Payload>(&self, index: DataIndex) -> bool;
    fn get_node<V: Payload>(&'a self, index: DataIndex) -> &'a RBNode<V>;
    fn get_child_index<V: Payload>(&self, index: DataIndex) -> DataIndex;
    fn is_internal<V: Payload>(&self, index: DataIndex) -> bool;
    fn get_sibling_index<V: Payload>(&self, index: DataIndex, parent_index: DataIndex)
        -> DataIndex;
}
#[cfg(not(feature = "certora"))]
pub(crate) trait RedBlackTreeReadOperationsHelpers<'a> {
    fn get_value<V: Payload>(&'a self, index: DataIndex) -> &'a V;
    fn has_left<V: Payload>(&self, index: DataIndex) -> bool;
    fn has_right<V: Payload>(&self, index: DataIndex) -> bool;
    fn get_right_index<V: Payload>(&self, index: DataIndex) -> DataIndex;
    fn get_left_index<V: Payload>(&self, index: DataIndex) -> DataIndex;
    fn get_color<V: Payload>(&self, index: DataIndex) -> Color;
    fn get_parent_index<V: Payload>(&self, index: DataIndex) -> DataIndex;
    fn is_left_child<V: Payload>(&self, index: DataIndex) -> bool;
    fn is_right_child<V: Payload>(&self, index: DataIndex) -> bool;
    fn get_node<V: Payload>(&'a self, index: DataIndex) -> &'a RBNode<V>;
    fn get_child_index<V: Payload>(&self, index: DataIndex) -> DataIndex;
    fn is_internal<V: Payload>(&self, index: DataIndex) -> bool;
    fn get_sibling_index<V: Payload>(&self, index: DataIndex, parent_index: DataIndex)
        -> DataIndex;
}

impl<'a, T> RedBlackTreeReadOperationsHelpers<'a> for T
where
    T: GetRedBlackTreeReadOnlyData<'a>,
{
    fn get_value<V: Payload>(&'a self, index: DataIndex) -> &'a V {
        debug_assert_ne!(index, NIL);
        let node: &RBNode<V> = get_helper::<RBNode<V>>(self.data(), index);
        &node.value
    }
    fn has_left<V: Payload>(&self, index: DataIndex) -> bool {
        debug_assert_ne!(index, NIL);
        let node: &RBNode<V> = get_helper::<RBNode<V>>(self.data(), index);
        node.left != NIL
    }
    fn has_right<V: Payload>(&self, index: DataIndex) -> bool {
        debug_assert_ne!(index, NIL);
        let node: &RBNode<V> = get_helper::<RBNode<V>>(self.data(), index);
        node.right != NIL
    }
    fn get_color<V: Payload>(&self, index: DataIndex) -> Color {
        if index == NIL {
            return Color::Black;
        }
        let node: &RBNode<V> = get_helper::<RBNode<V>>(self.data(), index);
        node.color
    }
    fn get_right_index<V: Payload>(&self, index: DataIndex) -> DataIndex {
        if index == NIL {
            return NIL;
        }
        let node: &RBNode<V> = get_helper::<RBNode<V>>(self.data(), index);
        node.right
    }
    fn get_left_index<V: Payload>(&self, index: DataIndex) -> DataIndex {
        if index == NIL {
            return NIL;
        }
        let node: &RBNode<V> = get_helper::<RBNode<V>>(self.data(), index);
        node.left
    }
    fn get_parent_index<V: Payload>(&self, index: DataIndex) -> DataIndex {
        if index == NIL {
            return NIL;
        }
        let node: &RBNode<V> = get_helper::<RBNode<V>>(self.data(), index);
        node.parent
    }

    fn is_left_child<V: Payload>(&self, index: DataIndex) -> bool {
        if index == self.root_index() {
            return false;
        }
        let parent_index: DataIndex = self.get_parent_index::<V>(index);
        self.get_left_index::<V>(parent_index) == index
    }
    fn is_right_child<V: Payload>(&self, index: DataIndex) -> bool {
        if index == self.root_index() {
            return false;
        }
        let parent_index: DataIndex = self.get_parent_index::<V>(index);
        self.get_right_index::<V>(parent_index) == index
    }
    fn get_node<V: Payload>(&'a self, index: DataIndex) -> &'a RBNode<V> {
        debug_assert_ne!(index, NIL);
        let node: &RBNode<V> = get_helper::<RBNode<V>>(self.data(), index);
        node
    }

    fn get_child_index<V: Payload>(&self, index: DataIndex) -> DataIndex {
        debug_assert_ne!(index, NIL);
        // Assert that there are not both. This is getting the unique child.
        debug_assert!(!(self.has_left::<V>(index) && self.has_right::<V>(index)));

        let left_child_index: DataIndex = self.get_left_index::<V>(index);
        let child_index: DataIndex = if left_child_index != NIL {
            left_child_index
        } else {
            self.get_right_index::<V>(index)
        };
        child_index
    }

    fn is_internal<V: Payload>(&self, index: DataIndex) -> bool {
        debug_assert_ne!(index, NIL);
        self.get_right_index::<V>(index) != NIL && self.get_left_index::<V>(index) != NIL
    }

    fn get_sibling_index<V: Payload>(
        &self,
        index: DataIndex,
        parent_index: DataIndex,
    ) -> DataIndex {
        debug_assert_ne!(parent_index, NIL);
        let parent_left_child_index: DataIndex = self.get_left_index::<V>(parent_index);
        if parent_left_child_index == index {
            self.get_right_index::<V>(parent_index)
        } else {
            parent_left_child_index
        }
    }
}

// Public just for certora.
#[cfg(feature = "certora")]
pub trait RedBlackTreeWriteOperationsHelpers<'a> {
    fn set_color<V: Payload>(&mut self, index: DataIndex, color: Color);
    fn set_parent_index<V: Payload>(&mut self, index: DataIndex, parent_index: DataIndex);
    fn set_left_index<V: Payload>(&mut self, index: DataIndex, left_index: DataIndex);
    fn set_right_index<V: Payload>(&mut self, index: DataIndex, right_index: DataIndex);
    fn rotate_left<V: Payload>(&mut self, index: DataIndex);
    fn rotate_right<V: Payload>(&mut self, index: DataIndex);
    fn swap_node_with_successor<V: Payload>(&mut self, index_0: DataIndex, index_1: DataIndex);
    fn update_parent_child<V: Payload>(&mut self, index: DataIndex);
}
#[cfg(not(feature = "certora"))]
pub(crate) trait RedBlackTreeWriteOperationsHelpers<'a> {
    fn set_color<V: Payload>(&mut self, index: DataIndex, color: Color);
    fn set_parent_index<V: Payload>(&mut self, index: DataIndex, parent_index: DataIndex);
    fn set_left_index<V: Payload>(&mut self, index: DataIndex, left_index: DataIndex);
    fn set_right_index<V: Payload>(&mut self, index: DataIndex, right_index: DataIndex);
    fn rotate_left<V: Payload>(&mut self, index: DataIndex);
    fn rotate_right<V: Payload>(&mut self, index: DataIndex);
    fn swap_node_with_successor<V: Payload>(&mut self, index_0: DataIndex, index_1: DataIndex);
    fn update_parent_child<V: Payload>(&mut self, index: DataIndex);
}
impl<'a, T> RedBlackTreeWriteOperationsHelpers<'a> for T
where
    T: GetRedBlackTreeData<'a>
        + RedBlackTreeReadOperationsHelpers<'a>
        + GetRedBlackTreeReadOnlyData<'a>,
{
    fn set_color<V: Payload>(&mut self, index: DataIndex, color: Color) {
        if index == NIL {
            return;
        }
        let node: &mut RBNode<V> = get_mut_helper::<RBNode<V>>(self.data(), index);
        node.color = color;
    }
    fn set_parent_index<V: Payload>(&mut self, index: DataIndex, parent_index: DataIndex) {
        if index == NIL {
            return;
        }
        let node: &mut RBNode<V> = get_mut_helper::<RBNode<V>>(self.data(), index);
        node.parent = parent_index;
    }
    fn set_left_index<V: Payload>(&mut self, index: DataIndex, left_index: DataIndex) {
        if index == NIL {
            return;
        }
        let node: &mut RBNode<V> = get_mut_helper::<RBNode<V>>(self.data(), index);
        node.left = left_index;
    }
    fn set_right_index<V: Payload>(&mut self, index: DataIndex, right_index: DataIndex) {
        if index == NIL {
            return;
        }
        let node: &mut RBNode<V> = get_mut_helper::<RBNode<V>>(self.data(), index);
        node.right = right_index;
    }

    fn rotate_left<V: Payload>(&mut self, index: DataIndex) {
        // Left rotate of G
        //
        //         GG                     GG
        //         |                      |
        //         G                      P
        //       /   \                  /   \
        //      U     P     --->      G      X
        //          /   \           /   \
        //        Y      X        U       Y

        let g_index: DataIndex = index;
        let p_index: DataIndex = self.get_right_index::<V>(g_index);
        let y_index: DataIndex = self.get_left_index::<V>(p_index);
        let gg_index: DataIndex = self.get_parent_index::<V>(index);

        // P
        {
            // Does not use the helpers to avoid redundant NIL checks.
            let p_node: &mut RBNode<V> = get_mut_helper::<RBNode<V>>(self.data(), p_index);
            p_node.parent = gg_index;
            p_node.left = g_index;
        }

        // Y
        self.set_parent_index::<V>(y_index, g_index);

        // G
        {
            let g_node: &mut RBNode<V> = get_mut_helper::<RBNode<V>>(self.data(), g_index);
            g_node.parent = p_index;
            g_node.right = y_index;
        }

        // X

        // GG
        if gg_index != NIL {
            if self.get_left_index::<V>(gg_index) == index {
                self.set_left_index::<V>(gg_index, p_index);
            }
            if self.get_right_index::<V>(gg_index) == index {
                self.set_right_index::<V>(gg_index, p_index);
            }
        }

        // U
        // Unchanged, just included for completeness

        // Root
        if self.root_index() == g_index {
            self.set_root_index(p_index);
        }
    }

    fn rotate_right<V: Payload>(&mut self, index: DataIndex) {
        // Right rotate of G
        //
        //         GG                     GG
        //         |                      |
        //         G                      P
        //       /   \                  /   \
        //      P     U     --->      X       G
        //    /  \                          /   \
        //  X     Y                       Y       U

        let g_index: DataIndex = index;
        let p_index: DataIndex = self.get_left_index::<V>(g_index);
        let y_index: DataIndex = self.get_right_index::<V>(p_index);
        let gg_index: DataIndex = self.get_parent_index::<V>(index);

        // P
        {
            // Does not use the helpers to avoid redundant NIL checks.
            let p_node: &mut RBNode<V> = get_mut_helper::<RBNode<V>>(self.data(), p_index);
            p_node.parent = gg_index;
            p_node.right = g_index;
        }

        // Y
        self.set_parent_index::<V>(y_index, g_index);

        // X

        // G
        {
            let g_node: &mut RBNode<V> = get_mut_helper::<RBNode<V>>(self.data(), g_index);
            g_node.parent = p_index;
            g_node.left = y_index;
        }

        // GG
        if gg_index != NIL {
            if self.get_left_index::<V>(gg_index) == index {
                self.set_left_index::<V>(gg_index, p_index);
            }
            if self.get_right_index::<V>(gg_index) == index {
                self.set_right_index::<V>(gg_index, p_index);
            }
        }

        // U
        // Unchanged, just included for completeness

        // Root
        if self.root_index() == g_index {
            self.set_root_index(p_index);
        }
    }

    fn swap_node_with_successor<V: Payload>(&mut self, index_0: DataIndex, index_1: DataIndex) {
        let parent_0: DataIndex = self.get_parent_index::<V>(index_0);
        let parent_1: DataIndex = self.get_parent_index::<V>(index_1);
        let left_0: DataIndex = self.get_left_index::<V>(index_0);
        let left_1: DataIndex = self.get_left_index::<V>(index_1);
        let right_0: DataIndex = self.get_right_index::<V>(index_0);
        let right_1: DataIndex = self.get_right_index::<V>(index_1);

        let is_left_0: bool = self.is_left_child::<V>(index_0);
        let is_left_1: bool = self.is_left_child::<V>(index_1);

        // Setting the above parent coming down for both.
        if is_left_0 {
            self.set_left_index::<V>(parent_0, index_1);
        } else {
            self.set_right_index::<V>(parent_0, index_1);
        }
        if is_left_1 {
            self.set_left_index::<V>(parent_1, index_0);
        } else {
            self.set_right_index::<V>(parent_1, index_0);
        }

        self.set_left_index::<V>(index_0, left_1);
        self.set_right_index::<V>(index_0, right_1);
        self.set_parent_index::<V>(index_0, parent_1);

        self.set_left_index::<V>(index_1, left_0);
        self.set_right_index::<V>(index_1, right_0);
        self.set_parent_index::<V>(index_1, parent_0);

        self.set_parent_index::<V>(left_0, index_1);
        self.set_parent_index::<V>(left_1, index_0);
        self.set_parent_index::<V>(right_0, index_1);
        self.set_parent_index::<V>(right_1, index_0);

        if parent_1 == index_0 {
            self.set_parent_index::<V>(index_0, index_1);
            self.set_parent_index::<V>(index_1, parent_0);
            self.set_right_index::<V>(index_1, index_0);
        }

        // Should not happen because we only swap with successor of an
        // internal node. Root is a successor of a leaf.
        debug_assert_ne!(self.root_index(), index_1);
        if self.root_index() == index_0 {
            self.set_root_index(index_1);
        }

        let index_0_color: Color = self.get_color::<V>(index_0);
        let index_1_color: Color = self.get_color::<V>(index_1);
        self.set_color::<V>(index_0, index_1_color);
        self.set_color::<V>(index_1, index_0_color);
    }

    // Take out the node in the middle and fix parent child relationships
    fn update_parent_child<V: Payload>(&mut self, index: DataIndex) {
        debug_assert_ne!(index, NIL);
        debug_assert!(!self.is_internal::<V>(index));

        let parent_index: DataIndex = self.get_parent_index::<V>(index);
        let child_index: DataIndex = self.get_child_index::<V>(index);

        trace!("TREE update parent child {parent_index}<-{index}<-{child_index}");
        self.set_parent_index::<V>(child_index, parent_index);
        if self.is_left_child::<V>(index) {
            self.set_left_index::<V>(parent_index, child_index);
        } else {
            self.set_right_index::<V>(parent_index, child_index);
        }
        if self.root_index() == index {
            self.set_root_index(child_index);
        }
    }
}

impl<'a, T> HyperTreeReadOperations<'a> for T
where
    T: GetRedBlackTreeReadOnlyData<'a>,
{
    /// Lookup the index of a given value.
    fn lookup_index<V: Payload>(&'a self, value: &V) -> DataIndex {
        if self.root_index() == NIL {
            return NIL;
        }

        let mut current_index: DataIndex = self.root_index();

        while self.get_value::<V>(current_index) != value {
            if self.get_value::<V>(current_index) > value {
                if self.has_left::<V>(current_index) {
                    current_index = self.get_left_index::<V>(current_index);
                } else {
                    return NIL;
                }
            } else if self.get_value::<V>(current_index) < value {
                if self.has_right::<V>(current_index) {
                    current_index = self.get_right_index::<V>(current_index);
                } else {
                    return NIL;
                }
            } else {
                // Check both subtrees for equal keys.
                let left_lookup: DataIndex = RedBlackTreeReadOnly::<V>::new(
                    self.data(),
                    self.get_left_index::<V>(current_index),
                    NIL,
                )
                .lookup_index(value);
                if left_lookup != NIL {
                    return left_lookup;
                }
                let right_lookup: DataIndex = RedBlackTreeReadOnly::<V>::new(
                    self.data(),
                    self.get_right_index::<V>(current_index),
                    NIL,
                )
                .lookup_index(value);
                if right_lookup != NIL {
                    return right_lookup;
                }
                return NIL;
            }
        }
        current_index
    }

    fn range<V: Payload>(&'a self, min: &V, max: &V) -> RedBlackTreeRangeIterator<'a, T, V> {
        let root = self.get_root_index();
        RedBlackTreeRangeIterator {
            tree: self,
            min,
            max,
            current_index: if root != NIL {
                self.lookup_index(min)
            } else {
                NIL
            },
            phantom: std::marker::PhantomData,
        }
    }

    fn lookup_max_index<V: Payload>(&'a self) -> DataIndex {
        let mut current_index = self.root_index();
        if current_index == NIL {
            return NIL;
        }
        loop {
            let right_index = self.get_right_index::<V>(current_index);
            if right_index == NIL {
                return current_index;
            }
            current_index = right_index;
        }
    }

    /// Get the max index. If a tree set this to NIL on a non-empty tree, this
    /// will always be NIL.
    fn get_max_index(&self) -> DataIndex {
        self.max_index()
    }

    /// Get the current root index.
    fn get_root_index(&self) -> DataIndex {
        self.root_index()
    }

    /// Get the previous index. This walks the tree, so does not care about equal keys.
    fn get_next_lower_index<V: Payload>(&'a self, index: DataIndex) -> DataIndex {
        if index == NIL {
            return NIL;
        }
        // Predecessor is below us.
        if self.get_left_index::<V>(index) != NIL {
            let mut current_index: DataIndex = self.get_left_index::<V>(index);
            while self.get_right_index::<V>(current_index) != NIL {
                current_index = self.get_right_index::<V>(current_index);
            }
            return current_index;
        }

        // Successor is above, keep going up while we are the left child
        let mut current_index: DataIndex = index;
        while self.is_left_child::<V>(current_index) {
            current_index = self.get_parent_index::<V>(current_index);
        }
        current_index = self.get_parent_index::<V>(current_index);

        current_index
    }

    /// Get the next index. This walks the tree, so does not care about equal
    /// keys. Used to swap an internal node with the next leaf, when insert
    /// or delete points at an internal node.
    /// It should never be called on leaf nodes.
    fn get_next_higher_index<V: Payload>(&'a self, index: DataIndex) -> DataIndex {
        debug_assert!(index != NIL);
        debug_assert!(self.get_right_index::<V>(index) != NIL);
        let mut current_index: DataIndex = self.get_right_index::<V>(index);
        while self.get_left_index::<V>(current_index) != NIL {
            current_index = self.get_left_index::<V>(current_index);
        }
        current_index
    }
}

#[cfg(any(test, feature = "fuzz", feature = "trace"))]
pub trait RedBlackTreeTestHelpers<'a, T: GetRedBlackTreeReadOnlyData<'a>> {
    fn node_iter<V: Payload>(&'a self) -> RedBlackTreeReadOnlyIterator<'a, T, V>;
    fn depth<V: Payload>(&'a self, index: DataIndex) -> i32;
    #[cfg(test)]
    fn max_depth<V: Payload>(&'a self) -> i32;
    #[cfg(test)]
    fn x<V: Payload>(&'a self, index: DataIndex) -> i32;
    #[cfg(test)]
    fn pretty_print<V: Payload>(&'a self);
    fn debug_print<V: Payload>(&'a self);
    fn verify_rb_tree<V: Payload>(&'a self);
    fn num_black_nodes_through_root<V: Payload>(&'a self, index: DataIndex) -> i32;
}

#[cfg(any(test, feature = "fuzz"))]
impl<'a, T> RedBlackTreeTestHelpers<'a, T> for T
where
    T: GetRedBlackTreeReadOnlyData<'a>,
{
    /// Sorted iterator starting from the min.
    fn node_iter<V: Payload>(&'a self) -> RedBlackTreeReadOnlyIterator<'a, T, V> {
        RedBlackTreeReadOnlyIterator {
            tree: self,
            index: self.get_max_index(),
            phantom: std::marker::PhantomData,
        }
    }

    // Only used in pretty printing, so can be slow
    fn depth<V: Payload>(&'a self, index: DataIndex) -> i32 {
        let mut depth = -1;
        let mut current_index: DataIndex = index;
        while current_index != NIL {
            current_index = self.get_parent_index::<V>(current_index);
            depth += 1;
        }
        depth
    }
    #[cfg(test)]
    fn max_depth<V: Payload>(&'a self) -> i32 {
        let max_depth: i32 = self
            .node_iter::<V>()
            .fold(0, |a, b| a.max(self.depth::<V>(b.0)));
        max_depth
    }
    #[cfg(test)]
    fn x<V: Payload>(&'a self, index: DataIndex) -> i32 {
        // Max depth
        let max_depth: i32 = self.max_depth::<V>();

        let mut x: i32 = 0;
        let mut current_index: DataIndex = index;
        while current_index != NIL {
            if self.is_left_child::<V>(current_index) {
                x -= i32::pow(2, (max_depth - self.depth::<V>(current_index)) as u32);
            }
            if self.is_right_child::<V>(current_index) {
                x += i32::pow(2, (max_depth - self.depth::<V>(current_index)) as u32);
            }
            current_index = self.get_parent_index::<V>(current_index);
        }
        x
    }

    #[cfg(test)]
    fn pretty_print<V: Payload>(&'a self) {
        // Get the max depth and max / min X
        let max_depth: i32 = self
            .node_iter::<V>()
            .fold(0, |a, b| a.max(self.depth::<V>(b.0)));
        let max_x: i32 = self
            .node_iter::<V>()
            .fold(0, |a, b| a.max(self.x::<V>(b.0)));
        let min_x: i32 = self
            .node_iter::<V>()
            .fold(0, |a, b| a.min(self.x::<V>(b.0)));
        trace!("=========Pretty Print===========");
        for y in 0..(max_depth + 1) {
            let mut row_str: String = String::new();
            for x in (min_x)..(max_x + 1) {
                let mut found: bool = false;
                for (index, node) in self.node_iter::<V>() {
                    if self.depth::<V>(index) == y && self.x::<V>(index) == x {
                        found = true;
                        let str = &format!("{:<5}", node);
                        if node.color == Color::Red {
                            // Cannot use with sbf. Enable when debugging
                            // locally without sbf.
                            #[cfg(colored)]
                            {
                                use colored::Colorize;
                                row_str += &format!("{}", str.red());
                            }
                            #[cfg(not(colored))]
                            {
                                row_str += str;
                            }
                        } else {
                            #[cfg(colored)]
                            {
                                use colored::Colorize;
                                row_str += &format!("{}", str.black());
                            }
                            #[cfg(not(colored))]
                            {
                                row_str += str;
                            }
                        }
                    }
                }
                if !found {
                    row_str += &format!("{:<8}", "");
                }
            }
            trace!("{}", row_str);
        }
        let mut end: String = String::new();
        for _x in (min_x)..(max_x + 1) {
            end += "=====";
        }
        trace!("{}", end);
    }

    fn debug_print<V: Payload>(&'a self) {
        trace!("====== Hypertree ======");

        for (index, node) in self.node_iter::<V>() {
            let mut row_str: String = String::new();

            row_str += &"  ".repeat(self.depth::<V>(index) as usize);

            row_str += if node.parent == NIL {
                "- "
            } else {
                if self.is_left_child::<V>(index) {
                    "└ "
                } else {
                    "┌ "
                }
            };

            let color: char = if node.color == Color::Black { 'B' } else { 'R' };
            let str: &String = &format!("{color}:{index}:{node}");
            if node.color == Color::Red {
                // Cannot use with sbf. Enable when debugging
                // locally without sbf.
                #[cfg(colored)]
                {
                    use colored::Colorize;
                    row_str += &format!("{}", str.red());
                }
                #[cfg(not(colored))]
                {
                    row_str += str;
                }
            } else {
                row_str += str;
            }
            trace!("{}", row_str);
        }

        trace!("=======================");
    }

    fn verify_rb_tree<V: Payload>(&'a self) {
        // Verify that all leaf nodes have the same number of black nodes to the root.
        let mut num_black: Option<i32> = None;

        for (index, node) in self.node_iter::<V>() {
            // Verify that all red nodes only have black children
            if node.color == Color::Red {
                assert_eq!(
                    self.get_color::<V>(self.get_left_index::<V>(index)),
                    Color::Black
                );
                assert_eq!(
                    self.get_color::<V>(self.get_right_index::<V>(index)),
                    Color::Black
                );
            }

            if !self.has_left::<V>(index) || !self.has_right::<V>(index) {
                match num_black {
                    Some(num_black) => {
                        assert_eq!(num_black, self.num_black_nodes_through_root::<V>(index))
                    }
                    #[allow(unused_assignments)] // the compiler has issues with "match"
                    None => num_black = Some(self.num_black_nodes_through_root::<V>(index)),
                }
            }
        }
    }

    fn num_black_nodes_through_root<V: Payload>(&'a self, index: DataIndex) -> i32 {
        let mut num_black_nodes: i32 = 0;
        let mut current_index: DataIndex = index;

        while current_index != NIL {
            if self.get_color::<V>(current_index) == Color::Black {
                num_black_nodes += 1;
            }
            current_index = self.get_parent_index::<V>(current_index);
        }
        num_black_nodes
    }
}

impl<'a, T> HyperTreeValueIteratorTrait<'a, T> for T
where
    T: GetRedBlackTreeReadOnlyData<'a> + HyperTreeReadOperations<'a>,
{
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

impl<'a, T: HyperTreeReadOperations<'a> + GetRedBlackTreeReadOnlyData<'a>, V: Payload> Iterator
    for HyperTreeValueReadOnlyIterator<'a, T, V>
{
    type Item = (DataIndex, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        let index: DataIndex = self.index;
        let next_index: DataIndex = self.tree.get_next_lower_index::<V>(self.index);
        if index == NIL {
            None
        } else {
            let result: &RBNode<V> = get_helper::<RBNode<V>>(self.tree.data(), index);
            self.index = next_index;
            Some((index, result.get_value()))
        }
    }
}

#[cfg(feature = "certora")]
#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Default)]
pub enum Color {
    #[default]
    Black = 0,
    Red = 1,
}
#[cfg(not(feature = "certora"))]
#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Default)]
pub(crate) enum Color {
    #[default]
    Black = 0,
    Red = 1,
}
unsafe impl Zeroable for Color {
    fn zeroed() -> Self {
        unsafe { core::mem::zeroed() }
    }
}

#[cfg(feature = "certora")]
impl nondet::Nondet for Color {
    fn nondet() -> Self {
        if nondet::nondet::<bool>() {
            Color::Black
        } else {
            Color::Red
        }
    }
}

#[cfg(feature = "certora")]
#[derive(Debug, Default, Copy, Clone, Zeroable)]
#[repr(C)]
/// Node in a RedBlack tree. The first 16 bytes are used for maintaining the
/// RedBlack and BST properties, the rest is the payload.
pub struct RBNode<V> {
    pub left: DataIndex,
    pub right: DataIndex,
    pub parent: DataIndex,
    pub color: Color,

    // Optional enum controlled by the application to identify the type of node.
    // Defaults to zero.
    pub payload_type: u8,

    pub _unused_padding: u16,
    pub value: V,
}
#[cfg(not(feature = "certora"))]
#[derive(Debug, Default, Copy, Clone, Zeroable)]
#[repr(C)]
/// Node in a RedBlack tree. The first 16 bytes are used for maintaining the
/// RedBlack and BST properties, the rest is the payload.
pub struct RBNode<V> {
    pub(crate) left: DataIndex,
    pub(crate) right: DataIndex,
    pub(crate) parent: DataIndex,
    pub(crate) color: Color,

    // Optional enum controlled by the application to identify the type of node.
    // Defaults to zero.
    pub(crate) payload_type: u8,

    pub(crate) _unused_padding: u16,
    pub(crate) value: V,
}
unsafe impl<V: Payload> Pod for RBNode<V> {}
impl<V: Payload> Get for RBNode<V> {}

impl<V: Payload> Ord for RBNode<V> {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.value).cmp(&(other.value))
    }
}

impl<V: Payload> PartialOrd for RBNode<V> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<V: Payload> PartialEq for RBNode<V> {
    fn eq(&self, other: &Self) -> bool {
        (self.value) == (other.value)
    }
}

impl<V: Payload> Eq for RBNode<V> {}

impl<V: Payload> std::fmt::Display for RBNode<V> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(fmt, "{}", self.value)
    }
}

impl<V: Payload> RBNode<V> {
    fn get_left_index(&self) -> DataIndex {
        self.left
    }
    fn get_right_index(&self) -> DataIndex {
        self.right
    }
    pub fn get_payload_type(&self) -> u8 {
        self.payload_type
    }
    pub fn set_payload_type(&mut self, payload_type: u8) {
        self.payload_type = payload_type;
    }
    pub fn get_mut_value(&mut self) -> &mut V {
        &mut self.value
    }
    pub fn get_value(&self) -> &V {
        &self.value
    }
}

impl<'a, V: Payload> HyperTreeWriteOperations<'a, V> for RedBlackTree<'a, V> {
    /// Insert and rebalance. The data at index should be already zeroed.
    fn insert(&mut self, index: DataIndex, value: V) {
        trace!("TREE insert {index}");

        // Case where this is now the root
        if self.root_index == NIL {
            self.root_index = index;
            self.max_index = index;
            let root_node: &mut RBNode<V> = get_mut_helper::<RBNode<V>>(self.data, index);
            let new_node: RBNode<V> = RBNode {
                left: NIL,
                right: NIL,
                parent: NIL,
                color: Color::Black,
                value,
                payload_type: 0,
                _unused_padding: 0,
            };
            *root_node = new_node;
            return;
        }

        // Case where we walk the tree to add then will go back and fix coloring
        let new_node: RBNode<V> = RBNode {
            left: NIL,
            right: NIL,
            parent: NIL,
            color: Color::Red,
            value,
            payload_type: 0,
            _unused_padding: 0,
        };

        if self.max_index != NIL && *get_helper::<RBNode<V>>(self.data, self.max_index) < new_node {
            self.max_index = index;
        }

        self.insert_node_no_fix(new_node, index);

        // Avoid recursion by doing a loop here.
        let mut node_to_fix: DataIndex = index;
        loop {
            node_to_fix = self.insert_fix(node_to_fix);
            if node_to_fix == NIL {
                break;
            }
        }

        #[cfg(test)]
        self.verify_rb_tree::<V>()
    }

    /// Remove a node by index and rebalance.
    fn remove_by_index(&mut self, index: DataIndex) {
        trace!("TREE remove {index}");

        // Silently fail on removing NIL nodes.
        if index == NIL {
            return;
        }
        if index == self.max_index {
            trace!(
                "TREE max {}->{}",
                self.max_index,
                self.get_next_lower_index::<V>(self.max_index)
            );
            self.max_index = self.get_next_lower_index::<V>(self.max_index);
        }

        // If it is an internal node, we copy the successor value here and call
        // delete on the successor. We could do either the successor or
        // predecessor. We pick the successor because we would prefer the side
        // of the tree with the max to be sparser.
        if self.is_internal::<V>(index) {
            // Swap nodes
            let successor_index: DataIndex = self.get_next_higher_index::<V>(index);
            self.swap_node_with_successor::<V>(index, successor_index);
        }

        // Now we are guaranteed that the node to delete is either a leaf or has
        // only one child. Because there is only one possible child, check if
        // either is Red since NIL is Black.
        let to_delete_color: Color = self.get_color::<V>(index);
        let child_color: Color = if self.get_color::<V>(self.get_left_index::<V>(index))
            == Color::Red
            || self.get_color::<V>(self.get_right_index::<V>(index)) == Color::Red
        {
            Color::Red
        } else {
            Color::Black
        };
        if child_color == Color::Red || to_delete_color == Color::Red {
            // Simple case make the new one Black and move the child onto current.
            let child_index: DataIndex = self.get_child_index::<V>(index);
            self.update_parent_child::<V>(index);
            self.set_color::<V>(child_index, Color::Black);
            return;
        }

        // Actually removes from the tree
        let child_index: DataIndex = self.get_child_index::<V>(index);
        let parent_index: DataIndex = self.get_parent_index::<V>(index);
        self.update_parent_child::<V>(index);

        // Avoid recursion by doing a loop here.
        let mut nodes_to_fix: (DataIndex, DataIndex) = (child_index, parent_index);
        loop {
            nodes_to_fix = self.remove_fix(nodes_to_fix.0, nodes_to_fix.1);
            if nodes_to_fix.0 == NIL && nodes_to_fix.1 == NIL {
                break;
            }
        }
    }
}

impl<'a, V: Payload> RedBlackTree<'a, V> {
    /// Creates a new RedBlackTree. Does not mutate data yet. Assumes the actual
    /// data in data is already well formed as a red black tree.
    pub fn new(data: &'a mut [u8], root_index: DataIndex, max_index: DataIndex) -> Self {
        RedBlackTree::<V> {
            root_index,
            data,
            phantom: std::marker::PhantomData,
            max_index,
        }
    }

    #[cfg(test)]
    fn remove_by_value(&mut self, value: &V) {
        let index: DataIndex = self.lookup_index(value);
        if index == NIL {
            return;
        }
        self.remove_by_index(index);
    }

    // Only publicly visible for formal verification.
    #[cfg(feature = "certora")]
    pub fn certora_remove_fix(
        &mut self,
        current_index: DataIndex,
        parent_index: DataIndex,
    ) -> (DataIndex, DataIndex) {
        self.remove_fix(current_index, parent_index)
    }

    fn remove_fix(
        &mut self,
        current_index: DataIndex,
        parent_index: DataIndex,
    ) -> (DataIndex, DataIndex) {
        // Current is double black. It could be NIL if we just deleted a leaf,
        // so we need the parent to know where in the tree we are.

        // If we get to the root, then we are done.
        if self.root_index == current_index {
            return (NIL, NIL);
        }

        let sibling_index: DataIndex = self.get_sibling_index::<V>(current_index, parent_index);
        let sibling_color: Color = self.get_color::<V>(sibling_index);
        let parent_color: Color = self.get_color::<V>(parent_index);

        let sibling_has_red_child: bool =
            self.get_color::<V>(self.get_left_index::<V>(sibling_index)) == Color::Red
                || self.get_color::<V>(self.get_right_index::<V>(sibling_index)) == Color::Red;

        // 3a
        if sibling_color == Color::Black && sibling_has_red_child {
            let sibling_left_child_index: DataIndex = self.get_left_index::<V>(sibling_index);
            let sibling_right_child_index: DataIndex = self.get_right_index::<V>(sibling_index);
            // i left left
            if self.get_color::<V>(sibling_left_child_index) == Color::Red
                && self.is_left_child::<V>(sibling_index)
            {
                self.set_color::<V>(sibling_left_child_index, Color::Black);
                self.set_color::<V>(parent_index, sibling_color);
                self.set_color::<V>(sibling_index, parent_color);
                self.rotate_right::<V>(parent_index);
                return (NIL, NIL);
            }
            // ii left right
            if self.get_color::<V>(sibling_right_child_index) == Color::Red
                && self.is_left_child::<V>(sibling_index)
            {
                self.set_color::<V>(sibling_right_child_index, parent_color);
                self.set_color::<V>(parent_index, Color::Black);
                self.set_color::<V>(sibling_index, Color::Black);
                self.rotate_left::<V>(sibling_index);
                self.rotate_right::<V>(parent_index);
                return (NIL, NIL);
            }
            // iii right right
            if self.get_color::<V>(sibling_right_child_index) == Color::Red
                && self.is_right_child::<V>(sibling_index)
            {
                self.set_color::<V>(sibling_right_child_index, Color::Black);
                self.set_color::<V>(parent_index, sibling_color);
                self.set_color::<V>(sibling_index, parent_color);
                self.rotate_left::<V>(parent_index);
                return (NIL, NIL);
            }
            // iv right left
            if self.get_color::<V>(sibling_left_child_index) == Color::Red
                && self.is_right_child::<V>(sibling_index)
            {
                self.set_color::<V>(sibling_left_child_index, parent_color);
                self.set_color::<V>(parent_index, Color::Black);
                self.set_color::<V>(sibling_index, Color::Black);
                self.rotate_right::<V>(sibling_index);
                self.rotate_left::<V>(parent_index);
                return (NIL, NIL);
            }
            unreachable!();
        }

        // 3b
        // Sibling is black and both children are black
        if sibling_color == Color::Black {
            self.set_color::<V>(sibling_index, Color::Red);
            if parent_color == Color::Black {
                return (parent_index, self.get_parent_index::<V>(parent_index));
            } else {
                self.set_color::<V>(parent_index, Color::Black);
                return (NIL, NIL);
            }
        }

        // 3c
        // Sibing is red
        if self.is_left_child::<V>(sibling_index) {
            self.rotate_right::<V>(parent_index);
            self.set_color::<V>(parent_index, Color::Red);
            self.set_color::<V>(sibling_index, Color::Black);
            return (current_index, parent_index);
        } else if self.is_right_child::<V>(sibling_index) {
            self.rotate_left::<V>(parent_index);
            self.set_color::<V>(parent_index, Color::Red);
            self.set_color::<V>(sibling_index, Color::Black);
            return (current_index, parent_index);
        }
        return (NIL, NIL);
    }

    /// Insert a node into the subtree without fixing. This node could be a leaf
    /// or a subtree itself.
    fn insert_node_no_fix(&mut self, node_to_insert: RBNode<V>, new_node_index: DataIndex) {
        let mut current_parent: &RBNode<V> = get_helper::<RBNode<V>>(self.data, self.root_index);
        let mut current_parent_index: DataIndex = self.root_index;

        // Keep trying to walk while there are children. Breaks when there isnt
        // the expected child or at a leaf.
        while current_parent.left != NIL || current_parent.right != NIL {
            match node_to_insert.cmp(current_parent) {
                Ordering::Greater => {
                    let right_index: DataIndex = current_parent.get_right_index();
                    if right_index != NIL {
                        // Keep going down the right subtree
                        current_parent = get_helper::<RBNode<V>>(self.data, right_index);
                        current_parent_index = right_index;
                    } else {
                        break;
                    }
                }
                Ordering::Less => {
                    let left_index: DataIndex = current_parent.get_left_index();
                    if left_index != NIL {
                        // Keep going down the left subtree
                        current_parent = get_helper::<RBNode<V>>(self.data, left_index);
                        current_parent_index = left_index;
                    } else {
                        break;
                    }
                }
                Ordering::Equal => {
                    // Equal. Defaults to left to preserve FIFO.
                    let left_index: DataIndex = current_parent.get_left_index();
                    if left_index != NIL {
                        // Keep going down the left subtree
                        current_parent = get_helper::<RBNode<V>>(self.data, left_index);
                        current_parent_index = left_index;
                    } else {
                        break;
                    }
                }
            }
        }
        // We ended at a leaf and need to add below.
        if *self.get_node(current_parent_index) < node_to_insert {
            self.set_right_index::<V>(current_parent_index, new_node_index);
        } else {
            self.set_left_index::<V>(current_parent_index, new_node_index);
        }

        // Put the leaf in the tree and update its parent.
        {
            let new_node: &mut RBNode<V> = get_mut_helper::<RBNode<V>>(self.data, new_node_index);
            *new_node = node_to_insert;
            new_node.parent = current_parent_index;
        }
    }

    // Only publicly visible for formal verification.
    #[cfg(feature = "certora")]
    pub fn certora_insert_fix(&mut self, index_to_fix: DataIndex) -> DataIndex {
        self.insert_fix(index_to_fix)
    }

    fn insert_fix(&mut self, index_to_fix: DataIndex) -> DataIndex {
        if self.root_index == index_to_fix {
            self.set_color::<V>(index_to_fix, Color::Black);
            return NIL;
        }

        // Check the color of the parent. If it is black, then nothing left to do.
        let parent_index: DataIndex = self.get_parent_index::<V>(index_to_fix);
        let parent_color: Color = self.get_color::<V>(parent_index);

        if parent_color == Color::Black {
            return NIL;
        }

        let grandparent_index: DataIndex = self.get_parent_index::<V>(parent_index);

        // 5 possibilities  https://www.geeksforgeeks.org/insertion-in-red-black-tree/#
        // 1. Uncle is red
        // 2. Uncle is black LL
        // 3. Uncle is black LR
        // 4. Uncle is black RR
        // 5. Uncle is black RL
        let uncle_index: DataIndex = if self.get_left_index::<V>(grandparent_index) == parent_index
        {
            self.get_right_index::<V>(grandparent_index)
        } else {
            self.get_left_index::<V>(grandparent_index)
        };
        let uncle_color: Color = self.get_color::<V>(uncle_index);

        trace!("FIX uncle index={uncle_index} color={uncle_color:?}");

        // Case I: Uncle is red
        if uncle_color == Color::Red {
            self.set_color::<V>(parent_index, Color::Black);
            self.set_color::<V>(uncle_index, Color::Black);
            self.set_color::<V>(grandparent_index, Color::Red);

            return grandparent_index;
        }

        let grandparent_color: Color = self.get_color::<V>(grandparent_index);
        let parent_is_left: bool = self.is_left_child::<V>(parent_index);
        let current_is_left: bool = self.is_left_child::<V>(index_to_fix);

        trace!("FIX G=[{grandparent_index}:{grandparent_color:?}] P=[{parent_index}:{parent_color:?}] Pi={parent_is_left} Ci={current_is_left}");

        if grandparent_index == NIL && parent_color == Color::Red {
            self.set_color::<V>(parent_index, Color::Black);
            return NIL;
        }

        let index_to_fix_color: Color = self.get_color::<V>(index_to_fix);
        // Case II: Uncle is black, left left
        if parent_is_left && current_is_left {
            self.rotate_right::<V>(grandparent_index);
            self.set_color::<V>(grandparent_index, parent_color);
            self.set_color::<V>(parent_index, grandparent_color);
        }
        // Case III: Uncle is black, left right
        else if parent_is_left && !current_is_left {
            self.rotate_left::<V>(parent_index);
            self.rotate_right::<V>(grandparent_index);
            self.set_color::<V>(index_to_fix, grandparent_color);
            self.set_color::<V>(grandparent_index, index_to_fix_color);
        }
        // Case IV: Uncle is black, right right
        else if !parent_is_left && !current_is_left {
            self.rotate_left::<V>(grandparent_index);
            self.set_color::<V>(grandparent_index, parent_color);
            self.set_color::<V>(parent_index, grandparent_color);
        }
        // Case V: Uncle is black, right left
        else if !parent_is_left && current_is_left {
            self.rotate_right::<V>(parent_index);
            self.rotate_left::<V>(grandparent_index);
            self.set_color::<V>(index_to_fix, grandparent_color);
            self.set_color::<V>(grandparent_index, index_to_fix_color);
        }
        NIL
    }
}

// Iterator that gives the RBNode information is only needed for testing.
// External users should use the HyperTreeValueIteratorTrait.
#[cfg(any(test, feature = "fuzz", feature = "trace"))]
pub struct RedBlackTreeReadOnlyIterator<'a, T: HyperTreeReadOperations<'a>, V: Payload> {
    tree: &'a T,
    index: DataIndex,

    phantom: std::marker::PhantomData<&'a V>,
}

#[cfg(any(test, feature = "fuzz", feature = "trace"))]
impl<'a, T: HyperTreeReadOperations<'a> + GetRedBlackTreeReadOnlyData<'a>, V: Payload> Iterator
    for RedBlackTreeReadOnlyIterator<'a, T, V>
{
    type Item = (DataIndex, &'a RBNode<V>);

    fn next(&mut self) -> Option<Self::Item> {
        let index: DataIndex = self.index;
        let next_index: DataIndex = self.tree.get_next_lower_index::<V>(self.index);
        if index == NIL {
            None
        } else {
            let result: &RBNode<V> = get_helper::<RBNode<V>>(self.tree.data(), index);
            self.index = next_index;
            Some((index, result))
        }
    }
}

// No IterMut because changing keys could break red-black properties.

#[cfg(test)]
pub(crate) mod test {
    use std::fmt::Display;

    use super::*;

    #[test]
    fn test_color_default() {
        assert_eq!(Color::default(), Color::Black);
        assert_eq!(Color::zeroed(), Color::Black);
    }

    #[derive(Copy, Clone, Pod, Zeroable, Debug)]
    #[repr(C)]
    pub(crate) struct TestOrderBid {
        order_id: u64,
        padding: [u8; 128],
    }

    impl Ord for TestOrderBid {
        fn cmp(&self, other: &Self) -> Ordering {
            (self.order_id).cmp(&(other.order_id))
        }
    }

    impl PartialOrd for TestOrderBid {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    impl PartialEq for TestOrderBid {
        fn eq(&self, other: &Self) -> bool {
            (self.order_id) == (other.order_id)
        }
    }

    impl Eq for TestOrderBid {}

    impl Display for TestOrderBid {
        fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "{}", self.order_id)
        }
    }

    impl TestOrderBid {
        pub(crate) fn new(order_id: u64) -> Self {
            TestOrderBid {
                order_id,
                padding: [0; 128],
            }
        }
    }

    #[derive(Copy, Clone, Pod, Zeroable, Debug)]
    #[repr(C)]
    pub(crate) struct TestOrderAsk {
        order_id: u64,
        padding: [u8; 128],
    }

    impl Ord for TestOrderAsk {
        fn cmp(&self, other: &Self) -> Ordering {
            other.order_id.cmp(&self.order_id)
        }
    }

    impl PartialOrd for TestOrderAsk {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    impl PartialEq for TestOrderAsk {
        fn eq(&self, other: &Self) -> bool {
            (self.order_id) == (other.order_id)
        }
    }

    impl Eq for TestOrderAsk {}

    impl Display for TestOrderAsk {
        fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "{}", self.order_id)
        }
    }

    impl TestOrderAsk {
        pub(crate) fn new(order_id: u64) -> Self {
            TestOrderAsk {
                order_id,
                padding: [0; 128],
            }
        }
    }

    // Blocks are
    // Left: DataIndex
    // Right: DataIndex
    // Parent: DataIndex
    // Color: DataIndex
    // TestOrder: 8 + 128
    // 8 + 8 + 8 + 8 + 8 + 128 = 168
    pub(crate) const TEST_BLOCK_WIDTH: DataIndex = 168;

    #[test]
    fn test_insert_basic() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: RedBlackTree<TestOrderBid> = RedBlackTree::new(&mut data, NIL, NIL);

        tree.insert(TEST_BLOCK_WIDTH * 0, TestOrderBid::new(1111));
        tree.insert(TEST_BLOCK_WIDTH, TestOrderBid::new(1234));
        tree.insert(TEST_BLOCK_WIDTH * 2, TestOrderBid::new(1000));
        tree.insert(TEST_BLOCK_WIDTH * 3, TestOrderBid::new(2000));
        tree.insert(TEST_BLOCK_WIDTH * 4, TestOrderBid::new(3000));
        tree.insert(TEST_BLOCK_WIDTH * 5, TestOrderBid::new(4000));
        tree.insert(TEST_BLOCK_WIDTH * 6, TestOrderBid::new(5000));
        tree.insert(TEST_BLOCK_WIDTH * 7, TestOrderBid::new(6000));
    }

    fn init_simple_tree(data: &mut [u8]) -> RedBlackTree<TestOrderBid> {
        let mut tree: RedBlackTree<TestOrderBid> = RedBlackTree::new(data, NIL, NIL);

        for i in 1..12 {
            tree.insert(TEST_BLOCK_WIDTH * i, TestOrderBid::new((i * 1_000).into()));
        }
        tree
    }

    #[test]
    fn test_pretty_print() {
        let mut data: [u8; 100000] = [0; 100000];
        let tree: RedBlackTree<TestOrderBid> = init_simple_tree(&mut data);
        tree.pretty_print::<TestOrderBid>();
    }

    #[test]
    fn test_debug_print() {
        let mut data: [u8; 100000] = [0; 100000];
        let tree: RedBlackTree<TestOrderBid> = init_simple_tree(&mut data);
        tree.debug_print::<TestOrderBid>();
    }

    #[test]
    fn test_insert_fix() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: RedBlackTree<TestOrderBid> = init_simple_tree(&mut data);

        // Should go left and right through the tree
        tree.insert(
            TEST_BLOCK_WIDTH * 32,
            TestOrderBid::new((15_900).try_into().unwrap()),
        );
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_remove_fix() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: RedBlackTree<TestOrderBid> = init_simple_tree(&mut data);

        for i in 1..12 {
            tree.remove_by_value(&TestOrderBid::new(i * 1_000));
        }
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_remove_fix_internal_successor_is_left() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: RedBlackTree<TestOrderBid> = init_simple_tree(&mut data);
        tree.remove_by_value(&TestOrderBid::new(7 * 1_000));
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_remove_fix_internal_right_right_parent_red() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: RedBlackTree<TestOrderBid> = init_simple_tree(&mut data);
        tree.remove_by_value(&TestOrderBid::new(6 * 1_000));
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_remove_fix_internal_successor_is_right_child() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: RedBlackTree<TestOrderBid> = init_simple_tree(&mut data);
        tree.remove_by_value(&TestOrderBid::new(2 * 1_000));
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_remove_only_has_right_after_swap() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: RedBlackTree<TestOrderBid> = init_simple_tree(&mut data);
        tree.remove_by_value(&TestOrderBid::new(5 * 1_000));
        tree.remove_by_value(&TestOrderBid::new(4 * 1_000));
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_remove_only_has_left_after_swap() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: RedBlackTree<TestOrderBid> = init_simple_tree(&mut data);
        tree.remove_by_value(&TestOrderBid::new(11 * 1_000));
        tree.remove_by_value(&TestOrderBid::new(10 * 1_000));
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_internal_remove() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: RedBlackTree<TestOrderBid> = init_simple_tree(&mut data);

        for i in 4..8 {
            tree.remove_by_value(&TestOrderBid::new(i * 1_000));
            tree.verify_rb_tree::<TestOrderBid>();
        }
    }

    #[test]
    fn test_rotate_right() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: RedBlackTree<TestOrderBid> = RedBlackTree::new(&mut data, NIL, NIL);

        for i in 1..12 {
            tree.insert(
                TEST_BLOCK_WIDTH * i,
                TestOrderBid::new(((12 - i) * 1_000).into()),
            );
        }
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_remove_nil() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: RedBlackTree<TestOrderBid> = init_simple_tree(&mut data);
        // Does not exist in the tree. Should fail silently.
        tree.remove_by_value(&TestOrderBid::new(99999));
        tree.remove_by_value(&TestOrderBid::new(1));
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_max() {
        let mut data: [u8; 100000] = [0; 100000];
        let tree: RedBlackTree<TestOrderBid> = init_simple_tree(&mut data);
        assert_eq!(
            tree.get_max_index(),
            tree.lookup_max_index::<TestOrderBid>()
        );
        assert_eq!(tree.get_max_index(), TEST_BLOCK_WIDTH * 11);
    }

    #[test]
    fn test_insert_right_left() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: RedBlackTree<TestOrderBid> = RedBlackTree::new(&mut data, NIL, NIL);
        tree.insert(TEST_BLOCK_WIDTH * 0, TestOrderBid::new(100));
        tree.insert(TEST_BLOCK_WIDTH, TestOrderBid::new(200));
        tree.insert(TEST_BLOCK_WIDTH * 2, TestOrderBid::new(300));
        tree.insert(TEST_BLOCK_WIDTH * 3, TestOrderBid::new(150));
        tree.insert(TEST_BLOCK_WIDTH * 4, TestOrderBid::new(125));
    }

    #[test]
    fn test_remove_left_left() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: RedBlackTree<TestOrderBid> = RedBlackTree::new(&mut data, NIL, NIL);
        tree.insert(TEST_BLOCK_WIDTH * 4, TestOrderBid::new(40));
        tree.insert(TEST_BLOCK_WIDTH * 3, TestOrderBid::new(30));
        tree.insert(TEST_BLOCK_WIDTH * 2, TestOrderBid::new(25));
        tree.insert(TEST_BLOCK_WIDTH, TestOrderBid::new(20));
        tree.insert(TEST_BLOCK_WIDTH * 0, TestOrderBid::new(15));

        tree.remove_by_value(&TestOrderBid::new(40));
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_remove_right_left() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: RedBlackTree<TestOrderBid> = RedBlackTree::new(&mut data, NIL, NIL);
        tree.insert(TEST_BLOCK_WIDTH * 0, TestOrderBid::new(20));
        tree.insert(TEST_BLOCK_WIDTH, TestOrderBid::new(30));
        tree.insert(TEST_BLOCK_WIDTH * 2, TestOrderBid::new(40));
        tree.insert(TEST_BLOCK_WIDTH * 3, TestOrderBid::new(35));

        tree.remove_by_value(&TestOrderBid::new(20));
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_remove_left_right() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: RedBlackTree<TestOrderBid> = RedBlackTree::new(&mut data, NIL, NIL);
        tree.insert(TEST_BLOCK_WIDTH * 0, TestOrderBid::new(20));
        tree.insert(TEST_BLOCK_WIDTH, TestOrderBid::new(30));
        tree.insert(TEST_BLOCK_WIDTH * 2, TestOrderBid::new(40));
        tree.insert(TEST_BLOCK_WIDTH * 3, TestOrderBid::new(25));

        tree.remove_by_value(&TestOrderBid::new(40));
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_remove_red_left_sibling() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: RedBlackTree<TestOrderBid> = RedBlackTree::new(&mut data, NIL, NIL);
        tree.insert(TEST_BLOCK_WIDTH * 0, TestOrderBid::new(30));
        tree.insert(TEST_BLOCK_WIDTH, TestOrderBid::new(20));
        tree.insert(TEST_BLOCK_WIDTH * 3, TestOrderBid::new(15));
        tree.insert(TEST_BLOCK_WIDTH * 2, TestOrderBid::new(10));
        tree.insert(TEST_BLOCK_WIDTH * 4, TestOrderBid::new(5));

        tree.insert(TEST_BLOCK_WIDTH * 5, TestOrderBid::new(1));
        tree.remove_by_value(&TestOrderBid::new(1));
        tree.remove_by_value(&TestOrderBid::new(30));
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_remove_red_right_sibling() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: RedBlackTree<TestOrderBid> = RedBlackTree::new(&mut data, NIL, NIL);
        tree.insert(TEST_BLOCK_WIDTH * 0, TestOrderBid::new(10));
        tree.insert(TEST_BLOCK_WIDTH, TestOrderBid::new(20));
        tree.insert(TEST_BLOCK_WIDTH * 3, TestOrderBid::new(25));
        tree.insert(TEST_BLOCK_WIDTH * 2, TestOrderBid::new(30));
        tree.insert(TEST_BLOCK_WIDTH * 4, TestOrderBid::new(35));

        tree.insert(TEST_BLOCK_WIDTH * 5, TestOrderBid::new(45));
        tree.remove_by_value(&TestOrderBid::new(45));
        tree.remove_by_value(&TestOrderBid::new(10));
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_insert_left_right() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: RedBlackTree<TestOrderBid> = RedBlackTree::new(&mut data, NIL, NIL);
        tree.insert(TEST_BLOCK_WIDTH * 0, TestOrderBid::new(100));
        tree.insert(TEST_BLOCK_WIDTH, TestOrderBid::new(200));
        tree.insert(TEST_BLOCK_WIDTH * 2, TestOrderBid::new(300));
        tree.insert(TEST_BLOCK_WIDTH * 3, TestOrderBid::new(250));
        tree.insert(TEST_BLOCK_WIDTH * 4, TestOrderBid::new(275));
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_insert_left_right_onto_empty() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: RedBlackTree<TestOrderBid> = init_simple_tree(&mut data);
        tree.insert(TEST_BLOCK_WIDTH * 12, TestOrderBid::new(4500));
        tree.insert(TEST_BLOCK_WIDTH * 13, TestOrderBid::new(5500));
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_get_next_lower_index() {
        let mut data: [u8; 100000] = [0; 100000];
        let tree: RedBlackTree<TestOrderBid> = init_simple_tree(&mut data);
        assert_eq!(tree.get_next_lower_index::<TestOrderBid>(NIL), NIL);
        assert_eq!(
            tree.get_next_lower_index::<TestOrderBid>(TEST_BLOCK_WIDTH * 6),
            TEST_BLOCK_WIDTH * 5
        );
        assert_eq!(
            tree.get_next_lower_index::<TestOrderBid>(TEST_BLOCK_WIDTH * 5),
            TEST_BLOCK_WIDTH * 4
        );
        assert_eq!(
            tree.get_next_lower_index::<TestOrderBid>(TEST_BLOCK_WIDTH * 4),
            TEST_BLOCK_WIDTH * 3
        );
        assert_eq!(
            tree.get_next_lower_index::<TestOrderBid>(TEST_BLOCK_WIDTH * 3),
            TEST_BLOCK_WIDTH * 2
        );
        assert_eq!(
            tree.get_next_lower_index::<TestOrderBid>(TEST_BLOCK_WIDTH * 2),
            TEST_BLOCK_WIDTH
        );
        assert_eq!(
            tree.get_next_lower_index::<TestOrderBid>(TEST_BLOCK_WIDTH),
            NIL
        );
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_empty_max() {
        let mut data: [u8; 100000] = [0; 100000];
        let tree: RedBlackTree<TestOrderBid> = RedBlackTree::new(&mut data, NIL, NIL);
        assert_eq!(tree.lookup_max_index::<TestOrderBid>(), NIL);
        assert_eq!(tree.get_max_index(), NIL);
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_node_equality() {
        let mut data1: [u8; 100000] = [0; 100000];
        let mut data2: [u8; 100000] = [0; 100000];
        let _tree1: RedBlackTree<TestOrderBid> = init_simple_tree(&mut data1);
        let _tree2: RedBlackTree<TestOrderBid> = init_simple_tree(&mut data2);
        assert_ne!(
            get_helper::<RBNode<TestOrderBid>>(&mut data1, 1 * TEST_BLOCK_WIDTH),
            get_helper::<RBNode<TestOrderBid>>(&mut data2, 2 * TEST_BLOCK_WIDTH)
        );
    }

    #[test]
    fn test_insert_equal() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: RedBlackTree<TestOrderBid> = init_simple_tree(&mut data);

        tree.insert(TEST_BLOCK_WIDTH * 12, TestOrderBid::new(4000));
        tree.insert(TEST_BLOCK_WIDTH * 13, TestOrderBid::new(5000));
        tree.insert(TEST_BLOCK_WIDTH * 14, TestOrderBid::new(1000));
        tree.insert(TEST_BLOCK_WIDTH * 15, TestOrderBid::new(1000));
        tree.insert(TEST_BLOCK_WIDTH * 16, TestOrderBid::new(1000));
        tree.insert(TEST_BLOCK_WIDTH * 17, TestOrderBid::new(1000));
        tree.insert(TEST_BLOCK_WIDTH * 18, TestOrderBid::new(1000));
        tree.insert(TEST_BLOCK_WIDTH * 19, TestOrderBid::new(1000));
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_insert_and_remove_complex() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: RedBlackTree<TestOrderBid> = RedBlackTree::new(&mut data, NIL, NIL);

        tree.insert(TEST_BLOCK_WIDTH * 0, TestOrderBid::new(0));
        tree.insert(TEST_BLOCK_WIDTH * 1, TestOrderBid::new(1064));
        tree.insert(TEST_BLOCK_WIDTH * 2, TestOrderBid::new(4128));
        tree.insert(TEST_BLOCK_WIDTH * 3, TestOrderBid::new(2192));
        tree.insert(TEST_BLOCK_WIDTH * 4, TestOrderBid::new(5256));
        tree.insert(TEST_BLOCK_WIDTH * 5, TestOrderBid::new(3320));
        tree.insert(TEST_BLOCK_WIDTH * 6, TestOrderBid::new(8384));
        tree.insert(TEST_BLOCK_WIDTH * 7, TestOrderBid::new(7448));
        tree.insert(TEST_BLOCK_WIDTH * 8, TestOrderBid::new(6512));
        tree.remove_by_index(TEST_BLOCK_WIDTH * 6);
        tree.remove_by_index(TEST_BLOCK_WIDTH * 7);
        tree.remove_by_index(TEST_BLOCK_WIDTH * 8);
        tree.remove_by_index(TEST_BLOCK_WIDTH * 4);
        tree.remove_by_index(TEST_BLOCK_WIDTH * 2);

        tree.verify_rb_tree::<TestOrderBid>();

        // Silently fails to remove NIL
        tree.remove_by_index(NIL);
    }

    //                   B
    //              /        \
    //             R          R
    //           /    \     /   \
    //          B     B     B   (B)
    //               /
    //             R
    // Remove (B)
    //                   5
    //              /        \
    //             2          7
    //           /    \     /   \
    //          1     4     6   (8)
    //               /
    //             3
    // Remove (8)
    #[test]
    fn test_regression_1() {
        let mut data: [u8; 100000] = [0; 100000];
        *get_mut_helper(&mut data, 1 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 2 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(1),
        };
        *get_mut_helper(&mut data, 2 * TEST_BLOCK_WIDTH) = RBNode {
            left: 1 * TEST_BLOCK_WIDTH,
            right: 4 * TEST_BLOCK_WIDTH,
            parent: 5 * TEST_BLOCK_WIDTH,
            color: Color::Red,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(2),
        };
        *get_mut_helper(&mut data, 3 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 4 * TEST_BLOCK_WIDTH,
            color: Color::Red,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(3),
        };
        *get_mut_helper(&mut data, 4 * TEST_BLOCK_WIDTH) = RBNode {
            left: 3 * TEST_BLOCK_WIDTH,
            right: NIL,
            parent: 2 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(4),
        };
        *get_mut_helper(&mut data, 5 * TEST_BLOCK_WIDTH) = RBNode {
            left: 2 * TEST_BLOCK_WIDTH,
            right: 7 * TEST_BLOCK_WIDTH,
            parent: NIL,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(5),
        };
        *get_mut_helper(&mut data, 6 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 7 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(6),
        };
        *get_mut_helper(&mut data, 7 * TEST_BLOCK_WIDTH) = RBNode {
            left: 6 * TEST_BLOCK_WIDTH,
            right: 8 * TEST_BLOCK_WIDTH,
            parent: 5 * TEST_BLOCK_WIDTH,
            color: Color::Red,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(7),
        };
        *get_mut_helper(&mut data, 8 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 7 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(8),
        };

        let mut tree: RedBlackTree<TestOrderBid> =
            RedBlackTree::new(&mut data, 5 * TEST_BLOCK_WIDTH, NIL);
        tree.verify_rb_tree::<TestOrderBid>();

        tree.remove_by_index(8 * TEST_BLOCK_WIDTH);

        tree.verify_rb_tree::<TestOrderBid>();
    }

    //                  (B)
    //              /        \
    //             R          B
    //           /    \     /
    //          B     B     R
    //               / \
    //             R    R
    // Remove (B)
    //                  (6)
    //              /        \
    //             2          8
    //           /    \     /
    //          1     4     7
    //               / \
    //             3    5
    // Remove (6)
    #[test]
    fn test_regression_2() {
        let mut data: [u8; 100000] = [0; 100000];
        *get_mut_helper(&mut data, 1 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 2 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(1),
        };
        *get_mut_helper(&mut data, 2 * TEST_BLOCK_WIDTH) = RBNode {
            left: 1 * TEST_BLOCK_WIDTH,
            right: 4 * TEST_BLOCK_WIDTH,
            parent: 6 * TEST_BLOCK_WIDTH,
            color: Color::Red,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(2),
        };
        *get_mut_helper(&mut data, 3 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 4 * TEST_BLOCK_WIDTH,
            color: Color::Red,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(3),
        };
        *get_mut_helper(&mut data, 4 * TEST_BLOCK_WIDTH) = RBNode {
            left: 3 * TEST_BLOCK_WIDTH,
            right: 5 * TEST_BLOCK_WIDTH,
            parent: 2 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(4),
        };
        *get_mut_helper(&mut data, 5 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 4 * TEST_BLOCK_WIDTH,
            color: Color::Red,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(5),
        };
        *get_mut_helper(&mut data, 6 * TEST_BLOCK_WIDTH) = RBNode {
            left: 2 * TEST_BLOCK_WIDTH,
            right: 8 * TEST_BLOCK_WIDTH,
            parent: NIL,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(6),
        };
        *get_mut_helper(&mut data, 7 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 8 * TEST_BLOCK_WIDTH,
            color: Color::Red,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(7),
        };
        *get_mut_helper(&mut data, 8 * TEST_BLOCK_WIDTH) = RBNode {
            left: 7 * TEST_BLOCK_WIDTH,
            right: NIL,
            parent: 6 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(8),
        };

        let mut tree: RedBlackTree<TestOrderBid> =
            RedBlackTree::new(&mut data, 6 * TEST_BLOCK_WIDTH, NIL);
        tree.verify_rb_tree::<TestOrderBid>();

        tree.remove_by_index(6 * TEST_BLOCK_WIDTH);

        tree.verify_rb_tree::<TestOrderBid>();
    }

    //                   B
    //              /        \
    //             B          B
    //           /    \     /   \
    //          B     B     R   (B)
    //               /     / \
    //             R      B   B
    //                         \
    //                          R
    // Remove (B)
    //                   5
    //              /        \
    //             2          10
    //           /    \     /   \
    //          1     4     7   (11)
    //               /     / \
    //             3      6   8
    //                         \
    //                          9
    // Remove (11)
    #[test]
    fn test_regression_3() {
        let mut data: [u8; 100000] = [0; 100000];
        *get_mut_helper(&mut data, 1 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 2 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(1),
        };
        *get_mut_helper(&mut data, 2 * TEST_BLOCK_WIDTH) = RBNode {
            left: 1 * TEST_BLOCK_WIDTH,
            right: 4 * TEST_BLOCK_WIDTH,
            parent: 5 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(2),
        };
        *get_mut_helper(&mut data, 3 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 4 * TEST_BLOCK_WIDTH,
            color: Color::Red,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(3),
        };
        *get_mut_helper(&mut data, 4 * TEST_BLOCK_WIDTH) = RBNode {
            left: 3 * TEST_BLOCK_WIDTH,
            right: NIL,
            parent: 2 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(4),
        };
        *get_mut_helper(&mut data, 5 * TEST_BLOCK_WIDTH) = RBNode {
            left: 2 * TEST_BLOCK_WIDTH,
            right: 10 * TEST_BLOCK_WIDTH,
            parent: NIL,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(5),
        };
        *get_mut_helper(&mut data, 6 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 7 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(6),
        };
        *get_mut_helper(&mut data, 7 * TEST_BLOCK_WIDTH) = RBNode {
            left: 6 * TEST_BLOCK_WIDTH,
            right: 8 * TEST_BLOCK_WIDTH,
            parent: 10 * TEST_BLOCK_WIDTH,
            color: Color::Red,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(7),
        };
        *get_mut_helper(&mut data, 8 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: 9 * TEST_BLOCK_WIDTH,
            parent: 7 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(8),
        };
        *get_mut_helper(&mut data, 9 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 8 * TEST_BLOCK_WIDTH,
            color: Color::Red,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(9),
        };
        *get_mut_helper(&mut data, 10 * TEST_BLOCK_WIDTH) = RBNode {
            left: 7 * TEST_BLOCK_WIDTH,
            right: 11 * TEST_BLOCK_WIDTH,
            parent: 5 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(10),
        };
        *get_mut_helper(&mut data, 11 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 10 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(11),
        };
        let mut tree: RedBlackTree<TestOrderBid> =
            RedBlackTree::new(&mut data, 5 * TEST_BLOCK_WIDTH, NIL);
        tree.verify_rb_tree::<TestOrderBid>();

        tree.remove_by_index(11 * TEST_BLOCK_WIDTH);

        tree.verify_rb_tree::<TestOrderBid>();
    }

    // This case would try to rotate beyond the root of the tree in the second
    // iteration of the rebalance step.
    //
    // indent spaces = depth*2
    // R/B = node color
    // 0-5 = index in backing array / TEST_BLOCK_WIDTH
    // 0-1 = node value
    //
    //     R:4:1
    //     * B:5:0
    //   B:2:0
    //     R:3:0
    // R:0:0
    //   B:1:0
    // Add (*)
    #[test]
    fn test_regression_4() {
        let mut data: [u8; 100000] = [0; 100000];
        *get_mut_helper(&mut data, 0 * TEST_BLOCK_WIDTH) = RBNode {
            left: 2 * TEST_BLOCK_WIDTH,
            right: 1 * TEST_BLOCK_WIDTH,
            parent: NIL,
            color: Color::Red,
            value: TestOrderAsk::new(0),
            payload_type: 0,
            _unused_padding: 0,
        };
        *get_mut_helper(&mut data, 1 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 0 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            value: TestOrderAsk::new(0),
            payload_type: 0,
            _unused_padding: 0,
        };
        *get_mut_helper(&mut data, 2 * TEST_BLOCK_WIDTH) = RBNode {
            left: 4 * TEST_BLOCK_WIDTH,
            right: 3 * TEST_BLOCK_WIDTH,
            parent: 0 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            value: TestOrderAsk::new(0),
            payload_type: 0,
            _unused_padding: 0,
        };
        *get_mut_helper(&mut data, 3 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 2 * TEST_BLOCK_WIDTH,
            color: Color::Red,
            value: TestOrderAsk::new(0),
            payload_type: 0,
            _unused_padding: 0,
        };
        *get_mut_helper(&mut data, 4 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 2 * TEST_BLOCK_WIDTH,
            color: Color::Red,
            value: TestOrderAsk::new(1),
            payload_type: 0,
            _unused_padding: 0,
        };
        let mut tree: RedBlackTree<TestOrderAsk> =
            RedBlackTree::new(&mut data, 0 * TEST_BLOCK_WIDTH, 1 * TEST_BLOCK_WIDTH);
        tree.verify_rb_tree::<TestOrderBid>();
        tree.pretty_print::<TestOrderBid>();

        tree.insert(5 * TEST_BLOCK_WIDTH, TestOrderAsk::new(0));
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_regression_5() {
        let mut data: [u8; 100000] = [0; 100000];
        *get_mut_helper(&mut data, 1 * TEST_BLOCK_WIDTH) = RBNode {
            left: 2 * TEST_BLOCK_WIDTH,
            right: 3 * TEST_BLOCK_WIDTH,
            parent: NIL,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(1),
        };
        *get_mut_helper(&mut data, 2 * TEST_BLOCK_WIDTH) = RBNode {
            left: 4 * TEST_BLOCK_WIDTH,
            right: 22 * TEST_BLOCK_WIDTH,
            parent: 1 * TEST_BLOCK_WIDTH,
            color: Color::Red,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(2),
        };
        *get_mut_helper(&mut data, 3 * TEST_BLOCK_WIDTH) = RBNode {
            left: 29 * TEST_BLOCK_WIDTH,
            right: 5 * TEST_BLOCK_WIDTH,
            parent: 1 * TEST_BLOCK_WIDTH,
            color: Color::Red,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(3),
        };
        *get_mut_helper(&mut data, 4 * TEST_BLOCK_WIDTH) = RBNode {
            left: 6 * TEST_BLOCK_WIDTH,
            right: 7 * TEST_BLOCK_WIDTH,
            parent: 2 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(4),
        };
        *get_mut_helper(&mut data, 5 * TEST_BLOCK_WIDTH) = RBNode {
            left: 8 * TEST_BLOCK_WIDTH,
            right: 9 * TEST_BLOCK_WIDTH,
            parent: 3 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(5),
        };
        *get_mut_helper(&mut data, 6 * TEST_BLOCK_WIDTH) = RBNode {
            left: 10 * TEST_BLOCK_WIDTH,
            right: 20 * TEST_BLOCK_WIDTH,
            parent: 4 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(6),
        };
        *get_mut_helper(&mut data, 7 * TEST_BLOCK_WIDTH) = RBNode {
            left: 11 * TEST_BLOCK_WIDTH,
            right: 12 * TEST_BLOCK_WIDTH,
            parent: 4 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(7),
        };
        *get_mut_helper(&mut data, 8 * TEST_BLOCK_WIDTH) = RBNode {
            left: 13 * TEST_BLOCK_WIDTH,
            right: 21 * TEST_BLOCK_WIDTH,
            parent: 5 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(8),
        };
        *get_mut_helper(&mut data, 9 * TEST_BLOCK_WIDTH) = RBNode {
            left: 14 * TEST_BLOCK_WIDTH,
            right: 15 * TEST_BLOCK_WIDTH,
            parent: 5 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(9),
        };
        *get_mut_helper(&mut data, 10 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 6 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(10),
        };
        *get_mut_helper(&mut data, 11 * TEST_BLOCK_WIDTH) = RBNode {
            left: 16 * TEST_BLOCK_WIDTH,
            right: 17 * TEST_BLOCK_WIDTH,
            parent: 7 * TEST_BLOCK_WIDTH,
            color: Color::Red,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(11),
        };
        *get_mut_helper(&mut data, 12 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 7 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(12),
        };
        *get_mut_helper(&mut data, 13 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 8 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(13),
        };
        *get_mut_helper(&mut data, 14 * TEST_BLOCK_WIDTH) = RBNode {
            left: 18 * TEST_BLOCK_WIDTH,
            right: 19 * TEST_BLOCK_WIDTH,
            parent: 9 * TEST_BLOCK_WIDTH,
            color: Color::Red,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(14),
        };
        *get_mut_helper(&mut data, 15 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 9 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(15),
        };
        *get_mut_helper(&mut data, 16 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 11 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(16),
        };
        *get_mut_helper(&mut data, 17 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 11 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(17),
        };
        *get_mut_helper(&mut data, 18 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 14 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(18),
        };
        *get_mut_helper(&mut data, 19 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 14 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(19),
        };
        *get_mut_helper(&mut data, 20 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 6 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(20),
        };
        *get_mut_helper(&mut data, 21 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 8 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(21),
        };
        *get_mut_helper(&mut data, 22 * TEST_BLOCK_WIDTH) = RBNode {
            left: 23 * TEST_BLOCK_WIDTH,
            right: 24 * TEST_BLOCK_WIDTH,
            parent: 2 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(22),
        };
        *get_mut_helper(&mut data, 23 * TEST_BLOCK_WIDTH) = RBNode {
            left: 25 * TEST_BLOCK_WIDTH,
            right: 26 * TEST_BLOCK_WIDTH,
            parent: 22 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(23),
        };
        *get_mut_helper(&mut data, 24 * TEST_BLOCK_WIDTH) = RBNode {
            left: 27 * TEST_BLOCK_WIDTH,
            right: 28 * TEST_BLOCK_WIDTH,
            parent: 22 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(24),
        };
        *get_mut_helper(&mut data, 25 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 23 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(25),
        };
        *get_mut_helper(&mut data, 26 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 23 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(26),
        };
        *get_mut_helper(&mut data, 27 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 24 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(27),
        };
        *get_mut_helper(&mut data, 28 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 24 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(28),
        };
        *get_mut_helper(&mut data, 29 * TEST_BLOCK_WIDTH) = RBNode {
            left: 30 * TEST_BLOCK_WIDTH,
            right: 31 * TEST_BLOCK_WIDTH,
            parent: 3 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(29),
        };
        *get_mut_helper(&mut data, 30 * TEST_BLOCK_WIDTH) = RBNode {
            left: 32 * TEST_BLOCK_WIDTH,
            right: 33 * TEST_BLOCK_WIDTH,
            parent: 29 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(30),
        };
        *get_mut_helper(&mut data, 31 * TEST_BLOCK_WIDTH) = RBNode {
            left: 34 * TEST_BLOCK_WIDTH,
            right: 35 * TEST_BLOCK_WIDTH,
            parent: 29 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(31),
        };
        *get_mut_helper(&mut data, 32 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 30 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(32),
        };
        *get_mut_helper(&mut data, 33 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 30 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(33),
        };
        *get_mut_helper(&mut data, 34 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 31 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(34),
        };
        *get_mut_helper(&mut data, 35 * TEST_BLOCK_WIDTH) = RBNode {
            left: NIL,
            right: NIL,
            parent: 31 * TEST_BLOCK_WIDTH,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value: TestOrderBid::new(35),
        };

        let mut tree: RedBlackTree<TestOrderBid> =
            RedBlackTree::new(&mut data, 1 * TEST_BLOCK_WIDTH, 15 * TEST_BLOCK_WIDTH);
        tree.verify_rb_tree::<TestOrderBid>();

        tree.remove_by_index(6 * TEST_BLOCK_WIDTH);

        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_read_only() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: RedBlackTree<TestOrderBid> = RedBlackTree::new(&mut data, NIL, NIL);

        tree.insert(TEST_BLOCK_WIDTH * 0, TestOrderBid::new(1111));
        tree.insert(TEST_BLOCK_WIDTH, TestOrderBid::new(1234));
        tree.insert(TEST_BLOCK_WIDTH * 2, TestOrderBid::new(1000));
        tree.insert(TEST_BLOCK_WIDTH * 3, TestOrderBid::new(2000));
        tree.insert(TEST_BLOCK_WIDTH * 4, TestOrderBid::new(3000));
        tree.insert(TEST_BLOCK_WIDTH * 5, TestOrderBid::new(4000));
        tree.insert(TEST_BLOCK_WIDTH * 6, TestOrderBid::new(5000));
        tree.insert(TEST_BLOCK_WIDTH * 7, TestOrderBid::new(6000));
        let root_index: DataIndex = tree.get_root_index();
        drop(tree);

        let tree: RedBlackTreeReadOnly<TestOrderBid> =
            RedBlackTreeReadOnly::new(&data, root_index, NIL);
        for _ in tree.iter::<TestOrderBid>() {
            println!("Iteration in read only tree");
        }
        tree.data();
        assert_eq!(tree.root_index(), root_index);
        assert_eq!(tree.max_index(), NIL);
    }

    #[derive(Copy, Clone, Pod, Zeroable)]
    #[repr(C)]
    struct TestOrder2 {
        order_id: u64,
        nonce: u64,
        padding: [u64; 15],
    }

    impl Ord for TestOrder2 {
        fn cmp(&self, other: &Self) -> Ordering {
            (self.order_id).cmp(&(other.order_id))
        }
    }

    impl PartialOrd for TestOrder2 {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    impl PartialEq for TestOrder2 {
        fn eq(&self, other: &Self) -> bool {
            (self.order_id) == (other.order_id) && (self.nonce) == (other.nonce)
        }
    }

    impl Eq for TestOrder2 {}

    impl Display for TestOrder2 {
        fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "{}", self.order_id)
        }
    }

    impl TestOrder2 {
        fn new(order_id: u64, nonce: u64) -> Self {
            TestOrder2 {
                order_id,
                nonce,
                padding: [0; 15],
            }
        }
    }

    // Equal lookup keys but not equal nodes.
    #[test]
    fn test_lookup_equal() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: RedBlackTree<TestOrder2> = RedBlackTree::new(&mut data, NIL, NIL);

        tree.insert(TEST_BLOCK_WIDTH * 0, TestOrder2::new(1000, 1234));
        tree.insert(TEST_BLOCK_WIDTH * 1, TestOrder2::new(1000, 2345));
        tree.insert(TEST_BLOCK_WIDTH * 2, TestOrder2::new(1000, 3456));
        tree.insert(TEST_BLOCK_WIDTH * 3, TestOrder2::new(1000, 4567));
        tree.insert(TEST_BLOCK_WIDTH * 4, TestOrder2::new(1000, 5678));
        tree.insert(TEST_BLOCK_WIDTH * 5, TestOrder2::new(1000, 6789));
        tree.insert(TEST_BLOCK_WIDTH * 6, TestOrder2::new(1000, 7890));
        tree.lookup_index(&TestOrder2::new(1_000, 1234));
        tree.lookup_index(&TestOrder2::new(1_000, 4567));
        tree.lookup_index(&TestOrder2::new(1_000, 7890));
    }
}

#[test]
fn test_hypertree_range_query() {
    let mut data: [u8; 100000] = [0; 100000];
    let tree = RedBlackTree::new(&mut data, NIL, NIL);
    let hypertree = HyperTree::new(tree);

    hypertree.insert(0, TestOrderBid::new(1000));
    hypertree.insert(1, TestOrderBid::new(2000));
    hypertree.insert(2, TestOrderBid::new(3000));
    hypertree.insert(3, TestOrderBid::new(4000));
    hypertree.insert(4, TestOrderBid::new(5000));

    let range_min = TestOrderBid::new(2000);
    let range_max = TestOrderBid::new(4000);

    let results: Vec<_> = hypertree.range(&range_min, &range_max).collect();

    assert_eq!(results.len(), 3);
    assert_eq!(results[0].1.order_id, 2000);
    assert_eq!(results[1].1.order_id, 3000);
    assert_eq!(results[2].1.order_id, 4000);
}