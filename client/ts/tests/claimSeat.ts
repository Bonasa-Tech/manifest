import { Connection, Keypair, PublicKey } from '@solana/web3.js';
import { ManifestClient } from '../src/client';
import { createMarket } from './createMarket';
import { Market } from '../src/market';
import { assert } from 'chai';
import { describeIfDirectTest } from './helpers/mocha';

async function testClaimSeat(): Promise<void> {
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
  market.prettyPrint();

  await claimSeat(connection, marketAddress, payerKeypair);

  const marketUpdated: Market = await Market.loadFromAddress({
    connection,
    address: marketAddress,
  });
  marketUpdated.prettyPrint();
  assert(
    marketUpdated.hasSeat(payerKeypair.publicKey),
    'claim seat did not have the seat claimed',
  );

  // Claiming on a second market. There is a wrapper, but not a claimed seat.
  const marketAddress2: PublicKey = await createMarket(
    connection,
    payerKeypair,
  );
  const market2: Market = await Market.loadFromAddress({
    connection,
    address: marketAddress2,
  });
  market2.prettyPrint();

  await claimSeat(connection, marketAddress2, payerKeypair);

  const marketUpdated2: Market = await Market.loadFromAddress({
    connection,
    address: marketAddress2,
  });
  marketUpdated2.prettyPrint();
  assert(
    marketUpdated2.hasSeat(payerKeypair.publicKey),
    'claim seat did not have the seat claimed on second seat',
  );

  // Test loading without needing to initialize on chain.
  await Market.loadFromAddress({
    connection,
    address: marketAddress2,
  });
}

export async function claimSeat(
  connection: Connection,
  market: PublicKey,
  payerKeypair: Keypair,
): Promise<void> {
  await ManifestClient.getClientForMarket(connection, market, payerKeypair);
}

describeIfDirectTest(module, 'Claim Seat test', () => {
  it('Claim seat', async () => {
    await testClaimSeat();
  });
});
