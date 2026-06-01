use crate::{
    utils_opt::{get_helper_unchecked, get_mut_helper_unchecked},
    Color, DataIndex, GetRedBlackTreeData, GetRedBlackTreeReadOnlyData, Payload, RBNode,
    RedBlackTreeReadOperationsHelpers, NIL,
};

#[cfg(feature = "certora")]
pub trait RedBlackTreeReadOperationsHelpersOpt<'a> {
    unsafe fn get_color_unchecked<V: Payload>(&self, index: DataIndex) -> Color;
    unsafe fn get_right_index_unchecked<V: Payload>(&self, index: DataIndex) -> DataIndex;
    unsafe fn get_left_index_unchecked<V: Payload>(&self, index: DataIndex) -> DataIndex;
    unsafe fn get_parent_index_unchecked<V: Payload>(&self, index: DataIndex) -> DataIndex;
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
        get_helper_unchecked::<RBNode<V>>(self.data(), index).color
    }

    #[inline(always)]
    unsafe fn get_right_index_unchecked<V: Payload>(&self, index: DataIndex) -> DataIndex {
        get_helper_unchecked::<RBNode<V>>(self.data(), index).right
    }

    #[inline(always)]
    unsafe fn get_left_index_unchecked<V: Payload>(&self, index: DataIndex) -> DataIndex {
        get_helper_unchecked::<RBNode<V>>(self.data(), index).left
    }

    #[inline(always)]
    unsafe fn get_parent_index_unchecked<V: Payload>(&self, index: DataIndex) -> DataIndex {
        get_helper_unchecked::<RBNode<V>>(self.data(), index).parent
    }

    #[inline(always)]
    unsafe fn get_node_unchecked<V: Payload>(&'a self, index: DataIndex) -> &'a RBNode<V> {
        get_helper_unchecked::<RBNode<V>>(self.data(), index)
    }
}

