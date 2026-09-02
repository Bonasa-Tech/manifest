use crate::cvt_vacuity_check;

use super::verification_utils::init_static;
use bytemuck::{Pod, Zeroable};
use cvt::{cvt_assert, cvt_assume};
use cvt_macros::rule;
pub use hypertree::red_black_tree::*;
use hypertree::{
    get_helper, get_mut_helper, DataIndex, HyperTreeReadOperations, HyperTreeWriteOperations, NIL,
};
use nondet::*;
use solana_program::account_info::AccountInfo;
use std::{cmp::Ordering, fmt::Display};

#[derive(Copy, Clone, Pod, Zeroable, Debug)]
#[repr(C)]
pub struct TestOrder {
    order_id: u64,
}

impl Ord for TestOrder {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.order_id).cmp(&(other.order_id))
    }
}

impl PartialOrd for TestOrder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for TestOrder {
    fn eq(&self, other: &Self) -> bool {
        (self.order_id) == (other.order_id)
    }
}

impl Eq for TestOrder {}

impl Display for TestOrder {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.order_id)
    }
}

impl TestOrder {
    pub fn new(order_id: u64) -> Self {
        TestOrder { order_id }
    }
}

// Blocks are
// Left: DataIndex
// Right: DataIndex
// Parent: DataIndex
// Color: DataIndex
// TestOrder: 8
// 8 + 8 + 8 + 8 + 8 = 40
const TEST_BLOCK_WIDTH: DataIndex = 40;

macro_rules! mk_rb_node {
    ($left: expr, $right: expr, $parent: expr, $color: expr, $value: expr) => {{
        let left_nd: DataIndex = nondet();
        let right_nd: DataIndex = nondet();
        let parent_nd: DataIndex = nondet();
        let color_nd = if (nondet::<bool>()) {
            Color::Black
        } else {
            Color::Red
        };
        cvt_assume!(left_nd == $left as DataIndex);
        cvt_assume!(right_nd == $right as DataIndex);
        cvt_assume!(parent_nd == $parent as DataIndex);
        cvt_assume!(color_nd == $color);
        RBNode {
            left: left_nd,
            right: right_nd,
            parent: parent_nd,
            color: color_nd,
            payload_type: 0,
            _unused_padding: 0,
            value: $value,
        }
    }};
}

// The rules for rotation (both left and right) hardcode the following facts:
// 1. position of nodes
// 2. color of the nodes (GG is black, G is red, etc.)
// 3. G is left child of GG
// 4. U, Y and X have no children

// The rules currently do not check that colors and values of the nodes are unmodified by rotations

// Left rotate of G
//
//         GG                     GG
//         |                      |
//         G                      P
//       /   \                  /   \
//      U     P     --->      G      X
//          /   \           /   \
//        Y      X        U       Y
#[rule]
pub fn rule_rotate_left() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let gg_index = 0 * TEST_BLOCK_WIDTH;
    let g_index = 1 * TEST_BLOCK_WIDTH;
    let u_index = 2 * TEST_BLOCK_WIDTH;
    let p_index = 3 * TEST_BLOCK_WIDTH;
    let y_index = 4 * TEST_BLOCK_WIDTH;
    let x_index = 5 * TEST_BLOCK_WIDTH;

    let gg_val: u64 = nondet();
    let g_val: u64 = nondet();
    let u_val: u64 = nondet();
    let p_val: u64 = nondet();
    let y_val: u64 = nondet();
    let x_val: u64 = nondet();

    // GG
    *get_mut_helper(&mut data, gg_index) =
        mk_rb_node!(g_index, NIL, NIL, Color::Black, TestOrder::new(gg_val));

    // G
    *get_mut_helper(&mut data, g_index) = mk_rb_node!(
        u_index,
        p_index,
        gg_index,
        Color::Red,
        TestOrder::new(g_val)
    );

    // U
    *get_mut_helper(&mut data, u_index) =
        mk_rb_node!(NIL, NIL, g_index, Color::Black, TestOrder::new(u_val));

    // P
    *get_mut_helper(&mut data, p_index) = mk_rb_node!(
        y_index,
        x_index,
        g_index,
        Color::Black,
        TestOrder::new(p_val)
    );

    // Y
    *get_mut_helper(&mut data, y_index) =
        mk_rb_node!(NIL, NIL, p_index, Color::Red, TestOrder::new(y_val));

    // X
    *get_mut_helper(&mut data, x_index) =
        mk_rb_node!(NIL, NIL, p_index, Color::Red, TestOrder::new(x_val));
    let mut tree: RedBlackTree<TestOrder> = RedBlackTree::new(&mut data, gg_index, NIL);

    tree.rotate_left::<TestOrder>(g_index);

    // Asserts

    // GG asserts
    let gg_left = tree.get_left_index::<TestOrder>(gg_index);
    cvt_assert!(gg_left == p_index);

    // P asserts
    let p_parent = tree.get_parent_index::<TestOrder>(p_index);
    let p_left = tree.get_left_index::<TestOrder>(p_index);
    let p_right = tree.get_right_index::<TestOrder>(p_index);
    cvt_assert!(p_parent == gg_index);
    cvt_assert!(p_left == g_index);
    cvt_assert!(p_right == x_index);

    // G asserts
    let g_parent = tree.get_parent_index::<TestOrder>(g_index);
    let g_left = tree.get_left_index::<TestOrder>(g_index);
    let g_right = tree.get_right_index::<TestOrder>(g_index);
    cvt_assert!(g_parent == p_index);
    cvt_assert!(g_left == u_index);
    cvt_assert!(g_right == y_index);

    // X asserts
    let x_parent = tree.get_parent_index::<TestOrder>(x_index);
    cvt_assert!(x_parent == p_index);

    // U asserts
    let u_parent = tree.get_parent_index::<TestOrder>(u_index);
    cvt_assert!(u_parent == g_index);

    // Y asserts
    let y_parent = tree.get_parent_index::<TestOrder>(y_index);
    cvt_assert!(y_parent == g_index);

    cvt_vacuity_check!();
}

// Right rotate of G
//
//         GG                     GG
//         |                      |
//         G                      P
//       /   \                  /   \
//      P     U     --->      X       G
//    /  \                          /   \
//  X     Y                       Y       U
#[rule]
pub fn rule_rotate_right() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let gg_index = 0 * TEST_BLOCK_WIDTH;
    let g_index = 1 * TEST_BLOCK_WIDTH;
    let u_index = 2 * TEST_BLOCK_WIDTH;
    let p_index = 3 * TEST_BLOCK_WIDTH;
    let y_index = 4 * TEST_BLOCK_WIDTH;
    let x_index = 5 * TEST_BLOCK_WIDTH;

    let gg_val: u64 = nondet();
    let g_val: u64 = nondet();
    let u_val: u64 = nondet();
    let p_val: u64 = nondet();
    let y_val: u64 = nondet();
    let x_val: u64 = nondet();

    // GG
    *get_mut_helper(&mut data, gg_index) =
        mk_rb_node!(g_index, NIL, NIL, Color::Black, TestOrder::new(gg_val));

    // G
    *get_mut_helper(&mut data, g_index) = mk_rb_node!(
        p_index,
        u_index,
        gg_index,
        Color::Red,
        TestOrder::new(g_val)
    );

    // P
    *get_mut_helper(&mut data, p_index) = mk_rb_node!(
        x_index,
        y_index,
        g_index,
        Color::Black,
        TestOrder::new(p_val)
    );

    // U
    *get_mut_helper(&mut data, u_index) =
        mk_rb_node!(NIL, NIL, g_index, Color::Black, TestOrder::new(u_val));

    // X
    *get_mut_helper(&mut data, x_index) =
        mk_rb_node!(NIL, NIL, p_index, Color::Red, TestOrder::new(x_val));

    // Y
    *get_mut_helper(&mut data, y_index) =
        mk_rb_node!(NIL, NIL, p_index, Color::Red, TestOrder::new(y_val));

    let mut tree: RedBlackTree<TestOrder> = RedBlackTree::new(&mut data, gg_index, NIL);

    tree.rotate_right::<TestOrder>(g_index);

    // Asserts

    // GG asserts
    let gg_left = tree.get_left_index::<TestOrder>(gg_index);
    cvt_assert!(gg_left == p_index);

    // P asserts
    let p_parent = tree.get_parent_index::<TestOrder>(p_index);
    let p_left = tree.get_left_index::<TestOrder>(p_index);
    let p_right = tree.get_right_index::<TestOrder>(p_index);
    cvt_assert!(p_parent == gg_index);
    cvt_assert!(p_left == x_index);
    cvt_assert!(p_right == g_index);

    // G asserts
    let g_parent = tree.get_parent_index::<TestOrder>(g_index);
    let g_left = tree.get_left_index::<TestOrder>(g_index);
    let g_right = tree.get_right_index::<TestOrder>(g_index);
    cvt_assert!(g_parent == p_index);
    cvt_assert!(g_left == y_index);
    cvt_assert!(g_right == u_index);

    // X asserts
    let x_parent = tree.get_parent_index::<TestOrder>(x_index);
    cvt_assert!(x_parent == p_index);

    // U asserts
    let u_parent = tree.get_parent_index::<TestOrder>(u_index);
    cvt_assert!(u_parent == g_index);

    // Y asserts
    let y_parent = tree.get_parent_index::<TestOrder>(y_index);
    cvt_assert!(y_parent == g_index);

    cvt_vacuity_check!();
}

// The rules for checking that the parent of the left and right child of a node
// is the node itself hardcode the following facts:
// 1. position of nodes
// 2. color of the nodes
// 3. 0 is the root
// 4. 1 is the left child of 0, its value is lesser or equal than the value of 0
// 5. 2 is the right child of 0, its value is greater or equal than the value of 0
// 6. 3 is the left child of 1, its value is lesser or equal than the value of 1
// 7. 4 is the right child of 1, its value is greater or equal than the value of 1
// 8. 2, 3, and 4 have no children

/// For each node, check that the parent of the left child is the node itself.
/// Check this before and after inserting a new node in the following tree.
/// Run with `-bmc 2`.
///
///            0
///          /  \
///        1     2
///      /   \
///    3      4
///
#[rule]
pub fn rule_insert_preserves_parent_of_left_child() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let zero_index = 0 * TEST_BLOCK_WIDTH;
    let one_index = 1 * TEST_BLOCK_WIDTH;
    let two_index = 2 * TEST_BLOCK_WIDTH;
    let three_index = 3 * TEST_BLOCK_WIDTH;
    let four_index = 4 * TEST_BLOCK_WIDTH;
    let five_index = 5 * TEST_BLOCK_WIDTH;

    let zero_val: u64 = nondet();
    let one_val: u64 = nondet_with(|x| *x <= zero_val);
    let two_val: u64 = nondet_with(|x| *x >= zero_val);
    let three_val: u64 = nondet_with(|x| *x <= one_val);
    let four_val: u64 = nondet_with(|x| *x >= one_val);
    let five_val: u64 = nondet();

    // 0
    *get_mut_helper(&mut data, zero_index) = mk_rb_node!(
        one_index,
        two_index,
        NIL,
        Color::Black,
        TestOrder::new(zero_val)
    );

    // 1
    *get_mut_helper(&mut data, one_index) = mk_rb_node!(
        three_index,
        four_index,
        zero_index,
        Color::Red,
        TestOrder::new(one_val)
    );

    // 2
    *get_mut_helper(&mut data, two_index) =
        mk_rb_node!(NIL, NIL, zero_index, Color::Black, TestOrder::new(two_val));

    // 3
    *get_mut_helper(&mut data, three_index) =
        mk_rb_node!(NIL, NIL, one_index, Color::Black, TestOrder::new(three_val));

    // 4
    *get_mut_helper(&mut data, four_index) =
        mk_rb_node!(NIL, NIL, one_index, Color::Black, TestOrder::new(four_val));

    let mut tree: RedBlackTree<TestOrder> = RedBlackTree::new(&mut data, zero_index, two_index);

    // Check that the parent of the left child of any node is the node itself
    // This loop is unrolled enough times with -bmc 2 because the Rust compiler
    // partially unrolls it at compile time.
    let size_before_insert = 5;
    for node_number in 0..size_before_insert {
        let node_index = node_number * TEST_BLOCK_WIDTH;
        let left_child_index = tree.get_left_index::<DataIndex>(node_index);
        if left_child_index != NIL {
            let left_child_parent_index = tree.get_parent_index::<DataIndex>(left_child_index);
            cvt_assert!(left_child_parent_index == node_index);
        }
    }

    // Insert node 5
    let five_order = TestOrder::new(five_val.into());
    tree.insert(five_index, five_order);

    // Check that the parent of the left child of any node is the node itself
    // This loop is unrolled enough times with -bmc 2 because the Rust compiler
    // partially unrolls it at compile time.
    let size_after_insert = 6;
    for node_number in 0..size_after_insert {
        let node_index = node_number * TEST_BLOCK_WIDTH;
        let left_child_index = tree.get_left_index::<DataIndex>(node_index);
        if left_child_index != NIL {
            let left_child_parent_index = tree.get_parent_index::<DataIndex>(left_child_index);
            cvt_assert!(left_child_parent_index == node_index);
        }
    }

    cvt_vacuity_check!();
}

/// For each node, check that the parent of the right child is the node itself.
/// Check this before and after inserting a new node in the following tree.
/// Run with `-bmc 2`.
///
///            0
///          /  \
///        1     2
///      /   \
///    3      4
///
#[rule]
pub fn rule_insert_preserves_parent_of_right_child() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let zero_index = 0 * TEST_BLOCK_WIDTH;
    let one_index = 1 * TEST_BLOCK_WIDTH;
    let two_index = 2 * TEST_BLOCK_WIDTH;
    let three_index = 3 * TEST_BLOCK_WIDTH;
    let four_index = 4 * TEST_BLOCK_WIDTH;
    let five_index = 5 * TEST_BLOCK_WIDTH;

    let zero_val: u64 = nondet();
    let one_val: u64 = nondet_with(|x| *x <= zero_val);
    let two_val: u64 = nondet_with(|x| *x >= zero_val);
    let three_val: u64 = nondet_with(|x| *x <= one_val);
    let four_val: u64 = nondet_with(|x| *x >= one_val);
    let five_val: u64 = nondet();

    // 0
    *get_mut_helper(&mut data, zero_index) = mk_rb_node!(
        one_index,
        two_index,
        NIL,
        Color::Black,
        TestOrder::new(zero_val)
    );

    // 1
    *get_mut_helper(&mut data, one_index) = mk_rb_node!(
        three_index,
        four_index,
        zero_index,
        Color::Red,
        TestOrder::new(one_val)
    );

    // 2
    *get_mut_helper(&mut data, two_index) =
        mk_rb_node!(NIL, NIL, zero_index, Color::Black, TestOrder::new(two_val));

    // 3
    *get_mut_helper(&mut data, three_index) =
        mk_rb_node!(NIL, NIL, one_index, Color::Black, TestOrder::new(three_val));

    // 4
    *get_mut_helper(&mut data, four_index) =
        mk_rb_node!(NIL, NIL, one_index, Color::Black, TestOrder::new(four_val));

    let mut tree: RedBlackTree<TestOrder> = RedBlackTree::new(&mut data, zero_index, two_index);

    // Check that the parent of the right child of any node is the node itself
    // This loop is unrolled enough times with -bmc 2 because the Rust compiler
    // partially unrolls it at compile time.
    let size_before_insert = 5;
    for node_number in 0..size_before_insert {
        let node_index = node_number * TEST_BLOCK_WIDTH;
        let right_child_index = tree.get_right_index::<DataIndex>(node_index);
        if right_child_index != NIL {
            let right_child_parent_index = tree.get_parent_index::<DataIndex>(right_child_index);
            cvt_assert!(right_child_parent_index == node_index);
        }
    }

    // Insert node 5
    let five_order = TestOrder::new(five_val.into());
    tree.insert(five_index, five_order);

    // Check that the parent of the right child of any node is the node itself
    // This loop is unrolled enough times with -bmc 2 because the Rust compiler
    // partially unrolls it at compile time.
    let size_after_insert = 6;
    for node_number in 0..size_after_insert {
        let node_index = node_number * TEST_BLOCK_WIDTH;
        let right_child_index = tree.get_right_index::<DataIndex>(node_index);
        if right_child_index != NIL {
            let right_child_parent_index = tree.get_parent_index::<DataIndex>(right_child_index);
            cvt_assert!(right_child_parent_index == node_index);
        }
    }

    cvt_vacuity_check!();
}

// The rules for checking that the parent of the root is NIL hardcode the following facts:
// 1. position of nodes
// 2. color of the nodes
// 3. 0 is the root
// 4. 1 is the left child of 0, its value is lesser or equal than the value of 0
// 5. 2 is the right child of 0, its value is greater or equal than the value of 0
// 6. 3 is the left child of 1, its value is lesser or equal than the value of 1
// 7. 4 is the right child of 1, its value is greater or equal than the value of 1
// 8. 2, 3, and 4 have no children

