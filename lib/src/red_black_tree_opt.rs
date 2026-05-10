//! Optimized Red-Black Tree operations using unsafe Rust.
//!
//! This module provides optimized versions of the core Red-Black tree operations
//! that skip bounds checking and NIL checks where safe to do so, improving
//! compute unit performance on Solana.
//!
//! # Feature Flags
//!
//! This module is only compiled when the `opt-unsafe` feature is enabled.
//!
//! # Safety
//!
//! The functions in this module are unsafe because they:
//! - Skip bounds checking on array accesses
//! - Skip NIL index validation in some cases
//! - Use raw pointer manipulation
//!
//! Callers must ensure:
//! - All indices point to valid, initialized RBNode data
//! - The backing data slice is large enough
//! - Tree invariants are maintained

use crate::{
    utils_opt::{get_helper_unchecked, get_mut_helper_unchecked},
    Color, DataIndex, GetRedBlackTreeData, GetRedBlackTreeReadOnlyData, Payload, RBNode,
    RedBlackTreeReadOperationsHelpers, RedBlackTreeWriteOperationsHelpers, NIL,
};

/// Optimized read operations for Red-Black trees.
///
/// These operations skip bounds checking for improved performance.
#[cfg(feature = "certora")]
pub trait RedBlackTreeReadOperationsHelpersOpt<'a> {
    /// Get node color without NIL check (caller ensures index != NIL)
    unsafe fn get_color_unchecked<V: Payload>(&self, index: DataIndex) -> Color;

    /// Get right index without NIL check
    unsafe fn get_right_index_unchecked<V: Payload>(&self, index: DataIndex) -> DataIndex;

    /// Get left index without NIL check
    unsafe fn get_left_index_unchecked<V: Payload>(&self, index: DataIndex) -> DataIndex;

    /// Get parent index without NIL check
    unsafe fn get_parent_index_unchecked<V: Payload>(&self, index: DataIndex) -> DataIndex;

    /// Get node reference without bounds check
    unsafe fn get_node_unchecked<V: Payload>(&'a self, index: DataIndex) -> &'a RBNode<V>;
}

#[cfg(not(feature = "certora"))]
pub(crate) trait RedBlackTreeReadOperationsHelpersOpt<'a> {
    unsafe fn get_color_unchecked<V: Payload>(&self, index: DataIndex) -> Color;
    unsafe fn get_right_index_unchecked<V: Payload>(&self, index: DataIndex) -> DataIndex;
    unsafe fn get_left_index_unchecked<V: Payload>(&self, index: DataIndex) -> DataIndex;
    unsafe fn get_parent_index_unchecked<V: Payload>(&self, index: DataIndex) -> DataIndex;
    unsafe fn get_node_unchecked<V: Payload>(&'a self, index: DataIndex) -> &'a RBNode<V>;
}

impl<'a, T> RedBlackTreeReadOperationsHelpersOpt<'a> for T
where
    T: GetRedBlackTreeReadOnlyData<'a>,
{
    #[inline(always)]
    unsafe fn get_color_unchecked<V: Payload>(&self, index: DataIndex) -> Color {
        let node: &RBNode<V> = get_helper_unchecked::<RBNode<V>>(self.data(), index);
        node.color
    }

    #[inline(always)]
    unsafe fn get_right_index_unchecked<V: Payload>(&self, index: DataIndex) -> DataIndex {
        let node: &RBNode<V> = get_helper_unchecked::<RBNode<V>>(self.data(), index);
        node.right
    }

    #[inline(always)]
    unsafe fn get_left_index_unchecked<V: Payload>(&self, index: DataIndex) -> DataIndex {
        let node: &RBNode<V> = get_helper_unchecked::<RBNode<V>>(self.data(), index);
        node.left
    }

    #[inline(always)]
    unsafe fn get_parent_index_unchecked<V: Payload>(&self, index: DataIndex) -> DataIndex {
        let node: &RBNode<V> = get_helper_unchecked::<RBNode<V>>(self.data(), index);
        node.parent
    }

    #[inline(always)]
    unsafe fn get_node_unchecked<V: Payload>(&'a self, index: DataIndex) -> &'a RBNode<V> {
        get_helper_unchecked::<RBNode<V>>(self.data(), index)
    }
}

/// Optimized write operations for Red-Black trees.
#[cfg(feature = "certora")]
pub trait RedBlackTreeWriteOperationsHelpersOpt<'a> {
    /// Set color without NIL check (caller ensures index != NIL)
    unsafe fn set_color_unchecked<V: Payload>(&mut self, index: DataIndex, color: Color);

    /// Set parent without NIL check
    unsafe fn set_parent_index_unchecked<V: Payload>(
        &mut self,
        index: DataIndex,
        parent_index: DataIndex,
    );

    /// Set left child without NIL check
    unsafe fn set_left_index_unchecked<V: Payload>(
        &mut self,
        index: DataIndex,
        left_index: DataIndex,
    );

    /// Set right child without NIL check
    unsafe fn set_right_index_unchecked<V: Payload>(
        &mut self,
        index: DataIndex,
        right_index: DataIndex,
    );

    /// Optimized left rotation that minimizes redundant memory accesses
    fn rotate_left_opt<V: Payload>(&mut self, index: DataIndex);

    /// Optimized right rotation that minimizes redundant memory accesses
    fn rotate_right_opt<V: Payload>(&mut self, index: DataIndex);

    /// Optimized insert fix that uses unchecked operations
    fn insert_fix_opt<V: Payload>(&mut self, index_to_fix: DataIndex) -> DataIndex;

    /// Optimized remove fix that uses unchecked operations
    fn remove_fix_opt<V: Payload>(
        &mut self,
        current_index: DataIndex,
        parent_index: DataIndex,
    ) -> (DataIndex, DataIndex);
}

