import { Connection, Keypair, PublicKey } from '@solana/web3.js';
import { ManifestClient } from '../src/client';
import { OrderType } from '../src/manifest/types';
import { createMarket } from './createMarket';
import { deposit } from './deposit';
import { Market } from '../src/market';
import { assert } from 'chai';
import { placeOrder } from './placeOrder';
import { WrapperMarketInfo, Wrapper } from '../src';
import { describeIfDirectTest } from './helpers/mocha';

async function testVolume(): Promise<void> {
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
  await deposit(
    connection,
    payerKeypair,
    marketAddress,
    market.quoteMint(),
    10,
  );
  await placeOrder(
    connection,
    payerKeypair,
    marketAddress,
    1,
    1,
    true,
    OrderType.Limit,
    0,
  );
  await placeOrder(
    connection,
    payerKeypair,
    marketAddress,
    1,
    1,
    false,
    OrderType.Limit,
    1,
  );

  await market.reload(connection);

  const client: ManifestClient = await ManifestClient.getClientForMarket(
    connection,
    marketAddress,
    payerKeypair,
  );
  await client.reload();

  // Test loading successfully.
  const wrapper: Wrapper = await Wrapper.loadFromAddress({
    connection,
    address: client.wrapper!.address,
  });
  const marketInfoParsed: WrapperMarketInfo =
    wrapper.marketInfoForMarket(marketAddress)!;

  // 2 because self trade.
  assert(
    Number(marketInfoParsed.quoteVolumeAtoms) == 2_000_000,
    'quote volume on wrapper',
  );
}

describeIfDirectTest(module, 'Volume test', () => {
  it('Volume', async () => {
    await testVolume();
  });
});