/// Check that the parent of the root is NIL before and after inserting a node
/// in the following tree. The value of the new node is nondet.
/// Run with `-bmc 2`.
///
///            0
///          /  \
///        1     2
///      /   \
///    3      4
///
#[rule]
pub fn rule_insert_preserves_root_parent_is_nil() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let zero_index = 0 * TEST_BLOCK_WIDTH;
    let one_index = 1 * TEST_BLOCK_WIDTH;
    let two_index = 2 * TEST_BLOCK_WIDTH;
    let three_index = 3 * TEST_BLOCK_WIDTH;
    let four_index = 4 * TEST_BLOCK_WIDTH;
    let five_index = 5 * TEST_BLOCK_WIDTH;

    let zero_val: u64 = nondet();
    let one_val: u64 = nondet_with(|x| *x <= zero_val);
    let two_val: u64 = nondet_with(|x| *x >= zero_val);
    let three_val: u64 = nondet_with(|x| *x <= one_val);
    let four_val: u64 = nondet_with(|x| *x >= one_val);
    let five_val: u64 = nondet();

    // 0
    *get_mut_helper(&mut data, zero_index) = mk_rb_node!(
        one_index,
        two_index,
        NIL,
        Color::Black,
        TestOrder::new(zero_val)
    );

    // 1
    *get_mut_helper(&mut data, one_index) = mk_rb_node!(
        three_index,
        four_index,
        zero_index,
        Color::Red,
        TestOrder::new(one_val)
    );

    // 2
    *get_mut_helper(&mut data, two_index) =
        mk_rb_node!(NIL, NIL, zero_index, Color::Black, TestOrder::new(two_val));

    // 3
    *get_mut_helper(&mut data, three_index) =
        mk_rb_node!(NIL, NIL, one_index, Color::Black, TestOrder::new(three_val));

    // 4
    *get_mut_helper(&mut data, four_index) =
        mk_rb_node!(NIL, NIL, one_index, Color::Black, TestOrder::new(four_val));

    let mut tree: RedBlackTree<TestOrder> = RedBlackTree::new(&mut data, zero_index, two_index);

    // Assert that the root parent is NIL before inserting node 5
    let root_parent = tree.get_parent_index::<TestOrder>(tree.root_index());
    cvt_assert!(root_parent == NIL);

    // Insert node 5
    let five_order = TestOrder::new(five_val.into());
    tree.insert(five_index, five_order);

    // Assert that the root parent is NIL after inserting node 5
    let root_parent = tree.get_parent_index::<TestOrder>(tree.root_index());
    cvt_assert!(root_parent == NIL);

    cvt_vacuity_check!();
}

/// Check that the root of a rb tree is black after inserting two nodes in an
/// empty tree.
#[rule]
pub fn rule_root_is_black_after_insert_empty_tree() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let mut tree: RedBlackTree<TestOrder> = RedBlackTree::new(&mut data, NIL, NIL);

    // Insert the first node
    tree.insert(0, TestOrder::new(nondet::<u64>()));
    let root_index = tree.root_index();
    let root = get_helper::<RBNode<TestOrder>>(tree.data(), root_index);
    // Check that the root is black
    cvt_assert!(root.color == Color::Black);

    // Insert the second node
    tree.insert(TEST_BLOCK_WIDTH, TestOrder::new(nondet::<u64>()));
    let root_index = tree.root_index();
    let root = get_helper::<RBNode<TestOrder>>(tree.data(), root_index);
    // Check that the root is black
    cvt_assert!(root.color == Color::Black);

    cvt_vacuity_check!();
}

/// Check that the root of the following rb tree is black before and after a
/// node with nondet value. Run with `-bmc 2`.
///
///            0
///          /  \
///        1     2
///      /   \
///    3      4
///
#[rule]
pub fn rule_root_is_black_after_insert_non_empty_tree() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let zero_index = 0 * TEST_BLOCK_WIDTH;
    let one_index = 1 * TEST_BLOCK_WIDTH;
    let two_index = 2 * TEST_BLOCK_WIDTH;
    let three_index = 3 * TEST_BLOCK_WIDTH;
    let four_index = 4 * TEST_BLOCK_WIDTH;
    let five_index = 5 * TEST_BLOCK_WIDTH;

    let zero_val: u64 = nondet();
    let one_val: u64 = nondet_with(|x| *x <= zero_val);
    let two_val: u64 = nondet_with(|x| *x >= zero_val);
    let three_val: u64 = nondet_with(|x| *x <= one_val);
    let four_val: u64 = nondet_with(|x| *x >= one_val);
    let five_val: u64 = nondet();

    // 0
    *get_mut_helper(&mut data, zero_index) = mk_rb_node!(
        one_index,
        two_index,
        NIL,
        Color::Black,
        TestOrder::new(zero_val)
    );

    // 1
    *get_mut_helper(&mut data, one_index) = mk_rb_node!(
        three_index,
        four_index,
        zero_index,
        Color::Red,
        TestOrder::new(one_val)
    );

    // 2
    *get_mut_helper(&mut data, two_index) =
        mk_rb_node!(NIL, NIL, zero_index, Color::Black, TestOrder::new(two_val));

    // 3
    *get_mut_helper(&mut data, three_index) =
        mk_rb_node!(NIL, NIL, one_index, Color::Black, TestOrder::new(three_val));

    // 4
    *get_mut_helper(&mut data, four_index) =
        mk_rb_node!(NIL, NIL, one_index, Color::Black, TestOrder::new(four_val));

    let mut tree: RedBlackTree<TestOrder> = RedBlackTree::new(&mut data, zero_index, two_index);

    let root_index = tree.root_index();
    let root = get_helper::<RBNode<TestOrder>>(tree.data(), root_index);
    cvt_assert!(root.color == Color::Black);

    // Insert node 5
    let five_order = TestOrder::new(five_val.into());
    tree.insert(five_index, five_order);

    let root_index = tree.root_index();
    let root = get_helper::<RBNode<TestOrder>>(tree.data(), root_index);
    cvt_assert!(root.color == Color::Black);

    cvt_vacuity_check!();
}

/// Check that the tree is still ordered after inserting a value smaller than
/// the smallest in the following tree. Run with `-bmc 2`. This rule is very
/// expensive, and one might have to set a higer timeout threshold.
/// The rule runs on the following tree:
///
///           B:0               B:0
///          /  \              /  \
///       B:1   B:2  --->   B:3   B:2
///      /                 /  \
///    R:3               R:4  R:1
///
#[rule]
pub fn rule_tree_is_ordered_after_insert_smallest_element() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let zero_index = 0 * TEST_BLOCK_WIDTH;
    let one_index = 1 * TEST_BLOCK_WIDTH;
    let two_index = 2 * TEST_BLOCK_WIDTH;
    let three_index = 3 * TEST_BLOCK_WIDTH;
    let four_index = 4 * TEST_BLOCK_WIDTH;

    let zero_val: u64 = nondet();
    let one_val: u64 = nondet_with(|x| *x <= zero_val);
    let two_val: u64 = nondet_with(|x| *x >= zero_val);
    let three_val: u64 = nondet_with(|x| *x <= one_val && *x <= zero_val);

    // 0
    *get_mut_helper(&mut data, zero_index) = mk_rb_node!(
        one_index,
        two_index,
        NIL,
        Color::Black,
        TestOrder::new(zero_val)
    );

    // 1
    *get_mut_helper(&mut data, one_index) = mk_rb_node!(
        three_index,
        NIL,
        zero_index,
        Color::Black,
        TestOrder::new(one_val)
    );

    // 2
    *get_mut_helper(&mut data, two_index) =
        mk_rb_node!(NIL, NIL, zero_index, Color::Black, TestOrder::new(two_val));

    // 3
    *get_mut_helper(&mut data, three_index) =
        mk_rb_node!(NIL, NIL, one_index, Color::Red, TestOrder::new(three_val));

    let mut tree: RedBlackTree<TestOrder> = RedBlackTree::new(&mut data, zero_index, two_index);

    let four_val = nondet::<u64>();
    cvt_assume!(four_val < three_val);

    tree.insert(four_index, TestOrder::new(four_val));

    let zero_index = tree.get_root_index();
    let zero_node = get_helper::<RBNode<TestOrder>>(&tree.data(), zero_index);

    // First layer of depth in the tree, left child
    let three_index = tree.get_left_index::<TestOrder>(zero_index);
    let three_node = get_helper::<RBNode<TestOrder>>(&tree.data(), three_index);
    cvt_assert!(three_node.value.order_id <= zero_node.value.order_id);

    // First layer of depth in the tree, right child
    let two_index = tree.get_right_index::<TestOrder>(zero_index);
    let two_node = get_helper::<RBNode<TestOrder>>(&tree.data(), two_index);
    cvt_assert!(two_node.value.order_id >= zero_node.value.order_id);

    // Second layer of depth in the tree, left child of the left child of the root
    let four_index = tree.get_left_index::<TestOrder>(three_index);
    let four_node = get_helper::<RBNode<TestOrder>>(&tree.data(), four_index);
    cvt_assert!(four_node.value.order_id <= three_node.value.order_id);

    // Second layer of depth in the tree, right child of the left child of the root
    let one_index = tree.get_right_index::<TestOrder>(three_index);
    let one_node = get_helper::<RBNode<TestOrder>>(&tree.data(), one_index);
    cvt_assert!(one_node.value.order_id >= three_node.value.order_id);
    cvt_assert!(one_node.value.order_id <= zero_node.value.order_id);

    cvt_vacuity_check!();
}

// We verify that the `insert_fix` implementation matches the `RB-Delete-Fixup`
// implementation in the 4th edition of "Introduction to Algorithms", ISBN
// 026204630X.
// The pseudo-code of the function is at page 351.
// A screenshot of the procedure can be found at the following link:
// https://drive.google.com/file/d/19WqPdl9q0cRUJSu_Ifj__dgAYb5l0dDr/view
// While `RB-Insert-Fixup` has a while loop that fixes all the indices from the
// node that has to be fixed up to the root, the function `insert_fix` performs
// one single iteration of the loop, from the node at index `index_to_fix`,
// which corresponds to the node `z` in the pseudo-code.
// In `insert_fix` there is no need to write a while loop, as the funciton is
// called from `insert` inside of a while loop.
// What we prove is that, for each possible case in the `RB-Insert-Fixup`
// function, the implementation matches the expected behaviour.
// There are three possible cases, and each one of them has a specular rule
// depending on whether the parent of the node to fix is the left or the right
// child.

/// Builds the following tree:
///
///               ?:0
///             /     \
///          ?:1      ?:2
///
macro_rules! build_tree_0 {
    ($data: expr, $val_0: expr, $val_1: expr, $val_2: expr) => {{
        let index_0: DataIndex = 0 * TEST_BLOCK_WIDTH;
        let index_1: DataIndex = 1 * TEST_BLOCK_WIDTH;
        let index_2: DataIndex = 2 * TEST_BLOCK_WIDTH;

        // Nodes with defined values.

        // 0
        *get_mut_helper(&mut $data, index_0) =
            mk_rb_node!(index_1, index_2, NIL, nondet(), TestOrder::new($val_0));

        // 1
        *get_mut_helper(&mut $data, index_1) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_0,
            nondet::<Color>(),
            TestOrder::new($val_1)
        );

        // 2
        *get_mut_helper(&mut $data, index_2) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_0,
            nondet::<Color>(),
            TestOrder::new($val_2)
        );

        RedBlackTree::new(&mut $data, index_0, index_2)
    }};
}

/// Check that `insert_fix` behaves as the reference implementation in case that
/// the node to be fixed has no parent, namely it is the root.
///
///               ?:0                B:0
///             /     \    -->     /    \
///          ?:1       ?:2       ?:1    ?:2
///
#[rule]
pub fn rule_insert_fix_matches_reference_no_parent() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;

    let val_0: u64 = nondet();
    let val_1: u64 = nondet_with(|x| *x <= val_0);
    let val_2: u64 = nondet_with(|x| *x >= val_0);

    let mut tree: RedBlackTree<TestOrder> = build_tree_0!(data, val_0, val_1, val_2);

    // Node 3 is the one that has to be fixed, since its parent 1 is red as well
    let next_to_fix_index = tree.certora_insert_fix(index_0);

    cvt_assert!(next_to_fix_index == NIL);

    // Assert that the colors and the indices of the nodes have been properly fixed.

    // The root is always black
    cvt_assert!(tree.get_color::<TestOrder>(index_0) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_0) == NIL);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_0) == index_1);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_0) == index_2);

    cvt_vacuity_check!();
}

/// Builds the following tree:
///
///                 ?:5
///                  |
///                 ?:0
///             /         \
///          R:1          R:2
///        /   \         /  \
///      R:3    B:4    ?:6  ?:7
///    /   \    /   \
///  ?:8   ?:9 ?:10 ?:11
///
macro_rules! build_tree_1 {
    ($data: expr, $val_0: expr, $val_1: expr, $val_2: expr, $val_3: expr, $val_4: expr) => {{
        let index_0: DataIndex = 0 * TEST_BLOCK_WIDTH;
        let index_1: DataIndex = 1 * TEST_BLOCK_WIDTH;
        let index_2: DataIndex = 2 * TEST_BLOCK_WIDTH;
        let index_3: DataIndex = 3 * TEST_BLOCK_WIDTH;
        let index_4: DataIndex = 4 * TEST_BLOCK_WIDTH;
        let index_5: DataIndex = 5 * TEST_BLOCK_WIDTH;
        let index_6: DataIndex = 6 * TEST_BLOCK_WIDTH;
        let index_7: DataIndex = 7 * TEST_BLOCK_WIDTH;
        let index_8: DataIndex = 8 * TEST_BLOCK_WIDTH;
        let index_9: DataIndex = 9 * TEST_BLOCK_WIDTH;
        let index_10: DataIndex = 10 * TEST_BLOCK_WIDTH;
        let index_11: DataIndex = 11 * TEST_BLOCK_WIDTH;

        // Nodes with defined values.

        // 0
        *get_mut_helper(&mut $data, index_0) =
            mk_rb_node!(index_1, index_2, index_5, nondet(), TestOrder::new($val_0));

        // 1
        *get_mut_helper(&mut $data, index_1) = mk_rb_node!(
            index_3,
            index_4,
            index_0,
            Color::Red,
            TestOrder::new($val_1)
        );

        // 2
        *get_mut_helper(&mut $data, index_2) = mk_rb_node!(
            index_6,
            index_7,
            index_0,
            Color::Red,
            TestOrder::new($val_2)
        );

        // 3
        *get_mut_helper(&mut $data, index_3) = mk_rb_node!(
            index_8,
            index_9,
            index_1,
            Color::Red,
            TestOrder::new($val_3)
        );

        // 4
        *get_mut_helper(&mut $data, index_4) = mk_rb_node!(
            index_10,
            index_11,
            index_1,
            Color::Black,
            TestOrder::new($val_4)
        );

        // Padding nodes.

        // 5
        *get_mut_helper(&mut $data, index_5) = mk_rb_node!(
            index_0,
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            nondet(),
            TestOrder::new(nondet())
        );

        // 6
        *get_mut_helper(&mut $data, index_6) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_2,
            nondet(),
            TestOrder::new(nondet())
        );

        // 7
        *get_mut_helper(&mut $data, index_7) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_2,
            nondet(),
            TestOrder::new(nondet())
        );

        // 8
        *get_mut_helper(&mut $data, index_8) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_3,
            nondet(),
            TestOrder::new(nondet())
        );

        // 9
        *get_mut_helper(&mut $data, index_9) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_3,
            nondet(),
            TestOrder::new(nondet())
        );

        // 10
        *get_mut_helper(&mut $data, index_10) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_4,
            nondet(),
            TestOrder::new(nondet())
        );

        // 11
        *get_mut_helper(&mut $data, index_11) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_4,
            nondet(),
            TestOrder::new(nondet())
        );

        RedBlackTree::new(&mut $data, index_5, index_7)
    }};
}

/// Check that `insert_fix` behaves as the reference implementation in case 1,
/// when the parent of the node that has to be fixed (3) is a left child.
///
///                 ?:5                               ?:5
///                  |                                 |
///                 ?:0                               R:0
///             /         \                       /         \
///          R:1          R:2     -->          B:1          B:2
///        /    \         /  \               /    \         /  \
///      R:3     B:4    ?:6  ?:7           R:3     B:4    ?:6  ?:7
///    /   \    /   \                    /   \    /   \
///  ?:8   ?:9 ?:10 ?:11               ?:8   ?:9 ?:10 ?:11
///
#[rule]
pub fn rule_insert_fix_matches_reference_case1_left_child() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let index_3 = 3 * TEST_BLOCK_WIDTH;
    let index_4 = 4 * TEST_BLOCK_WIDTH;
    let index_5 = 5 * TEST_BLOCK_WIDTH;
    let index_6 = 6 * TEST_BLOCK_WIDTH;
    let index_7 = 7 * TEST_BLOCK_WIDTH;
    let index_8 = 8 * TEST_BLOCK_WIDTH;
    let index_9 = 9 * TEST_BLOCK_WIDTH;
    let index_10 = 10 * TEST_BLOCK_WIDTH;
    let index_11 = 11 * TEST_BLOCK_WIDTH;

    let val_0: u64 = nondet();
    let val_1: u64 = nondet_with(|x| *x <= val_0);
    let val_2: u64 = nondet_with(|x| *x >= val_0);
    let val_3: u64 = nondet_with(|x| *x <= val_1);
    let val_4: u64 = nondet_with(|x| *x >= val_1 && *x <= val_0);

    let mut tree: RedBlackTree<TestOrder> = build_tree_1!(data, val_0, val_1, val_2, val_3, val_4);

    // Node 3 is the one that has to be fixed, since its parent 1 is red as well
    let next_to_fix_index = tree.certora_insert_fix(index_3);

    cvt_assert!(next_to_fix_index == index_0);

    // Assert that the colors and the indices of the nodes have been properly fixed.

    cvt_assert!(tree.get_color::<TestOrder>(index_0) == Color::Red);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_0) == index_5);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_0) == index_1);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_0) == index_2);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == index_3);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == index_4);

    cvt_assert!(tree.get_color::<TestOrder>(index_2) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_2) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_2) == index_6);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_2) == index_7);

    cvt_assert!(tree.get_color::<TestOrder>(index_3) == Color::Red);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_3) == index_1);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_3) == index_8);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_3) == index_9);

    cvt_assert!(tree.get_color::<TestOrder>(index_4) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_4) == index_1);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_4) == index_10);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_4) == index_11);

    cvt_vacuity_check!();
}

