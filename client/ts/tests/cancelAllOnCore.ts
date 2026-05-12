import {
  Connection,
  Keypair,
  PublicKey,
  Transaction,
  sendAndConfirmTransaction,
} from '@solana/web3.js';
import { ManifestClient } from '../src/client';
import { OrderType } from '../src/manifest/types';
import { createMarket } from './createMarket';
import { deposit } from './deposit';
import { placeOrder } from './placeOrder';
import { Market } from '../src/market';
import { assert } from 'chai';
import { describeIfDirectTest } from './helpers/mocha';

async function testCancelAllOnCore(): Promise<void> {
  // Setup connection and accounts
  const connection: Connection = new Connection(
    'http://127.0.0.1:8899',
    'confirmed',
  );
  const payerKeypair: Keypair = Keypair.generate();

  // Create a market and load it
  console.log('Creating market for cancelAllOnCore test...');
  const marketAddress: PublicKey = await createMarket(connection, payerKeypair);
  const market: Market = await Market.loadFromAddress({
    connection,
    address: marketAddress,
  });

  console.log('Depositing funds...');
  await deposit(connection, payerKeypair, marketAddress, market.baseMint(), 50);
  await deposit(
    connection,
    payerKeypair,
    marketAddress,
    market.quoteMint(),
    50,
  );

  // Place two reverse orders
  console.log('Placing reverse orders...');
  await placeOrder(
    connection,
    payerKeypair,
    marketAddress,
    5,
    5,
    false, // sell
    OrderType.Reverse,
    0,
  );

  await placeOrder(
    connection,
    payerKeypair,
    marketAddress,
    3,
    4,
    true, // buy
    OrderType.Reverse,
    1,
  );

  console.log('Filling one of the reverse orders...');
  await placeOrder(
    connection,
    payerKeypair,
    marketAddress,
    3, // partial size
    5, // matching price
    true, // buy to match the sell
    OrderType.ImmediateOrCancel,
    0,
  );

  // Wait for the fill
  console.log('Waiting for fill to process...');
  await new Promise((resolve) => setTimeout(resolve, 2000));

  // Check the orderbook state before cancellation
  await market.reload(connection);
  console.log('Market state before cancelAllOnCore:');
  market.prettyPrint();

  const beforeCount = market.openOrders().length;
  console.log(`Market has ${beforeCount} open orders before cancelAllOnCore`);

  // Execute cancelAllOnCore
  console.log('Executing cancelAllOnCore...');
  await cancelAllOnCore(connection, payerKeypair, marketAddress);

  // Verify all orders are gone
  await market.reload(connection);
  console.log('Market state after cancelAllOnCore:');
  market.prettyPrint();

  // Check that no orders from our trader remain
  const remainingOrders = market
    .openOrders()
    .filter(
      (order) => order.trader.toBase58() === payerKeypair.publicKey.toBase58(),
    );

  assert(remainingOrders.length === 0, 'All reverse orders should be canceled');
  console.log('Test passed: All reverse orders have been canceled');
}

export async function cancelAllOnCore(
  connection: Connection,
  payerKeypair: Keypair,
  marketAddress: PublicKey,
): Promise<void> {
  const client: ManifestClient = await ManifestClient.getClientForMarket(
    connection,
    marketAddress,
    payerKeypair,
  );

  const cancelInstructions = await client.cancelAllOnCoreIx();

  if (cancelInstructions.length === 0) {
    console.log('No orders to cancel');
    return;
  }
  for (let i = 0; i < cancelInstructions.length; i++) {
    const transaction = new Transaction();
    transaction.add(cancelInstructions[i]);

    const signature = await sendAndConfirmTransaction(
      connection,
      transaction,
      [payerKeypair],
      {
        skipPreflight: true,
      },
    );

    console.log(`Canceled batch of orders in transaction: ${signature}`);
  }
}

describeIfDirectTest(module, 'Cancel All On Core Test', () => {
  it('Should cancel all reverse orders on the core program', async () => {
    await testCancelAllOnCore();
  });
});
