use std::collections::HashSet;

use crate::{
    get_helper, get_mut_helper, trace, Color, DataIndex, HyperTreeData, HyperTreeReadOperations,
    HyperTreeWriteOperations, Payload, RBNode, RedBlackTreeReadOnly, NIL,
};

// Overview of the structs and traits in this file.
//
// Public
//  struct LinkedList<'a, V: Payload>
//    fn new(data: &'a mut [u8], head_index: DataIndex) -> Self
//    fn get_mut_value(&mut self, index: DataIndex) -> &mut V
//    fn get_next_index(&self, index: DataIndex) -> DataIndex
//    fn get_prev_index(&self, index: DataIndex) -> DataIndex
//    HyperTreeReadOperations
//    HyperTreeWriteOperations
//    HyperTreeData (and through it HyperTreeValueIteratorTrait)
//  struct LinkedListReadOnly<'a, V: Payload>
//    fn new(data: &'a [u8], head_index: DataIndex) -> Self
//    fn get_next_index(&self, index: DataIndex) -> DataIndex
//    fn get_prev_index(&self, index: DataIndex) -> DataIndex
//    HyperTreeReadOperations
//    HyperTreeData (and through it HyperTreeValueIteratorTrait)
//  fn convert_red_black_tree_to_linked_list<V>(data, root_index) -> DataIndex
//  fn validate_linked_list<V>(data, head_index) -> Result<usize, &str>

/// An unordered doubly linked list that fits the hypertree abstraction, for
/// collections that are always walked in full and small enough that a tree is
/// pure overhead: a trader's open orders on one market in the wrapper, for
/// example.
///
/// Every operation is O(1) except `lookup_index`, which is a linear scan, and
/// the costs on chain are a few dozen CU each: an insert writes the new node
/// and one link on the old head, a remove rewrites two links, and a step of
/// the iterator reads one link. Compare the red-black tree at hundreds of CU
/// per insert or remove and about a hundred per step of a walk.
///
/// The nodes are the same 16 byte header plus payload as [`RBNode`] so that a
/// red-black tree can be turned into a list in place (see
/// [`convert_red_black_tree_to_linked_list`]) and the two can share one pool of
/// free blocks.
///
/// In the header `right` is the next node, `parent` is the previous one,
/// `left` is always [`NIL`] and `color` is always black; `payload_type` is
/// left to the application as for the tree.
///
/// Storing the previous node in `parent` rather than in `left` is what makes
/// these bytes readable as a tree: a list laid out this way is exactly a
/// right-leaning spine, every node the right child of the one before it, with
/// consistent parent links and no left children.
///
/// That buys compatibility with one specific reader, and the claim is worth
/// stating narrowly. The deployed TypeScript client parser walks from the
/// root and checks only that each child's parent link points back at it, so
/// it accepts a spine and, since the in-order traversal of a right spine is
/// the spine itself, hands back the same nodes in the same order the list
/// walk does. That is what matters here: a structure converted in place under
/// a released client has to stay legible to clients that have not been
/// updated. Putting the previous node in `left` would instead present that
/// parser with a node whose parent link does not match the node that pointed
/// at it, and it would reject the account.
///
/// It is not compatibility with red-black tree readers in general.
/// [`validate_red_black_tree`](crate::validate_red_black_tree) checks the full
/// invariants, and a list of more than one node fails them: the key ordering
/// is not maintained and the black heights of the NIL leaves hanging off the
/// spine differ. Nor is a tree walk interchangeable with a list walk in this
/// crate: the tree's iterator is keyed and ordered, while these nodes are
/// unordered and only the head-to-tail sequence is meaningful, and
/// `lookup_index` on a list is a linear scan. Use [`validate_linked_list`] and
/// the list's own iterator for these bytes.
///
/// Insert puts the new node at the head. The `HyperTreeReadOperations` names
/// are kept for the abstraction: "max" is the head, "next lower" is the next
/// node and "next higher" is the previous one.
pub struct LinkedList<'a, V: Payload> {
    /// The address within data of the first node.
    head_index: DataIndex,
    /// Unowned byte array which contains all the data for this list and possibly more.
    data: &'a mut [u8],

    phantom: std::marker::PhantomData<&'a V>,
}

/// A [`LinkedList`] over data that does not need to be mutable.
pub struct LinkedListReadOnly<'a, V: Payload> {
    /// The address within data of the first node.
    head_index: DataIndex,
    /// Unowned byte array which contains all the data for this list and possibly more.
    data: &'a [u8],

    phantom: std::marker::PhantomData<&'a V>,
}