/// Builds the following tree:
///
///                 ?:5
///                  |
///                 B:0
///             /         \
///          R:1          B:2
///        /   \         /  \
///      B:3    R:4    ?:6  ?:7
///    /   \    /   \
///  ?:8   ?:9 ?:10 ?:11
///
macro_rules! build_tree_2 {
    ($data: expr, $val_0: expr, $val_1: expr, $val_2: expr, $val_3: expr, $val_4: expr) => {{
        let index_0 = 0 * TEST_BLOCK_WIDTH;
        let index_1 = 1 * TEST_BLOCK_WIDTH;
        let index_2 = 2 * TEST_BLOCK_WIDTH;
        let index_3 = 3 * TEST_BLOCK_WIDTH;
        let index_4 = 4 * TEST_BLOCK_WIDTH;
        let index_5 = 5 * TEST_BLOCK_WIDTH;
        let index_6 = 6 * TEST_BLOCK_WIDTH;
        let index_7 = 7 * TEST_BLOCK_WIDTH;
        let index_8 = 8 * TEST_BLOCK_WIDTH;
        let index_9 = 9 * TEST_BLOCK_WIDTH;
        let index_10 = 10 * TEST_BLOCK_WIDTH;
        let index_11 = 11 * TEST_BLOCK_WIDTH;

        // Nodes with defined values.

        // 0
        *get_mut_helper(&mut $data, index_0) = mk_rb_node!(
            index_1,
            index_2,
            index_5,
            Color::Black,
            TestOrder::new($val_0)
        );

        // 1
        *get_mut_helper(&mut $data, index_1) = mk_rb_node!(
            index_3,
            index_4,
            index_0,
            Color::Red,
            TestOrder::new($val_1)
        );

        // 2
        *get_mut_helper(&mut $data, index_2) = mk_rb_node!(
            index_6,
            index_7,
            index_0,
            Color::Black,
            TestOrder::new($val_2)
        );

        // 3
        *get_mut_helper(&mut $data, index_3) = mk_rb_node!(
            index_8,
            index_9,
            index_1,
            Color::Black,
            TestOrder::new($val_3)
        );

        // 4
        *get_mut_helper(&mut $data, index_4) = mk_rb_node!(
            index_10,
            index_11,
            index_1,
            Color::Red,
            TestOrder::new($val_4)
        );

        // Padding nodes.

        // 5
        *get_mut_helper(&mut $data, index_5) = mk_rb_node!(
            index_0,
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            nondet(),
            TestOrder::new(nondet())
        );

        // 6
        *get_mut_helper(&mut $data, index_6) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_2,
            nondet(),
            TestOrder::new(nondet())
        );

        // 7
        *get_mut_helper(&mut $data, index_7) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_2,
            nondet(),
            TestOrder::new(nondet())
        );

        // 8
        *get_mut_helper(&mut $data, index_8) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_3,
            nondet(),
            TestOrder::new(nondet())
        );

        // 9
        *get_mut_helper(&mut $data, index_9) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_3,
            nondet(),
            TestOrder::new(nondet())
        );

        // 10
        *get_mut_helper(&mut $data, index_10) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_4,
            nondet(),
            TestOrder::new(nondet())
        );

        // 11
        *get_mut_helper(&mut $data, index_11) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_4,
            nondet(),
            TestOrder::new(nondet())
        );

        RedBlackTree::new(&mut $data, index_5, index_7)
    }};
}

/// Check that `insert_fix` behaves as the reference implementation in case 2,
/// when the parent of the node that has to be fixed (4) is a left child.
///
///                 ?:5                               ?:5
///                  |                                 |
///                 B:0                               B:4
///             /         \       -->           /           \
///          R:1          B:2                R:1             R:0
///        /    \         /  \             /    \          /    \
///      B:3     R:4    ?:6  ?:7         B:3    ?:10    ?:11    B:2
///    /   \    /   \                  /   \                   /   \
///  ?:8   ?:9 ?:10 ?:11             ?:8   ?:9               ?:6   ?:7
///
#[rule]
pub fn rule_insert_fix_matches_reference_case2_left_child() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let index_3 = 3 * TEST_BLOCK_WIDTH;
    let index_4 = 4 * TEST_BLOCK_WIDTH;
    let index_5 = 5 * TEST_BLOCK_WIDTH;
    let index_6 = 6 * TEST_BLOCK_WIDTH;
    let index_7 = 7 * TEST_BLOCK_WIDTH;
    let index_8 = 8 * TEST_BLOCK_WIDTH;
    let index_9 = 9 * TEST_BLOCK_WIDTH;
    let index_10 = 10 * TEST_BLOCK_WIDTH;
    let index_11 = 11 * TEST_BLOCK_WIDTH;

    let val_0: u64 = nondet();
    let val_1: u64 = nondet_with(|x| *x <= val_0);
    let val_2: u64 = nondet_with(|x| *x >= val_0);
    let val_3: u64 = nondet_with(|x| *x <= val_1);
    let val_4: u64 = nondet_with(|x| *x >= val_1 && *x <= val_0);

    let mut tree: RedBlackTree<TestOrder> = build_tree_2!(data, val_0, val_1, val_2, val_3, val_4);

    // Node 4 is the one that has to be fixed, since its parent 1 is red as well
    let next_to_fix_index = tree.certora_insert_fix(index_4);

    cvt_assert!(next_to_fix_index == NIL);

    // Assert that the colors and the indices of the nodes have been properly fixed.

    cvt_assert!(tree.get_color::<TestOrder>(index_0) == Color::Red);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_0) == index_4);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_0) == index_11);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_0) == index_2);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == Color::Red);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_4);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == index_3);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == index_10);

    cvt_assert!(tree.get_color::<TestOrder>(index_2) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_2) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_2) == index_6);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_2) == index_7);

    cvt_assert!(tree.get_color::<TestOrder>(index_3) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_3) == index_1);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_3) == index_8);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_3) == index_9);

    cvt_assert!(tree.get_color::<TestOrder>(index_4) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_4) == index_5);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_4) == index_1);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_4) == index_0);

    cvt_vacuity_check!();
}

/// Builds the following tree:
///
///                 ?:5
///                  |
///                 B:0
///             /         \
///          R:1          B:2
///        /   \         /  \
///      R:3    B:4    ?:6  ?:7
///    /   \    /   \
///  ?:8   ?:9 ?:10 ?:11
///
macro_rules! build_tree_3 {
    ($data: expr, $val_0: expr, $val_1: expr, $val_2: expr, $val_3: expr, $val_4: expr) => {{
        let index_0 = 0 * TEST_BLOCK_WIDTH;
        let index_1 = 1 * TEST_BLOCK_WIDTH;
        let index_2 = 2 * TEST_BLOCK_WIDTH;
        let index_3 = 3 * TEST_BLOCK_WIDTH;
        let index_4 = 4 * TEST_BLOCK_WIDTH;
        let index_5 = 5 * TEST_BLOCK_WIDTH;
        let index_6 = 6 * TEST_BLOCK_WIDTH;
        let index_7 = 7 * TEST_BLOCK_WIDTH;
        let index_8 = 8 * TEST_BLOCK_WIDTH;
        let index_9 = 9 * TEST_BLOCK_WIDTH;
        let index_10 = 10 * TEST_BLOCK_WIDTH;
        let index_11 = 11 * TEST_BLOCK_WIDTH;

        // Nodes with defined values.

        // 0
        *get_mut_helper(&mut $data, index_0) = mk_rb_node!(
            index_1,
            index_2,
            index_5,
            Color::Black,
            TestOrder::new($val_0)
        );

        // 1
        *get_mut_helper(&mut $data, index_1) = mk_rb_node!(
            index_3,
            index_4,
            index_0,
            Color::Red,
            TestOrder::new($val_1)
        );

        // 2
        *get_mut_helper(&mut $data, index_2) = mk_rb_node!(
            index_6,
            index_7,
            index_0,
            Color::Black,
            TestOrder::new($val_2)
        );

        // 3
        *get_mut_helper(&mut $data, index_3) = mk_rb_node!(
            index_8,
            index_9,
            index_1,
            Color::Red,
            TestOrder::new($val_3)
        );

        // 4
        *get_mut_helper(&mut $data, index_4) = mk_rb_node!(
            index_10,
            index_11,
            index_1,
            Color::Black,
            TestOrder::new($val_4)
        );

        // Padding nodes.

        // 5
        *get_mut_helper(&mut $data, index_5) = mk_rb_node!(
            index_0,
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            nondet(),
            TestOrder::new(nondet())
        );

        // 6
        *get_mut_helper(&mut $data, index_6) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_2,
            nondet(),
            TestOrder::new(nondet())
        );

        // 7
        *get_mut_helper(&mut $data, index_7) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_2,
            nondet(),
            TestOrder::new(nondet())
        );

        // 8
        *get_mut_helper(&mut $data, index_8) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_3,
            nondet(),
            TestOrder::new(nondet())
        );

        // 9
        *get_mut_helper(&mut $data, index_9) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_3,
            nondet(),
            TestOrder::new(nondet())
        );

        // 10
        *get_mut_helper(&mut $data, index_10) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_4,
            nondet(),
            TestOrder::new(nondet())
        );

        // 11
        *get_mut_helper(&mut $data, index_11) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_4,
            nondet(),
            TestOrder::new(nondet())
        );

        RedBlackTree::new(&mut $data, index_5, index_7)
    }};
}

/// Check that `insert_fix` behaves as the reference implementation in case 3,
/// when the parent of the node that has to be fixed (3) is a left child.
///
///                 ?:5                             ?:5
///                  |                               |
///                 B:0                             B:1
///             /         \       -->           /         \
///          R:1          B:2                R:3            R:0
///        /    \         /  \             /    \         /    \
///      R:3     B:4    ?:6  ?:7         ?:8    ?:9    B:4     B:2
///    /   \    /   \                                 /  \    /   \
///  ?:9   ?:9 ?:10 ?:11                           ?:10 ?:11 ?:6 ?:7
///
#[rule]
pub fn rule_insert_fix_matches_reference_case3_left_child() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let index_3 = 3 * TEST_BLOCK_WIDTH;
    let index_4 = 4 * TEST_BLOCK_WIDTH;
    let index_5 = 5 * TEST_BLOCK_WIDTH;
    let index_6 = 6 * TEST_BLOCK_WIDTH;
    let index_7 = 7 * TEST_BLOCK_WIDTH;
    let index_8 = 8 * TEST_BLOCK_WIDTH;
    let index_9 = 9 * TEST_BLOCK_WIDTH;
    let index_10 = 10 * TEST_BLOCK_WIDTH;
    let index_11 = 11 * TEST_BLOCK_WIDTH;

    let val_0: u64 = nondet();
    let val_1: u64 = nondet_with(|x| *x <= val_0);
    let val_2: u64 = nondet_with(|x| *x >= val_0);
    let val_3: u64 = nondet_with(|x| *x <= val_1);
    let val_4: u64 = nondet_with(|x| *x >= val_1 && *x <= val_0);

    let mut tree: RedBlackTree<TestOrder> = build_tree_3!(data, val_0, val_1, val_2, val_3, val_4);

    // Node 3 is the one that has to be fixed, since its parent 1 is red as well
    let next_to_fix_index = tree.certora_insert_fix(index_3);

    cvt_assert!(next_to_fix_index == NIL);

    // Assert that the colors and the indices of the nodes have been properly fixed.

    cvt_assert!(tree.get_color::<TestOrder>(index_0) == Color::Red);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_0) == index_1);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_0) == index_4);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_0) == index_2);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_5);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == index_3);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == index_0);

    cvt_assert!(tree.get_color::<TestOrder>(index_2) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_2) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_2) == index_6);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_2) == index_7);

    cvt_assert!(tree.get_color::<TestOrder>(index_3) == Color::Red);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_3) == index_1);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_3) == index_8);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_3) == index_9);

    cvt_assert!(tree.get_color::<TestOrder>(index_4) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_4) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_4) == index_10);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_4) == index_11);

    cvt_vacuity_check!();
}

/// Builds the following tree:
///
///                 ?:5
///                  |
///                 B:0
///             /         \
///          R:1           R:2
///        /   \         /    \
///      ?:6    ?:7    B:3    R:4
///                  /  \    /   \
///                ?:8  ?:9 ?:10 ?:11
///
macro_rules! build_tree_4 {
    ($data: expr, $val_0: expr, $val_1: expr, $val_2: expr, $val_3: expr, $val_4: expr) => {{
        let index_0 = 0 * TEST_BLOCK_WIDTH;
        let index_1 = 1 * TEST_BLOCK_WIDTH;
        let index_2 = 2 * TEST_BLOCK_WIDTH;
        let index_3 = 3 * TEST_BLOCK_WIDTH;
        let index_4 = 4 * TEST_BLOCK_WIDTH;
        let index_5 = 5 * TEST_BLOCK_WIDTH;
        let index_6 = 6 * TEST_BLOCK_WIDTH;
        let index_7 = 7 * TEST_BLOCK_WIDTH;
        let index_8 = 8 * TEST_BLOCK_WIDTH;
        let index_9 = 9 * TEST_BLOCK_WIDTH;
        let index_10 = 10 * TEST_BLOCK_WIDTH;
        let index_11 = 11 * TEST_BLOCK_WIDTH;

        // Nodes with defined values.

        // 0
        *get_mut_helper(&mut $data, index_0) = mk_rb_node!(
            index_1,
            index_2,
            index_5,
            Color::Black,
            TestOrder::new($val_0)
        );

        // 1
        *get_mut_helper(&mut $data, index_1) = mk_rb_node!(
            index_6,
            index_7,
            index_0,
            Color::Red,
            TestOrder::new($val_1)
        );

        // 2
        *get_mut_helper(&mut $data, index_2) = mk_rb_node!(
            index_3,
            index_4,
            index_0,
            Color::Red,
            TestOrder::new($val_2)
        );

        // 3
        *get_mut_helper(&mut $data, index_3) = mk_rb_node!(
            index_8,
            index_9,
            index_2,
            Color::Black,
            TestOrder::new($val_3)
        );

        // 4
        *get_mut_helper(&mut $data, index_4) = mk_rb_node!(
            index_10,
            index_11,
            index_2,
            Color::Red,
            TestOrder::new($val_4)
        );

        // Padding nodes.

        // 5
        *get_mut_helper(&mut $data, index_5) = mk_rb_node!(
            index_0,
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            nondet(),
            TestOrder::new(nondet())
        );

        // 6
        *get_mut_helper(&mut $data, index_6) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_1,
            nondet(),
            TestOrder::new(nondet())
        );

        // 7
        *get_mut_helper(&mut $data, index_7) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_1,
            nondet(),
            TestOrder::new(nondet())
        );

        // 8
        *get_mut_helper(&mut $data, index_8) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_3,
            nondet(),
            TestOrder::new(nondet())
        );

        // 9
        *get_mut_helper(&mut $data, index_9) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_3,
            nondet(),
            TestOrder::new(nondet())
        );

        // 10
        *get_mut_helper(&mut $data, index_10) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_4,
            nondet(),
            TestOrder::new(nondet())
        );

        // 11
        *get_mut_helper(&mut $data, index_11) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_4,
            nondet(),
            TestOrder::new(nondet())
        );

        RedBlackTree::new(&mut $data, index_5, index_11)
    }};
}

/// Check that `insert_fix` behaves as the reference implementation in case 1,
/// when the parent of the node that has to be fixed (4) is a right child.
///
///                 ?:5                               ?:5
///                  |                                 |
///                 B:0                               R:0
///             /         \                       /         \
///          R:1           R:2        -->      B:1           B:2
///        /   \         /    \              /   \         /    \
///      ?:6    ?:7    B:3    R:4          ?:6    ?:7    B:3    R:4
///                  /  \    /   \                     /  \    /   \
///                ?:8  ?:9 ?:10 ?:11                ?:8  ?:9 ?:10 ?:11
///
#[rule]
pub fn rule_insert_fix_matches_reference_case1_right_child() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let index_3 = 3 * TEST_BLOCK_WIDTH;
    let index_4 = 4 * TEST_BLOCK_WIDTH;
    let index_5 = 5 * TEST_BLOCK_WIDTH;
    let index_6 = 6 * TEST_BLOCK_WIDTH;
    let index_7 = 7 * TEST_BLOCK_WIDTH;
    let index_8 = 8 * TEST_BLOCK_WIDTH;
    let index_9 = 9 * TEST_BLOCK_WIDTH;
    let index_10 = 10 * TEST_BLOCK_WIDTH;
    let index_11 = 11 * TEST_BLOCK_WIDTH;

    let val_0: u64 = nondet();
    let val_1: u64 = nondet_with(|x| *x <= val_0);
    let val_2: u64 = nondet_with(|x| *x >= val_0);
    let val_3: u64 = nondet_with(|x| *x <= val_1 && *x <= val_0);
    let val_4: u64 = nondet_with(|x| *x >= val_1);

    let mut tree: RedBlackTree<TestOrder> = build_tree_4!(data, val_0, val_1, val_2, val_3, val_4);

    // Node 4 is the one that has to be fixed, since its parent 2 is red as well
    let next_to_fix_index = tree.certora_insert_fix(index_4);

    cvt_assert!(next_to_fix_index == index_0);

    // Assert that the colors and the indices of the nodes have been properly fixed.

    cvt_assert!(tree.get_color::<TestOrder>(index_0) == Color::Red);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_0) == index_5);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_0) == index_1);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_0) == index_2);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == index_6);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == index_7);

    cvt_assert!(tree.get_color::<TestOrder>(index_2) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_2) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_2) == index_3);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_2) == index_4);

    cvt_assert!(tree.get_color::<TestOrder>(index_3) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_3) == index_2);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_3) == index_8);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_3) == index_9);

    cvt_assert!(tree.get_color::<TestOrder>(index_4) == Color::Red);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_4) == index_2);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_4) == index_10);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_4) == index_11);

    cvt_vacuity_check!();
}

