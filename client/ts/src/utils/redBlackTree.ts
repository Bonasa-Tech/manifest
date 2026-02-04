import { BeetArgsStruct, bignum } from '@metaplex-foundation/beet';
import { redBlackTreeHeaderBeet } from './beet';
import { toNum } from './numbers';
import { NIL } from '../constants';

export type RedBlackTreeNodeHeader = {
  left: bignum;
  right: bignum;
  parent: bignum;
  color: bignum;
};
const NUM_TREE_HEADER_BYTES = 16;

/**
 * Deserializes a RedBlackTree from a given buffer into a list
 * @description This deserializes the RedBlackTree defined in https://github.com/Bonasa-Tech/manifest/blob/main/src/state/red_black_tree.rs
 *
 * @param data The data buffer to deserialize
 * @param rootIndex Index in the buffer for the root
 * @param valueDeserializer The deserializer for the tree value
 */
export function deserializeRedBlackTree<Value>(
  data: Buffer,
  rootIndex: number,
  valueDeserializer: BeetArgsStruct<Value>,
): Value[] {
  const result: Value[] = [];
  const [rootHeaderValue] = redBlackTreeHeaderBeet.deserialize(
    data.subarray(rootIndex, rootIndex + NUM_TREE_HEADER_BYTES),
  );

  // Find the min
  let currentHeader = rootHeaderValue;
  let currentIndex = rootIndex;
  while (toNum(currentHeader.left) != NIL) {
    currentIndex = toNum(currentHeader.left);

    const [currentHeaderTemp] = redBlackTreeHeaderBeet.deserialize(
      data.subarray(currentIndex, currentIndex + NUM_TREE_HEADER_BYTES),
    );
    currentHeader = currentHeaderTemp;
  }

  // Keep going while there is a successor.
  const [currentValue] = valueDeserializer.deserialize(
    data.subarray(
      currentIndex + NUM_TREE_HEADER_BYTES,
      currentIndex + NUM_TREE_HEADER_BYTES + valueDeserializer.byteSize,
    ),
  );

  result.push(currentValue);
  while (getSuccessorIndex(data, currentIndex) != NIL) {
    currentIndex = getSuccessorIndex(data, currentIndex);
    const [currentValue] = valueDeserializer.deserialize(
      data.subarray(
        currentIndex + NUM_TREE_HEADER_BYTES,
        currentIndex + NUM_TREE_HEADER_BYTES + valueDeserializer.byteSize,
      ),
    );
    result.push(currentValue);
  }

  return result;
}

function getSuccessorIndex(data: Buffer, index: number) {
  if (index == NIL) {
    return NIL;
  }
  let [currentHeader] = redBlackTreeHeaderBeet.deserialize(
    data.subarray(index, index + NUM_TREE_HEADER_BYTES),
  );
  let currentIndex = index;

  // Successor is below, go right then all the way to the left.
  if (toNum(currentHeader.right) != NIL) {
    currentIndex = toNum(currentHeader.right);
    currentHeader = redBlackTreeHeaderBeet.deserialize(
      data.subarray(currentIndex, currentIndex + NUM_TREE_HEADER_BYTES),
    )[0];
    while (toNum(currentHeader.left) != NIL) {
      currentIndex = toNum(currentHeader.left);
      currentHeader = redBlackTreeHeaderBeet.deserialize(
        data.subarray(currentIndex, currentIndex + NUM_TREE_HEADER_BYTES),
      )[0];
    }
    return currentIndex;
  }

  if (currentHeader.parent == NIL) {
    return NIL;
  }
  let [parentHeader] = redBlackTreeHeaderBeet.deserialize(
    data.subarray(
      toNum(currentHeader.parent),
      toNum(currentHeader.parent) + NUM_TREE_HEADER_BYTES,
    ),
  );
  // Successor is above, keep going up while we are the right child
  while (toNum(parentHeader.right) == currentIndex) {
    currentIndex = toNum(currentHeader.parent);
    if (currentIndex == NIL) {
      return NIL;
    }
    currentHeader = redBlackTreeHeaderBeet.deserialize(
      data.subarray(currentIndex, currentIndex + NUM_TREE_HEADER_BYTES),
    )[0];
    if (currentHeader.parent == NIL) {
      return NIL;
    }
    parentHeader = redBlackTreeHeaderBeet.deserialize(
      data.subarray(
        toNum(currentHeader.parent),
        toNum(currentHeader.parent) + NUM_TREE_HEADER_BYTES,
      ),
    )[0];
  }

  // Go up once more.
  currentHeader = redBlackTreeHeaderBeet.deserialize(
    data.subarray(currentIndex, currentIndex + NUM_TREE_HEADER_BYTES),
  )[0];
  if (currentHeader.parent == NIL) {
    return NIL;
  }
  return toNum(currentHeader.parent);
}