impl<'a, V: Payload> LinkedList<'a, V> {
    /// Creates a list over existing, well formed list data. `head_index` is
    /// [`NIL`] for an empty list.
    pub fn new(data: &'a mut [u8], head_index: DataIndex) -> Self {
        LinkedList::<V> {
            head_index,
            data,
            phantom: std::marker::PhantomData,
        }
    }

    /// Mutable access to the payload of a node, for updating an entry in place
    /// while walking with `get_next_index`.
    pub fn get_mut_value(&mut self, index: DataIndex) -> &mut V {
        get_mut_helper::<RBNode<V>>(self.data, index).get_mut_value()
    }

    /// The next node, NIL at the end. Same as `get_next_lower_index` but
    /// without borrowing the list for its whole lifetime, so a walk can
    /// update and remove nodes as it goes.
    pub fn get_next_index(&self, index: DataIndex) -> DataIndex {
        next_index::<V>(self.data, index)
    }

    /// The previous node, NIL at the head.
    pub fn get_prev_index(&self, index: DataIndex) -> DataIndex {
        prev_index::<V>(self.data, index)
    }
}

impl<'a, V: Payload> LinkedListReadOnly<'a, V> {
    /// Creates a read only list over existing, well formed list data.
    /// `head_index` is [`NIL`] for an empty list.
    pub fn new(data: &'a [u8], head_index: DataIndex) -> Self {
        LinkedListReadOnly::<V> {
            head_index,
            data,
            phantom: std::marker::PhantomData,
        }
    }

    /// The next node, NIL at the end.
    pub fn get_next_index(&self, index: DataIndex) -> DataIndex {
        next_index::<V>(self.data, index)
    }

    /// The previous node, NIL at the head.
    pub fn get_prev_index(&self, index: DataIndex) -> DataIndex {
        prev_index::<V>(self.data, index)
    }
}

impl<'a, V: Payload> HyperTreeData<'a> for LinkedList<'a, V> {
    fn hypertree_data(&self) -> &[u8] {
        self.data
    }
}

impl<'a, V: Payload> HyperTreeData<'a> for LinkedListReadOnly<'a, V> {
    fn hypertree_data(&self) -> &[u8] {
        self.data
    }
}

fn next_index<V: Payload>(data: &[u8], index: DataIndex) -> DataIndex {
    if index == NIL {
        return NIL;
    }
    get_helper::<RBNode<V>>(data, index).right
}

fn prev_index<V: Payload>(data: &[u8], index: DataIndex) -> DataIndex {
    if index == NIL {
        return NIL;
    }
    get_helper::<RBNode<V>>(data, index).parent
}

fn lookup_index<V: Payload>(data: &[u8], head_index: DataIndex, value: &V) -> DataIndex {
    let mut index: DataIndex = head_index;
    while index != NIL {
        let node: &RBNode<V> = get_helper::<RBNode<V>>(data, index);
        if node.get_value() == value {
            return index;
        }
        index = node.right;
    }
    NIL
}

// The two list types get their own impls rather than one blanket impl over a
// shared data trait because a second blanket impl of HyperTreeReadOperations
// would overlap with the red-black tree's.
macro_rules! impl_linked_list_read_operations {
    ($list:ident) => {
        impl<'a, V: Payload> HyperTreeReadOperations<'a> for $list<'a, V> {
            /// Linear scan for a node equal to the value.
            fn lookup_index<W: Payload>(&'a self, value: &W) -> DataIndex {
                lookup_index::<W>(self.data, self.head_index, value)
            }

            /// The head, which is where iteration starts.
            fn lookup_max_index<W: Payload>(&'a self) -> DataIndex {
                self.head_index
            }

            /// The head, which is where iteration starts.
            fn get_max_index(&self) -> DataIndex {
                self.head_index
            }

            /// The head. This is what must be persisted to reopen the list.
            fn get_root_index(&self) -> DataIndex {
                self.head_index
            }

            /// The next node, NIL at the end of the list.
            fn get_next_lower_index<W: Payload>(&'a self, index: DataIndex) -> DataIndex {
                next_index::<W>(self.data, index)
            }

            /// The previous node, NIL at the head.
            fn get_next_higher_index<W: Payload>(&'a self, index: DataIndex) -> DataIndex {
                prev_index::<W>(self.data, index)
            }
        }
    };
}
impl_linked_list_read_operations!(LinkedList);
impl_linked_list_read_operations!(LinkedListReadOnly);

