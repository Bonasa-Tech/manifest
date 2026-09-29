import { test } from 'node:test';
import assert from 'node:assert/strict';
import { analyzeMarket, DISCRIMINANT, NIL } from './assess-market-rent.mjs';

function fixture() {
  // Three seats: empty, funded, zero balance with a resting order.
  // Two free blocks: one interior and one at the tail.
  const data = Buffer.alloc(256 + 6 * 80);
  data.writeBigUInt64LE(DISCRIMINANT);
  data.writeUInt32LE(480, 152);
  for (const offset of [156, 160, 164, 168, 172, 176])
    data.writeUInt32LE(NIL, offset);
  function node(index, left = NIL, right = NIL, parent = NIL) {
    for (const [offset, value] of [
      [0, left],
      [4, right],
      [8, parent],
    ])
      data.writeUInt32LE(value, 256 + index + offset);
  }
  data.writeUInt32LE(0, 172);
  node(0, 160, 240);
  node(160, NIL, NIL, 0);
  node(240, NIL, NIL, 0);
  data.writeBigUInt64LE(1n, 256 + 160 + 16 + 32);
  data.writeUInt32LE(320, 156);
  node(320);
  data.writeUInt32LE(240, 256 + 320 + 48);
  data.writeUInt32LE(80, 176);
  data.writeUInt32LE(400, 256 + 80);
  data.writeUInt32LE(NIL, 256 + 400);
  return data;
}

test('separates free compaction, tail trimming, and strictly empty seats', () => {
  const result = analyzeMarket(fixture());
  assert.equal(result.freeNodes, 2);
  assert.equal(result.tailFreeNodes, 1);
  assert.equal(result.seats, 3);
  assert.equal(result.reclaimableSeats, 1);
  assert.equal(result.zeroBalanceSeatsWithOrders, 1);
  assert.deepEqual(result.reclaimableSeatIndices, [0]);
  assert.equal(result.targetBytes.tailOnly, 656);
  assert.equal(result.targetBytes.compactFree, 576);
  assert.equal(result.targetBytes.compactFreeAndSeats, 496);
  assert.equal(result.targetBytes.keep2, 656);
  assert.equal(result.targetBytes.keep8, 736);
});

test('one quote atom prevents seat reclamation', () => {
  const data = fixture();
  data.writeBigUInt64LE(1n, 256 + 16 + 40);
  assert.equal(analyzeMarket(data).reclaimableSeats, 0);
});

test('expired/global orders still protect their seats', () => {
  const data = fixture();
  data.writeUInt32LE(1, 256 + 320 + 16 + 36);
  data[256 + 320 + 16 + 41] = 3;
  assert.equal(analyzeMarket(data).zeroBalanceSeatsWithOrders, 1);
});

test('rejects cycles, missing seats, truncated accounts, and orphan nodes', () => {
  for (const mutate of [
    (data) => data.writeUInt32LE(80, 256 + 400),
    (data) => data.writeUInt32LE(80, 256 + 320 + 48),
    (data) => data.writeUInt32LE(NIL, 176),
    (data) => data.writeUInt32LE(0, 256),
  ]) {
    const data = fixture();
    mutate(data);
    assert.throws(() => analyzeMarket(data));
  }
  assert.throws(() => analyzeMarket(fixture().subarray(0, 700)));
});
