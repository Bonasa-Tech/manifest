import { expect } from 'chai';
import { BeetArgsStruct, u32 } from '@metaplex-foundation/beet';
import { NIL } from '../src/constants';
import {
  createRedBlackTreeParseContext,
  deserializeRedBlackTree,
} from '../src/utils/redBlackTree';

type TestValue = { value: number };
const valueBeet = new BeetArgsStruct<TestValue>([['value', u32]], 'TestValue');

function writeNode(
  data: Buffer,
  offset: number,
  left: number,
  right: number,
  parent: number,
  value: number,
): void {
  data.writeUInt32LE(left, offset);
  data.writeUInt32LE(right, offset + 4);
  data.writeUInt32LE(parent, offset + 8);
  data.writeUInt8(0, offset + 12);
  data.writeUInt32LE(value, offset + 16);
}

describe('red-black tree validation', () => {
  it('deserializes a validated tree in order', () => {
    const data = Buffer.alloc(44);
    writeNode(data, 0, NIL, 24, NIL, 1);
    writeNode(data, 24, NIL, NIL, 0, 2);
    expect(deserializeRedBlackTree(data, 0, valueBeet)).to.deep.equal([
      { value: 1 },
      { value: 2 },
    ]);
  });

  it('rejects cycles and out-of-bounds offsets', () => {
    const cyclic = Buffer.alloc(20);
    writeNode(cyclic, 0, 0, NIL, NIL, 1);
    expect(() => deserializeRedBlackTree(cyclic, 0, valueBeet)).to.throw(
      /Cycle or duplicate/,
    );

    const outOfBounds = Buffer.alloc(20);
    writeNode(outOfBounds, 0, NIL, 24, NIL, 1);
    expect(() => deserializeRedBlackTree(outOfBounds, 0, valueBeet)).to.throw(
      /offset/,
    );
  });

  it('rejects subtree reuse across parses sharing one account budget', () => {
    const data = Buffer.alloc(20);
    writeNode(data, 0, NIL, NIL, NIL, 1);
    const context = createRedBlackTreeParseContext(data.length);
    expect(deserializeRedBlackTree(data, 0, valueBeet, context)).to.deep.equal([
      { value: 1 },
    ]);
    expect(() => deserializeRedBlackTree(data, 0, valueBeet, context)).to.throw(
      /overlaps a previously parsed node/,
    );
  });
});