impl<'a, V: Payload> HyperTreeWriteOperations<'a, V> for LinkedList<'a, V> {
    /// Insert at the head. The block at index must be free.
    fn insert(&mut self, index: DataIndex, value: V) {
        trace!("LIST insert {index}");
        debug_assert_ne!(index, NIL);
        let old_head_index: DataIndex = self.head_index;
        let node: &mut RBNode<V> = get_mut_helper::<RBNode<V>>(self.data, index);
        *node = RBNode {
            left: NIL,
            right: old_head_index,
            parent: NIL,
            color: Color::Black,
            payload_type: 0,
            _unused_padding: 0,
            value,
        };
        if old_head_index != NIL {
            get_mut_helper::<RBNode<V>>(self.data, old_head_index).parent = index;
        }
        self.head_index = index;
    }

    /// Unlink the node at index, which must be in this list. The block is
    /// left for the caller to free; its links are cleared.
    fn remove_by_index(&mut self, index: DataIndex) {
        trace!("LIST remove {index}");

        // Silently fail on removing NIL nodes, like the tree.
        if index == NIL {
            return;
        }
        let (prev_index, next_index): (DataIndex, DataIndex) = {
            let node: &mut RBNode<V> = get_mut_helper::<RBNode<V>>(self.data, index);
            let links: (DataIndex, DataIndex) = (node.parent, node.right);
            node.parent = NIL;
            node.right = NIL;
            links
        };
        if prev_index == NIL {
            debug_assert_eq!(
                self.head_index, index,
                "removing a node that is not in the list"
            );
            self.head_index = next_index;
        } else {
            get_mut_helper::<RBNode<V>>(self.data, prev_index).right = next_index;
        }
        if next_index != NIL {
            get_mut_helper::<RBNode<V>>(self.data, next_index).parent = prev_index;
        }
    }
}

/// Relinks the nodes of a red-black tree rooted at `root_index` into a
/// [`LinkedList`] in place and returns the head. The nodes keep their
/// addresses and payloads; only the 16 byte headers change. The list is in the
/// tree's iteration order, so the head is the tree's max.
///
/// The tree must be well formed, which is the case for the accounts this is
/// meant for: a program converting a structure it owns and has maintained
/// itself. It is not defensive, and it rewrites as it goes, so feeding it
/// malformed links can loop, panic, or stop partway and leave the account
/// neither a tree nor a list. A caller holding bytes it did not produce
/// should run [`crate::validate_red_black_tree`] first.
///
/// Allocates nothing, because the program heap is a bump allocator that never
/// frees: a scratch buffer proportional to the number of nodes would put a
/// ceiling on the tree this can convert, and reaching it would strand the
/// account it was meant to migrate.
///
/// It gets away with that by clobbering the header fields in the order the
/// tree walk stops needing them. The first pass walks from the max downwards
/// and writes each node's successor into `right`, which the walk only ever
/// reads for nodes below the one it is on, and those are all still untouched.
/// It leaves `left` and `parent` alone, which is what the walk uses to climb.
/// The second pass follows the chain that pass one just built and rewrites
/// `left`, `parent` and `color`, needing no tree structure at all.
pub fn convert_red_black_tree_to_linked_list<V: Payload>(
    data: &mut [u8],
    root_index: DataIndex,
) -> DataIndex {
    if root_index == NIL {
        return NIL;
    }
    // The head is the tree's max, read before anything is overwritten.
    let head_index: DataIndex = {
        let tree: RedBlackTreeReadOnly<V> = RedBlackTreeReadOnly::<V>::new(data, root_index, NIL);
        tree.lookup_max_index::<V>()
    };

    // Pass one: successor into `right`, walking max downwards.
    let mut index: DataIndex = head_index;
    while index != NIL {
        let next_index: DataIndex = {
            let tree: RedBlackTreeReadOnly<V> =
                RedBlackTreeReadOnly::<V>::new(data, root_index, NIL);
            tree.get_next_lower_index::<V>(index)
        };
        get_mut_helper::<RBNode<V>>(data, index).right = next_index;
        index = next_index;
    }

    // Pass two: the chain is enough to finish the headers.
    let mut prev_index: DataIndex = NIL;
    let mut index: DataIndex = head_index;
    while index != NIL {
        let node: &mut RBNode<V> = get_mut_helper::<RBNode<V>>(data, index);
        let next_index: DataIndex = node.right;
        node.left = NIL;
        node.parent = prev_index;
        node.color = Color::Black;
        prev_index = index;
        index = next_index;
    }
    head_index
}

