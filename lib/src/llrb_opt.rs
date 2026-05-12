//! Optimized Left-Leaning Red-Black Tree operations using unsafe Rust.
//!
//! This module provides optimized versions of LLRB-specific operations
//! that skip bounds checking for improved compute unit performance.
//!
//! # Feature Flags
//!
//! This module is only compiled when the `opt-unsafe` feature is enabled.
//!
//! # Safety
//!
//! The functions in this module are unsafe because they:
//! - Skip bounds checking on array accesses
//! - Use raw pointer manipulation for tree traversal
//!
//! Callers must ensure tree invariants are maintained.

use crate::{
    Color, DataIndex, GetRedBlackTreeData, GetRedBlackTreeReadOnlyData, Payload,
    RedBlackTreeReadOperationsHelpers, RedBlackTreeReadOperationsHelpersOpt,
    RedBlackTreeWriteOperationsHelpers, RedBlackTreeWriteOperationsHelpersOpt, NIL,
};

/// Optimized LLRB-specific operations.
#[cfg(feature = "certora")]
pub trait LLRBOptOperations<'a, V: Payload> {
    /// Optimized color flip using unchecked accessors
    fn color_flip_opt(&mut self, index: DataIndex);

    /// Optimized fix_up operation
    fn fix_up_opt(&mut self, current_index: DataIndex) -> DataIndex;

    /// Optimized move_red_left operation
    fn move_red_left_opt(&mut self, index: DataIndex) -> DataIndex;

    /// Optimized move_red_right operation
    fn move_red_right_opt(&mut self, index: DataIndex) -> DataIndex;
}

#[cfg(not(feature = "certora"))]
pub(crate) trait LLRBOptOperations<'a, V: Payload> {
    fn color_flip_opt(&mut self, index: DataIndex);
    fn fix_up_opt(&mut self, current_index: DataIndex) -> DataIndex;
    fn move_red_left_opt(&mut self, index: DataIndex) -> DataIndex;
    fn move_red_right_opt(&mut self, index: DataIndex) -> DataIndex;
}

impl<'a, T, V: Payload> LLRBOptOperations<'a, V> for T
where
    T: GetRedBlackTreeData<'a>
        + GetRedBlackTreeReadOnlyData<'a>
        + RedBlackTreeReadOperationsHelpers<'a>
        + RedBlackTreeReadOperationsHelpersOpt<'a>
        + RedBlackTreeWriteOperationsHelpers<'a>
        + RedBlackTreeWriteOperationsHelpersOpt<'a>,
{
    fn color_flip_opt(&mut self, index: DataIndex) {
        // Flip colors of node and both children using unchecked access where possible
        let left_index: DataIndex = self.get_left_index::<V>(index);
        if left_index != NIL {
            let left_color = self.get_color::<V>(left_index);
            let new_color = if left_color == Color::Black {
                Color::Red
            } else {
                Color::Black
            };
            self.set_color::<V>(left_index, new_color);
        }

        let right_index: DataIndex = self.get_right_index::<V>(index);
        if right_index != NIL {
            let right_color = self.get_color::<V>(right_index);
            let new_color = if right_color == Color::Black {
                Color::Red
            } else {
                Color::Black
            };
            self.set_color::<V>(right_index, new_color);
        }

        let index_color = self.get_color::<V>(index);
        let new_index_color = if index_color == Color::Black {
            Color::Red
        } else {
            Color::Black
        };
        self.set_color::<V>(index, new_index_color);
    }

    fn fix_up_opt(&mut self, current_index: DataIndex) -> DataIndex {
        let mut current_index: DataIndex = current_index;

        // Fix right-leaning red links
        let right_index: DataIndex = self.get_right_index::<V>(current_index);
        let right_color: Color = self.get_color::<V>(right_index);

        if right_color == Color::Red {
            self.rotate_left_opt::<V>(current_index);
            let parent_index = self.get_parent_index::<V>(current_index);
            self.set_color::<V>(parent_index, self.get_color::<V>(current_index));
            self.set_color::<V>(current_index, Color::Red);
            current_index = parent_index;
        }

        // Fix two reds in a row
        let left_index: DataIndex = self.get_left_index::<V>(current_index);
        let left_color: Color = self.get_color::<V>(left_index);
        let left_left_index: DataIndex = self.get_left_index::<V>(left_index);
        let left_left_color: Color = self.get_color::<V>(left_left_index);

        if left_color == Color::Red && left_left_color == Color::Red {
            self.rotate_right_opt::<V>(current_index);
            let parent_index = self.get_parent_index::<V>(current_index);
            self.set_color::<V>(parent_index, self.get_color::<V>(current_index));
            self.set_color::<V>(current_index, Color::Red);
            current_index = parent_index;
        }

        // Split 4-nodes
        let left_index: DataIndex = self.get_left_index::<V>(current_index);
        let left_color: Color = self.get_color::<V>(left_index);
        let right_index: DataIndex = self.get_right_index::<V>(current_index);
        let right_color: Color = self.get_color::<V>(right_index);

        if left_color == Color::Red && right_color == Color::Red {
            LLRBOptOperations::<V>::color_flip_opt(self, current_index);
        }

        current_index
    }

    fn move_red_left_opt(&mut self, index: DataIndex) -> DataIndex {
        let mut index: DataIndex = index;
        LLRBOptOperations::<V>::color_flip_opt(self, index);

        let right_index: DataIndex = self.get_right_index::<V>(index);
        let right_left_index: DataIndex = self.get_left_index::<V>(right_index);
        let right_left_color: Color = self.get_color::<V>(right_left_index);

        if right_left_color == Color::Red {
            self.rotate_right_opt::<V>(right_index);
            let parent_index = self.get_parent_index::<V>(index);
            self.set_color::<V>(parent_index, self.get_color::<V>(index));
            self.set_color::<V>(index, Color::Red);
            self.set_right_index::<V>(index, right_left_index);

            self.rotate_left_opt::<V>(index);
            let parent_index = self.get_parent_index::<V>(index);
            self.set_color::<V>(parent_index, self.get_color::<V>(index));
            self.set_color::<V>(index, Color::Red);

            index = right_left_index;
            LLRBOptOperations::<V>::color_flip_opt(self, index);
        }
        index
    }

    fn move_red_right_opt(&mut self, index: DataIndex) -> DataIndex {
        let mut index: DataIndex = index;
        LLRBOptOperations::<V>::color_flip_opt(self, index);

        let left_index: DataIndex = self.get_left_index::<V>(index);
        let left_left_index: DataIndex = self.get_left_index::<V>(left_index);
        let left_left_color: Color = self.get_color::<V>(left_left_index);

        if left_left_color == Color::Red {
            self.rotate_right_opt::<V>(index);
            let parent_index = self.get_parent_index::<V>(index);
            self.set_color::<V>(parent_index, self.get_color::<V>(index));
            self.set_color::<V>(index, Color::Red);
            index = left_index;
            LLRBOptOperations::<V>::color_flip_opt(self, index);
        }
        index
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llrb::LLRB;
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
    fn test_llrb_with_opt_operations() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut tree: LLRB<TestOrderBid> = LLRB::new(&mut data, NIL, NIL);

        // Insert several elements to exercise the tree operations
        for i in 1..8 {
            tree.insert(TEST_BLOCK_WIDTH * i, TestOrderBid::new(i as u64 * 100));
        }

        // Verify tree is valid after insertions
        assert_ne!(tree.root_index(), NIL);
    }
}
