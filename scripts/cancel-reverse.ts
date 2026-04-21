import {
  ManifestClient,
  RestingOrder,
  OrderType,
  createBatchUpdateInstruction,
} from '@bonasa-tech/manifest-sdk';
import { bignum } from '@metaplex-foundation/beet';
import {
  Connection,
  Keypair,
  PublicKey,
  sendAndConfirmTransaction,
  Transaction,
  TransactionInstruction,
} from '@solana/web3.js';

const { RPC_URL } = process.env;
if (!RPC_URL) {
  throw new Error('RPC_URL missing from env');
}

const marketPk = new PublicKey(process.env.MARKET_PK!);
const keypair: Keypair = Keypair.fromSecretKey(
  Uint8Array.from(process.env.PRIVATE_KEY!.split(',').map(Number)),
);

// Script for cancelling all reverse orders for a maker on a market. This is
// intentionally not exposed on UI.
const run = async () => {
  const connection: Connection = new Connection(RPC_URL);
  const client: ManifestClient = await ManifestClient.getClientReadOnly(
    connection,
    marketPk,
  );
  const openOrders: RestingOrder[] = client.market.openOrders();
  for (const openOrder of openOrders) {
    if (
      (openOrder.orderType == OrderType.Reverse ||
        openOrder.orderType == OrderType.ReverseTight) &&
      openOrder.trader.toBase58() == keypair.publicKey.toBase58()
    ) {
      const seqNum: bignum = openOrder.sequenceNumber;
      const cancelIx: TransactionInstruction = createBatchUpdateInstruction(
        {
          payer: keypair.publicKey,
          market: marketPk,
        },
        {
          params: {
            cancels: [
              {
                orderSequenceNumber: seqNum,
                orderIndexHint: null,
              },
            ],
            orders: [],
            traderIndexHint: null,
          },
        },
      );
      const signature = await sendAndConfirmTransaction(
        connection,
        new Transaction().add(cancelIx),
        [keypair],
        {
          skipPreflight: true,
        },
      );
      console.log('Cancelled in', signature);
    }
  }
};

run().catch((e) => {
  console.error('fatal error', e);
  throw e;
});