/// Builds the following tree:
///
///                 ?:5
///                  |
///                 B:0
///             /         \
///          B:1           R:2
///        /   \         /    \
///      ?:6    ?:7    R:3    B:4
///                  /  \    /   \
///                ?:8  ?:9 ?:10 ?:11
///
macro_rules! build_tree_5 {
    ($data: expr, $val_0: expr, $val_1: expr, $val_2: expr, $val_3: expr, $val_4: expr) => {{
        let index_0 = 0 * TEST_BLOCK_WIDTH;
        let index_1 = 1 * TEST_BLOCK_WIDTH;
        let index_2 = 2 * TEST_BLOCK_WIDTH;
        let index_3 = 3 * TEST_BLOCK_WIDTH;
        let index_4 = 4 * TEST_BLOCK_WIDTH;
        let index_5 = 5 * TEST_BLOCK_WIDTH;
        let index_6 = 6 * TEST_BLOCK_WIDTH;
        let index_7 = 7 * TEST_BLOCK_WIDTH;
        let index_8 = 8 * TEST_BLOCK_WIDTH;
        let index_9 = 9 * TEST_BLOCK_WIDTH;
        let index_10 = 10 * TEST_BLOCK_WIDTH;
        let index_11 = 11 * TEST_BLOCK_WIDTH;

        // Nodes with defined values.

        // 0
        *get_mut_helper(&mut $data, index_0) = mk_rb_node!(
            index_1,
            index_2,
            index_5,
            Color::Black,
            TestOrder::new($val_0)
        );

        // 1
        *get_mut_helper(&mut $data, index_1) = mk_rb_node!(
            index_6,
            index_7,
            index_0,
            Color::Black,
            TestOrder::new($val_1)
        );

        // 2
        *get_mut_helper(&mut $data, index_2) = mk_rb_node!(
            index_3,
            index_4,
            index_0,
            Color::Red,
            TestOrder::new($val_2)
        );

        // 3
        *get_mut_helper(&mut $data, index_3) = mk_rb_node!(
            index_8,
            index_9,
            index_2,
            Color::Red,
            TestOrder::new($val_3)
        );

        // 4
        *get_mut_helper(&mut $data, index_4) = mk_rb_node!(
            index_10,
            index_11,
            index_2,
            Color::Black,
            TestOrder::new($val_4)
        );

        // Padding nodes.

        // 5
        *get_mut_helper(&mut $data, index_5) = mk_rb_node!(
            index_0,
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            nondet(),
            TestOrder::new(nondet())
        );

        // 6
        *get_mut_helper(&mut $data, index_6) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_1,
            nondet(),
            TestOrder::new(nondet())
        );

        // 7
        *get_mut_helper(&mut $data, index_7) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_1,
            nondet(),
            TestOrder::new(nondet())
        );

        // 8
        *get_mut_helper(&mut $data, index_8) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_3,
            nondet(),
            TestOrder::new(nondet())
        );

        // 9
        *get_mut_helper(&mut $data, index_9) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_3,
            nondet(),
            TestOrder::new(nondet())
        );

        // 10
        *get_mut_helper(&mut $data, index_10) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_4,
            nondet(),
            TestOrder::new(nondet())
        );

        // 11
        *get_mut_helper(&mut $data, index_11) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_4,
            nondet(),
            TestOrder::new(nondet())
        );

        RedBlackTree::new(&mut $data, index_5, index_7)
    }};
}

/// Check that `insert_fix` behaves as the reference implementation in case 2,
/// when the parent of the node that has to be fixed (3) is a right child.
///
///                 ?:5                                  ?:5
///                  |                                    |
///                 B:0                                  B:3
///             /         \                          /         \
///          B:1           R:2        -->         R:0           R:2
///        /   \         /    \                 /   \         /    \
///      ?:6    ?:7    R:3    B:4             B:1    ?:8    ?:9   B:4
///                  /  \    /   \          /  \                 /    \
///                ?:8  ?:9 ?:10 ?:11    ?:6  ?:7              ?:10  ?:11
///
#[rule]
pub fn rule_insert_fix_matches_reference_case2_right_child() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let index_3 = 3 * TEST_BLOCK_WIDTH;
    let index_4 = 4 * TEST_BLOCK_WIDTH;
    let index_5 = 5 * TEST_BLOCK_WIDTH;
    let index_6 = 6 * TEST_BLOCK_WIDTH;
    let index_7 = 7 * TEST_BLOCK_WIDTH;
    let index_8 = 8 * TEST_BLOCK_WIDTH;
    let index_9 = 9 * TEST_BLOCK_WIDTH;
    let index_10 = 10 * TEST_BLOCK_WIDTH;
    let index_11 = 11 * TEST_BLOCK_WIDTH;

    let val_0: u64 = nondet();
    let val_1: u64 = nondet_with(|x| *x <= val_0);
    let val_2: u64 = nondet_with(|x| *x >= val_0);
    let val_3: u64 = nondet_with(|x| *x <= val_1 && *x >= val_0);
    let val_4: u64 = nondet_with(|x| *x >= val_1);

    let mut tree: RedBlackTree<TestOrder> = build_tree_5!(data, val_0, val_1, val_2, val_3, val_4);

    // Node 3 is the one that has to be fixed, since its parent 2 is red as well
    let next_to_fix_index = tree.certora_insert_fix(index_3);

    cvt_assert!(next_to_fix_index == NIL);

    // Assert that the colors and the indices of the nodes have been properly fixed.

    cvt_assert!(tree.get_color::<TestOrder>(index_0) == Color::Red);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_0) == index_3);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_0) == index_1);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_0) == index_8);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == index_6);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == index_7);

    cvt_assert!(tree.get_color::<TestOrder>(index_2) == Color::Red);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_2) == index_3);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_2) == index_9);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_2) == index_4);

    cvt_assert!(tree.get_color::<TestOrder>(index_3) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_3) == index_5);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_3) == index_0);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_3) == index_2);

    cvt_assert!(tree.get_color::<TestOrder>(index_4) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_4) == index_2);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_4) == index_10);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_4) == index_11);

    cvt_vacuity_check!();
}

/// Builds the following tree:
///
///                 ?:5
///                  |
///                 B:0
///             /         \
///          B:1           R:2
///        /   \         /    \
///      ?:6    ?:7    B:3    R:4
///                  /  \    /   \
///                ?:8  ?:9 ?:10 ?:11
///
macro_rules! build_tree_6 {
    ($data: expr, $val_0: expr, $val_1: expr, $val_2: expr, $val_3: expr, $val_4: expr) => {{
        let index_0 = 0 * TEST_BLOCK_WIDTH;
        let index_1 = 1 * TEST_BLOCK_WIDTH;
        let index_2 = 2 * TEST_BLOCK_WIDTH;
        let index_3 = 3 * TEST_BLOCK_WIDTH;
        let index_4 = 4 * TEST_BLOCK_WIDTH;
        let index_5 = 5 * TEST_BLOCK_WIDTH;
        let index_6 = 6 * TEST_BLOCK_WIDTH;
        let index_7 = 7 * TEST_BLOCK_WIDTH;
        let index_8 = 8 * TEST_BLOCK_WIDTH;
        let index_9 = 9 * TEST_BLOCK_WIDTH;
        let index_10 = 10 * TEST_BLOCK_WIDTH;
        let index_11 = 11 * TEST_BLOCK_WIDTH;

        // Nodes with defined values.

        // 0
        *get_mut_helper(&mut $data, index_0) = mk_rb_node!(
            index_1,
            index_2,
            index_5,
            Color::Black,
            TestOrder::new($val_0)
        );

        // 1
        *get_mut_helper(&mut $data, index_1) = mk_rb_node!(
            index_6,
            index_7,
            index_0,
            Color::Black,
            TestOrder::new($val_1)
        );

        // 2
        *get_mut_helper(&mut $data, index_2) = mk_rb_node!(
            index_3,
            index_4,
            index_0,
            Color::Red,
            TestOrder::new($val_2)
        );

        // 3
        *get_mut_helper(&mut $data, index_3) = mk_rb_node!(
            index_8,
            index_9,
            index_2,
            Color::Black,
            TestOrder::new($val_3)
        );

        // 4
        *get_mut_helper(&mut $data, index_4) = mk_rb_node!(
            index_10,
            index_11,
            index_2,
            Color::Red,
            TestOrder::new($val_4)
        );

        // Padding nodes.

        // 5
        *get_mut_helper(&mut $data, index_5) = mk_rb_node!(
            index_0,
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            nondet(),
            TestOrder::new(nondet())
        );

        // 6
        *get_mut_helper(&mut $data, index_6) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_1,
            nondet(),
            TestOrder::new(nondet())
        );

        // 7
        *get_mut_helper(&mut $data, index_7) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_1,
            nondet(),
            TestOrder::new(nondet())
        );

        // 8
        *get_mut_helper(&mut $data, index_8) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_3,
            nondet(),
            TestOrder::new(nondet())
        );

        // 9
        *get_mut_helper(&mut $data, index_9) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_3,
            nondet(),
            TestOrder::new(nondet())
        );

        // 10
        *get_mut_helper(&mut $data, index_10) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_4,
            nondet(),
            TestOrder::new(nondet())
        );

        // 11
        *get_mut_helper(&mut $data, index_11) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_4,
            nondet(),
            TestOrder::new(nondet())
        );

        RedBlackTree::new(&mut $data, index_5, index_7)
    }};
}

/// Check that `insert_fix` behaves as the reference implementation in case 3,
/// when the parent of the node that has to be fixed (4) is a right child.
///
///                 ?:5                                  ?:5
///                  |                                    |
///                 B:0                                  B:2
///             /         \                          /         \
///          B:1           R:2        -->         R:0           R:4
///        /   \         /    \                 /   \         /    \
///      ?:6    ?:7    B:3    R:4             B:1    B:3    ?:10   ?:11
///                  /  \    /   \          /  \    /   \
///                ?:8  ?:9 ?:10 ?:11    ?:6  ?:7 ?:8  ?:9
///
#[rule]
pub fn rule_insert_fix_matches_reference_case3_right_child() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let index_3 = 3 * TEST_BLOCK_WIDTH;
    let index_4 = 4 * TEST_BLOCK_WIDTH;
    let index_5 = 5 * TEST_BLOCK_WIDTH;
    let index_6 = 6 * TEST_BLOCK_WIDTH;
    let index_7 = 7 * TEST_BLOCK_WIDTH;
    let index_8 = 8 * TEST_BLOCK_WIDTH;
    let index_9 = 9 * TEST_BLOCK_WIDTH;
    let index_10 = 10 * TEST_BLOCK_WIDTH;
    let index_11 = 11 * TEST_BLOCK_WIDTH;

    let val_0: u64 = nondet();
    let val_1: u64 = nondet_with(|x| *x <= val_0);
    let val_2: u64 = nondet_with(|x| *x >= val_0);
    let val_3: u64 = nondet_with(|x| *x <= val_1 && *x <= val_0);
    let val_4: u64 = nondet_with(|x| *x >= val_1);

    let mut tree: RedBlackTree<TestOrder> = build_tree_6!(data, val_0, val_1, val_2, val_3, val_4);

    // Node 4 is the one that has to be fixed, since its parent 2 is red as well
    let next_to_fix_index = tree.certora_insert_fix(index_4);

    cvt_assert!(next_to_fix_index == NIL);

    // Assert that the colors and the indices of the nodes have been properly fixed.

    cvt_assert!(tree.get_color::<TestOrder>(index_0) == Color::Red);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_0) == index_2);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_0) == index_1);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_0) == index_3);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == index_6);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == index_7);

    cvt_assert!(tree.get_color::<TestOrder>(index_2) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_2) == index_5);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_2) == index_0);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_2) == index_4);

    cvt_assert!(tree.get_color::<TestOrder>(index_3) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_3) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_3) == index_8);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_3) == index_9);

    cvt_assert!(tree.get_color::<TestOrder>(index_4) == Color::Red);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_4) == index_2);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_4) == index_10);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_4) == index_11);

    cvt_vacuity_check!();
}

/// Builds the following tree:
///
///                C5:5
///                  |
///                C0:0
///             /         \
///         C1:1          C2:2
///        /   \         /    \
///     C6:6   C7:7   C3:3     C4:4
///                  /  \     /   \
///               C8:8 C9:9 C10:10 C11:11
///
macro_rules! build_tree_shape_1 {
    ($data: expr,
     $val_0: expr,
     $val_1: expr,
     $val_2: expr,
     $val_3: expr,
     $val_4: expr,
     $c0: expr,
     $c1: expr,
     $c2: expr,
     $c3: expr,
     $c4: expr,
     $c5: expr,
     $c6: expr,
     $c7: expr,
     $c8: expr,
     $c9: expr,
     $c10: expr,
     $c11: expr,
        ) => {{
        let index_0 = 0 * TEST_BLOCK_WIDTH;
        let index_1 = 1 * TEST_BLOCK_WIDTH;
        let index_2 = 2 * TEST_BLOCK_WIDTH;
        let index_3 = 3 * TEST_BLOCK_WIDTH;
        let index_4 = 4 * TEST_BLOCK_WIDTH;
        let index_5 = 5 * TEST_BLOCK_WIDTH;
        let index_6 = 6 * TEST_BLOCK_WIDTH;
        let index_7 = 7 * TEST_BLOCK_WIDTH;
        let index_8 = 8 * TEST_BLOCK_WIDTH;
        let index_9 = 9 * TEST_BLOCK_WIDTH;
        let index_10 = 10 * TEST_BLOCK_WIDTH;
        let index_11 = 11 * TEST_BLOCK_WIDTH;

        // Nodes with defined values.

        // 0
        *get_mut_helper(&mut $data, index_0) =
            mk_rb_node!(index_1, index_2, index_5, $c0, TestOrder::new($val_0));

        // 1
        *get_mut_helper(&mut $data, index_1) =
            mk_rb_node!(index_6, index_7, index_0, $c1, TestOrder::new($val_1));

        // 2
        *get_mut_helper(&mut $data, index_2) =
            mk_rb_node!(index_3, index_4, index_0, $c2, TestOrder::new($val_2));

        // 3
        *get_mut_helper(&mut $data, index_3) =
            mk_rb_node!(index_8, index_9, index_2, $c3, TestOrder::new($val_3));

        // 4
        *get_mut_helper(&mut $data, index_4) =
            mk_rb_node!(index_10, index_11, index_2, $c4, TestOrder::new($val_4));

        // Padding nodes.

        // 5
        *get_mut_helper(&mut $data, index_5) = mk_rb_node!(
            index_0,
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            $c5,
            TestOrder::new(nondet())
        );

        // 6
        *get_mut_helper(&mut $data, index_6) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_1,
            $c6,
            TestOrder::new(nondet())
        );

        // 7
        *get_mut_helper(&mut $data, index_7) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_1,
            $c7,
            TestOrder::new(nondet())
        );

        // 8
        *get_mut_helper(&mut $data, index_8) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_3,
            $c8,
            TestOrder::new(nondet())
        );

        // 9
        *get_mut_helper(&mut $data, index_9) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_3,
            $c9,
            TestOrder::new(nondet())
        );

        // 10
        *get_mut_helper(&mut $data, index_10) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_4,
            $c10,
            TestOrder::new(nondet())
        );

        // 11
        *get_mut_helper(&mut $data, index_11) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_4,
            $c11,
            TestOrder::new(nondet())
        );

        RedBlackTree::new(&mut $data, index_5, index_11)
    }};
}

/// Builds the following tree:
///
///                 C5:5
///                   |
///                 C0:0
///             /            \
///         C1:1             C2:2
///        /     \          /    \
///     C3:3     C4:4    C6:6     C7:7
///    /  \      /   \
/// C8:8 C9:9 C10:10 C11:11
///
macro_rules! build_tree_shape_2 {
    ($data: expr,
            $val_0: expr,
            $val_1: expr,
            $val_2: expr,
            $val_3: expr,
            $val_4: expr,
            $c0: expr,
            $c1: expr,
            $c2: expr,
            $c3: expr,
            $c4: expr,
            $c5: expr,
            $c6: expr,
            $c7: expr,
            $c8: expr,
            $c9: expr,
            $c10: expr,
            $c11: expr,
        ) => {{
        let index_0 = 0 * TEST_BLOCK_WIDTH;
        let index_1 = 1 * TEST_BLOCK_WIDTH;
        let index_2 = 2 * TEST_BLOCK_WIDTH;
        let index_3 = 3 * TEST_BLOCK_WIDTH;
        let index_4 = 4 * TEST_BLOCK_WIDTH;
        let index_5 = 5 * TEST_BLOCK_WIDTH;
        let index_6 = 6 * TEST_BLOCK_WIDTH;
        let index_7 = 7 * TEST_BLOCK_WIDTH;
        let index_8 = 8 * TEST_BLOCK_WIDTH;
        let index_9 = 9 * TEST_BLOCK_WIDTH;
        let index_10 = 10 * TEST_BLOCK_WIDTH;
        let index_11 = 11 * TEST_BLOCK_WIDTH;

        // Nodes with defined values.

        // 0
        *get_mut_helper(&mut $data, index_0) =
            mk_rb_node!(index_1, index_2, index_5, $c0, TestOrder::new($val_0));

        // 1
        *get_mut_helper(&mut $data, index_1) =
            mk_rb_node!(index_3, index_4, index_0, $c1, TestOrder::new($val_1));

        // 2
        *get_mut_helper(&mut $data, index_2) =
            mk_rb_node!(index_6, index_7, index_0, $c2, TestOrder::new($val_2));

        // 3
        *get_mut_helper(&mut $data, index_3) =
            mk_rb_node!(index_8, index_9, index_1, $c3, TestOrder::new($val_3));

        // 4
        *get_mut_helper(&mut $data, index_4) =
            mk_rb_node!(index_10, index_11, index_1, $c4, TestOrder::new($val_4));

        // Padding nodes.

        // 5
        *get_mut_helper(&mut $data, index_5) =
            mk_rb_node!(index_0, NIL, NIL, $c5, TestOrder::new(nondet()));

        // 6
        *get_mut_helper(&mut $data, index_6) =
            mk_rb_node!(NIL, NIL, index_2, $c6, TestOrder::new(nondet()));

        // 7
        *get_mut_helper(&mut $data, index_7) =
            mk_rb_node!(NIL, NIL, index_2, $c7, TestOrder::new(nondet()));

        // 8
        *get_mut_helper(&mut $data, index_8) =
            mk_rb_node!(NIL, NIL, index_3, $c8, TestOrder::new(nondet()));

        // 9
        *get_mut_helper(&mut $data, index_9) =
            mk_rb_node!(NIL, NIL, index_3, $c9, TestOrder::new(nondet()));

        // 10
        *get_mut_helper(&mut $data, index_10) =
            mk_rb_node!(NIL, NIL, index_4, $c10, TestOrder::new(nondet()));

        // 11
        *get_mut_helper(&mut $data, index_11) =
            mk_rb_node!(NIL, NIL, index_4, $c11, TestOrder::new(nondet()));

        RedBlackTree::new(&mut $data, index_5, index_11)
    }};
}

