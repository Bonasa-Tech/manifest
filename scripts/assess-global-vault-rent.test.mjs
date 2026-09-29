import { test } from 'node:test';
import assert from 'node:assert/strict';
import bs58 from 'bs58';
import { analyzeGlobal, analyzeVault } from './assess-global-vault-rent.mjs';
const NIL = 0xffffffff;
const rent = (bytes) => (bytes + 128) * 5080;

function globalFixture() {
  const data = Buffer.alloc(96 + 6 * 64);
  data.writeBigUInt64LE(10787423733276977665n);
  data.writeUInt32LE(0, 72);
  data.writeUInt32LE(64, 76);
  data.writeUInt32LE(256, 84);
  data.writeUInt32LE(384, 88);
  data.writeUInt16LE(2, 94);
  for (const [i, child, parent] of [
    [0, 128, NIL],
    [64, 192, NIL],
    [128, NIL, 0],
    [192, NIL, 64],
  ]) {
    data.writeUInt32LE(child, 96 + i);
    data.writeUInt32LE(NIL, 96 + i + 4);
    data.writeUInt32LE(parent, 96 + i + 8);
  }
  data.writeUInt32LE(64, 96 + 48);
  data.writeUInt32LE(192, 96 + 128 + 48);
  data.fill(1, 96 + 128 + 16, 96 + 128 + 48);
  data.fill(1, 96 + 192 + 16, 96 + 192 + 48);
  data.writeBigUInt64LE(1n, 96 + 192 + 48);
  data.writeUInt32LE(320, 96 + 256);
  data.writeUInt32LE(NIL, 96 + 320);
  return data;
}
test('globals reclaim two blocks per empty seat, preserve referenced zero seats', () => {
  const data = globalFixture();
  const result = analyzeGlobal(data);
  assert.equal(result.freeNodes, 2);
  assert.equal(result.reclaimableSeats, 1);
  assert.equal(result.targetBytes, 224);
  assert.equal(result.depositedAtoms, '1');
  assert.equal(
    analyzeGlobal(data, new Set([bs58.encode(Buffer.alloc(32))]))
      .reclaimableSeats,
    0,
  );
});
test('global broken deposit references fail', () => {
  const data = globalFixture();
  data.writeUInt32LE(128, 96 + 48);
  assert.throws(() => analyzeGlobal(data));
});
test('native token principal and stored reserve are not excess rent', () => {
  const data = Buffer.alloc(165);
  data[108] = 1;
  data.writeBigUInt64LE(10000000000n, 64);
  data.writeUInt32LE(1, 109);
  data.writeBigUInt64LE(2039280n, 113);
  const row = analyzeVault(data, 10002039280, rent);
  assert.equal(row.excessLamports, 0);
  assert.equal(row.closeCandidate, false);
  assert.equal(row.nativeReserveReductionLamports, 550840);
});
test('token fee withholding prevents classifying an empty account as closable', () => {
  const data = Buffer.alloc(178);
  data[108] = 1;
  data[165] = 2;
  data.writeUInt16LE(2, 166);
  data.writeUInt16LE(8, 168);
  data.writeBigUInt64LE(1n, 170);
  assert.equal(analyzeVault(data, rent(178) + 100, rent).closeCandidate, false);
  assert.equal(analyzeVault(data, rent(178) + 100, rent).excessLamports, 100);
});
