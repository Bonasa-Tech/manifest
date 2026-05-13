#[cfg(test)]
use crate::red_black_tree::RedBlackTreeTestHelpers;
use crate::{
    get_helper, get_mut_helper, trace, Color, DataIndex, GetRedBlackTreeData,
    GetRedBlackTreeReadOnlyData, HyperTreeWriteOperations, Payload, RBNode,
    RedBlackTreeReadOperationsHelpers, RedBlackTreeWriteOperationsHelpers, NIL,
};

/// A Left Leaning Red-Black tree which supports random access O(log n) and get max O(1)
/// https://tjkendev.github.io/bst-visualization/red-black-tree/left-leaning.html
/// This does not properly handle equal key values like the regular RBTree
/// because this does top-down deletions and does not branch when it finds an
/// equal key, just stops there.
pub struct LLRB<'a, V: Payload> {
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

impl<'a, V: Payload> GetRedBlackTreeData<'a> for LLRB<'a, V> {
    fn data(&mut self) -> &mut [u8] {
        self.data
    }

    fn set_root_index(&mut self, root_index: DataIndex) {
        self.root_index = root_index;
    }
}

/// A Red-Black tree which supports random access O(log n) and get max O(1),
/// but does not require the data to be mutable.
pub struct LLRBReadOnly<'a, V: Payload> {
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

impl<'a, V: Payload> LLRBReadOnly<'a, V> {
    pub fn new(data: &'a [u8], root_index: DataIndex, max_index: DataIndex) -> Self {
        LLRBReadOnly::<V> {
            root_index,
            data,
            max_index,
            phantom: std::marker::PhantomData,
        }
    }
}

impl<'a, V: Payload> GetRedBlackTreeReadOnlyData<'a> for LLRB<'a, V> {
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

impl<'a, V: Payload> GetRedBlackTreeReadOnlyData<'a> for LLRBReadOnly<'a, V> {
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

impl<'a, V: Payload> LLRB<'a, V> {
    /// Creates a new LLRB. Does not mutate data yet. Assumes the actual
    /// data in data is already well formed as a red black tree.
    pub fn new(data: &'a mut [u8], root_index: DataIndex, max_index: DataIndex) -> Self {
        LLRB::<V> {
            root_index,
            data,
            max_index,
            phantom: std::marker::PhantomData,
        }
    }

    /// Flip the color of this node and the children.
    /// Optimized to use toggle_color which avoids branching.
    fn color_flip(&mut self, index: DataIndex) {
        let left_index: DataIndex = self.get_left_index::<V>(index);
        self.toggle_color::<V>(left_index);

        let right_index: DataIndex = self.get_right_index::<V>(index);
        self.toggle_color::<V>(right_index);

        self.toggle_color::<V>(index);
    }

    fn insert_recursive(
        &mut self,
        current_index: DataIndex,
        to_insert_index: DataIndex,
    ) -> DataIndex {
        let mut current_index: DataIndex = current_index;
        if current_index == NIL {
            return to_insert_index;
        }

        let current: &RBNode<V> = get_helper::<RBNode<V>>(self.data, current_index);
        let to_insert: &RBNode<V> = get_helper::<RBNode<V>>(self.data, to_insert_index);
        if to_insert > current {
            let new_right: DataIndex =
                self.insert_recursive(self.get_right_index::<V>(current_index), to_insert_index);
            if new_right != NIL {
                self.set_parent_index::<V>(new_right, current_index);
                self.set_right_index::<V>(current_index, new_right);
            }
        } else {
            let new_left: DataIndex =
                self.insert_recursive(self.get_left_index::<V>(current_index), to_insert_index);
            if new_left != NIL {
                self.set_left_index::<V>(current_index, new_left);
                self.set_parent_index::<V>(new_left, current_index);
            }
        }

        let left_color: Color = self.get_color::<V>(self.get_left_index::<V>(current_index));
        let right_color: Color = self.get_color::<V>(self.get_right_index::<V>(current_index));
        if right_color == Color::Red && left_color == Color::Black {
            self.rotate_left::<V>(current_index);
            self.set_color::<V>(
                self.get_parent_index::<V>(current_index),
                self.get_color::<V>(current_index),
            );
            self.set_color::<V>(current_index, Color::Red);
            current_index = self.get_parent_index::<V>(current_index);
        }

        let left_color: Color = self.get_color::<V>(self.get_left_index::<V>(current_index));
        let left_left_color: Color =
            self.get_color::<V>(self.get_left_index::<V>(self.get_left_index::<V>(current_index)));
        if left_color == Color::Red && left_left_color == Color::Red {
            self.rotate_right::<V>(current_index);
            self.set_color::<V>(
                self.get_parent_index::<V>(current_index),
                self.get_color::<V>(current_index),
            );
            self.set_color::<V>(current_index, Color::Red);
            current_index = self.get_parent_index::<V>(current_index);
        }

        // This block can go just after the NIL check or here. Here gives 2-3.
        let left_color: Color = self.get_color::<V>(self.get_left_index::<V>(current_index));
        let right_color: Color = self.get_color::<V>(self.get_right_index::<V>(current_index));
        if left_color == Color::Red && right_color == Color::Red {
            self.color_flip(current_index);
        }

        NIL
    }

