import { expect } from 'chai';
import { Keypair, PublicKey } from '@solana/web3.js';
import BN from 'bn.js';
import { FIXED_WRAPPER_HEADER_SIZE, NIL } from '../src/constants';
import { OrderType } from '../src/manifest';
import { Wrapper, WrapperOpenOrder } from '../src/wrapperObj';
import { marketInfoBeet, wrapperOpenOrderBeet } from '../src/wrapper/types';
import { Wrapper as ReleasedWrapper } from '@cks-systems/manifest-sdk-old/dist/cjs/wrapperObj';

const NODE_HEADER_SIZE = 16;
const BLOCK_SIZE = 96;
/** `payloadType` the program stamps on a market info once it has converted. */
const ORDERS_LAYOUT_TREE = 0;
const ORDERS_LAYOUT_LIST = 1;

function writeHeader(
  data: Buffer,
  offset: number,
  left: number,
  right: number,
  parent: number,
  color: number,
  payloadType: number,
): void {
  data.writeUInt32LE(left, offset);
  data.writeUInt32LE(right, offset + 4);
  data.writeUInt32LE(parent, offset + 8);
  data.writeUInt8(color, offset + 12);
  data.writeUInt8(payloadType, offset + 13);
  data.writeUInt16LE(0, offset + 14);
}

function orderBytes(clientOrderId: number): Buffer {
  const [bytes] = wrapperOpenOrderBeet.serialize({
    price: new BN(0),
    clientOrderId: new BN(clientOrderId),
    orderSequenceNumber: new BN(clientOrderId + 10),
    numBaseAtoms: new BN(1),
    marketDataIndex: 0,
    lastValidSlot: 0,
    isBid: false,
    orderType: OrderType.Limit,
    padding: new Array(30).fill(0),
  });
  return bytes;
}

/**
 * A wrapper with one market info (block 0) whose two open orders live in
 * blocks 1 and 2, laid out either as the tree older wrappers have or as the
 * list the program converts them to on first use.
 */
function wrapperBuffer(market: PublicKey, layout: number): Buffer {
  const dynamic = Buffer.alloc(3 * BLOCK_SIZE);
  writeHeader(dynamic, 0, NIL, NIL, NIL, 0, layout);
  const [marketInfoBytes] = marketInfoBeet.serialize({
    market,
    ordersRootIndex: BLOCK_SIZE,
    traderIndex: 0,
    baseBalance: new BN(0),
    quoteBalance: new BN(0),
    quoteVolume: new BN(0),
    lastUpdatedSlot: 0,
    numOpenGlobalOrders: 0,
    lastSyncedOrderSequenceNumber: new BN(0),
  });
  marketInfoBytes.copy(dynamic, NODE_HEADER_SIZE);

  // Block 1 holds client order id 2, block 2 holds client order id 1.
  if (layout == ORDERS_LAYOUT_LIST) {
    // Head is block 1, next is block 2. The previous node lives in `parent`
    // and there are no left children, which is a right-leaning tree spine.
    writeHeader(dynamic, BLOCK_SIZE, NIL, 2 * BLOCK_SIZE, NIL, 0, 0);
    writeHeader(dynamic, 2 * BLOCK_SIZE, NIL, NIL, BLOCK_SIZE, 0, 0);
  } else {
    // Root is block 1, its left child is block 2.
    writeHeader(dynamic, BLOCK_SIZE, 2 * BLOCK_SIZE, NIL, NIL, 0, 0);
    writeHeader(dynamic, 2 * BLOCK_SIZE, NIL, NIL, BLOCK_SIZE, 1, 0);
  }
  orderBytes(2).copy(dynamic, BLOCK_SIZE + NODE_HEADER_SIZE);
  orderBytes(1).copy(dynamic, 2 * BLOCK_SIZE + NODE_HEADER_SIZE);

  const header = Buffer.alloc(FIXED_WRAPPER_HEADER_SIZE);
  header.writeBigUInt64LE(1n, 0);
  Keypair.generate().publicKey.toBuffer().copy(header, 8);
  header.writeUInt32LE(dynamic.length, 40);
  header.writeUInt32LE(NIL, 44);
  header.writeUInt32LE(0, 48);
  return Buffer.concat([header, dynamic]);
}

// The program keeps a market's open orders in a linked list, but lays that
// list out as a right-leaning tree spine: the previous node in `parent`, no
// left children. This client has no idea any of that happened. It parses
// every wrapper as a red-black tree, and these tests are what says it may
// keep doing so, here and in the versions already released.
describe('wrapper open orders layouts', () => {
  const market: PublicKey = Keypair.generate().publicKey;
  const clientOrderIds = (orders: WrapperOpenOrder[]): number[] =>
    orders.map((order: WrapperOpenOrder) => Number(order.clientOrderId));

  it('reads a converted wrapper with the tree parser', () => {
    // A spine passes the parser's checks, since it validates that each
    // child's parent link points back at it and nothing about ordering or
    // balance, and the in-order walk of a right spine is the spine itself.
    // So the orders come back in list order, head first.
    const wrapper = Wrapper.loadFromBuffer({
      address: Keypair.generate().publicKey,
      buffer: wrapperBuffer(market, ORDERS_LAYOUT_LIST),
    });
    expect(clientOrderIds(wrapper.openOrdersForMarket(market)!)).to.deep.equal([
      2, 1,
    ]);
  });

  it('is read by a client released before the list existed', () => {
    // The claim this design rests on is about clients already deployed, not
    // about the parser in this repository, so it is pinned against one: the
    // published SDK that predates the list, resolved as manifest-sdk-old.
    const buffer = wrapperBuffer(market, ORDERS_LAYOUT_LIST);
    const released = ReleasedWrapper.loadFromBuffer({
      address: Keypair.generate().publicKey,
      buffer,
    });
    const orders = released.openOrdersForMarket(market)!;
    expect(orders.map((order) => Number(order.clientOrderId))).to.deep.equal([
      2, 1,
    ]);
  });

  it('reads a wrapper that has not been converted yet', () => {
    const wrapper = Wrapper.loadFromBuffer({
      address: Keypair.generate().publicKey,
      buffer: wrapperBuffer(market, ORDERS_LAYOUT_TREE),
    });
    expect(clientOrderIds(wrapper.openOrdersForMarket(market)!)).to.deep.equal([
      1, 2,
    ]);
  });
});