// We verify that the `certora_remove_fix` implementation matches the `RB-Insert-Fixup`
// implementation in the 4th edition of "Introduction to Algorithms", ISBN
// 026204630X.
// The pseudo-code of the function is at page 339.
// A screenshot of the procedure can be found at the following link:
// https://drive.google.com/file/d/11Dz6GcUFcQuMhHs2e1dfGdkkmYzC8XcD/view
// While `RB-Delete-Fixup` has a while loop that fixes all the indices from the
// node that has to be fixed up to the root, the function `remove_fix` performs
// one single iteration of the loop, from the node at index `current_index`,
// which corresponds to the node `x` in the pseudo-code.
// In `remove_fix` there is no need to write a while loop, as the funciton is
// called from `remove` inside of a while loop.
// What we prove is that, for each possible case in the `RB-Delete-Fixup`
// function, the implementation matches the expected behaviour.
// There are four possible cases, and each one of them has a specular rule
// depending on whether the node to fix is the left or the right child.

/// Check that `remove_fix` behaves as the reference implementation in case 1,
/// when the node that has to be fixed (1) is a left child.
///
///                 ?:5                                  ?:5
///                  |                                    |
///                ?:0                                   B:2
///             /         \                          /         \
///         B:1            R:2        -->        R:0            ?:4
///        /   \         /    \                /   \         /    \
///      ?:6   ?:7     ?:3     ?:4           B:1   ?:3     ?:10   ?:11
///                  /  \     /   \        /  \   /   \
///                ?:8  ?:9 ?:10  ?:11  ?:6  ?:7 ?:8  ?:9
///
#[rule]
pub fn rule_remove_fix_matches_reference_case1_left_child() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let index_3 = 3 * TEST_BLOCK_WIDTH;
    let index_4 = 4 * TEST_BLOCK_WIDTH;
    let index_5 = 5 * TEST_BLOCK_WIDTH;
    let index_6 = 6 * TEST_BLOCK_WIDTH;
    let index_7 = 7 * TEST_BLOCK_WIDTH;
    let index_8 = 8 * TEST_BLOCK_WIDTH;
    let index_9 = 9 * TEST_BLOCK_WIDTH;
    let index_10 = 10 * TEST_BLOCK_WIDTH;
    let index_11 = 11 * TEST_BLOCK_WIDTH;

    let val_0: u64 = nondet();
    let val_1: u64 = nondet_with(|x| *x <= val_0);
    let val_2: u64 = nondet_with(|x| *x >= val_0);
    let val_3: u64 = nondet_with(|x| *x <= val_2 && *x >= val_0);
    let val_4: u64 = nondet_with(|x| *x >= val_2);

    let zero_initial_color = nondet();
    let three_initial_color = nondet();
    let four_inial_color = nondet();

    let mut tree: RedBlackTree<TestOrder> = build_tree_shape_1!(
        data,
        val_0,
        val_1,
        val_2,
        val_3,
        val_4,
        zero_initial_color,  // 0
        Color::Black,        // 1
        Color::Red,          // 2
        three_initial_color, // 3
        four_inial_color,    // 4
        nondet(),            // 5
        nondet(),            // 6
        nondet(),            // 7
        nondet(),            // 8
        nondet(),            // 9
        nondet(),            // 10
        nondet(),            // 11
    );

    let (next_to_fix_index, next_to_fix_index_parent) = tree.certora_remove_fix(index_1, index_0);

    // In the reference implementation they recolor the sibling and the parent,
    // and then they perform a rotation. After that, they keep considering the
    // other cases, since case 1 can be then reduced to one of the subsequent
    // cases.
    // This is not necessary in `remove_fix`, since the index that is returned
    // is `index_1`, which will then be handled in the outer while loop in the
    // function `remove`.
    // Therefore, we check the state of case 1 after executing the lines 5-8 in
    // the pseudocode.
    cvt_assert!(next_to_fix_index == index_1);
    cvt_assert!(next_to_fix_index_parent == index_0);

    cvt_assert!(tree.get_color::<TestOrder>(index_0) == Color::Red);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_0) == index_2);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_0) == index_1);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_0) == index_3);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == index_6);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == index_7);

    cvt_assert!(tree.get_color::<TestOrder>(index_2) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_2) == index_5);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_2) == index_0);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_2) == index_4);

    cvt_assert!(tree.get_color::<TestOrder>(index_3) == three_initial_color);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_3) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_3) == index_8);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_3) == index_9);

    cvt_assert!(tree.get_color::<TestOrder>(index_4) == four_inial_color);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_4) == index_2);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_4) == index_10);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_4) == index_11);

    cvt_vacuity_check!();
}

/// Check that `remove_fix` behaves as the reference implementation in case 1,
/// when the node that has to be fixed (2) is a right child.
///
///                  ?:5                                    ?:5
///                   |                                      |
///                  ?:0                                    B:1
///             /            \                         /             \
///          R:1             B:2       -->          ?:3              R:0
///        /     \          /    \                /    \          /      \
///     ?:3      ?:4      ?:6   ?:7             ?:8    ?:9      ?:4      B:2
///    /  \     /   \                                          /  \     /   \
///  ?:8  ?:9 ?:10  ?:11                                    ?:10 ?:11 ?:6  ?:7
#[rule]
pub fn rule_remove_fix_matches_reference_case1_right_child() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let index_3 = 3 * TEST_BLOCK_WIDTH;
    let index_4 = 4 * TEST_BLOCK_WIDTH;
    let index_5 = 5 * TEST_BLOCK_WIDTH;
    let index_6 = 6 * TEST_BLOCK_WIDTH;
    let index_7 = 7 * TEST_BLOCK_WIDTH;
    let index_8 = 8 * TEST_BLOCK_WIDTH;
    let index_9 = 9 * TEST_BLOCK_WIDTH;
    let index_10 = 10 * TEST_BLOCK_WIDTH;
    let index_11 = 11 * TEST_BLOCK_WIDTH;

    let val_0: u64 = nondet();
    let val_1: u64 = nondet_with(|x| *x <= val_0);
    let val_2: u64 = nondet_with(|x| *x >= val_0);
    let val_3: u64 = nondet_with(|x| *x <= val_2 && *x >= val_0);
    let val_4: u64 = nondet_with(|x| *x >= val_2);

    let zero_initial_color = nondet();
    let three_initial_color = nondet();
    let four_inial_color = nondet();

    let mut tree: RedBlackTree<TestOrder> = build_tree_shape_2!(
        data,
        val_0,
        val_1,
        val_2,
        val_3,
        val_4,
        zero_initial_color,  // 0
        Color::Red,          // 1
        Color::Black,        // 2
        three_initial_color, // 3
        four_inial_color,    // 4
        nondet(),            // 5
        nondet(),            // 6
        nondet(),            // 7
        nondet(),            // 8
        nondet(),            // 9
        nondet(),            // 10
        nondet(),            // 11
    );

    let (next_to_fix_index, next_to_fix_index_parent) = tree.certora_remove_fix(index_2, index_0);

    // In the reference implementation they recolor the sibling and the parent,
    // and then they perform a rotation. After that, they keep considering the
    // other cases, since case 1 can be then reduced to one of the subsequent
    // cases.
    // This is not necessary in `remove_fix`, since the index that is returned
    // is `index_2`, which will then be handled in the outer while loop in the
    // function `remove`.
    // Therefore, we check the state of case 1 after executing the lines 26-29
    // in the pseudocode.
    cvt_assert!(next_to_fix_index == index_2);
    cvt_assert!(next_to_fix_index_parent == index_0);

    cvt_assert!(tree.get_color::<TestOrder>(index_0) == Color::Red);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_0) == index_1);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_0) == index_4);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_0) == index_2);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_5);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == index_3);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == index_0);

    cvt_assert!(tree.get_color::<TestOrder>(index_2) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_2) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_2) == index_6);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_2) == index_7);

    cvt_assert!(tree.get_color::<TestOrder>(index_3) == three_initial_color);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_3) == index_1);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_3) == index_8);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_3) == index_9);

    cvt_assert!(tree.get_color::<TestOrder>(index_4) == four_inial_color);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_4) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_4) == index_10);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_4) == index_11);

    cvt_vacuity_check!();
}

/// Check that `remove_fix` behaves as the reference implementation in case 2,
/// when the node that has to be fixed (1) is a left child.
///
///                 ?:5                              ?:5
///                  |                                |
///                ?:0                              B:0
///             /         \                      /         \
///         B:1            B:2        -->    B:1            R:2
///        /   \         /    \             /   \         /    \
///      ?:6   ?:7     B:3     B:4        ?:6   ?:7     B:3     B:4
///                  /  \     /   \                   /  \     /   \
///                ?:8  ?:9 ?:10  ?:11            ?:8  ?:9 ?:10  ?:11
///
#[rule]
pub fn rule_remove_fix_matches_reference_case2_left_child() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let index_3 = 3 * TEST_BLOCK_WIDTH;
    let index_4 = 4 * TEST_BLOCK_WIDTH;
    let index_5 = 5 * TEST_BLOCK_WIDTH;
    let index_6 = 6 * TEST_BLOCK_WIDTH;
    let index_7 = 7 * TEST_BLOCK_WIDTH;
    let index_8 = 8 * TEST_BLOCK_WIDTH;
    let index_9 = 9 * TEST_BLOCK_WIDTH;
    let index_10 = 10 * TEST_BLOCK_WIDTH;
    let index_11 = 11 * TEST_BLOCK_WIDTH;

    let val_0: u64 = nondet();
    let val_1: u64 = nondet_with(|x| *x <= val_0);
    let val_2: u64 = nondet_with(|x| *x >= val_0);
    let val_3: u64 = nondet_with(|x| *x <= val_1);
    let val_4: u64 = nondet_with(|x| *x >= val_1 && *x >= val_0);

    let zero_initial_color = nondet();

    let mut tree: RedBlackTree<TestOrder> = build_tree_shape_1!(
        data,
        val_0,
        val_1,
        val_2,
        val_3,
        val_4,
        zero_initial_color, // 0
        Color::Black,       // 1
        Color::Black,       // 2
        Color::Black,       // 3
        Color::Black,       // 4
        nondet(),           // 5
        nondet(),           // 6
        nondet(),           // 7
        nondet(),           // 8
        nondet(),           // 9
        nondet(),           // 10
        nondet(),           // 11
    );

    let (next_to_fix_index, next_to_fix_index_parent) = tree.certora_remove_fix(index_1, index_0);

    if zero_initial_color == Color::Red {
        // If node 0 was red, the procedure should stop.
        cvt_assert!(next_to_fix_index == NIL);
        cvt_assert!(next_to_fix_index_parent == NIL);
    }
    if zero_initial_color == Color::Black {
        // If node 0 was black, the procedure should continue on node 0.
        cvt_assert!(next_to_fix_index == index_0);
        cvt_assert!(next_to_fix_index_parent == index_5);
    }

    // The color of node 0 is always black in the reference implementation.
    // If node 0 was black, its color does not change in case 2 (lines 10-11).
    // If node 0 was red, its color does not change in the while loop, but when
    // the loop ends because the condition is false: node 0 is red.
    // Then, before returning, the color of node 0 (corresponding to variable
    // `x` in the pseudo-code) is set to black.
    cvt_assert!(tree.get_color::<TestOrder>(index_0) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_0) == index_5);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_0) == index_1);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_0) == index_2);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == index_6);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == index_7);

    cvt_assert!(tree.get_color::<TestOrder>(index_2) == Color::Red);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_2) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_2) == index_3);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_2) == index_4);

    cvt_assert!(tree.get_color::<TestOrder>(index_3) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_3) == index_2);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_3) == index_8);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_3) == index_9);

    cvt_assert!(tree.get_color::<TestOrder>(index_4) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_4) == index_2);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_4) == index_10);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_4) == index_11);

    cvt_vacuity_check!();
}

/// Check that `remove_fix` behaves as the reference implementation in case 2,
/// when the node that has to be fixed (2) is a right child.
///
///                  ?:5                                        ?:5
///                   |                                          |
///                  ?:0                                        B:0
///             /            \                             /            \
///          B:1             B:2       -->              R:1             B:2
///        /     \          /    \                    /     \          /    \
///     B:3      B:4      ?:6   ?:7                B:3      B:4      ?:6   ?:7
///    /  \     /   \                             /  \     /   \
///  ?:8  ?:9 ?:10  ?:11                        ?:8  ?:9 ?:10  ?:11
#[rule]
pub fn rule_remove_fix_matches_reference_case2_right_child() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let index_3 = 3 * TEST_BLOCK_WIDTH;
    let index_4 = 4 * TEST_BLOCK_WIDTH;
    let index_5 = 5 * TEST_BLOCK_WIDTH;
    let index_6 = 6 * TEST_BLOCK_WIDTH;
    let index_7 = 7 * TEST_BLOCK_WIDTH;
    let index_8 = 8 * TEST_BLOCK_WIDTH;
    let index_9 = 9 * TEST_BLOCK_WIDTH;
    let index_10 = 10 * TEST_BLOCK_WIDTH;
    let index_11 = 11 * TEST_BLOCK_WIDTH;

    let val_0: u64 = nondet();
    let val_1: u64 = nondet_with(|x| *x <= val_0);
    let val_2: u64 = nondet_with(|x| *x >= val_0);
    let val_3: u64 = nondet_with(|x| *x <= val_1);
    let val_4: u64 = nondet_with(|x| *x >= val_1 && *x >= val_0);

    let zero_initial_color = nondet();

    let mut tree: RedBlackTree<TestOrder> = build_tree_shape_2!(
        data,
        val_0,
        val_1,
        val_2,
        val_3,
        val_4,
        zero_initial_color, // 0
        Color::Black,       // 1
        Color::Black,       // 2
        Color::Black,       // 3
        Color::Black,       // 4
        nondet(),           // 5
        nondet(),           // 6
        nondet(),           // 7
        nondet(),           // 8
        nondet(),           // 9
        nondet(),           // 10
        nondet(),           // 11
    );

    let (next_to_fix_index, next_to_fix_index_parent) = tree.certora_remove_fix(index_2, index_0);

    if zero_initial_color == Color::Red {
        // If node 0 was red, the procedure should stop.
        cvt_assert!(next_to_fix_index == NIL);
        cvt_assert!(next_to_fix_index_parent == NIL);
    }
    if zero_initial_color == Color::Black {
        // If node 0 was black, the procedure should continue on node 0.
        cvt_assert!(next_to_fix_index == index_0);
        cvt_assert!(next_to_fix_index_parent == index_5);
    }

    // The color of node 0 is always black in the reference implementation.
    // If node 0 was black, its color does not change in case 2 (lines 30-31).
    // If node 0 was red, its color does not change in the while loop, but when
    // the loop ends because the condition is false: node 0 is red.
    // Then, before returning, the color of node 0 (corresponding to variable
    // `x` in the pseudo-code) is set to black.
    cvt_assert!(tree.get_color::<TestOrder>(index_0) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_0) == index_5);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_0) == index_1);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_0) == index_2);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == Color::Red);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == index_3);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == index_4);

    cvt_assert!(tree.get_color::<TestOrder>(index_2) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_2) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_2) == index_6);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_2) == index_7);

    cvt_assert!(tree.get_color::<TestOrder>(index_3) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_3) == index_1);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_3) == index_8);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_3) == index_9);

    cvt_assert!(tree.get_color::<TestOrder>(index_4) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_4) == index_1);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_4) == index_10);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_4) == index_11);

    cvt_vacuity_check!();
}