    fn delete_recursive(
        &mut self,
        current_index: DataIndex,
        to_delete_index: DataIndex,
    ) -> DataIndex {
        let mut current_index: DataIndex = current_index;

        if current_index == NIL {
            return NIL;
        }

        let current: &RBNode<V> = get_helper::<RBNode<V>>(self.data, current_index);
        let to_delete: &RBNode<V> = get_helper::<RBNode<V>>(self.data, to_delete_index);
        if to_delete < current {
            let left_index: DataIndex = self.get_left_index::<V>(current_index);
            let left_color: Color = self.get_color::<V>(left_index);
            let left_left_color: Color = self.get_color::<V>(self.get_left_index::<V>(left_index));
            if left_index == NIL || (left_color == Color::Black && left_left_color == Color::Black)
            {
                current_index = self.move_red_left(current_index);
            }
            let left_index: DataIndex = self.get_left_index::<V>(current_index);
            let delete_recursive_result: DataIndex =
                self.delete_recursive(left_index, to_delete_index);
            self.set_left_index::<V>(current_index, delete_recursive_result);
        } else {
            let left_index: DataIndex = self.get_left_index::<V>(current_index);
            let left_color: Color = self.get_color::<V>(left_index);
            if left_color == Color::Red {
                self.rotate_right::<V>(current_index);
                self.set_color::<V>(
                    self.get_parent_index::<V>(current_index),
                    self.get_color::<V>(current_index),
                );
                self.set_color::<V>(current_index, Color::Red);
                current_index = self.get_parent_index::<V>(current_index);
            }

            let right_index: DataIndex = self.get_right_index::<V>(current_index);
            let right_color: Color = self.get_color::<V>(right_index);
            let right_left_index: DataIndex = self.get_left_index::<V>(right_index);
            let right_left_color: Color = self.get_color::<V>(right_left_index);
            if right_index == NIL
                || (right_color == Color::Black && right_left_color == Color::Black)
            {
                current_index = self.move_red_right(current_index);
            }

            if to_delete_index == current_index {
                // Swap with the successor
                let min: DataIndex = self.get_min(self.get_right_index::<V>(current_index));
                self.swap_node_with_successor::<V>(current_index, min);

                // deleteMin on the right subtree
                let right_index: DataIndex = self.get_right_index::<V>(min);
                let delete_min_result: DataIndex = self.delete_min(right_index);
                self.set_right_index::<V>(current_index, delete_min_result);

                // Finish the swap
                current_index = min;
            } else {
                let right_index: DataIndex = self.get_right_index::<V>(current_index);
                let delete_recursive_result: DataIndex =
                    self.delete_recursive(right_index, to_delete_index);
                self.set_right_index::<V>(current_index, delete_recursive_result);
            }
        }
        self.fix_up(current_index)
    }

    // Go left til cant go left anymore
    fn get_min(&self, index: DataIndex) -> DataIndex {
        let mut current_index: DataIndex = index;
        while self.get_left_index::<V>(current_index) != NIL {
            current_index = self.get_left_index::<V>(current_index);
        }
        current_index
    }

    fn move_red_left(&mut self, index: DataIndex) -> DataIndex {
        let mut index: DataIndex = index;
        self.color_flip(index);
        let right_index: DataIndex = self.get_right_index::<V>(index);
        let right_left_index: DataIndex = self.get_left_index::<V>(right_index);
        let right_left_color: Color = self.get_color::<V>(right_left_index);
        if right_left_color == Color::Red {
            self.rotate_right::<V>(right_index);
            self.set_color::<V>(
                self.get_parent_index::<V>(index),
                self.get_color::<V>(index),
            );
            self.set_color::<V>(index, Color::Red);
            self.set_right_index::<V>(index, right_left_index);

            self.rotate_left::<V>(index);
            self.set_color::<V>(
                self.get_parent_index::<V>(index),
                self.get_color::<V>(index),
            );
            self.set_color::<V>(index, Color::Red);

            index = right_left_index;

            self.color_flip(index);
        }
        index
    }