/// Validates all offsets and links reachable from an account-backed list
/// before callers use the zero-copy traversal API on untrusted bytes. Returns
/// the number of nodes.
///
/// `Ok` means the list can then be walked with [`LinkedListReadOnly`], which
/// is the whole point of calling this, so it checks everything that walk
/// needs. That includes the alignment of each node's address, `data` included
/// and not just the offset within it: the traversal reads through
/// [`get_helper`], which requires it, and a buffer straight out of an RPC
/// response carries no such guarantee. Its own reads are unaligned so that it
/// answers rather than panicking on the buffers it is there to reject.
pub fn validate_linked_list<V: Payload>(
    data: &[u8],
    head_index: DataIndex,
) -> Result<usize, &'static str> {
    let node_size: usize = std::mem::size_of::<RBNode<V>>();
    let mut visited: HashSet<DataIndex> = HashSet::new();
    // Starts in visit order, to check afterwards that no two nodes share a
    // byte. Tracking starts alone would accept a chain whose nodes overlap,
    // where writing one silently rewrites the links of another.
    let mut starts: Vec<usize> = Vec::new();
    let mut prev_index: DataIndex = NIL;
    let mut index: DataIndex = head_index;
    while index != NIL {
        let start: usize = index as usize;
        // The address the reader will use, not the offset on its own.
        if (data.as_ptr() as usize).wrapping_add(start) % std::mem::align_of::<RBNode<V>>() != 0 {
            return Err("linked list node is misaligned");
        }
        let end: usize = match start.checked_add(node_size) {
            Some(end) if end <= data.len() => end,
            _ => return Err("linked list node is out of bounds"),
        };
        if !visited.insert(index) {
            return Err("linked list has a cycle");
        }
        starts.push(start);
        let node: RBNode<V> = crate::read_unaligned::<RBNode<V>>(&data[start..end]);
        if node.parent != prev_index {
            return Err("linked list previous link is inconsistent");
        }
        // A list is a right-leaning spine, so no node has a left child. This
        // is what keeps it readable by a tree parser, see the type docs.
        if node.left != NIL {
            return Err("linked list node has a left child");
        }
        prev_index = index;
        index = node.right;
    }
    starts.sort_unstable();
    if starts
        .windows(2)
        .any(|pair: &[usize]| pair[1] - pair[0] < node_size)
    {
        return Err("linked list nodes overlap");
    }
    Ok(visited.len())
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{
        red_black_tree::test::{TestOrderBid, TEST_BLOCK_WIDTH},
        HyperTreeValueIteratorTrait, RedBlackTree,
    };

    fn values(data: &[u8], head_index: DataIndex) -> Vec<u64> {
        LinkedListReadOnly::<TestOrderBid>::new(data, head_index)
            .iter::<TestOrderBid>()
            .map(|(_, order)| order.order_id())
            .collect()
    }

    fn assert_well_formed(data: &[u8], head_index: DataIndex, expected_len: usize) {
        assert_eq!(
            validate_linked_list::<TestOrderBid>(data, head_index),
            Ok(expected_len)
        );
    }

    #[test]
    fn test_insert_iter_and_lookup() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut list: LinkedList<TestOrderBid> = LinkedList::new(&mut data, NIL);
        assert_eq!(list.get_root_index(), NIL);
        assert_eq!(list.get_max_index(), NIL);
        assert_eq!(list.lookup_max_index::<TestOrderBid>(), NIL);
        assert_eq!(list.iter::<TestOrderBid>().count(), 0);
        assert_eq!(list.lookup_index(&TestOrderBid::new(1)), NIL);

        for i in 1..=8 {
            list.insert(TEST_BLOCK_WIDTH * i, TestOrderBid::new((i * 100).into()));
            assert_eq!(list.get_root_index(), TEST_BLOCK_WIDTH * i);
            assert_eq!(list.get_max_index(), TEST_BLOCK_WIDTH * i);
        }
        let head: DataIndex = list.get_root_index();

        // Newest first.
        assert_eq!(
            values(&data, head),
            vec![800, 700, 600, 500, 400, 300, 200, 100]
        );
        assert_well_formed(&data, head, 8);

        let list: LinkedListReadOnly<TestOrderBid> = LinkedListReadOnly::new(&data, head);
        for i in 1..=8 {
            assert_eq!(
                list.lookup_index(&TestOrderBid::new((i * 100).into())),
                TEST_BLOCK_WIDTH * i
            );
        }
        assert_eq!(list.lookup_index(&TestOrderBid::new(150)), NIL);
        assert_eq!(list.lookup_max_index::<TestOrderBid>(), head);

        // Walking both ways.
        assert_eq!(
            list.get_next_lower_index::<TestOrderBid>(TEST_BLOCK_WIDTH * 8),
            TEST_BLOCK_WIDTH * 7
        );
        assert_eq!(
            list.get_next_lower_index::<TestOrderBid>(TEST_BLOCK_WIDTH),
            NIL
        );
        assert_eq!(list.get_next_lower_index::<TestOrderBid>(NIL), NIL);
        assert_eq!(
            list.get_next_higher_index::<TestOrderBid>(TEST_BLOCK_WIDTH),
            TEST_BLOCK_WIDTH * 2
        );
        assert_eq!(
            list.get_next_higher_index::<TestOrderBid>(TEST_BLOCK_WIDTH * 8),
            NIL
        );
        assert_eq!(list.get_next_higher_index::<TestOrderBid>(NIL), NIL);
    }

    #[test]
    fn test_remove() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut list: LinkedList<TestOrderBid> = LinkedList::new(&mut data, NIL);
        for i in 1..=5 {
            list.insert(TEST_BLOCK_WIDTH * i, TestOrderBid::new(i.into()));
        }
        // 5 4 3 2 1

        // Removing NIL is a no-op.
        list.remove_by_index(NIL);
        assert_eq!(
            values(list.data, list.get_root_index()),
            vec![5, 4, 3, 2, 1]
        );

        // Middle.
        list.remove_by_index(TEST_BLOCK_WIDTH * 3);
        assert_eq!(values(list.data, list.get_root_index()), vec![5, 4, 2, 1]);
        assert_well_formed(list.data, list.get_root_index(), 4);
        {
            let removed: &RBNode<TestOrderBid> =
                get_helper::<RBNode<TestOrderBid>>(list.data, TEST_BLOCK_WIDTH * 3);
            assert_eq!(removed.parent, NIL);
            assert_eq!(removed.right, NIL);
        }

        // Head.
        list.remove_by_index(TEST_BLOCK_WIDTH * 5);
        assert_eq!(list.get_root_index(), TEST_BLOCK_WIDTH * 4);
        assert_eq!(values(list.data, list.get_root_index()), vec![4, 2, 1]);
        assert_well_formed(list.data, list.get_root_index(), 3);

        // Tail.
        list.remove_by_index(TEST_BLOCK_WIDTH);
        assert_eq!(values(list.data, list.get_root_index()), vec![4, 2]);
        assert_well_formed(list.data, list.get_root_index(), 2);

        // Insert into the gap reuses the block.
        list.insert(TEST_BLOCK_WIDTH * 3, TestOrderBid::new(33));
        assert_eq!(values(list.data, list.get_root_index()), vec![33, 4, 2]);
        assert_well_formed(list.data, list.get_root_index(), 3);

        // Down to empty.
        list.remove_by_index(TEST_BLOCK_WIDTH * 4);
        list.remove_by_index(TEST_BLOCK_WIDTH * 3);
        list.remove_by_index(TEST_BLOCK_WIDTH * 2);
        assert_eq!(list.get_root_index(), NIL);
        assert_eq!(values(list.data, NIL), Vec::<u64>::new());
        assert_well_formed(list.data, NIL, 0);

        // And back.
        list.insert(TEST_BLOCK_WIDTH * 7, TestOrderBid::new(7));
        assert_eq!(values(list.data, list.get_root_index()), vec![7]);
    }

    #[test]
    fn test_walk_while_mutating() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut list: LinkedList<TestOrderBid> = LinkedList::new(&mut data, NIL);
        for i in 1..=6 {
            list.insert(TEST_BLOCK_WIDTH * i, TestOrderBid::new(i.into()));
        }
        // Drop the even ones and double the odd ones in one pass.
        let mut index: DataIndex = list.get_root_index();
        while index != NIL {
            let next_index: DataIndex = list.get_next_index(index);
            let order_id: u64 = list.get_mut_value(index).order_id();
            if order_id % 2 == 0 {
                list.remove_by_index(index);
            } else {
                *list.get_mut_value(index) = TestOrderBid::new(order_id * 2);
            }
            index = next_index;
        }
        assert_eq!(values(list.data, list.get_root_index()), vec![10, 6, 2]);
        assert_well_formed(list.data, list.get_root_index(), 3);
        assert_eq!(
            list.get_prev_index(list.get_next_index(list.get_root_index())),
            list.get_root_index()
        );
        assert_eq!(list.get_prev_index(list.get_root_index()), NIL);
        assert_eq!(list.get_next_index(NIL), NIL);
        assert_eq!(list.get_prev_index(NIL), NIL);
    }

    #[test]
    fn test_get_mut_value() {
        let mut data: [u8; 100000] = [0; 100000];
        let mut list: LinkedList<TestOrderBid> = LinkedList::new(&mut data, NIL);
        list.insert(TEST_BLOCK_WIDTH, TestOrderBid::new(1));
        list.insert(TEST_BLOCK_WIDTH * 2, TestOrderBid::new(2));
        *list.get_mut_value(TEST_BLOCK_WIDTH) = TestOrderBid::new(10);
        assert_eq!(values(list.data, list.get_root_index()), vec![2, 10]);
        assert_eq!(list.lookup_index(&TestOrderBid::new(10)), TEST_BLOCK_WIDTH);
        assert_eq!(list.lookup_index(&TestOrderBid::new(1)), NIL);
    }

    fn check_conversion(order_ids: &[u64]) {
        let mut data: [u8; 100000] = [0; 100000];
        let root_index: DataIndex = {
            let mut tree: RedBlackTree<TestOrderBid> = RedBlackTree::new(&mut data, NIL, NIL);
            for (i, order_id) in order_ids.iter().enumerate() {
                tree.insert(
                    TEST_BLOCK_WIDTH * (i as DataIndex + 1),
                    TestOrderBid::new(*order_id),
                );
            }
            tree.get_root_index()
        };
        let mut expected: Vec<u64> = order_ids.to_vec();
        expected.sort_unstable_by(|a, b| b.cmp(a));

        let head_index: DataIndex =
            convert_red_black_tree_to_linked_list::<TestOrderBid>(&mut data, root_index);

        // Same nodes, tree iteration order, all headers rewritten.
        assert_eq!(values(&data, head_index), expected);
        assert_well_formed(&data, head_index, order_ids.len());
        for i in 1..=order_ids.len() {
            let node: &RBNode<TestOrderBid> =
                get_helper::<RBNode<TestOrderBid>>(&data, TEST_BLOCK_WIDTH * i as DataIndex);
            assert_eq!(node.left, NIL, "a list node has no left child");
            assert_eq!(node.color, Color::Black);
            assert_eq!(node.payload_type, 0);
        }
        if let Some(max) = expected.first() {
            let list: LinkedListReadOnly<TestOrderBid> = LinkedListReadOnly::new(&data, head_index);
            assert_eq!(list.lookup_index(&TestOrderBid::new(*max)), head_index);
        }

        // The list keeps working afterwards.
        let mut list: LinkedList<TestOrderBid> = LinkedList::new(&mut data, head_index);
        if let Some(last) = expected.last() {
            let index: DataIndex = list.lookup_index(&TestOrderBid::new(*last));
            list.remove_by_index(index);
            expected.pop();
        }
        let new_index: DataIndex = TEST_BLOCK_WIDTH * (order_ids.len() as DataIndex + 2);
        list.insert(new_index, TestOrderBid::new(u64::MAX));
        expected.insert(0, u64::MAX);
        let head_index: DataIndex = list.get_root_index();
        assert_eq!(values(&data, head_index), expected);
        assert_well_formed(&data, head_index, expected.len());
    }

    #[test]
    fn test_convert_from_tree() {
        check_conversion(&[]);
        check_conversion(&[1]);
        check_conversion(&[1, 2]);
        check_conversion(&[2, 1]);
        // Ascending and descending inserts give the most rotations.
        check_conversion(&(1..=11).map(|i| i * 1_000).collect::<Vec<u64>>());
        check_conversion(&(1..=11).rev().map(|i| i * 1_000).collect::<Vec<u64>>());
        check_conversion(&[50, 20, 80, 10, 30, 70, 90, 25, 35, 65, 75, 5, 15, 95, 85]);
        check_conversion(&(1..=100).map(|i| (i * 7919) % 1_000).collect::<Vec<u64>>());
    }

    /// Random inserts and removes against a Vec model, checking the walk and
    /// the links after every step.
    #[test]
    fn test_random_operations_match_model() {
        const SLOTS: DataIndex = 64;
        let mut data: Vec<u8> = vec![0; (SLOTS as usize + 1) * TEST_BLOCK_WIDTH as usize];
        let mut head_index: DataIndex = NIL;
        // (slot, order id), newest first.
        let mut model: Vec<(DataIndex, u64)> = Vec::new();
        let mut free_slots: Vec<DataIndex> = (1..=SLOTS).collect();
        let mut rng: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next_random = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        let mut next_order_id: u64 = 1;

        for _ in 0..5_000 {
            let r: u64 = next_random();
            let do_insert: bool = model.is_empty() || (!free_slots.is_empty() && r % 3 != 0);
            let mut list: LinkedList<TestOrderBid> = LinkedList::new(&mut data, head_index);
            if do_insert {
                let slot: DataIndex = free_slots.swap_remove((r >> 8) as usize % free_slots.len());
                list.insert(slot * TEST_BLOCK_WIDTH, TestOrderBid::new(next_order_id));
                model.insert(0, (slot, next_order_id));
                next_order_id += 1;
            } else {
                let position: usize = (r >> 8) as usize % model.len();
                let (slot, order_id) = model.remove(position);
                assert_eq!(
                    list.lookup_index(&TestOrderBid::new(order_id)),
                    slot * TEST_BLOCK_WIDTH
                );
                list.remove_by_index(slot * TEST_BLOCK_WIDTH);
                free_slots.push(slot);
            }
            head_index = list.get_root_index();

            let expected: Vec<u64> = model.iter().map(|(_, id)| *id).collect();
            assert_eq!(values(&data, head_index), expected);
            assert_well_formed(&data, head_index, model.len());
            assert_eq!(
                head_index,
                model
                    .first()
                    .map_or(NIL, |(slot, _)| slot * TEST_BLOCK_WIDTH)
            );
        }
    }

    /// The conversion is used to migrate live accounts, so it must not depend
    /// on scratch space: the program heap is a bump allocator that never
    /// frees, and a buffer proportional to the node count would put a ceiling
    /// on the trees it can migrate. This converts more nodes than such a
    /// buffer would have comfortably held.
    #[test]
    fn test_convert_large_tree() {
        const NODES: DataIndex = 400;
        let mut data: Vec<u8> = vec![0; (NODES as usize + 1) * TEST_BLOCK_WIDTH as usize];
        let root_index: DataIndex = {
            let mut tree: RedBlackTree<TestOrderBid> = RedBlackTree::new(&mut data, NIL, NIL);
            // Mixed insertion order so the tree is neither degenerate nor
            // perfectly balanced.
            for i in 0..NODES {
                let order_id: u64 = ((i as u64).wrapping_mul(7919)) % 10_000;
                tree.insert((i + 1) * TEST_BLOCK_WIDTH, TestOrderBid::new(order_id));
            }
            tree.get_root_index()
        };
        let mut expected: Vec<u64> = (0..NODES)
            .map(|i| ((i as u64).wrapping_mul(7919)) % 10_000)
            .collect();
        expected.sort_unstable_by(|a: &u64, b: &u64| b.cmp(a));

        let head_index: DataIndex =
            convert_red_black_tree_to_linked_list::<TestOrderBid>(&mut data, root_index);

        assert_eq!(values(&data, head_index), expected);
        assert_well_formed(&data, head_index, NODES as usize);
    }

    /// Two nodes whose headers are both intact and consistently linked, but
    /// whose byte ranges overlap: the second sits inside the first's payload,
    /// so writing either one silently rewrites the other.
    #[test]
    fn test_validate_rejects_overlapping_nodes() {
        let node_size: usize = std::mem::size_of::<RBNode<TestOrderBid>>();
        let mut data: Vec<u8> = vec![0; node_size * 2];
        // Node at 0, pointing at a node 16 bytes into its own payload:
        // left NIL, right 16, parent NIL.
        data[0..4].copy_from_slice(&NIL.to_le_bytes());
        data[4..8].copy_from_slice(&16_u32.to_le_bytes());
        data[8..12].copy_from_slice(&NIL.to_le_bytes());
        // Node at 16, linking back to it: left NIL, right NIL, parent 0.
        data[16..20].copy_from_slice(&NIL.to_le_bytes());
        data[20..24].copy_from_slice(&NIL.to_le_bytes());
        data[24..28].copy_from_slice(&0_u32.to_le_bytes());

        assert_eq!(
            validate_linked_list::<TestOrderBid>(&data, 0),
            Err("linked list nodes overlap"),
        );
    }

    /// A buffer from an RPC response has no alignment guarantee, and the
    /// traversal that follows validation cannot read it. The validator has to
    /// answer rather than panic, and the answer has to be no: saying yes here
    /// would hand the caller a list whose first read panics.
    #[test]
    fn test_validate_rejects_an_unaligned_buffer() {
        let node_size: usize = std::mem::size_of::<RBNode<TestOrderBid>>();
        let mut backing: Vec<u8> = vec![0; node_size + 8];
        let align: usize = std::mem::align_of::<RBNode<TestOrderBid>>();
        // Start one byte past whatever alignment the allocation happened to
        // have, so the node address cannot satisfy the reader.
        let offset: usize = align - (backing.as_ptr() as usize % align) + 1;
        let data: &mut [u8] = &mut backing[offset..];
        data[0..4].copy_from_slice(&NIL.to_le_bytes());
        data[4..8].copy_from_slice(&NIL.to_le_bytes());
        data[8..12].copy_from_slice(&NIL.to_le_bytes());

        assert_eq!(
            validate_linked_list::<TestOrderBid>(data, 0),
            Err("linked list node is misaligned"),
        );
    }

    /// The contract that matters: whatever the validator accepts, the reader
    /// it exists to guard can actually walk.
    #[test]
    fn test_validated_lists_can_be_walked() {
        let mut data: [u8; 100000] = [0; 100000];
        let head_index: DataIndex = {
            let mut list: LinkedList<TestOrderBid> = LinkedList::new(&mut data, NIL);
            for i in 1..=5 {
                list.insert(TEST_BLOCK_WIDTH * i, TestOrderBid::new(i.into()));
            }
            list.get_root_index()
        };
        assert_eq!(
            validate_linked_list::<TestOrderBid>(&data, head_index),
            Ok(5)
        );
        // Would panic rather than fail if validation let an unreadable buffer
        // through.
        assert_eq!(values(&data, head_index), vec![5, 4, 3, 2, 1]);
    }

    /// The reason the previous node lives in `parent`: a reader that only
    /// knows red-black trees has to be able to walk these bytes, because the
    /// conversion happens under clients that have not been updated. Every
    /// node is the right child of the one before it, no node has a left
    /// child, and every parent link points at the node that pointed here.
    #[test]
    fn test_a_list_is_a_valid_tree_shape() {
        let mut data: [u8; 100000] = [0; 100000];
        let head_index: DataIndex = {
            let mut list: LinkedList<TestOrderBid> = LinkedList::new(&mut data, NIL);
            for i in 1..=6 {
                list.insert(TEST_BLOCK_WIDTH * i, TestOrderBid::new(i.into()));
            }
            list.get_root_index()
        };

        // Walk it the way a tree reader would: from the root, down the right
        // children, checking the parent link on the way.
        let mut walked: Vec<u64> = Vec::new();
        let mut expected_parent: DataIndex = NIL;
        let mut index: DataIndex = head_index;
        while index != NIL {
            let node: &RBNode<TestOrderBid> = get_helper::<RBNode<TestOrderBid>>(&data, index);
            assert_eq!(node.parent, expected_parent, "parent link at {index}");
            assert_eq!(node.left, NIL, "no left child at {index}");
            assert!(node.color == Color::Black || node.color == Color::Red);
            walked.push(node.get_value().order_id());
            expected_parent = index;
            index = node.right;
        }
        // In-order traversal of a right spine is the spine itself, so a reader
        // that only walks links yields exactly what the list does.
        assert_eq!(walked, values(&data, head_index));
        assert_eq!(walked, vec![6, 5, 4, 3, 2, 1]);

        // That compatibility stops at link-walking readers. The full validator
        // here checks the ordering and black-height invariants too, which a
        // spine does not satisfy, so it must reject these same bytes.
        assert!(
            crate::validate_red_black_tree::<TestOrderBid>(&data, head_index, NIL).is_err(),
            "a list is not a valid red-black tree, only a readable shape",
        );
    }

    #[test]
    fn test_validate_rejects_bad_links() {
        let mut data: [u8; 100000] = [0; 100000];
        let head_index: DataIndex = {
            let mut list: LinkedList<TestOrderBid> = LinkedList::new(&mut data, NIL);
            for i in 1..=3 {
                list.insert(TEST_BLOCK_WIDTH * i, TestOrderBid::new(i.into()));
            }
            list.get_root_index()
        };
        assert_well_formed(&data, head_index, 3);

        // Out of bounds head.
        assert!(validate_linked_list::<TestOrderBid>(&data, 100000).is_err());
        assert!(validate_linked_list::<TestOrderBid>(&data, 99_000).is_err());
        // Misaligned.
        assert!(validate_linked_list::<TestOrderBid>(&data, 3).is_err());

        // A cycle at the tail.
        let mut cyclic: [u8; 100000] = data;
        get_mut_helper::<RBNode<TestOrderBid>>(&mut cyclic, TEST_BLOCK_WIDTH).right = head_index;
        assert_eq!(
            validate_linked_list::<TestOrderBid>(&cyclic, head_index),
            Err("linked list has a cycle")
        );

        // A back link that does not match.
        let mut broken: [u8; 100000] = data;
        get_mut_helper::<RBNode<TestOrderBid>>(&mut broken, TEST_BLOCK_WIDTH * 2).parent = NIL;
        assert_eq!(
            validate_linked_list::<TestOrderBid>(&broken, head_index),
            Err("linked list previous link is inconsistent")
        );

        // A left child, which a spine never has.
        let mut treeish: [u8; 100000] = data;
        get_mut_helper::<RBNode<TestOrderBid>>(&mut treeish, TEST_BLOCK_WIDTH * 2).left =
            head_index;
        assert_eq!(
            validate_linked_list::<TestOrderBid>(&treeish, head_index),
            Err("linked list node has a left child")
        );
    }
}
