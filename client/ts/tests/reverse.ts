import { Connection, Keypair, PublicKey } from '@solana/web3.js';
import { OrderType } from '../src/manifest/types';
import { createMarket } from './createMarket';
import { deposit } from './deposit';
import { Market } from '../src/market';
import { assert } from 'chai';
import { placeOrder } from './placeOrder';
import { describeIfDirectTest } from './helpers/mocha';

async function testReverse(): Promise<void> {
  const connection: Connection = new Connection(
    'http://127.0.0.1:8899',
    'confirmed',
  );
  const payerKeypair: Keypair = Keypair.generate();

  const marketAddress: PublicKey = await createMarket(connection, payerKeypair);
  const market: Market = await Market.loadFromAddress({
    connection,
    address: marketAddress,
  });

  // Deposit base and quote.
  await deposit(connection, payerKeypair, marketAddress, market.baseMint(), 10);
  await deposit(
    connection,
    payerKeypair,
    marketAddress,
    market.quoteMint(),
    10,
  );
  const spreadBps: number = 2_000;
  await placeOrder(
    connection,
    payerKeypair,
    marketAddress,
    5,
    5,
    false,
    OrderType.Reverse,
    0,
    0,
    spreadBps,
  );
  // Bid 1@6, should fill and result in a new order on the book.
  await placeOrder(
    connection,
    payerKeypair,
    marketAddress,
    1,
    6,
    true,
    OrderType.Limit,
    1,
  );

  await market.reload(connection);
  market.prettyPrint();

  // Asks are sorted worst to best.
  assert(market.asks().length == 1, 'place asks did not work');
  assert(
    Number(market.asks()[0].numBaseTokens) == 4,
    `ask top of book wrong size expected ${4} actual ${Number(market.asks()[0].numBaseTokens)}`,
  );
  assert(
    market.asks()[0].tokenPrice == 5,
    `ask top of book wrong price ${market.asks()[0].tokenPrice}`,
  );
  assert(market.bids().length == 1, 'place bids did not work');
  assert(
    market.bids()[0].tokenPrice == 4,
    `bid top of book wrong price ${market.bids()[0].tokenPrice}`,
  );
  assert(
    Number(market.bids()[0].numBaseTokens) == 1.25,
    `bids top of book wrong size expected ${1.25} actual ${Number(market.bids()[0].numBaseTokens)}`,
  );

  // Test spreadBps extraction
  assert(
    market.asks()[0].spreadBps === 2000,
    `ask spreadBps should be 2000, got ${market.asks()[0].spreadBps}`,
  );
  assert(
    market.bids()[0].spreadBps === 2000,
    `bid spreadBps should be 2000, got ${market.bids()[0].spreadBps}`,
  );
}

describeIfDirectTest(module, 'Reverse test', () => {
  it('Reverse', async () => {
    await testReverse();
  });
});
