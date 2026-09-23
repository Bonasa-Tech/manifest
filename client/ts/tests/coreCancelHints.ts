import {
  PACKET_DATA_SIZE,
  PublicKey,
  Transaction,
  TransactionInstruction,
} from '@solana/web3.js';
import { assert } from 'chai';
import { ManifestClient } from '../src/client';
import { BatchUpdateStruct } from '../src/manifest/instructions/BatchUpdate';

const key = (value: number): PublicKey =>
  new PublicKey(new Uint8Array(32).fill(value));

describe('core cancellation index hints', () => {
  for (const method of [
    'cancelAllOnCoreIx',
    'cancelBidsOnCoreIx',
    'cancelAsksOnCoreIx',
  ] as const) {
    it(`${method} keeps snapshot indices, ownership and legacy fallback`, async () => {
      const client = Object.create(ManifestClient.prototype) as ManifestClient;
      const orders = [
        { trader: key(2), sequenceNumber: 10n, dataIndex: 0 },
        { trader: key(3), sequenceNumber: 11n, dataIndex: 80 },
        { trader: key(2), sequenceNumber: 12n, dataIndex: 160 },
        { trader: key(2), sequenceNumber: 13n },
      ];
      Object.assign(client, {
        payer: key(2),
        market: {
          address: key(4),
          openOrders: () => orders,
          bidsL2: () => orders,
          asksL2: () => orders,
        },
        baseMint: { address: key(5) },
        quoteMint: { address: key(6) },
        isBase22: false,
        isQuote22: false,
      });
      const instructions = await client[method]();
      assert.lengthOf(instructions, 1);
      const [{ params }] = BatchUpdateStruct.deserialize(instructions[0].data);
      assert.deepEqual(
        params.cancels.map(({ orderSequenceNumber, orderIndexHint }) => ({
          sequence: orderSequenceNumber.toString(),
          index: orderIndexHint,
        })),
        [
          { sequence: '10', index: 0 },
          { sequence: '12', index: 160 },
          { sequence: '13', index: null },
        ],
      );
      const withoutHints = await client[method](false);
      const [unhinted] = BatchUpdateStruct.deserialize(withoutHints[0].data);
      assert.deepEqual(
        unhinted.params.cancels.map((cancel) => cancel.orderIndexHint),
        [null, null, null],
      );
    });

    it(`${method} keeps each batch sendable and exposes hinted packing limits`, async () => {
      const client = Object.create(ManifestClient.prototype) as ManifestClient;
      let count = 48;
      const orders = () =>
        Array.from({ length: count }, (_, index) => ({
          trader: key(2),
          sequenceNumber: BigInt(index),
          dataIndex: 160 + index * 80,
        }));
      Object.assign(client, {
        payer: key(2),
        market: {
          address: key(4),
          openOrders: orders,
          bidsL2: orders,
          asksL2: orders,
        },
        baseMint: { address: key(5) },
        quoteMint: { address: key(6) },
        isBase22: false,
        isQuote22: false,
      });
      const serialize = (instructions: TransactionInstruction[]) =>
        new Transaction({
          feePayer: key(2),
          recentBlockhash: key(9).toBase58(),
        })
          .add(...instructions)
          .serialize({ requireAllSignatures: false, verifySignatures: false });

      assert.isAtMost(
        serialize(await client[method]()).length,
        PACKET_DATA_SIZE,
      );
      count = 49;
      const hinted = await client[method]();
      for (const instruction of hinted) {
        assert.isAtMost(serialize([instruction]).length, PACKET_DATA_SIZE);
      }
      // cancelAll also supplies global accounts; side-only cancels have a
      // smaller account list and therefore a different packing limit.
      if (method !== 'cancelAllOnCoreIx') {
        return;
      }
      assert.throws(() => serialize(hinted), 'Transaction too large');
      count = 67;
      assert.isAtMost(
        serialize(await client[method](false)).length,
        PACKET_DATA_SIZE,
      );
      count = 68;
      const unhinted = await client[method](false);
      assert.throws(() => serialize(unhinted), 'Transaction too large');
    });
  }
});