    fn move_red_right(&mut self, index: DataIndex) -> DataIndex {
        let mut index: DataIndex = index;
        self.color_flip(index);
        let left_index: DataIndex = self.get_left_index::<V>(index);
        let left_left_index: DataIndex = self.get_left_index::<V>(left_index);
        let left_left_color: Color = self.get_color::<V>(left_left_index);
        if left_left_color == Color::Red {
            self.rotate_right::<V>(index);
            self.set_color::<V>(
                self.get_parent_index::<V>(index),
                self.get_color::<V>(index),
            );
            self.set_color::<V>(index, Color::Red);
            index = left_index;

            self.color_flip(index);
        }
        index
    }

    fn fix_up(&mut self, current_index: DataIndex) -> DataIndex {
        let mut current_index: DataIndex = current_index;
        let right_index: DataIndex = self.get_right_index::<V>(current_index);
        let right_color: Color = self.get_color::<V>(right_index);

        if right_color == Color::Red {
            self.rotate_left::<V>(current_index);
            self.set_color::<V>(
                self.get_parent_index::<V>(current_index),
                self.get_color::<V>(current_index),
            );
            self.set_color::<V>(current_index, Color::Red);
            current_index = self.get_parent_index::<V>(current_index);
        }

        let left_index: DataIndex = self.get_left_index::<V>(current_index);
        let left_color: Color = self.get_color::<V>(left_index);
        let left_left_index: DataIndex = self.get_left_index::<V>(left_index);
        let left_left_color: Color = self.get_color::<V>(left_left_index);

        if left_color == Color::Red && left_left_color == Color::Red {
            self.rotate_right::<V>(current_index);
            self.set_color::<V>(
                self.get_parent_index::<V>(current_index),
                self.get_color::<V>(current_index),
            );
            self.set_color::<V>(current_index, Color::Red);
            current_index = self.get_parent_index::<V>(current_index);
        }

        let left_index: DataIndex = self.get_left_index::<V>(current_index);
        let left_color: Color = self.get_color::<V>(left_index);
        let right_index: DataIndex = self.get_right_index::<V>(current_index);
        let right_color: Color = self.get_color::<V>(right_index);

        if left_color == Color::Red && right_color == Color::Red {
            self.color_flip(current_index);
        }
        current_index
    }

    fn delete_min(&mut self, current_index: DataIndex) -> DataIndex {
        let mut current_index: DataIndex = current_index;
        if self.get_left_index::<V>(current_index) == NIL {
            return NIL;
        }
        let left_index: DataIndex = self.get_left_index::<V>(current_index);
        let left_color: Color = self.get_color::<V>(left_index);
        let left_left_index: DataIndex = self.get_left_index::<V>(left_index);
        let left_left_color: Color = self.get_color::<V>(left_left_index);
        if left_color == Color::Black && left_left_color == Color::Black {
            current_index = self.move_red_left(current_index);
        }
        let left_index: DataIndex = self.get_left_index::<V>(current_index);
        let delete_min_result: DataIndex = self.delete_min(left_index);
        self.set_left_index::<V>(current_index, delete_min_result);

        self.fix_up(current_index)
    }

    #[cfg(test)]
    fn remove_by_value(&mut self, value: &V) {
        use crate::HyperTreeReadOperations;

        let index: DataIndex = self.lookup_index(value);
        if index == NIL {
            return;
        }
        self.remove_by_index(index);
    }
}

impl<'a, V: Payload> HyperTreeWriteOperations<'a, V> for LLRB<'a, V> {
    /// Insert and rebalance. The data at index should be already zeroed.
    fn insert(&mut self, index: DataIndex, value: V) {
        trace!("TREE insert {index}");

        let new_node: RBNode<V> = RBNode {
            left: NIL,
            right: NIL,
            parent: NIL,
            color: Color::Red,
            value,
            payload_type: 0,
            _unused_padding: 0,
        };
        *get_mut_helper::<RBNode<V>>(self.data, index) = new_node;

        // Case where this is now the root
        if self.root_index == NIL {
            self.root_index = index;
            self.max_index = index;
            self.set_color::<V>(self.root_index, Color::Black);
            return;
        }

        self.insert_recursive(self.root_index, index);
        self.set_color::<V>(self.root_index, Color::Black);

        if value > *self.get_value::<V>(self.max_index) {
            self.max_index = index;
        }

        #[cfg(test)]
        self.verify_rb_tree::<V>()
    }

    /// Remove a node by index and rebalance.
    fn remove_by_index(&mut self, index: DataIndex) {
        if index == self.max_index {
            self.max_index = self.get_parent_index::<V>(self.max_index);
        }
        self.delete_recursive(self.root_index, index);
    }
}

