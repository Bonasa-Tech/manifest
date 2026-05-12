import { Connection, Keypair, PublicKey } from '@solana/web3.js';
import { ManifestClient } from '../src/client';
import { Market } from '../src/market';
import { assert } from 'chai';
import { createMarket } from './createMarket';
import { describeIfDirectTest } from './helpers/mocha';

async function testListMarket(): Promise<void> {
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

  const marketPks: PublicKey[] = await ManifestClient.listMarketsForMints(
    connection,
    market.baseMint(),
    market.quoteMint(),
  );
  assert(marketPks.length == 1);
  assert(marketPks[0].toBase58() == marketAddress.toBase58());
}

describeIfDirectTest(module, 'List Market test', () => {
  it('List Market', async () => {
    await testListMarket();
  });
});