/// Check that `remove_fix` behaves as the reference implementation in case 3,
/// when the node that has to be fixed (1) is a left child.
///
///                 ?:5                                 ?:5
///                  |                                   |
///                ?:0                                 ?:3
///             /         \                         /         \
///         B:1            B:2        -->       B:0            B:2
///        /   \         /    \                /   \         /    \
///      ?:6   ?:7     R:3     B:4           B:1   ?:8     ?:9     B:4
///                  /  \     /   \         /  \                  /   \
///                ?:8  ?:9 ?:10  ?:11   ?:6  ?:7              ?:10  ?:11
///
/// Observe that node 3 should have the initial color of node 0, which is what
/// currently fails.
#[rule]
pub fn rule_remove_fix_matches_reference_case3_left_child() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let index_3 = 3 * TEST_BLOCK_WIDTH;
    let index_4 = 4 * TEST_BLOCK_WIDTH;
    let index_5 = 5 * TEST_BLOCK_WIDTH;
    let index_6 = 6 * TEST_BLOCK_WIDTH;
    let index_7 = 7 * TEST_BLOCK_WIDTH;
    let index_8 = 8 * TEST_BLOCK_WIDTH;
    let index_9 = 9 * TEST_BLOCK_WIDTH;
    let index_10 = 10 * TEST_BLOCK_WIDTH;
    let index_11 = 11 * TEST_BLOCK_WIDTH;

    let val_0: u64 = nondet();
    let val_1: u64 = nondet_with(|x| *x <= val_0);
    let val_2: u64 = nondet_with(|x| *x >= val_0);
    let val_3: u64 = nondet_with(|x| *x <= val_2 && *x >= val_0);
    let val_4: u64 = nondet_with(|x| *x >= val_2);

    let zero_initial_color = nondet();

    let mut tree: RedBlackTree<TestOrder> = build_tree_shape_1!(
        data,
        val_0,
        val_1,
        val_2,
        val_3,
        val_4,
        zero_initial_color, // 0
        Color::Black,       // 1
        Color::Black,       // 2
        Color::Red,         // 3
        Color::Black,       // 4
        nondet(),           // 5
        nondet(),           // 6
        nondet(),           // 7
        nondet(),           // 8
        nondet(),           // 9
        nondet(),           // 10
        nondet(),           // 11
    );

    let (next_to_fix_index, next_to_fix_index_parent) = tree.certora_remove_fix(index_1, index_0);

    // Nothing to fix in this case.
    cvt_assert!(next_to_fix_index == NIL);
    cvt_assert!(next_to_fix_index_parent == NIL);

    cvt_assert!(tree.get_color::<TestOrder>(index_0) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_0) == index_3);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_0) == index_1);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_0) == index_8);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == index_6);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == index_7);

    cvt_assert!(tree.get_color::<TestOrder>(index_2) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_2) == index_3);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_2) == index_9);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_2) == index_4);

    cvt_assert!(tree.get_color::<TestOrder>(index_3) == zero_initial_color);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_3) == index_5);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_3) == index_0);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_3) == index_2);

    cvt_assert!(tree.get_color::<TestOrder>(index_4) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_4) == index_2);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_4) == index_10);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_4) == index_11);

    cvt_vacuity_check!();
}

/// Check that `remove_fix` behaves as the reference implementation in case 3,
/// when the node that has to be fixed (2) is a right child.
///
///                  ?:5                                        ?:5
///                   |                                          |
///                  ?:0                                        ?:4
///             /            \                             /            \
///          B:1             B:2       -->              B:1             B:0
///        /     \          /    \                    /     \          /    \
///     B:3      R:4      ?:6   ?:7                B:3      ?:10     ?:11   B:2
///    /  \     /   \                             /  \                     /   \
///  ?:8  ?:9 ?:10  ?:11                        ?:8  ?:9                 ?:6   ?:7
///
/// Observe that node 4 should have the initial color of node 0, which is what
/// currently fails.
#[rule]
pub fn rule_remove_fix_matches_reference_case3_right_child() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let index_3 = 3 * TEST_BLOCK_WIDTH;
    let index_4 = 4 * TEST_BLOCK_WIDTH;
    let index_5 = 5 * TEST_BLOCK_WIDTH;
    let index_6 = 6 * TEST_BLOCK_WIDTH;
    let index_7 = 7 * TEST_BLOCK_WIDTH;
    let index_8 = 8 * TEST_BLOCK_WIDTH;
    let index_9 = 9 * TEST_BLOCK_WIDTH;
    let index_10 = 10 * TEST_BLOCK_WIDTH;
    let index_11 = 11 * TEST_BLOCK_WIDTH;

    let val_0: u64 = nondet();
    let val_1: u64 = nondet_with(|x| *x <= val_0);
    let val_2: u64 = nondet_with(|x| *x >= val_0);
    let val_3: u64 = nondet_with(|x| *x <= val_1);
    let val_4: u64 = nondet_with(|x| *x >= val_1 && *x >= val_0);

    let zero_initial_color = nondet();

    let mut tree: RedBlackTree<TestOrder> = build_tree_shape_2!(
        data,
        val_0,
        val_1,
        val_2,
        val_3,
        val_4,
        zero_initial_color, // 0
        Color::Black,       // 1
        Color::Black,       // 2
        Color::Black,       // 3
        Color::Red,         // 4
        nondet(),           // 5
        nondet(),           // 6
        nondet(),           // 7
        nondet(),           // 8
        nondet(),           // 9
        nondet(),           // 10
        nondet(),           // 11
    );

    let (next_to_fix_index, next_to_fix_index_parent) = tree.certora_remove_fix(index_2, index_0);

    // Nothing to fix in this case.
    cvt_assert!(next_to_fix_index == NIL);
    cvt_assert!(next_to_fix_index_parent == NIL);

    cvt_assert!(tree.get_color::<TestOrder>(index_0) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_0) == index_4);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_0) == index_11);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_0) == index_2);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_4);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == index_3);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == index_10);

    cvt_assert!(tree.get_color::<TestOrder>(index_2) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_2) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_2) == index_6);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_2) == index_7);

    cvt_assert!(tree.get_color::<TestOrder>(index_3) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_3) == index_1);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_3) == index_8);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_3) == index_9);

    cvt_assert!(tree.get_color::<TestOrder>(index_4) == zero_initial_color);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_4) == index_5);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_4) == index_1);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_4) == index_0);

    cvt_vacuity_check!();
}

/// Check that `remove_fix` behaves as the reference implementation in case 4,
/// when the node that has to be fixed (1) is a left child.
///
///                 ?:5                                  ?:5
///                  |                                    |
///                ?:0                                   ?:2
///             /         \                          /         \
///         B:1            B:2        -->        B:0            B:4
///        /   \         /    \                /   \         /    \
///      ?:6   ?:7     B:3     R:4           B:1   B:3     ?:10   ?:11
///                  /  \     /   \        /  \    /  \
///                ?:8  ?:9 ?:10  ?:11  ?:6  ?:7 ?:8  ?:9
///
/// Observe that node 2 should have the initial color of node 0.
#[rule]
pub fn rule_remove_fix_matches_reference_case4_left_child() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let index_3 = 3 * TEST_BLOCK_WIDTH;
    let index_4 = 4 * TEST_BLOCK_WIDTH;
    let index_5 = 5 * TEST_BLOCK_WIDTH;
    let index_6 = 6 * TEST_BLOCK_WIDTH;
    let index_7 = 7 * TEST_BLOCK_WIDTH;
    let index_8 = 8 * TEST_BLOCK_WIDTH;
    let index_9 = 9 * TEST_BLOCK_WIDTH;
    let index_10 = 10 * TEST_BLOCK_WIDTH;
    let index_11 = 11 * TEST_BLOCK_WIDTH;

    let val_0: u64 = nondet();
    let val_1: u64 = nondet_with(|x| *x <= val_0);
    let val_2: u64 = nondet_with(|x| *x >= val_0);
    let val_3: u64 = nondet_with(|x| *x <= val_2 && *x >= val_0);
    let val_4: u64 = nondet_with(|x| *x >= val_2);

    let zero_initial_color = nondet();

    let mut tree: RedBlackTree<TestOrder> = build_tree_shape_1!(
        data,
        val_0,
        val_1,
        val_2,
        val_3,
        val_4,
        zero_initial_color, // 0
        Color::Black,       // 1
        Color::Black,       // 2
        Color::Black,       // 3
        Color::Red,         // 4
        nondet(),           // 5
        nondet(),           // 6
        nondet(),           // 7
        nondet(),           // 8
        nondet(),           // 9
        nondet(),           // 10
        nondet(),           // 11
    );

    let (next_to_fix_index, next_to_fix_index_parent) = tree.certora_remove_fix(index_1, index_0);

    // Nothing to fix in this case.
    cvt_assert!(next_to_fix_index == NIL);
    cvt_assert!(next_to_fix_index_parent == NIL);

    cvt_assert!(tree.get_color::<TestOrder>(index_0) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_0) == index_2);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_0) == index_1);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_0) == index_3);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == index_6);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == index_7);

    cvt_assert!(tree.get_color::<TestOrder>(index_2) == zero_initial_color);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_2) == index_5);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_2) == index_0);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_2) == index_4);

    cvt_assert!(tree.get_color::<TestOrder>(index_3) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_3) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_3) == index_8);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_3) == index_9);

    cvt_assert!(tree.get_color::<TestOrder>(index_4) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_4) == index_2);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_4) == index_10);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_4) == index_11);

    cvt_vacuity_check!();
}

/// Check that `remove_fix` behaves as the reference implementation in case 3,
/// when the node that has to be fixed (2) is a right child.
///
///                  ?:5                                   ?:5
///                   |                                     |
///                  ?:0                                  ?:1
///             /            \                         /         \
///          B:1             B:2       -->         B:3            B:0
///        /     \          /    \                /   \         /    \
///     R:3      B:4      ?:6   ?:7             ?:8   ?:9     B:4     B:2
///    /  \     /   \                                       /  \      /  \
///  ?:8  ?:9 ?:10  ?:11                                 ?:10 ?:11  ?:6  ?:7
///
/// Observe that node 1 should have the initial color of node 0.
#[rule]
pub fn rule_remove_fix_matches_reference_case4_right_child() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let index_3 = 3 * TEST_BLOCK_WIDTH;
    let index_4 = 4 * TEST_BLOCK_WIDTH;
    let index_5 = 5 * TEST_BLOCK_WIDTH;
    let index_6 = 6 * TEST_BLOCK_WIDTH;
    let index_7 = 7 * TEST_BLOCK_WIDTH;
    let index_8 = 8 * TEST_BLOCK_WIDTH;
    let index_9 = 9 * TEST_BLOCK_WIDTH;
    let index_10 = 10 * TEST_BLOCK_WIDTH;
    let index_11 = 11 * TEST_BLOCK_WIDTH;

    let val_0: u64 = nondet();
    let val_1: u64 = nondet_with(|x| *x <= val_0);
    let val_2: u64 = nondet_with(|x| *x >= val_0);
    let val_3: u64 = nondet_with(|x| *x <= val_1);
    let val_4: u64 = nondet_with(|x| *x >= val_1 && *x >= val_0);

    let zero_initial_color = nondet();

    let mut tree: RedBlackTree<TestOrder> = build_tree_shape_2!(
        data,
        val_0,
        val_1,
        val_2,
        val_3,
        val_4,
        zero_initial_color, // 0
        Color::Black,       // 1
        Color::Black,       // 2
        Color::Red,         // 3
        Color::Black,       // 4
        nondet(),           // 5
        nondet(),           // 6
        nondet(),           // 7
        nondet(),           // 8
        nondet(),           // 9
        nondet(),           // 10
        nondet(),           // 11
    );

    let (next_to_fix_index, next_to_fix_index_parent) = tree.certora_remove_fix(index_2, index_0);

    // Nothing to fix in this case.
    cvt_assert!(next_to_fix_index == NIL);
    cvt_assert!(next_to_fix_index_parent == NIL);

    cvt_assert!(tree.get_color::<TestOrder>(index_0) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_0) == index_1);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_0) == index_4);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_0) == index_2);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == zero_initial_color);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_5);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == index_3);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == index_0);

    cvt_assert!(tree.get_color::<TestOrder>(index_2) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_2) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_2) == index_6);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_2) == index_7);

    cvt_assert!(tree.get_color::<TestOrder>(index_3) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_3) == index_1);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_3) == index_8);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_3) == index_9);

    cvt_assert!(tree.get_color::<TestOrder>(index_4) == Color::Black);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_4) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_4) == index_10);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_4) == index_11);

    cvt_vacuity_check!();
}

/// Check that `insert` correctly updates the `max_index` after inserting in an
/// empty tree.
#[rule]
pub fn rule_insert_updates_max_index_empty_tree() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let mut tree: RedBlackTree<TestOrder> = RedBlackTree::new(&mut data, NIL, NIL);

    cvt_assert!(tree.max_index() == NIL);

    tree.insert(0, TestOrder::new(nondet::<u64>()));

    cvt_assert!(tree.max_index() == 0);
    cvt_vacuity_check!();
}

/// Builds the following tree:
///
///               B:0
///             /     \
///        subtree1   B:1
///                  /
///             subtree2
///
/// The subtrees are arbitrary trees for more coverage.
macro_rules! build_tree_7 {
    ($data: expr, $val_0: expr, $val_1: expr) => {{
        let index_0 = 0 * TEST_BLOCK_WIDTH;
        let index_1 = 1 * TEST_BLOCK_WIDTH;

        // 0
        *get_mut_helper(&mut $data, index_0) = mk_rb_node!(
            nondet::<DataIndex>(),
            index_1,
            NIL,
            Color::Black,
            TestOrder::new($val_0)
        );

        // 1
        *get_mut_helper(&mut $data, index_1) = mk_rb_node!(
            nondet::<DataIndex>(),
            NIL,
            index_0,
            Color::Black,
            TestOrder::new($val_1)
        );

        RedBlackTree::new(&mut $data, index_0, index_1)
    }};
}

/// Check that `insert` correctly updates the `max_index` after inserting the
/// max element is a non-empty tree.
#[rule]
pub fn rule_insert_updates_max_index_non_empty_tree_max() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let val_0: u64 = nondet();
    let val_1: u64 = nondet_with(|x| *x >= val_0);
    let val_2: u64 = nondet_with(|x| *x > val_1);

    // Node 0 and 1 are black to speed up the verification (no rotations
    // required by insert).
    let mut tree: RedBlackTree<TestOrder> = build_tree_7!(data, val_0, val_1);

    cvt_assert!(tree.max_index() == index_1);
    tree.insert(index_2, TestOrder::new(val_2));

    // Check that insert correctly updated the max index.
    cvt_assert!(tree.max_index() == index_2);

    cvt_vacuity_check!();
}

/// Builds the following tree:
///
///               B:0
///             /     \
///        subtree1   B:1
///
/// The subtree is an arbitrary tree for more coverage.
macro_rules! build_tree_8 {
    ($data: expr, $val_0: expr, $val_1: expr) => {{
        let index_0 = 0 * TEST_BLOCK_WIDTH;
        let index_1 = 1 * TEST_BLOCK_WIDTH;

        // 0
        *get_mut_helper(&mut $data, index_0) = mk_rb_node!(
            nondet::<DataIndex>(),
            index_1,
            NIL,
            Color::Black,
            TestOrder::new($val_0)
        );

        // 1
        *get_mut_helper(&mut $data, index_1) =
            mk_rb_node!(NIL, NIL, index_0, Color::Black, TestOrder::new($val_1));

        RedBlackTree::new(&mut $data, index_0, index_1)
    }};
}

/// Check that `insert` correctly updates the `max_index` after inserting an
/// element that is not the max in a non-empty tree.
#[rule]
pub fn rule_insert_updates_max_index_non_empty_tree_not_max() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let val_0: u64 = nondet();
    let val_1: u64 = nondet_with(|x| *x >= val_0);
    let val_2: u64 = nondet_with(|x| *x > val_0 && *x < val_1);

    // Node 0 and 1 are black to speed up the verification (no rotations
    // required by insert).
    let mut tree: RedBlackTree<TestOrder> = build_tree_8!(data, val_0, val_1);

    cvt_assert!(tree.max_index() == index_1);
    tree.insert(index_2, TestOrder::new(val_2));

    // Check that insert did not update the max index.
    cvt_assert!(tree.max_index() == index_1);

    cvt_vacuity_check!();
}

/// Builds the following tree:
///
///             B:2
///           /   \
///     subtree1  B:0
///             /     \
///        subtree2   R:1
///
/// The subtrees are arbitrary trees for more coverage.
macro_rules! build_tree_9 {
    ($data: expr, $val_0: expr, $val_1: expr, $val_2: expr) => {{
        let index_0 = 0 * TEST_BLOCK_WIDTH;
        let index_1 = 1 * TEST_BLOCK_WIDTH;
        let index_2 = 2 * TEST_BLOCK_WIDTH;

        // 2
        *get_mut_helper(&mut $data, index_2) = mk_rb_node!(
            nondet_with(|i: &DataIndex| *i != index_0 && *i != index_1),
            index_0,
            NIL,
            Color::Black,
            TestOrder::new($val_2)
        );

        // 0
        *get_mut_helper(&mut $data, index_0) = mk_rb_node!(
            nondet_with(|i: &DataIndex| *i != index_2 && *i != index_1),
            index_1,
            index_2,
            Color::Black,
            TestOrder::new($val_0)
        );

        // 1
        *get_mut_helper(&mut $data, index_1) =
            mk_rb_node!(NIL, NIL, index_0, Color::Red, TestOrder::new($val_1));

        RedBlackTree::new(&mut $data, index_2, index_1)
    }};
}

/// Check that `remove` correctly updates the `max_index` after removing the
/// only element in a 1-element tree.
#[rule]
pub fn rule_remove_updates_max_index_single_node_tree() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0;

    // 0
    *get_mut_helper(&mut data, index_0) =
        mk_rb_node!(NIL, NIL, NIL, Color::Black, TestOrder::new(nondet()));

    let mut tree: RedBlackTree<TestOrder> = RedBlackTree::new(&mut data, index_0, index_0);

    cvt_assert!(tree.max_index() == index_0);

    tree.remove_by_index(index_0);

    cvt_assert!(tree.max_index() == NIL);
    cvt_vacuity_check!();
}

/// Check that `remove` correctly updates the `max_index` after removing the
/// max element.
#[rule]
pub fn rule_remove_updates_max_index_non_empty_tree_max() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let _index_2 = 2 * TEST_BLOCK_WIDTH;
    let val_2: u64 = nondet();
    let val_0: u64 = nondet_with(|x| *x > val_2);
    let val_1: u64 = nondet_with(|x| *x > val_0);

    // Node 0 and 1 are respectively black and red to speed up the verification
    // (no rotations required by remove).
    let mut tree: RedBlackTree<TestOrder> = build_tree_9!(data, val_0, val_1, val_2);

    cvt_assert!(tree.max_index() == index_1);
    tree.remove_by_index(index_1);

    // Check that remove correctly updated the max index.
    cvt_assert!(tree.max_index() == index_0);

    cvt_vacuity_check!();
}