#[cfg(feature = "certora")]
pub trait RedBlackTreeWriteOperationsHelpersOpt<'a> {
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
        get_mut_helper_unchecked::<RBNode<V>>(self.data(), index).color = color;
    }

    #[inline(always)]
    unsafe fn set_parent_index_unchecked<V: Payload>(
        &mut self,
        index: DataIndex,
        parent_index: DataIndex,
    ) {
        get_mut_helper_unchecked::<RBNode<V>>(self.data(), index).parent = parent_index;
    }

    #[inline(always)]
    unsafe fn set_left_index_unchecked<V: Payload>(
        &mut self,
        index: DataIndex,
        left_index: DataIndex,
    ) {
        get_mut_helper_unchecked::<RBNode<V>>(self.data(), index).left = left_index;
    }

    #[inline(always)]
    unsafe fn set_right_index_unchecked<V: Payload>(
        &mut self,
        index: DataIndex,
        right_index: DataIndex,
    ) {
        get_mut_helper_unchecked::<RBNode<V>>(self.data(), index).right = right_index;
    }

    fn rotate_left_opt<V: Payload>(&mut self, index: DataIndex) {
        // Left rotate of G
        //         GG                     GG
        //         |                      |
        //         G                      P
        //       /   \                  /   \
        //      U     P     --->      G      X
        //          /   \           /   \
        //        Y      X        U       Y
        let g_index: DataIndex = index;

        // Batch-read G and P in two unchecked loads
        let (p_index, y_index, gg_index) = unsafe {
            let g_node: &RBNode<V> = get_helper_unchecked(self.data(), g_index);
            let p_idx = g_node.right;
            let gg_idx = g_node.parent;
            let p_node: &RBNode<V> = get_helper_unchecked(self.data(), p_idx);
            (p_idx, p_node.left, gg_idx)
        };

        // P: set parent and left in one write
        unsafe {
            let p_node: &mut RBNode<V> = get_mut_helper_unchecked(self.data(), p_index);
            p_node.parent = gg_index;
            p_node.left = g_index;
        }

        // Y: update parent only if non-NIL
        if y_index != NIL {
            unsafe {
                get_mut_helper_unchecked::<RBNode<V>>(self.data(), y_index).parent = g_index;
            }
        }

        // G: set parent and right in one write
        unsafe {
            let g_node: &mut RBNode<V> = get_mut_helper_unchecked(self.data(), g_index);
            g_node.parent = p_index;
            g_node.right = y_index;
        }

        // GG: update child pointer — fully unchecked
        if gg_index != NIL {
            unsafe {
                let gg_node: &mut RBNode<V> = get_mut_helper_unchecked(self.data(), gg_index);
                if gg_node.left == index {
                    gg_node.left = p_index;
                }
                if gg_node.right == index {
                    gg_node.right = p_index;
                }
            }
        }

        if self.root_index() == g_index {
            self.set_root_index(p_index);
        }
    }

    fn rotate_right_opt<V: Payload>(&mut self, index: DataIndex) {
        // Right rotate of G
        //         GG                     GG
        //         |                      |
        //         G                      P
        //       /   \                  /   \
        //      P     U     --->      X       G
        //    /  \                          /   \
        //  X     Y                       Y       U
        let g_index: DataIndex = index;

        let (p_index, y_index, gg_index) = unsafe {
            let g_node: &RBNode<V> = get_helper_unchecked(self.data(), g_index);
            let p_idx = g_node.left;
            let gg_idx = g_node.parent;
            let p_node: &RBNode<V> = get_helper_unchecked(self.data(), p_idx);
            (p_idx, p_node.right, gg_idx)
        };

        unsafe {
            let p_node: &mut RBNode<V> = get_mut_helper_unchecked(self.data(), p_index);
            p_node.parent = gg_index;
            p_node.right = g_index;
        }

        if y_index != NIL {
            unsafe {
                get_mut_helper_unchecked::<RBNode<V>>(self.data(), y_index).parent = g_index;
            }
        }

        unsafe {
            let g_node: &mut RBNode<V> = get_mut_helper_unchecked(self.data(), g_index);
            g_node.parent = p_index;
            g_node.left = y_index;
        }

        if gg_index != NIL {
            unsafe {
                let gg_node: &mut RBNode<V> = get_mut_helper_unchecked(self.data(), gg_index);
                if gg_node.left == index {
                    gg_node.left = p_index;
                }
                if gg_node.right == index {
                    gg_node.right = p_index;
                }
            }
        }

        if self.root_index() == g_index {
            self.set_root_index(p_index);
        }
    }

    fn insert_fix_opt<V: Payload>(&mut self, index_to_fix: DataIndex) -> DataIndex {
        if self.root_index() == index_to_fix {
            unsafe {
                self.set_color_unchecked::<V>(index_to_fix, Color::Black);
            }
            return NIL;
        }

        // Batch-read: node → parent → parent color
        let (parent_index, parent_color) = unsafe {
            let node: &RBNode<V> = get_helper_unchecked(self.data(), index_to_fix);
            let pi = node.parent;
            let pn: &RBNode<V> = get_helper_unchecked(self.data(), pi);
            (pi, pn.color)
        };

        if parent_color == Color::Black {
            return NIL;
        }

        // Read grandparent
        let grandparent_index = unsafe {
            get_helper_unchecked::<RBNode<V>>(self.data(), parent_index).parent
        };

        if grandparent_index == NIL {
            unsafe {
                self.set_color_unchecked::<V>(parent_index, Color::Black);
            }
            return NIL;
        }

        // Batch-read grandparent: left, color, derive uncle
        let (gp_left, gp_color, uncle_index) = unsafe {
            let gp: &RBNode<V> = get_helper_unchecked(self.data(), grandparent_index);
            let uncle = if gp.left == parent_index {
                gp.right
            } else {
                gp.left
            };
            (gp.left, gp.color, uncle)
        };

        let uncle_color = if uncle_index == NIL {
            Color::Black
        } else {
            unsafe { get_helper_unchecked::<RBNode<V>>(self.data(), uncle_index).color }
        };

        // Case I: Uncle is red — recolor and recurse up
        if uncle_color == Color::Red {
            unsafe {
                self.set_color_unchecked::<V>(parent_index, Color::Black);
                self.set_color_unchecked::<V>(uncle_index, Color::Black);
                self.set_color_unchecked::<V>(grandparent_index, Color::Red);
            }
            return grandparent_index;
        }

        // Cases II-V: Uncle is black — rotate
        let parent_is_left = gp_left == parent_index;
        let current_is_left = unsafe {
            get_helper_unchecked::<RBNode<V>>(self.data(), parent_index).left == index_to_fix
        };
        let index_to_fix_color = unsafe {
            get_helper_unchecked::<RBNode<V>>(self.data(), index_to_fix).color
        };

        if parent_is_left && current_is_left {
            // Case II: left-left
            self.rotate_right_opt::<V>(grandparent_index);
            unsafe {
                self.set_color_unchecked::<V>(grandparent_index, parent_color);
                self.set_color_unchecked::<V>(parent_index, gp_color);
            }
        } else if parent_is_left {
            // Case III: left-right
            self.rotate_left_opt::<V>(parent_index);
            self.rotate_right_opt::<V>(grandparent_index);
            unsafe {
                self.set_color_unchecked::<V>(index_to_fix, gp_color);
                self.set_color_unchecked::<V>(grandparent_index, index_to_fix_color);
            }
        } else if !current_is_left {
            // Case IV: right-right
            self.rotate_left_opt::<V>(grandparent_index);
            unsafe {
                self.set_color_unchecked::<V>(grandparent_index, parent_color);
                self.set_color_unchecked::<V>(parent_index, gp_color);
            }
        } else {
            // Case V: right-left
            self.rotate_right_opt::<V>(parent_index);
            self.rotate_left_opt::<V>(grandparent_index);
            unsafe {
                self.set_color_unchecked::<V>(index_to_fix, gp_color);
                self.set_color_unchecked::<V>(grandparent_index, index_to_fix_color);
            }
        }

        NIL
    }

    fn remove_fix_opt<V: Payload>(
        &mut self,
        current_index: DataIndex,
        parent_index: DataIndex,
    ) -> (DataIndex, DataIndex) {
        if self.root_index() == current_index {
            return (NIL, NIL);
        }

        // Determine sibling from parent's children — one unchecked read
        let (sibling_index, sibling_is_left) = unsafe {
            let pn: &RBNode<V> = get_helper_unchecked(self.data(), parent_index);
            if pn.left == current_index {
                (pn.right, false)
            } else {
                (pn.left, true)
            }
        };

        // Batch-read sibling fields
        let (sibling_color, sib_left, sib_right) = unsafe {
            let sn: &RBNode<V> = get_helper_unchecked(self.data(), sibling_index);
            (sn.color, sn.left, sn.right)
        };

        let parent_color =
            unsafe { get_helper_unchecked::<RBNode<V>>(self.data(), parent_index).color };

        // Read sibling children colors with NIL guard (NIL = Black)
        let sib_left_color = if sib_left == NIL {
            Color::Black
        } else {
            unsafe { get_helper_unchecked::<RBNode<V>>(self.data(), sib_left).color }
        };
        let sib_right_color = if sib_right == NIL {
            Color::Black
        } else {
            unsafe { get_helper_unchecked::<RBNode<V>>(self.data(), sib_right).color }
        };

        let sibling_has_red_child =
            sib_left_color == Color::Red || sib_right_color == Color::Red;

        // 3a: Sibling is black and has a red child
        if sibling_color == Color::Black && sibling_has_red_child {
            // i: left-left
            if sib_left_color == Color::Red && sibling_is_left {
                unsafe {
                    self.set_color_unchecked::<V>(sib_left, Color::Black);
                    self.set_color_unchecked::<V>(parent_index, sibling_color);
                    self.set_color_unchecked::<V>(sibling_index, parent_color);
                }
                self.rotate_right_opt::<V>(parent_index);
                return (NIL, NIL);
            }
            // ii: left-right
            if sib_right_color == Color::Red && sibling_is_left {
                unsafe {
                    self.set_color_unchecked::<V>(sib_right, parent_color);
                    self.set_color_unchecked::<V>(parent_index, Color::Black);
                    self.set_color_unchecked::<V>(sibling_index, Color::Black);
                }
                self.rotate_left_opt::<V>(sibling_index);
                self.rotate_right_opt::<V>(parent_index);
                return (NIL, NIL);
            }
            // iii: right-right
            if sib_right_color == Color::Red && !sibling_is_left {
                unsafe {
                    self.set_color_unchecked::<V>(sib_right, Color::Black);
                    self.set_color_unchecked::<V>(parent_index, sibling_color);
                    self.set_color_unchecked::<V>(sibling_index, parent_color);
                }
                self.rotate_left_opt::<V>(parent_index);
                return (NIL, NIL);
            }
            // iv: right-left
            if sib_left_color == Color::Red && !sibling_is_left {
                unsafe {
                    self.set_color_unchecked::<V>(sib_left, parent_color);
                    self.set_color_unchecked::<V>(parent_index, Color::Black);
                    self.set_color_unchecked::<V>(sibling_index, Color::Black);
                }
                self.rotate_right_opt::<V>(sibling_index);
                self.rotate_left_opt::<V>(parent_index);
                return (NIL, NIL);
            }
            unreachable!();
        }

        // 3b: Sibling is black, both children black
        if sibling_color == Color::Black {
            unsafe {
                self.set_color_unchecked::<V>(sibling_index, Color::Red);
            }
            if parent_color == Color::Black {
                let pp = unsafe {
                    get_helper_unchecked::<RBNode<V>>(self.data(), parent_index).parent
                };
                return (parent_index, pp);
            } else {
                unsafe {
                    self.set_color_unchecked::<V>(parent_index, Color::Black);
                }
                return (NIL, NIL);
            }
        }

        // 3c: Sibling is red
        if sibling_is_left {
            self.rotate_right_opt::<V>(parent_index);
        } else {
            self.rotate_left_opt::<V>(parent_index);
        }
        unsafe {
            self.set_color_unchecked::<V>(parent_index, Color::Red);
            self.set_color_unchecked::<V>(sibling_index, Color::Black);
        }
        (current_index, parent_index)
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
        let mut data1: [u8; 100000] = [0; 100000];
        let mut data2: [u8; 100000] = [0; 100000];

        let mut tree1: RedBlackTree<TestOrderBid> = RedBlackTree::new(&mut data1, NIL, NIL);
        let mut tree2: RedBlackTree<TestOrderBid> = RedBlackTree::new(&mut data2, NIL, NIL);

        for i in 1..8 {
            tree1.insert(TEST_BLOCK_WIDTH * i, TestOrderBid::new(i as u64 * 100));
            tree2.insert(TEST_BLOCK_WIDTH * i, TestOrderBid::new(i as u64 * 100));
        }

        assert_eq!(tree1.root_index(), tree2.root_index());
    }

    #[test]
    fn test_rotate_right_opt_equivalence() {
        let mut data1: [u8; 100000] = [0; 100000];
        let mut data2: [u8; 100000] = [0; 100000];

        let mut tree1: RedBlackTree<TestOrderBid> = RedBlackTree::new(&mut data1, NIL, NIL);
        let mut tree2: RedBlackTree<TestOrderBid> = RedBlackTree::new(&mut data2, NIL, NIL);

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