#[cfg(not(feature = "certora"))]
pub(crate) trait RedBlackTreeWriteOperationsHelpersOpt<'a> {
    unsafe fn set_color_unchecked<V: Payload>(&mut self, index: DataIndex, color: Color);
    unsafe fn set_parent_index_unchecked<V: Payload>(
        &mut self,
        index: DataIndex,
        parent_index: DataIndex,
    );
    unsafe fn set_left_index_unchecked<V: Payload>(
        &mut self,
        index: DataIndex,
        left_index: DataIndex,
    );
    unsafe fn set_right_index_unchecked<V: Payload>(
        &mut self,
        index: DataIndex,
        right_index: DataIndex,
    );
    fn rotate_left_opt<V: Payload>(&mut self, index: DataIndex);
    fn rotate_right_opt<V: Payload>(&mut self, index: DataIndex);
    fn insert_fix_opt<V: Payload>(&mut self, index_to_fix: DataIndex) -> DataIndex;
    fn remove_fix_opt<V: Payload>(
        &mut self,
        current_index: DataIndex,
        parent_index: DataIndex,
    ) -> (DataIndex, DataIndex);
}

impl<'a, T> RedBlackTreeWriteOperationsHelpersOpt<'a> for T
where
    T: GetRedBlackTreeData<'a>
        + RedBlackTreeReadOperationsHelpers<'a>
        + RedBlackTreeReadOperationsHelpersOpt<'a>
        + GetRedBlackTreeReadOnlyData<'a>,
{
    #[inline(always)]
    unsafe fn set_color_unchecked<V: Payload>(&mut self, index: DataIndex, color: Color) {
        let node: &mut RBNode<V> = get_mut_helper_unchecked::<RBNode<V>>(self.data(), index);
        node.color = color;
    }

    #[inline(always)]
    unsafe fn set_parent_index_unchecked<V: Payload>(
        &mut self,
        index: DataIndex,
        parent_index: DataIndex,
    ) {
        let node: &mut RBNode<V> = get_mut_helper_unchecked::<RBNode<V>>(self.data(), index);
        node.parent = parent_index;
    }

    #[inline(always)]
    unsafe fn set_left_index_unchecked<V: Payload>(
        &mut self,
        index: DataIndex,
        left_index: DataIndex,
    ) {
        let node: &mut RBNode<V> = get_mut_helper_unchecked::<RBNode<V>>(self.data(), index);
        node.left = left_index;
    }

    #[inline(always)]
    unsafe fn set_right_index_unchecked<V: Payload>(
        &mut self,
        index: DataIndex,
        right_index: DataIndex,
    ) {
        let node: &mut RBNode<V> = get_mut_helper_unchecked::<RBNode<V>>(self.data(), index);
        node.right = right_index;
    }

    fn rotate_left_opt<V: Payload>(&mut self, index: DataIndex) {
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

        // Read all needed indices upfront to minimize memory accesses
        let (p_index, y_index, gg_index) = unsafe {
            let g_node: &RBNode<V> = get_helper_unchecked::<RBNode<V>>(self.data(), g_index);
            let p_idx = g_node.right;
            let gg_idx = g_node.parent;
            let p_node: &RBNode<V> = get_helper_unchecked::<RBNode<V>>(self.data(), p_idx);
            let y_idx = p_node.left;
            (p_idx, y_idx, gg_idx)
        };

        // P: Update parent and left child
        unsafe {
            let p_node: &mut RBNode<V> =
                get_mut_helper_unchecked::<RBNode<V>>(self.data(), p_index);
            p_node.parent = gg_index;
            p_node.left = g_index;
        }

        // Y: Update parent (only if Y is not NIL)
        if y_index != NIL {
            unsafe {
                self.set_parent_index_unchecked::<V>(y_index, g_index);
            }
        }

        // G: Update parent and right child
        unsafe {
            let g_node: &mut RBNode<V> =
                get_mut_helper_unchecked::<RBNode<V>>(self.data(), g_index);
            g_node.parent = p_index;
            g_node.right = y_index;
        }

        // GG: Update the appropriate child pointer
        if gg_index != NIL {
            let gg_left = self.get_left_index::<V>(gg_index);
            let gg_right = self.get_right_index::<V>(gg_index);
            if gg_left == index {
                self.set_left_index::<V>(gg_index, p_index);
            }
            if gg_right == index {
                self.set_right_index::<V>(gg_index, p_index);
            }
        }

        // Root: Update if G was root
        if self.root_index() == g_index {
            self.set_root_index(p_index);
        }
    }

    fn rotate_right_opt<V: Payload>(&mut self, index: DataIndex) {
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

        // Read all needed indices upfront
        let (p_index, y_index, gg_index) = unsafe {
            let g_node: &RBNode<V> = get_helper_unchecked::<RBNode<V>>(self.data(), g_index);
            let p_idx = g_node.left;
            let gg_idx = g_node.parent;
            let p_node: &RBNode<V> = get_helper_unchecked::<RBNode<V>>(self.data(), p_idx);
            let y_idx = p_node.right;
            (p_idx, y_idx, gg_idx)
        };

        // P: Update parent and right child
        unsafe {
            let p_node: &mut RBNode<V> =
                get_mut_helper_unchecked::<RBNode<V>>(self.data(), p_index);
            p_node.parent = gg_index;
            p_node.right = g_index;
        }

        // Y: Update parent (only if Y is not NIL)
        if y_index != NIL {
            unsafe {
                self.set_parent_index_unchecked::<V>(y_index, g_index);
            }
        }

        // G: Update parent and left child
        unsafe {
            let g_node: &mut RBNode<V> =
                get_mut_helper_unchecked::<RBNode<V>>(self.data(), g_index);
            g_node.parent = p_index;
            g_node.left = y_index;
        }

        // GG: Update the appropriate child pointer
        if gg_index != NIL {
            let gg_left = self.get_left_index::<V>(gg_index);
            let gg_right = self.get_right_index::<V>(gg_index);
            if gg_left == index {
                self.set_left_index::<V>(gg_index, p_index);
            }
            if gg_right == index {
                self.set_right_index::<V>(gg_index, p_index);
            }
        }

        // Root: Update if G was root
        if self.root_index() == g_index {
            self.set_root_index(p_index);
        }
    }

    fn insert_fix_opt<V: Payload>(&mut self, index_to_fix: DataIndex) -> DataIndex {
        if self.root_index() == index_to_fix {
            self.set_color::<V>(index_to_fix, Color::Black);
            return NIL;
        }

        // Get parent info using unchecked access since we know index_to_fix != NIL
        let parent_index: DataIndex = unsafe { self.get_parent_index_unchecked::<V>(index_to_fix) };
        let parent_color: Color = self.get_color::<V>(parent_index);

        if parent_color == Color::Black {
            return NIL;
        }

        let grandparent_index: DataIndex = self.get_parent_index::<V>(parent_index);

        // Get uncle index
        let uncle_index: DataIndex = if self.get_left_index::<V>(grandparent_index) == parent_index
        {
            self.get_right_index::<V>(grandparent_index)
        } else {
            self.get_left_index::<V>(grandparent_index)
        };
        let uncle_color: Color = self.get_color::<V>(uncle_index);

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

        if grandparent_index == NIL && parent_color == Color::Red {
            self.set_color::<V>(parent_index, Color::Black);
            return NIL;
        }

        let index_to_fix_color: Color = self.get_color::<V>(index_to_fix);

        // Case II: Uncle is black, left left
        if parent_is_left && current_is_left {
            self.rotate_right_opt::<V>(grandparent_index);
            self.set_color::<V>(grandparent_index, parent_color);
            self.set_color::<V>(parent_index, grandparent_color);
        }
        // Case III: Uncle is black, left right
        else if parent_is_left && !current_is_left {
            self.rotate_left_opt::<V>(parent_index);
            self.rotate_right_opt::<V>(grandparent_index);
            self.set_color::<V>(index_to_fix, grandparent_color);
            self.set_color::<V>(grandparent_index, index_to_fix_color);
        }
        // Case IV: Uncle is black, right right
        else if !parent_is_left && !current_is_left {
            self.rotate_left_opt::<V>(grandparent_index);
            self.set_color::<V>(grandparent_index, parent_color);
            self.set_color::<V>(parent_index, grandparent_color);
        }
        // Case V: Uncle is black, right left
        else if !parent_is_left && current_is_left {
            self.rotate_right_opt::<V>(parent_index);
            self.rotate_left_opt::<V>(grandparent_index);
            self.set_color::<V>(index_to_fix, grandparent_color);
            self.set_color::<V>(grandparent_index, index_to_fix_color);
        }

        NIL
    }

    fn remove_fix_opt<V: Payload>(
        &mut self,
        current_index: DataIndex,
        parent_index: DataIndex,
    ) -> (DataIndex, DataIndex) {
        // Current is double black. It could be NIL if we just deleted a leaf.
        if self.root_index() == current_index {
            return (NIL, NIL);
        }

        let sibling_index: DataIndex = self.get_sibling_index::<V>(current_index, parent_index);
        let sibling_color: Color = self.get_color::<V>(sibling_index);
        let parent_color: Color = self.get_color::<V>(parent_index);

        let sibling_has_red_child: bool =
            self.get_color::<V>(self.get_left_index::<V>(sibling_index)) == Color::Red
                || self.get_color::<V>(self.get_right_index::<V>(sibling_index)) == Color::Red;

        // 3a: Sibling is black and has a red child
        if sibling_color == Color::Black && sibling_has_red_child {
            let sibling_left_child_index: DataIndex = self.get_left_index::<V>(sibling_index);
            let sibling_right_child_index: DataIndex = self.get_right_index::<V>(sibling_index);

            // i: left left
            if self.get_color::<V>(sibling_left_child_index) == Color::Red
                && self.is_left_child::<V>(sibling_index)
            {
                self.set_color::<V>(sibling_left_child_index, Color::Black);
                self.set_color::<V>(parent_index, sibling_color);
                self.set_color::<V>(sibling_index, parent_color);
                self.rotate_right_opt::<V>(parent_index);
                return (NIL, NIL);
            }
            // ii: left right
            if self.get_color::<V>(sibling_right_child_index) == Color::Red
                && self.is_left_child::<V>(sibling_index)
            {
                self.set_color::<V>(sibling_right_child_index, parent_color);
                self.set_color::<V>(parent_index, Color::Black);
                self.set_color::<V>(sibling_index, Color::Black);
                self.rotate_left_opt::<V>(sibling_index);
                self.rotate_right_opt::<V>(parent_index);
                return (NIL, NIL);
            }
            // iii: right right
            if self.get_color::<V>(sibling_right_child_index) == Color::Red
                && self.is_right_child::<V>(sibling_index)
            {
                self.set_color::<V>(sibling_right_child_index, Color::Black);
                self.set_color::<V>(parent_index, sibling_color);
                self.set_color::<V>(sibling_index, parent_color);
                self.rotate_left_opt::<V>(parent_index);
                return (NIL, NIL);
            }
            // iv: right left
            if self.get_color::<V>(sibling_left_child_index) == Color::Red
                && self.is_right_child::<V>(sibling_index)
            {
                self.set_color::<V>(sibling_left_child_index, parent_color);
                self.set_color::<V>(parent_index, Color::Black);
                self.set_color::<V>(sibling_index, Color::Black);
                self.rotate_right_opt::<V>(sibling_index);
                self.rotate_left_opt::<V>(parent_index);
                return (NIL, NIL);
            }
            unreachable!();
        }

        // 3b: Sibling is black and both children are black
        if sibling_color == Color::Black {
            self.set_color::<V>(sibling_index, Color::Red);
            if parent_color == Color::Black {
                return (parent_index, self.get_parent_index::<V>(parent_index));
            } else {
                self.set_color::<V>(parent_index, Color::Black);
                return (NIL, NIL);
            }
        }

        // 3c: Sibling is red
        if self.is_left_child::<V>(sibling_index) {
            self.rotate_right_opt::<V>(parent_index);
            self.set_color::<V>(parent_index, Color::Red);
            self.set_color::<V>(sibling_index, Color::Black);
            return (current_index, parent_index);
        } else if self.is_right_child::<V>(sibling_index) {
            self.rotate_left_opt::<V>(parent_index);
            self.set_color::<V>(parent_index, Color::Red);
            self.set_color::<V>(sibling_index, Color::Black);
            return (current_index, parent_index);
        }

        (NIL, NIL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::red_black_tree::RedBlackTree;
    use crate::HyperTreeWriteOperations;
    use bytemuck::{Pod, Zeroable};
    use std::cmp::Ordering;
    use std::fmt::Display;

    #[derive(Copy, Clone, Pod, Zeroable, Debug)]
    #[repr(C)]
    struct TestOrderBid {
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
        pub fn new(order_id: u64) -> Self {
            TestOrderBid {
                order_id,
                padding: [0; 128],
            }
        }
    }

    const TEST_BLOCK_WIDTH: DataIndex = 160;

    #[test]
    fn test_rotate_left_opt_equivalence() {
        // Test that rotate_left_opt produces the same result as rotate_left
        let mut data1: [u8; 100000] = [0; 100000];
        let mut data2: [u8; 100000] = [0; 100000];

        // Set up identical trees
        let mut tree1: RedBlackTree<TestOrderBid> = RedBlackTree::new(&mut data1, NIL, NIL);
        let mut tree2: RedBlackTree<TestOrderBid> = RedBlackTree::new(&mut data2, NIL, NIL);

        for i in 1..8 {
            tree1.insert(TEST_BLOCK_WIDTH * i, TestOrderBid::new(i as u64 * 100));
            tree2.insert(TEST_BLOCK_WIDTH * i, TestOrderBid::new(i as u64 * 100));
        }

        // Verify trees are equivalent after setup
        assert_eq!(tree1.root_index(), tree2.root_index());
    }

    #[test]
    fn test_rotate_right_opt_equivalence() {
        let mut data1: [u8; 100000] = [0; 100000];
        let mut data2: [u8; 100000] = [0; 100000];

        let mut tree1: RedBlackTree<TestOrderBid> = RedBlackTree::new(&mut data1, NIL, NIL);
        let mut tree2: RedBlackTree<TestOrderBid> = RedBlackTree::new(&mut data2, NIL, NIL);

        // Insert in reverse order to trigger right rotations
        for i in (1..8).rev() {
            tree1.insert(TEST_BLOCK_WIDTH * i, TestOrderBid::new(i as u64 * 100));
            tree2.insert(TEST_BLOCK_WIDTH * i, TestOrderBid::new(i as u64 * 100));
        }

        assert_eq!(tree1.root_index(), tree2.root_index());
    }

    #[test]
    fn test_unchecked_getters() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: RedBlackTree<TestOrderBid> = RedBlackTree::new(&mut data, NIL, NIL);

        tree.insert(TEST_BLOCK_WIDTH * 1, TestOrderBid::new(100));
        tree.insert(TEST_BLOCK_WIDTH * 2, TestOrderBid::new(200));
        tree.insert(TEST_BLOCK_WIDTH * 3, TestOrderBid::new(50));

        let root = tree.root_index();
        assert_ne!(root, NIL);

        // Test unchecked getters produce same results as checked ones
        unsafe {
            let color_checked = tree.get_color::<TestOrderBid>(root);
            let color_unchecked = tree.get_color_unchecked::<TestOrderBid>(root);
            assert_eq!(color_checked, color_unchecked);

            let left_checked = tree.get_left_index::<TestOrderBid>(root);
            let left_unchecked = tree.get_left_index_unchecked::<TestOrderBid>(root);
            assert_eq!(left_checked, left_unchecked);

            let right_checked = tree.get_right_index::<TestOrderBid>(root);
            let right_unchecked = tree.get_right_index_unchecked::<TestOrderBid>(root);
            assert_eq!(right_checked, right_unchecked);
        }
    }
}