/// Builds the following tree:
///
///             B:2
///           /   \
///     subtree1  B:0
///                   \
///                   R:1
///
/// The subtrees are arbitrary trees for more coverage.
macro_rules! build_tree_9_1 {
    ($data: expr, $val_0: expr, $val_1: expr, $val_2: expr) => {{
        let index_0 = 0 * TEST_BLOCK_WIDTH;
        let index_1 = 1 * TEST_BLOCK_WIDTH;
        let index_2 = 2 * TEST_BLOCK_WIDTH;

        // 2
        *get_mut_helper(&mut $data, index_2) = mk_rb_node!(
            nondet_with(|i: &DataIndex| *i != index_0 && *i != index_1),
            index_0,
            NIL,
            Color::Black,
            TestOrder::new($val_2)
        );

        // 0
        *get_mut_helper(&mut $data, index_0) =
            mk_rb_node!(NIL, index_1, index_2, Color::Black, TestOrder::new($val_0));

        // 1
        *get_mut_helper(&mut $data, index_1) =
            mk_rb_node!(NIL, NIL, index_0, Color::Red, TestOrder::new($val_1));

        RedBlackTree::new(&mut $data, index_2, index_1)
    }};
}

/// Check that `remove` correctly updates the `max_index` after removing an
/// element which is not the max.
#[rule]
pub fn rule_remove_updates_max_index_non_empty_tree_not_max() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let _index_2 = 2 * TEST_BLOCK_WIDTH;
    let val_2: u64 = nondet();
    let val_0: u64 = nondet_with(|x| *x > val_2);
    let val_1: u64 = nondet_with(|x| *x > val_0);

    // Node 0 and 1 are respectively black and red to speed up the verification
    // (no rotations required by remove).
    let mut tree: RedBlackTree<TestOrder> = build_tree_9_1!(data, val_0, val_1, val_2);

    cvt_assert!(tree.max_index() == index_1);
    tree.remove_by_index(index_0);

    // Check that remove did not update the max index.
    cvt_assert!(tree.max_index() == index_1);

    cvt_vacuity_check!();
}

macro_rules! build_tree_10 {
    ($data: expr, $c1: expr, $c5: expr) => {{
        let index_0 = 0 * TEST_BLOCK_WIDTH;
        let index_1 = 1 * TEST_BLOCK_WIDTH;
        let index_2 = 2 * TEST_BLOCK_WIDTH;
        let index_3 = 3 * TEST_BLOCK_WIDTH;
        let index_4 = 4 * TEST_BLOCK_WIDTH;
        let index_5 = 5 * TEST_BLOCK_WIDTH;
        let index_6 = 6 * TEST_BLOCK_WIDTH;
        let index_7 = 7 * TEST_BLOCK_WIDTH;

        // 0
        *get_mut_helper(&mut $data, index_0) = mk_rb_node!(
            index_1,
            nondet_with(|i: &DataIndex| *i != index_1),
            nondet::<DataIndex>(),
            nondet(),
            TestOrder::new(nondet())
        );

        // 1
        *get_mut_helper(&mut $data, index_1) =
            mk_rb_node!(index_2, index_3, index_0, $c1, TestOrder::new(nondet()));

        // 2
        *get_mut_helper(&mut $data, index_2) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_1,
            nondet(),
            TestOrder::new(nondet())
        );

        // 3
        *get_mut_helper(&mut $data, index_3) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_1,
            nondet(),
            TestOrder::new(nondet())
        );

        // 4
        *get_mut_helper(&mut $data, index_4) = mk_rb_node!(
            index_5,
            nondet_with(|i: &DataIndex| *i != index_5),
            nondet::<DataIndex>(),
            nondet(),
            TestOrder::new(nondet())
        );

        // 5
        *get_mut_helper(&mut $data, index_5) =
            mk_rb_node!(index_6, index_7, index_4, $c5, TestOrder::new(nondet()));

        // 6
        *get_mut_helper(&mut $data, index_6) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_5,
            nondet(),
            TestOrder::new(nondet())
        );

        // 7
        *get_mut_helper(&mut $data, index_7) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_5,
            nondet(),
            TestOrder::new(nondet())
        );

        RedBlackTree::new(&mut $data, index_0, index_7)
    }};
}

// TODO: Remove the rules that are expected to fail
/// Checks that `swap_nodes` behaves as expected in the case that the two nodes
/// are internal nodes, and both are left children.
#[rule]
pub fn rule_swap_internal_nodes_left_children() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let index_3 = 3 * TEST_BLOCK_WIDTH;
    let index_4 = 4 * TEST_BLOCK_WIDTH;
    let index_5 = 5 * TEST_BLOCK_WIDTH;
    let index_6 = 6 * TEST_BLOCK_WIDTH;
    let index_7 = 7 * TEST_BLOCK_WIDTH;

    let initial_color_1: Color = nondet();
    let initial_color_5: Color = nondet();

    let mut tree: RedBlackTree<TestOrder> = build_tree_10!(data, initial_color_1, initial_color_5);

    tree.swap_node_with_successor::<TestOrder>(index_1, index_5);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == initial_color_5);
    cvt_assert!(tree.get_color::<TestOrder>(index_5) == initial_color_1);

    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_4);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_5) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_0) == index_5);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_4) == index_1);

    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == index_6);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_5) == index_2);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_6) == index_1);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_2) == index_5);

    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == index_7);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_5) == index_3);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_7) == index_1);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_3) == index_5);

    cvt_vacuity_check!();
}

macro_rules! build_tree_11 {
    ($data: expr, $c1: expr, $c5: expr) => {{
        let index_0 = 0 * TEST_BLOCK_WIDTH;
        let index_1 = 1 * TEST_BLOCK_WIDTH;
        let index_2 = 2 * TEST_BLOCK_WIDTH;
        let index_3 = 3 * TEST_BLOCK_WIDTH;
        let index_4 = 4 * TEST_BLOCK_WIDTH;
        let index_5 = 5 * TEST_BLOCK_WIDTH;
        let index_6 = 6 * TEST_BLOCK_WIDTH;
        let index_7 = 7 * TEST_BLOCK_WIDTH;

        // 0
        *get_mut_helper(&mut $data, index_0) = mk_rb_node!(
            nondet_with(|i: &DataIndex| *i != index_1),
            index_1,
            nondet::<DataIndex>(),
            nondet(),
            TestOrder::new(nondet())
        );

        // 1
        *get_mut_helper(&mut $data, index_1) =
            mk_rb_node!(index_2, index_3, index_0, $c1, TestOrder::new(nondet()));

        // 2
        *get_mut_helper(&mut $data, index_2) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_1,
            nondet(),
            TestOrder::new(nondet())
        );

        // 3
        *get_mut_helper(&mut $data, index_3) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_1,
            nondet(),
            TestOrder::new(nondet())
        );

        // 4
        *get_mut_helper(&mut $data, index_4) = mk_rb_node!(
            nondet_with(|i: &DataIndex| *i != index_5),
            index_5,
            nondet::<DataIndex>(),
            nondet(),
            TestOrder::new(nondet())
        );

        // 5
        *get_mut_helper(&mut $data, index_5) =
            mk_rb_node!(index_6, index_7, index_4, $c5, TestOrder::new(nondet()));

        // 6
        *get_mut_helper(&mut $data, index_6) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_5,
            nondet(),
            TestOrder::new(nondet())
        );

        // 7
        *get_mut_helper(&mut $data, index_7) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_5,
            nondet(),
            TestOrder::new(nondet())
        );

        RedBlackTree::new(&mut $data, index_0, index_7)
    }};
}

/// Checks that `swap_nodes` behaves as expected in the case that the two nodes
/// are internal nodes, and both are right children.
#[rule]
pub fn rule_swap_internal_nodes_right_children() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let index_3 = 3 * TEST_BLOCK_WIDTH;
    let index_4 = 4 * TEST_BLOCK_WIDTH;
    let index_5 = 5 * TEST_BLOCK_WIDTH;
    let index_6 = 6 * TEST_BLOCK_WIDTH;
    let index_7 = 7 * TEST_BLOCK_WIDTH;

    let initial_color_1: Color = nondet();
    let initial_color_5: Color = nondet();

    let mut tree: RedBlackTree<TestOrder> = build_tree_11!(data, initial_color_1, initial_color_5);

    tree.swap_node_with_successor::<TestOrder>(index_1, index_5);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == initial_color_5);
    cvt_assert!(tree.get_color::<TestOrder>(index_5) == initial_color_1);

    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_4);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_5) == index_0);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_0) == index_5);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_4) == index_1);

    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == index_6);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_5) == index_2);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_6) == index_1);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_2) == index_5);

    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == index_7);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_5) == index_3);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_7) == index_1);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_3) == index_5);

    cvt_vacuity_check!();
}

macro_rules! build_tree_12 {
    ($data: expr, $c1: expr, $c5: expr) => {{
        let _index_0 = 0 * TEST_BLOCK_WIDTH;
        let index_1 = 1 * TEST_BLOCK_WIDTH;
        let index_2 = 2 * TEST_BLOCK_WIDTH;
        let index_3 = 3 * TEST_BLOCK_WIDTH;
        let index_4 = 4 * TEST_BLOCK_WIDTH;
        let index_5 = 5 * TEST_BLOCK_WIDTH;
        let index_6 = 6 * TEST_BLOCK_WIDTH;
        let index_7 = 7 * TEST_BLOCK_WIDTH;

        // 1
        *get_mut_helper(&mut $data, index_1) =
            mk_rb_node!(index_2, index_3, NIL, $c1, TestOrder::new(nondet()));

        // 2
        *get_mut_helper(&mut $data, index_2) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_1,
            nondet(),
            TestOrder::new(nondet())
        );

        // 3
        *get_mut_helper(&mut $data, index_3) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_1,
            nondet(),
            TestOrder::new(nondet())
        );

        // 4
        *get_mut_helper(&mut $data, index_4) = mk_rb_node!(
            nondet_with(|i: &DataIndex| *i != index_5),
            index_5,
            nondet::<DataIndex>(),
            nondet(),
            TestOrder::new(nondet())
        );

        // 5
        *get_mut_helper(&mut $data, index_5) =
            mk_rb_node!(index_6, index_7, index_4, $c5, TestOrder::new(nondet()));

        // 6
        *get_mut_helper(&mut $data, index_6) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_5,
            nondet(),
            TestOrder::new(nondet())
        );

        // 7
        *get_mut_helper(&mut $data, index_7) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_5,
            nondet(),
            TestOrder::new(nondet())
        );

        RedBlackTree::new(&mut $data, index_1, index_7)
    }};
}

/// Checks that `swap_nodes` behaves as expected in the case that the two nodes
/// are internal nodes, and the first one is the root.
#[rule]
pub fn rule_swap_internal_nodes_first_is_root() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let _index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let index_3 = 3 * TEST_BLOCK_WIDTH;
    let index_4 = 4 * TEST_BLOCK_WIDTH;
    let index_5 = 5 * TEST_BLOCK_WIDTH;
    let index_6 = 6 * TEST_BLOCK_WIDTH;
    let index_7 = 7 * TEST_BLOCK_WIDTH;

    let initial_color_1: Color = nondet();
    let initial_color_5: Color = nondet();

    let mut tree: RedBlackTree<TestOrder> = build_tree_12!(data, initial_color_1, initial_color_5);

    tree.swap_node_with_successor::<TestOrder>(index_1, index_5);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == initial_color_5);
    cvt_assert!(tree.get_color::<TestOrder>(index_5) == initial_color_1);

    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_4);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_5) == NIL);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_4) == index_1);
    cvt_assert!(tree.get_root_index() == index_5);

    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == index_6);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_5) == index_2);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_6) == index_1);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_2) == index_5);

    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == index_7);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_5) == index_3);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_7) == index_1);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_3) == index_5);

    cvt_vacuity_check!();
}

macro_rules! build_tree_13 {
    ($data: expr, $c1: expr, $c5: expr) => {{
        let index_0 = 0 * TEST_BLOCK_WIDTH;
        let index_1 = 1 * TEST_BLOCK_WIDTH;
        let index_2 = 2 * TEST_BLOCK_WIDTH;
        let index_3 = 3 * TEST_BLOCK_WIDTH;
        let _index_4 = 4 * TEST_BLOCK_WIDTH;
        let index_5 = 5 * TEST_BLOCK_WIDTH;
        let index_6 = 6 * TEST_BLOCK_WIDTH;
        let index_7 = 7 * TEST_BLOCK_WIDTH;

        // 0
        *get_mut_helper(&mut $data, index_0) = mk_rb_node!(
            nondet_with(|i: &DataIndex| *i != index_1),
            index_1,
            nondet::<DataIndex>(),
            nondet(),
            TestOrder::new(nondet())
        );

        // 1
        *get_mut_helper(&mut $data, index_1) =
            mk_rb_node!(index_2, index_3, index_0, $c1, TestOrder::new(nondet()));

        // 2
        *get_mut_helper(&mut $data, index_2) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_1,
            nondet(),
            TestOrder::new(nondet())
        );

        // 3
        *get_mut_helper(&mut $data, index_3) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_1,
            nondet(),
            TestOrder::new(nondet())
        );

        // 5
        *get_mut_helper(&mut $data, index_5) =
            mk_rb_node!(index_6, index_7, NIL, $c5, TestOrder::new(nondet()));

        // 6
        *get_mut_helper(&mut $data, index_6) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_5,
            nondet(),
            TestOrder::new(nondet())
        );

        // 7
        *get_mut_helper(&mut $data, index_7) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_5,
            nondet(),
            TestOrder::new(nondet())
        );

        RedBlackTree::new(&mut $data, index_5, index_7)
    }};
}

/// Checks that `swap_nodes` behaves as expected in the case that the two nodes
/// are internal nodes, and the second one is the root.
/// This rule does not pass because the code currently asserts that the second
/// node is not the root.
#[rule]
pub fn rule_swap_internal_nodes_second_is_root() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let index_3 = 3 * TEST_BLOCK_WIDTH;
    let _index_4 = 4 * TEST_BLOCK_WIDTH;
    let index_5 = 5 * TEST_BLOCK_WIDTH;
    let index_6 = 6 * TEST_BLOCK_WIDTH;
    let index_7 = 7 * TEST_BLOCK_WIDTH;

    let initial_color_1: Color = nondet();
    let initial_color_5: Color = nondet();

    let mut tree: RedBlackTree<TestOrder> = build_tree_13!(data, initial_color_1, initial_color_5);

    tree.swap_node_with_successor::<TestOrder>(index_1, index_5);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == initial_color_5);
    cvt_assert!(tree.get_color::<TestOrder>(index_5) == initial_color_1);

    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == NIL);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_5) == index_0);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_0) == index_5);
    cvt_assert!(tree.get_root_index() == index_1);

    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == index_6);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_5) == index_2);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_6) == index_1);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_2) == index_5);

    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == index_7);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_5) == index_3);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_7) == index_1);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_3) == index_5);

    cvt_vacuity_check!();
}

macro_rules! build_tree_14 {
    ($data: expr, $c1: expr, $c5: expr) => {{
        let index_0: DataIndex = 0 * TEST_BLOCK_WIDTH;
        let index_1: DataIndex = 1 * TEST_BLOCK_WIDTH;
        let _index_2 = 2 * TEST_BLOCK_WIDTH;
        let index_3: DataIndex = 3 * TEST_BLOCK_WIDTH;
        let index_4: DataIndex = 4 * TEST_BLOCK_WIDTH;
        let index_5: DataIndex = 5 * TEST_BLOCK_WIDTH;
        let index_6: DataIndex = 6 * TEST_BLOCK_WIDTH;
        let index_7: DataIndex = 7 * TEST_BLOCK_WIDTH;

        // 0
        *get_mut_helper(&mut $data, index_0) = mk_rb_node!(
            nondet_with(|i: &DataIndex| *i != index_1),
            index_1,
            nondet::<DataIndex>(),
            nondet(),
            TestOrder::new(nondet())
        );

        // 1
        *get_mut_helper(&mut $data, index_1) =
            mk_rb_node!(NIL, index_3, index_0, $c1, TestOrder::new(nondet()));

        // 3
        *get_mut_helper(&mut $data, index_3) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_1,
            nondet(),
            TestOrder::new(nondet())
        );

        // 4
        *get_mut_helper(&mut $data, index_4) = mk_rb_node!(
            nondet_with(|i: &DataIndex| *i != index_5),
            index_5,
            nondet::<DataIndex>(),
            nondet(),
            TestOrder::new(nondet())
        );

        // 5
        *get_mut_helper(&mut $data, index_5) =
            mk_rb_node!(index_6, NIL, index_4, $c5, TestOrder::new(nondet()));

        // 6
        *get_mut_helper(&mut $data, index_6) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_5,
            nondet(),
            TestOrder::new(nondet())
        );

        RedBlackTree::new(&mut $data, index_0, index_7)
    }};
}

/// Checks that `swap_nodes` behaves as expected in the case that the two nodes
/// have only one child, respectively the left and the right.
#[rule]
pub fn rule_swap_nodes_with_one_child_left_right() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0: DataIndex = 0 * TEST_BLOCK_WIDTH;
    let index_1: DataIndex = 1 * TEST_BLOCK_WIDTH;
    let _index_2 = 2 * TEST_BLOCK_WIDTH;
    let index_3: DataIndex = 3 * TEST_BLOCK_WIDTH;
    let index_4: DataIndex = 4 * TEST_BLOCK_WIDTH;
    let index_5: DataIndex = 5 * TEST_BLOCK_WIDTH;
    let index_6: DataIndex = 6 * TEST_BLOCK_WIDTH;
    let _index_7: DataIndex = 7 * TEST_BLOCK_WIDTH;

    let initial_color_1: Color = nondet();
    let initial_color_5: Color = nondet();

    let mut tree: RedBlackTree<TestOrder> = build_tree_14!(data, initial_color_1, initial_color_5);

    tree.swap_node_with_successor::<TestOrder>(index_1, index_5);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == initial_color_5);
    cvt_assert!(tree.get_color::<TestOrder>(index_5) == initial_color_1);

    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_4);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_5) == index_0);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_0) == index_5);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_4) == index_1);

    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == index_6);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_5) == NIL);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_6) == index_1);

    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == NIL);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_5) == index_3);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_3) == index_5);

    cvt_vacuity_check!();
}

