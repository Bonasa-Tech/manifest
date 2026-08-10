import { BeetArgsStruct, bignum } from '@metaplex-foundation/beet';
import { redBlackTreeHeaderBeet } from './beet';
import { toNum } from './numbers';
import { NIL } from '../constants';

export type RedBlackTreeNodeHeader = {
  left: bignum;
  right: bignum;
  parent: bignum;
  color: bignum;
  payloadType: bignum;
  padding: bignum;
};

const NUM_TREE_HEADER_BYTES = 16;
const TREE_NODE_ALIGNMENT = 8;

export type RedBlackTreeParseContext = {
  claimedBytes: Uint8Array;
  parsedNodes: number;
  maxNodes: number;
};

export function createRedBlackTreeParseContext(
  dataLength: number,
): RedBlackTreeParseContext {
  return {
    claimedBytes: new Uint8Array(dataLength),
    parsedNodes: 0,
    maxNodes: Math.floor(dataLength / NUM_TREE_HEADER_BYTES),
  };
}

/**
 * Deserializes an account-backed red-black tree after validating every
 * reachable node. RPC account bytes are untrusted, so malformed offsets and
 * cycles must fail closed instead of reading truncated buffers or looping.
 */
export function deserializeRedBlackTree<Value>(
  data: Buffer,
  rootIndex: number,
  valueDeserializer: BeetArgsStruct<Value>,
  context: RedBlackTreeParseContext = createRedBlackTreeParseContext(
    data.length,
  ),
): Value[] {
  if (rootIndex === NIL) {
    return [];
  }

  const nodeSize = NUM_TREE_HEADER_BYTES + valueDeserializer.byteSize;
  const headers = new Map<number, RedBlackTreeNodeHeader>();
  const visiting = new Set<number>();
  const visited = new Set<number>();

  const readHeader = (index: number): RedBlackTreeNodeHeader => {
    if (
      !Number.isSafeInteger(index) ||
      index < 0 ||
      index % TREE_NODE_ALIGNMENT !== 0 ||
      index + nodeSize > data.length
    ) {
      throw new Error(`Invalid red-black tree node offset: ${index}`);
    }
    const cached = headers.get(index);
    if (cached) return cached;
    if (context.claimedBytes.length !== data.length) {
      throw new Error('Red-black tree parse context has the wrong size');
    }
    for (let byte = index; byte < index + nodeSize; byte += 1) {
      if (context.claimedBytes[byte] !== 0) {
        throw new Error(
          `Red-black tree node overlaps a previously parsed node at offset ${index}`,
        );
      }
    }
    if (context.parsedNodes >= context.maxNodes) {
      throw new Error('Red-black tree aggregate node budget exceeded');
    }
    context.claimedBytes.fill(1, index, index + nodeSize);
    context.parsedNodes += 1;
    const [header] = redBlackTreeHeaderBeet.deserialize(
      data.subarray(index, index + NUM_TREE_HEADER_BYTES),
    );
    const color = toNum(header.color);
    if (color !== 0 && color !== 1) {
      throw new Error(`Invalid red-black tree color at offset ${index}`);
    }
    headers.set(index, header);
    return header;
  };

  // Avoid recursion: account data controls the number of reachable nodes, so
  // a malformed but acyclic tree must not be able to exhaust the JS stack.
  const validationStack: Array<{
    index: number;
    expectedParent: number;
    complete: boolean;
  }> = [{ index: rootIndex, expectedParent: NIL, complete: false }];
  while (validationStack.length > 0) {
    const { index, expectedParent, complete } = validationStack.pop()!;
    if (index === NIL) continue;

    if (complete) {
      visiting.delete(index);
      visited.add(index);
      continue;
    }
    if (visiting.has(index) || visited.has(index)) {
      throw new Error(
        `Cycle or duplicate red-black tree node at offset ${index}`,
      );
    }

    visiting.add(index);
    const header = readHeader(index);
    if (toNum(header.parent) !== expectedParent) {
      throw new Error(`Invalid red-black tree parent at offset ${index}`);
    }
    validationStack.push({ index, expectedParent, complete: true });
    validationStack.push({
      index: toNum(header.right),
      expectedParent: index,
      complete: false,
    });
    validationStack.push({
      index: toNum(header.left),
      expectedParent: index,
      complete: false,
    });
  }

  const result: Value[] = [];
  const stack: number[] = [];
  let current = rootIndex;
  while (current !== NIL || stack.length > 0) {
    while (current !== NIL) {
      stack.push(current);
      current = toNum(readHeader(current).left);
    }
    const index = stack.pop()!;
    const [value] = valueDeserializer.deserialize(
      data.subarray(
        index + NUM_TREE_HEADER_BYTES,
        index + NUM_TREE_HEADER_BYTES + valueDeserializer.byteSize,
      ),
    );
    result.push(value);
    current = toNum(readHeader(index).right);
  }
  return result;
}
