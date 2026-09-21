import { PublicKey } from '@solana/web3.js';
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
  }
});