macro_rules! build_tree_15 {
    ($data: expr, $c1: expr, $c5: expr) => {{
        let index_0 = 0 * TEST_BLOCK_WIDTH;
        let index_1 = 1 * TEST_BLOCK_WIDTH;
        let index_2 = 2 * TEST_BLOCK_WIDTH;
        let _index_3 = 3 * TEST_BLOCK_WIDTH;
        let index_4 = 4 * TEST_BLOCK_WIDTH;
        let index_5 = 5 * TEST_BLOCK_WIDTH;
        let _index_6 = 6 * TEST_BLOCK_WIDTH;
        let index_7 = 7 * TEST_BLOCK_WIDTH;

        // 0
        *get_mut_helper(&mut $data, index_0) = mk_rb_node!(
            index_1,
            nondet_with(|i: &DataIndex| *i != index_1),
            nondet::<DataIndex>(),
            nondet(),
            TestOrder::new(nondet())
        );

        // 1
        *get_mut_helper(&mut $data, index_1) =
            mk_rb_node!(index_2, NIL, index_0, $c1, TestOrder::new(nondet()));

        // 2
        *get_mut_helper(&mut $data, index_2) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_1,
            nondet(),
            TestOrder::new(nondet())
        );

        // 4
        *get_mut_helper(&mut $data, index_4) = mk_rb_node!(
            index_5,
            nondet_with(|i: &DataIndex| *i != index_5),
            nondet::<DataIndex>(),
            nondet(),
            TestOrder::new(nondet())
        );

        // 5
        *get_mut_helper(&mut $data, index_5) =
            mk_rb_node!(NIL, index_7, index_4, $c5, TestOrder::new(nondet()));

        // 7
        *get_mut_helper(&mut $data, index_7) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_5,
            nondet(),
            TestOrder::new(nondet())
        );

        RedBlackTree::new(&mut $data, index_0, index_7)
    }};
}

/// Checks that `swap_nodes` behaves as expected in the case that the two nodes
/// have only one child, respectively the right and the left.
#[rule]
pub fn rule_swap_nodes_with_one_child_right_left() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let _index_3 = 3 * TEST_BLOCK_WIDTH;
    let index_4 = 4 * TEST_BLOCK_WIDTH;
    let index_5 = 5 * TEST_BLOCK_WIDTH;
    let _index_6 = 6 * TEST_BLOCK_WIDTH;
    let index_7 = 7 * TEST_BLOCK_WIDTH;

    let initial_color_1: Color = nondet();
    let initial_color_5: Color = nondet();

    let mut tree: RedBlackTree<TestOrder> = build_tree_15!(data, initial_color_1, initial_color_5);

    tree.swap_node_with_successor::<TestOrder>(index_1, index_5);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == initial_color_5);
    cvt_assert!(tree.get_color::<TestOrder>(index_5) == initial_color_1);

    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_4);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_5) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_0) == index_5);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_4) == index_1);

    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == NIL);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_5) == index_2);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_2) == index_5);

    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == index_7);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_5) == NIL);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_7) == index_1);

    cvt_vacuity_check!();
}

macro_rules! build_tree_16 {
    ($data: expr, $c1: expr, $c5: expr) => {{
        let index_0 = 0 * TEST_BLOCK_WIDTH;
        let index_1 = 1 * TEST_BLOCK_WIDTH;
        let _index_2 = 2 * TEST_BLOCK_WIDTH;
        let _index_3 = 3 * TEST_BLOCK_WIDTH;
        let index_4 = 4 * TEST_BLOCK_WIDTH;
        let index_5 = 5 * TEST_BLOCK_WIDTH;
        let _index_6 = 6 * TEST_BLOCK_WIDTH;
        let _index_7 = 7 * TEST_BLOCK_WIDTH;

        // 0
        *get_mut_helper(&mut $data, index_0) = mk_rb_node!(
            index_1,
            nondet_with(|i: &DataIndex| *i != index_1),
            nondet::<DataIndex>(),
            nondet(),
            TestOrder::new(nondet())
        );

        // 1
        *get_mut_helper(&mut $data, index_1) =
            mk_rb_node!(NIL, NIL, index_0, $c1, TestOrder::new(nondet()));

        // 4
        *get_mut_helper(&mut $data, index_4) = mk_rb_node!(
            index_5,
            nondet_with(|i: &DataIndex| *i != index_5),
            nondet::<DataIndex>(),
            nondet(),
            TestOrder::new(nondet())
        );

        // 5
        *get_mut_helper(&mut $data, index_5) =
            mk_rb_node!(NIL, NIL, index_4, $c5, TestOrder::new(nondet()));

        RedBlackTree::new(&mut $data, index_0, index_5)
    }};
}

/// Checks that `swap_nodes` behaves as expected in the case that the two nodes
/// are leaves.
#[rule]
pub fn rule_swap_leaves() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let _index_2 = 2 * TEST_BLOCK_WIDTH;
    let _index_3 = 3 * TEST_BLOCK_WIDTH;
    let index_4 = 4 * TEST_BLOCK_WIDTH;
    let index_5 = 5 * TEST_BLOCK_WIDTH;
    let _index_6 = 6 * TEST_BLOCK_WIDTH;
    let _index_7 = 7 * TEST_BLOCK_WIDTH;

    let initial_color_1: Color = nondet();
    let initial_color_5: Color = nondet();

    let mut tree: RedBlackTree<TestOrder> = build_tree_16!(data, initial_color_1, initial_color_5);

    tree.swap_node_with_successor::<TestOrder>(index_1, index_5);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == initial_color_5);
    cvt_assert!(tree.get_color::<TestOrder>(index_5) == initial_color_1);

    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_4);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_5) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_0) == index_5);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_4) == index_1);

    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == NIL);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_5) == NIL);

    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == NIL);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_5) == NIL);

    cvt_vacuity_check!();
}

macro_rules! build_tree_17 {
    ($data: expr, $c1: expr, $c3: expr) => {{
        let index_0 = 0 * TEST_BLOCK_WIDTH;
        let index_1 = 1 * TEST_BLOCK_WIDTH;
        let index_2 = 2 * TEST_BLOCK_WIDTH;
        let index_3 = 3 * TEST_BLOCK_WIDTH;
        let index_4 = 4 * TEST_BLOCK_WIDTH;
        let index_5 = 5 * TEST_BLOCK_WIDTH;

        // 0
        *get_mut_helper(&mut $data, index_0) = mk_rb_node!(
            index_1,
            nondet_with(|i: &DataIndex| *i != index_1 && *i != index_3),
            nondet::<DataIndex>(),
            nondet(),
            TestOrder::new(nondet())
        );

        // 1
        *get_mut_helper(&mut $data, index_1) =
            mk_rb_node!(index_2, index_3, index_0, $c1, TestOrder::new(nondet()));

        // 2
        *get_mut_helper(&mut $data, index_2) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_1,
            nondet(),
            TestOrder::new(nondet())
        );

        // 3
        *get_mut_helper(&mut $data, index_3) =
            mk_rb_node!(index_4, index_5, index_1, $c3, TestOrder::new(nondet()));

        // 4
        *get_mut_helper(&mut $data, index_4) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_3,
            nondet(),
            TestOrder::new(nondet())
        );

        // 5
        *get_mut_helper(&mut $data, index_5) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_3,
            nondet(),
            TestOrder::new(nondet())
        );

        RedBlackTree::new(&mut $data, index_0, index_5)
    }};
}

/// Checks that `swap_nodes` behaves as expected in the case that the two nodes
/// are parent and right child.
#[rule]
pub fn rule_swap_parent_right_child() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let index_3 = 3 * TEST_BLOCK_WIDTH;
    let index_4 = 4 * TEST_BLOCK_WIDTH;
    let index_5 = 5 * TEST_BLOCK_WIDTH;

    let initial_color_1: Color = nondet();
    let initial_color_3: Color = nondet();

    let mut tree: RedBlackTree<TestOrder> = build_tree_17!(data, initial_color_1, initial_color_3);

    tree.swap_node_with_successor::<TestOrder>(index_1, index_3);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == initial_color_3);
    cvt_assert!(tree.get_color::<TestOrder>(index_3) == initial_color_1);

    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_3);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_3) == index_0);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_0) == index_3);

    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == index_4);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_3) == index_2);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_4) == index_1);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_2) == index_3);

    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == index_5);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_3) == index_1);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_5) == index_1);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_3);

    cvt_vacuity_check!();
}

macro_rules! build_tree_18 {
    ($data: expr, $c1: expr, $c3: expr) => {{
        let index_0 = 0 * TEST_BLOCK_WIDTH;
        let index_1 = 1 * TEST_BLOCK_WIDTH;
        let index_2 = 2 * TEST_BLOCK_WIDTH;
        let index_3 = 3 * TEST_BLOCK_WIDTH;
        let index_4 = 4 * TEST_BLOCK_WIDTH;
        let index_5 = 5 * TEST_BLOCK_WIDTH;

        // 0
        *get_mut_helper(&mut $data, index_0) = mk_rb_node!(
            nondet_with(|i: &DataIndex| *i != index_1 && *i != index_3),
            index_1,
            nondet::<DataIndex>(),
            nondet(),
            TestOrder::new(nondet())
        );

        // 1
        *get_mut_helper(&mut $data, index_1) =
            mk_rb_node!(index_3, index_2, index_0, $c1, TestOrder::new(nondet()));

        // 2
        *get_mut_helper(&mut $data, index_2) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_1,
            nondet(),
            TestOrder::new(nondet())
        );

        // 3
        *get_mut_helper(&mut $data, index_3) =
            mk_rb_node!(index_4, index_5, index_1, $c3, TestOrder::new(nondet()));

        // 4
        *get_mut_helper(&mut $data, index_4) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_3,
            nondet(),
            TestOrder::new(nondet())
        );

        // 5
        *get_mut_helper(&mut $data, index_5) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_3,
            nondet(),
            TestOrder::new(nondet())
        );

        RedBlackTree::new(&mut $data, index_0, index_2)
    }};
}

/// Checks that `swap_nodes` behaves as expected in the case that the two nodes
/// are parent and left child.
/// This rule is currently violated due to the non-properly handled edge case.
#[rule]
pub fn rule_swap_parent_left_child() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let index_3 = 3 * TEST_BLOCK_WIDTH;
    let index_4 = 4 * TEST_BLOCK_WIDTH;
    let index_5 = 5 * TEST_BLOCK_WIDTH;

    let initial_color_1: Color = nondet();
    let initial_color_3: Color = nondet();

    let mut tree: RedBlackTree<TestOrder> = build_tree_18!(data, initial_color_1, initial_color_3);

    tree.swap_node_with_successor::<TestOrder>(index_1, index_3);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == initial_color_3);
    cvt_assert!(tree.get_color::<TestOrder>(index_3) == initial_color_1);

    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_3);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_3) == index_0);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_0) == index_3);

    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == index_4);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_3) == index_1);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_4) == index_1);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_3);

    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == index_5);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_3) == index_2);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_5) == index_1);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_2) == index_3);

    cvt_vacuity_check!();
}

macro_rules! build_tree_19 {
    ($data: expr, $c1: expr, $c3: expr) => {{
        let index_0 = 0 * TEST_BLOCK_WIDTH;
        let index_1 = 1 * TEST_BLOCK_WIDTH;
        let index_2 = 2 * TEST_BLOCK_WIDTH;
        let index_3 = 3 * TEST_BLOCK_WIDTH;
        let index_4 = 4 * TEST_BLOCK_WIDTH;
        let index_5 = 5 * TEST_BLOCK_WIDTH;

        // 0
        *get_mut_helper(&mut $data, index_0) = mk_rb_node!(
            index_3,
            nondet_with(|i: &DataIndex| *i != index_1 && *i != index_3),
            nondet::<DataIndex>(),
            nondet(),
            TestOrder::new(nondet())
        );

        // 1
        *get_mut_helper(&mut $data, index_1) =
            mk_rb_node!(index_4, index_5, index_3, $c1, TestOrder::new(nondet()));

        // 2
        *get_mut_helper(&mut $data, index_2) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_3,
            nondet(),
            TestOrder::new(nondet())
        );

        // 3
        *get_mut_helper(&mut $data, index_3) =
            mk_rb_node!(index_2, index_1, index_0, $c3, TestOrder::new(nondet()));

        // 4
        *get_mut_helper(&mut $data, index_4) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_1,
            nondet(),
            TestOrder::new(nondet())
        );

        // 5
        *get_mut_helper(&mut $data, index_5) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_1,
            nondet(),
            TestOrder::new(nondet())
        );

        RedBlackTree::new(&mut $data, index_0, index_5)
    }};
}

/// Checks that `swap_nodes` behaves as expected in the case that the two nodes
/// are right child and parent.
/// This rule is currently violated due to the non-properly handled edge case.
#[rule]
pub fn rule_swap_right_child_parent() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let index_3 = 3 * TEST_BLOCK_WIDTH;
    let index_4 = 4 * TEST_BLOCK_WIDTH;
    let index_5 = 5 * TEST_BLOCK_WIDTH;

    let initial_color_1: Color = nondet();
    let initial_color_3: Color = nondet();

    let mut tree: RedBlackTree<TestOrder> = build_tree_19!(data, initial_color_1, initial_color_3);

    tree.swap_node_with_successor::<TestOrder>(index_1, index_3);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == initial_color_3);
    cvt_assert!(tree.get_color::<TestOrder>(index_3) == initial_color_1);

    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_0);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_3) == index_1);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_0) == index_1);

    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == index_2);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_3) == index_4);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_4) == index_3);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_2) == index_1);

    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == index_3);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_3) == index_5);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_5) == index_3);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_0);

    cvt_vacuity_check!();
}

macro_rules! build_tree_20 {
    ($data: expr, $c1: expr, $c3: expr) => {{
        let index_0 = 0 * TEST_BLOCK_WIDTH;
        let index_1 = 1 * TEST_BLOCK_WIDTH;
        let index_2 = 2 * TEST_BLOCK_WIDTH;
        let index_3 = 3 * TEST_BLOCK_WIDTH;
        let index_4 = 4 * TEST_BLOCK_WIDTH;
        let index_5 = 5 * TEST_BLOCK_WIDTH;

        // 0
        *get_mut_helper(&mut $data, index_0) = mk_rb_node!(
            nondet_with(|i: &DataIndex| *i != index_1 && *i != index_3),
            index_3,
            nondet::<DataIndex>(),
            nondet(),
            TestOrder::new(nondet())
        );

        // 1
        *get_mut_helper(&mut $data, index_1) =
            mk_rb_node!(index_4, index_5, index_3, $c1, TestOrder::new(nondet()));

        // 2
        *get_mut_helper(&mut $data, index_2) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_3,
            nondet(),
            TestOrder::new(nondet())
        );

        // 3
        *get_mut_helper(&mut $data, index_3) =
            mk_rb_node!(index_1, index_2, index_0, $c3, TestOrder::new(nondet()));

        // 4
        *get_mut_helper(&mut $data, index_4) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_1,
            nondet(),
            TestOrder::new(nondet())
        );

        // 5
        *get_mut_helper(&mut $data, index_5) = mk_rb_node!(
            nondet::<DataIndex>(),
            nondet::<DataIndex>(),
            index_1,
            nondet(),
            TestOrder::new(nondet())
        );

        RedBlackTree::new(&mut $data, index_0, index_2)
    }};
}

/// Checks that `swap_nodes` behaves as expected in the case that the two nodes
/// are left child and parent.
/// This rule is currently violated due to the non-properly handled edge case.
#[rule]
pub fn rule_swap_left_child_parent() {
    init_static();

    let acc_infos: [AccountInfo; 16] = acc_infos_with_mem_layout!();
    let acc_info = &acc_infos[0];
    let mut data = acc_info.data.borrow_mut();

    let index_0 = 0 * TEST_BLOCK_WIDTH;
    let index_1 = 1 * TEST_BLOCK_WIDTH;
    let index_2 = 2 * TEST_BLOCK_WIDTH;
    let index_3 = 3 * TEST_BLOCK_WIDTH;
    let index_4 = 4 * TEST_BLOCK_WIDTH;
    let index_5 = 5 * TEST_BLOCK_WIDTH;

    let initial_color_1: Color = nondet();
    let initial_color_3: Color = nondet();

    let mut tree: RedBlackTree<TestOrder> = build_tree_20!(data, initial_color_1, initial_color_3);

    tree.swap_node_with_successor::<TestOrder>(index_1, index_3);

    cvt_assert!(tree.get_color::<TestOrder>(index_1) == initial_color_3);
    cvt_assert!(tree.get_color::<TestOrder>(index_3) == initial_color_1);

    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_0);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_3) == index_1);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_0) == index_1);

    cvt_assert!(tree.get_left_index::<TestOrder>(index_1) == index_3);
    cvt_assert!(tree.get_left_index::<TestOrder>(index_3) == index_4);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_4) == index_3);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_2) == index_1);

    cvt_assert!(tree.get_right_index::<TestOrder>(index_1) == index_2);
    cvt_assert!(tree.get_right_index::<TestOrder>(index_3) == index_5);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_5) == index_3);
    cvt_assert!(tree.get_parent_index::<TestOrder>(index_1) == index_0);

    cvt_vacuity_check!();
}
