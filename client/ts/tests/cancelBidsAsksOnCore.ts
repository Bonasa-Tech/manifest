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

async function testCancelBidsAsksOnCore(): Promise<void> {
  // Setup connection and accounts
  const connection: Connection = new Connection(
    'http://127.0.0.1:8899',
    'confirmed',
  );
  const payerKeypair: Keypair = Keypair.generate();

  // Create a market and load it
  console.log('Creating market for cancelBidsAsksOnCore test...');
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

  // Place reverse orders (this mirrors the cancelAllOnCore test)
  console.log('Placing reverse orders...');

  // Place a reverse sell order (ask)
  await placeOrder(
    connection,
    payerKeypair,
    marketAddress,
    5,
    5,
    false, // sell (ask)
    OrderType.Reverse,
    0,
  );

  // Place a reverse buy order (bid)
  await placeOrder(
    connection,
    payerKeypair,
    marketAddress,
    3,
    4,
    true, // buy (bid)
    OrderType.Reverse,
    1,
  );

  // Place additional reverse orders to test multiple cancellations
  await placeOrder(
    connection,
    payerKeypair,
    marketAddress,
    4,
    3.5,
    true, // buy (bid)
    OrderType.Reverse,
    2,
  );

  await placeOrder(
    connection,
    payerKeypair,
    marketAddress,
    6,
    5.5,
    false, // sell (ask)
    OrderType.Reverse,
    3,
  );

  console.log('Filling some of the reverse orders...');

  // Fill part of the sell order (matches with bid at price 5)
  await placeOrder(
    connection,
    payerKeypair,
    marketAddress,
    2, // partial size
    5, // matching price
    true, // buy to match the sell
    OrderType.ImmediateOrCancel,
    4,
  );

  // Fill part of the buy order (matches with ask at price 4)
  await placeOrder(
    connection,
    payerKeypair,
    marketAddress,
    1, // partial size
    4, // matching price
    false, // sell to match the buy
    OrderType.ImmediateOrCancel,
    5,
  );

  // Wait for fills to process
  console.log('Waiting for fills to process...');
  await new Promise((resolve) => setTimeout(resolve, 2000));

  // Check the orderbook state before cancellation
  await market.reload(connection);
  console.log('Market state before cancellation:');
  market.prettyPrint();

  const bidsBeforeCancel = market
    .bidsL2()
    .filter(
      (order) => order.trader.toBase58() === payerKeypair.publicKey.toBase58(),
    );
  const asksBeforeCancel = market
    .asksL2()
    .filter(
      (order) => order.trader.toBase58() === payerKeypair.publicKey.toBase58(),
    );

  console.log(`Found ${bidsBeforeCancel.length} bid orders from our trader`);
  console.log(`Found ${asksBeforeCancel.length} ask orders from our trader`);

  const totalOrdersBeforeCancel =
    bidsBeforeCancel.length + asksBeforeCancel.length;
  console.log(`Total orders before cancellation: ${totalOrdersBeforeCancel}`);

  // Should have remaining orders after partial fills
  assert(bidsBeforeCancel.length > 0, 'Should have remaining bid orders');
  assert(asksBeforeCancel.length > 0, 'Should have remaining ask orders');

  // Test 1: Cancel all bids only
  console.log('Testing cancelBidsOnCore...');
  await cancelBidsOnCore(connection, payerKeypair, marketAddress);

  // Verify only bids were canceled
  await market.reload(connection);
  console.log('Market state after canceling bids:');
  market.prettyPrint();

  const bidsAfterBidCancel = market
    .bidsL2()
    .filter(
      (order) => order.trader.toBase58() === payerKeypair.publicKey.toBase58(),
    );
  const asksAfterBidCancel = market
    .asksL2()
    .filter(
      (order) => order.trader.toBase58() === payerKeypair.publicKey.toBase58(),
    );

  assert(bidsAfterBidCancel.length === 0, 'All bid orders should be canceled');
  assert(
    asksAfterBidCancel.length === asksBeforeCancel.length,
    'Ask orders should remain unchanged',
  );

  console.log(
    `Bids canceled: ${bidsBeforeCancel.length}, Asks remaining: ${asksAfterBidCancel.length}`,
  );

  // Test 2: Cancel all asks only
  console.log('Testing cancelAsksOnCore...');
  await cancelAsksOnCore(connection, payerKeypair, marketAddress);

  // Verify all remaining orders are canceled
  await market.reload(connection);
  console.log('Market state after canceling asks:');
  market.prettyPrint();

  const bidsAfterAskCancel = market
    .bidsL2()
    .filter(
      (order) => order.trader.toBase58() === payerKeypair.publicKey.toBase58(),
    );
  const asksAfterAskCancel = market
    .asksL2()
    .filter(
      (order) => order.trader.toBase58() === payerKeypair.publicKey.toBase58(),
    );

  assert(bidsAfterAskCancel.length === 0, 'Bid orders should remain canceled');
  assert(asksAfterAskCancel.length === 0, 'All ask orders should be canceled');

  console.log(
    'Test passed: Both cancelBidsOnCore and cancelAsksOnCore work correctly with reverse orders and fills',
  );
}

export async function cancelBidsOnCore(
  connection: Connection,
  payerKeypair: Keypair,
  marketAddress: PublicKey,
): Promise<void> {
  const client: ManifestClient = await ManifestClient.getClientForMarket(
    connection,
    marketAddress,
    payerKeypair,
  );

  const cancelInstructions = await client.cancelBidsOnCoreIx();

  if (cancelInstructions.length === 0) {
    console.log('No bid orders to cancel');
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

    console.log(`Canceled batch of bid orders in transaction: ${signature}`);
  }
}

export async function cancelAsksOnCore(
  connection: Connection,
  payerKeypair: Keypair,
  marketAddress: PublicKey,
): Promise<void> {
  const client: ManifestClient = await ManifestClient.getClientForMarket(
    connection,
    marketAddress,
    payerKeypair,
  );

  const cancelInstructions = await client.cancelAsksOnCoreIx();

  if (cancelInstructions.length === 0) {
    console.log('No ask orders to cancel');
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

    console.log(`Canceled batch of ask orders in transaction: ${signature}`);
  }
}

describeIfDirectTest(module, 'Cancel Bids/Asks On Core Test', () => {
  it('Should cancel bid and ask orders separately on the core program', async () => {
    await testCancelBidsAsksOnCore();
  });
});