#[cfg(test)]
mod test {
    use crate::{
        red_black_tree::test::{TestOrderAsk, TestOrderBid, TEST_BLOCK_WIDTH},
        HyperTreeReadOperations,
    };

    use super::*;

    #[test]
    fn test_insert_basic() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: LLRB<TestOrderBid> = LLRB::new(&mut data, NIL, NIL);

        tree.insert(TEST_BLOCK_WIDTH * 0, TestOrderBid::new(100));

        tree.insert(TEST_BLOCK_WIDTH, TestOrderBid::new(200));
        tree.insert(TEST_BLOCK_WIDTH * 2, TestOrderBid::new(300));
        tree.insert(TEST_BLOCK_WIDTH * 3, TestOrderBid::new(400));
        tree.insert(TEST_BLOCK_WIDTH * 4, TestOrderBid::new(500));
        tree.insert(TEST_BLOCK_WIDTH * 5, TestOrderBid::new(600));
        tree.insert(TEST_BLOCK_WIDTH * 6, TestOrderBid::new(700));
        tree.insert(TEST_BLOCK_WIDTH * 7, TestOrderBid::new(800));
    }

    fn init_simple_tree(data: &mut [u8]) -> LLRB<TestOrderBid> {
        let mut tree: LLRB<TestOrderBid> = LLRB::new(data, NIL, NIL);

        for i in 1..12 {
            tree.insert(TEST_BLOCK_WIDTH * i, TestOrderBid::new((i * 1_000).into()));
        }
        tree
    }

    #[test]
    fn test_pretty_print() {
        let mut data: [u8; 100000] = [0; 100000];
        let tree: LLRB<TestOrderBid> = init_simple_tree(&mut data);
        tree.pretty_print::<TestOrderBid>();
    }

    #[test]
    fn test_insert_fix() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: LLRB<TestOrderBid> = init_simple_tree(&mut data);

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
        let mut tree: LLRB<TestOrderBid> = init_simple_tree(&mut data);

        for i in 1..12 {
            tree.remove_by_value(&TestOrderBid::new(i * 1_000));
        }
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_remove_fix_internal_successor_is_left() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: LLRB<TestOrderBid> = init_simple_tree(&mut data);
        tree.remove_by_value(&TestOrderBid::new(7 * 1_000));
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_remove_fix_internal_right_right_parent_red() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: LLRB<TestOrderBid> = init_simple_tree(&mut data);
        tree.remove_by_value(&TestOrderBid::new(6 * 1_000));
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_remove_fix_internal_successor_is_right_child() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: LLRB<TestOrderBid> = init_simple_tree(&mut data);
        tree.remove_by_value(&TestOrderBid::new(2 * 1_000));
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_remove_only_has_right_after_swap() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: LLRB<TestOrderBid> = init_simple_tree(&mut data);
        tree.remove_by_value(&TestOrderBid::new(5 * 1_000));
        tree.remove_by_value(&TestOrderBid::new(4 * 1_000));
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_remove_only_has_left_after_swap() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: LLRB<TestOrderBid> = init_simple_tree(&mut data);
        tree.remove_by_value(&TestOrderBid::new(11 * 1_000));
        tree.remove_by_value(&TestOrderBid::new(10 * 1_000));
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_internal_remove() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: LLRB<TestOrderBid> = init_simple_tree(&mut data);

        for i in 4..8 {
            tree.remove_by_value(&TestOrderBid::new(i * 1_000));
            tree.verify_rb_tree::<TestOrderBid>();
        }
    }

    #[test]
    fn test_rotate_right() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: LLRB<TestOrderBid> = LLRB::new(&mut data, NIL, NIL);

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
        let mut tree: LLRB<TestOrderBid> = init_simple_tree(&mut data);
        // Does not exist in the tree. Should fail silently.
        tree.remove_by_value(&TestOrderBid::new(99999));
        tree.remove_by_value(&TestOrderBid::new(1));
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_max() {
        let mut data: [u8; 100000] = [0; 100000];
        let tree: LLRB<TestOrderBid> = init_simple_tree(&mut data);
        assert_eq!(tree.get_max_index(), TEST_BLOCK_WIDTH * 11);
    }

    #[test]
    fn test_insert_right_left() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: LLRB<TestOrderBid> = LLRB::new(&mut data, NIL, NIL);
        tree.insert(TEST_BLOCK_WIDTH * 0, TestOrderBid::new(100));
        tree.insert(TEST_BLOCK_WIDTH, TestOrderBid::new(200));
        tree.insert(TEST_BLOCK_WIDTH * 2, TestOrderBid::new(300));
        tree.insert(TEST_BLOCK_WIDTH * 3, TestOrderBid::new(150));
        tree.insert(TEST_BLOCK_WIDTH * 4, TestOrderBid::new(125));
    }

    #[test]
    fn test_remove_left_left() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: LLRB<TestOrderBid> = LLRB::new(&mut data, NIL, NIL);
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
        let mut tree: LLRB<TestOrderBid> = LLRB::new(&mut data, NIL, NIL);
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
        let mut tree: LLRB<TestOrderBid> = LLRB::new(&mut data, NIL, NIL);
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
        let mut tree: LLRB<TestOrderBid> = LLRB::new(&mut data, NIL, NIL);
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
        let mut tree: LLRB<TestOrderBid> = LLRB::new(&mut data, NIL, NIL);
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
        let mut tree: LLRB<TestOrderBid> = LLRB::new(&mut data, NIL, NIL);
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
        let mut tree: LLRB<TestOrderBid> = init_simple_tree(&mut data);
        tree.insert(TEST_BLOCK_WIDTH * 12, TestOrderBid::new(4500));
        tree.insert(TEST_BLOCK_WIDTH * 13, TestOrderBid::new(5500));
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_empty_max() {
        let mut data: [u8; 100000] = [0; 100000];
        let tree: LLRB<TestOrderBid> = LLRB::new(&mut data, NIL, NIL);
        assert_eq!(tree.get_max_index(), NIL);
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_node_equality() {
        let mut data1: [u8; 100000] = [0; 100000];
        let mut data2: [u8; 100000] = [0; 100000];
        let _tree1: LLRB<TestOrderBid> = init_simple_tree(&mut data1);
        let _tree2: LLRB<TestOrderBid> = init_simple_tree(&mut data2);
        assert_ne!(
            get_helper::<RBNode<TestOrderBid>>(&mut data1, 1 * TEST_BLOCK_WIDTH),
            get_helper::<RBNode<TestOrderBid>>(&mut data2, 2 * TEST_BLOCK_WIDTH)
        );
    }

    #[test]
    fn test_insert_equal() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: LLRB<TestOrderBid> = init_simple_tree(&mut data);

        tree.insert(TEST_BLOCK_WIDTH * 12, TestOrderBid::new(4000));
        tree.insert(TEST_BLOCK_WIDTH * 13, TestOrderBid::new(5000));
        tree.insert(TEST_BLOCK_WIDTH * 14, TestOrderBid::new(1000));
        tree.insert(TEST_BLOCK_WIDTH * 15, TestOrderBid::new(1000));
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_insert_and_remove_complex() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: LLRB<TestOrderBid> = LLRB::new(&mut data, NIL, NIL);

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

        let mut tree: LLRB<TestOrderBid> = LLRB::new(&mut data, 5 * TEST_BLOCK_WIDTH, NIL);
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

        let mut tree: LLRB<TestOrderBid> = LLRB::new(&mut data, 6 * TEST_BLOCK_WIDTH, NIL);
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
        // Not a valid left leaner because of node 9
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
        let mut tree: LLRB<TestOrderAsk> =
            LLRB::new(&mut data, 0 * TEST_BLOCK_WIDTH, 1 * TEST_BLOCK_WIDTH);
        tree.verify_rb_tree::<TestOrderBid>();

        tree.insert(5 * TEST_BLOCK_WIDTH, TestOrderAsk::new(0));
        tree.verify_rb_tree::<TestOrderBid>();
    }

    #[test]
    fn test_read_only() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: LLRB<TestOrderBid> = LLRB::new(&mut data, NIL, NIL);

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

        let tree: LLRBReadOnly<TestOrderBid> = LLRBReadOnly::new(&data, root_index, NIL);
        for _ in tree.node_iter::<TestOrderBid>() {
            println!("Iteration in read only tree");
        }
        tree.data();
        assert_eq!(tree.root_index(), root_index);
        assert_eq!(tree.max_index(), NIL);
    }
}
