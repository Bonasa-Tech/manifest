import { Connection, Keypair, PublicKey } from '@solana/web3.js';
import { Market } from '../src/market';
import { createMarket } from './createMarket';
import { ManifestClient } from '../src';
import { assert } from 'chai';
import { placeOrder } from './placeOrder';
import { OrderType } from '../src/manifest';
import { deposit } from './deposit';
import { areFloatsEqual } from './utils';
import { describeIfDirectTest } from './helpers/mocha';

async function setupMarketState(): Promise<Market> {
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

  await ManifestClient.getClientForMarket(
    connection,
    marketAddress,
    payerKeypair,
  );

  await Promise.all([
    deposit(connection, payerKeypair, marketAddress, market.quoteMint(), 99),
    deposit(connection, payerKeypair, marketAddress, market.baseMint(), 99),
  ]);

  // setup an orderbook with 5 orders on bid and ask side
  await Promise.all([
    ...[1, 2, 3, 4, 5].map((i) =>
      placeOrder(
        connection,
        payerKeypair,
        marketAddress,
        1,
        1 - i * 0.01,
        true,
        OrderType.Limit,
        0,
      ),
    ),
    ...[1, 2, 3, 4, 5].map((i) =>
      placeOrder(
        connection,
        payerKeypair,
        marketAddress,
        1,
        1 + i * 0.01,
        false,
        OrderType.Limit,
        0,
      ),
    ),
  ]);

  market.prettyPrint();

  return market;
}

async function testMarket(): Promise<void> {
  const connection: Connection = new Connection(
    'http://127.0.0.1:8899',
    'confirmed',
  );
  const payerKeypair: Keypair = Keypair.generate();
  const marketAddress: PublicKey = await createMarket(connection, payerKeypair);

  // Test loading successfully.
  const market: Market = await Market.loadFromAddress({
    connection,
    address: marketAddress,
  });
  market.prettyPrint();

  // Test loading fails on bad address
  try {
    await Market.loadFromAddress({
      connection,
      address: Keypair.generate().publicKey,
    });
    assert(false, 'expected load from address fail');
  } catch (err) {
    assert(true, 'expected load from address fail');
  }

  // Test reloading successful.
  await market.reload(connection);

  // Test reloading fail.
  try {
    await market.reload(new Connection('https://api.devnet.solana.com'));
    assert(false, 'expected reload fail');
  } catch (err) {
    assert(true, 'expected reload fail');
  }

  // Market withdrawable balance not init
  assert(
    market.getWithdrawableBalanceTokens(payerKeypair.publicKey, true) == 0,
    'Get withdrawable balance with no seat',
  );

  // Init seat.
  await ManifestClient.getClientForMarket(
    connection,
    marketAddress,
    payerKeypair,
  );

  // Place an order to get more coverage on the pretty print.
  await deposit(
    connection,
    payerKeypair,
    marketAddress,
    market.quoteMint(),
    10,
  );
  // Market withdrawable balance after deposit
  assert(
    market.getWithdrawableBalanceTokens(payerKeypair.publicKey, false) == 0,
    'Get withdrawable balance after deposit',
  );

  assert(market.baseDecimals() == 9, 'base decimals');
  assert(market.quoteDecimals() == 6, 'quote decimals');

  // Put orders on both sides to test pretty printing.
  await deposit(connection, payerKeypair, marketAddress, market.baseMint(), 10);
  await placeOrder(
    connection,
    payerKeypair,
    marketAddress,
    1,
    5,
    false,
    OrderType.Limit,
    0,
  );
  await placeOrder(
    connection,
    payerKeypair,
    marketAddress,
    1,
    5,
    true,
    OrderType.Limit,
    0,
  );

  market.prettyPrint();
}

describeIfDirectTest(module, 'Market test', () => {
  let market: Market;
  let connection: Connection;

  before(async () => {
    market = await setupMarketState();
    connection = new Connection('http://127.0.0.1:8899', 'confirmed');
    await market.reload(connection);
  });

  it('getBidsL2', () => {
    const b = market.bidsL2();

    assert(
      b.length === 5,
      `5 l2 bids should exist from setup function in before clause. got: ${b.length}`,
    );

    assert(
      areFloatsEqual(b[0].tokenPrice, 0.99, 1e-4),
      `l2BidPrice: want: 0.99 got: ${b[0].tokenPrice}`,
    );
    assert(
      areFloatsEqual(b[1].tokenPrice, 0.98, 1e-4),
      `l2BidPrice: want: 0.98 got: ${b[1].tokenPrice}`,
    );
    assert(
      areFloatsEqual(b[2].tokenPrice, 0.97, 1e-4),
      `l2BidPrice: want: 0.97 got: ${b[2].tokenPrice}`,
    );
    assert(
      areFloatsEqual(b[3].tokenPrice, 0.96, 1e-4),
      `l2BidPrice: want: 0.96 got: ${b[3].tokenPrice}`,
    );
    assert(
      areFloatsEqual(b[4].tokenPrice, 0.95, 1e-4),
      `l2BidPrice: want: 0.95 got: ${b[4].tokenPrice}`,
    );
  });

  it('getAsksL2', () => {
    const a = market.asksL2();

    assert(
      a.length === 5,
      `5 l2 asks should exist from setup function in before clause. got: ${a.length}`,
    );

    assert(
      areFloatsEqual(a[0].tokenPrice, 1.01, 1e-4),
      `l2BidPrice: want: 1.01 got: ${a[0].tokenPrice}`,
    );
    assert(
      areFloatsEqual(a[1].tokenPrice, 1.02, 1e-4),
      `l2BidPrice: want: 1.02 got: ${a[0].tokenPrice}`,
    );
    assert(
      areFloatsEqual(a[2].tokenPrice, 1.03, 1e-4),
      `l2BidPrice: want: 1.03 got: ${a[0].tokenPrice}`,
    );
    assert(
      areFloatsEqual(a[3].tokenPrice, 1.04, 1e-4),
      `l2BidPrice: want: 1.04 got: ${a[0].tokenPrice}`,
    );
    assert(
      areFloatsEqual(a[4].tokenPrice, 1.05, 1e-4),
      `l2BidPrice: want: 1.05 got: ${a[0].tokenPrice}`,
    );
  });

  it('bestBidPrice', async () => {
    const price = market.bestBidPrice() || 0;
    assert(
      areFloatsEqual(price, 0.99, 1e-4),
      `bestBidPrice: want: 0.99 got: ${price}`,
    );
  });

  it('bestAskPrice', async () => {
    const price = market.bestAskPrice() || 0;
    assert(
      areFloatsEqual(price, 1.01, 1e-4),
      `bestAskPrice: want: 1.01 got: ${price}`,
    );
  });

  it('Market', async () => {
    await testMarket();
  });
});
