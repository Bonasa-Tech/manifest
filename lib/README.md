# Hypertree

Hypertree is a library for creating efficient data structures on solana that do
not own the underlying byte array. This special feature allows overlapping data
structures within the same region of memory, inside of the same account.

Structures:

- `RedBlackTree`: ordered, O(log n) insert, remove and lookup, O(1) max. Used
  for the order books and seats on a market.
- `LinkedList`: unordered doubly linked list over the same node blocks, O(1)
  insert and remove and a cheap full walk. For small collections that are
  always walked in full, like the wrapper's open orders on a market; a tree
  can be converted to a list in place.
- `FreeList`: the pool of free blocks the structures above allocate from.
