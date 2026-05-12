import { Connection, Keypair, PublicKey } from '@solana/web3.js';
import { OrderType } from '../src/manifest/types';
import { createMarket } from './createMarket';
import { deposit } from './deposit';
import { Market } from '../src/market';
import { assert } from 'chai';
import { placeOrder } from './placeOrder';
import { describeIfDirectTest } from './helpers/mocha';

async function testExpiredAsk(): Promise<void> {
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

  await deposit(connection, payerKeypair, marketAddress, market.baseMint(), 10);
  // Fails to place obviously already expired order.
  try {
    await placeOrder(
      connection,
      payerKeypair,
      marketAddress,
      5,
      5,
      false,
      OrderType.Limit,
      0,
      20,
    );
    assert(false);
  } catch (err) {
    assert(true);
  }
}

describeIfDirectTest(module, 'Expired Order test', () => {
  it('Expired Ask', async () => {
    await testExpiredAsk();
  });
});
