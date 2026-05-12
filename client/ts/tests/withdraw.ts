import {
  Connection,
  Keypair,
  sendAndConfirmTransaction,
  PublicKey,
  Transaction,
} from '@solana/web3.js';
import { ManifestClient } from '../src/client';
import { createMarket } from './createMarket';
import { deposit } from './deposit';
import { Market } from '../src/market';
import { assert } from 'chai';
import { describeIfDirectTest } from './helpers/mocha';

async function testWithdraw(): Promise<void> {
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
  await withdraw(connection, payerKeypair, marketAddress, market.baseMint(), 5);

  await market.reload(connection);
  assert(
    market.getWithdrawableBalanceTokens(payerKeypair.publicKey, true) == 5,
    'withdraw withdrawable balance check base',
  );
  assert(
    market.getWithdrawableBalanceTokens(payerKeypair.publicKey, false) == 0,
    'withdraw withdrawable balance check quote',
  );
  market.prettyPrint();
}

export async function withdraw(
  connection: Connection,
  payerKeypair: Keypair,
  marketAddress: PublicKey,
  mint: PublicKey,
  amountTokens: number,
): Promise<void> {
  const client: ManifestClient = await ManifestClient.getClientForMarket(
    connection,
    marketAddress,
    payerKeypair,
  );
  const withdrawIx = client.withdrawIx(
    payerKeypair.publicKey,
    mint,
    amountTokens,
  );

  const signature = await sendAndConfirmTransaction(
    connection,
    new Transaction().add(withdrawIx),
    [payerKeypair],
  );
  console.log(`Withdrew ${amountTokens} tokens in ${signature}`);
}

describeIfDirectTest(module, 'Withdraw test', () => {
  it('Withdraw', async () => {
    await testWithdraw();
  });
});
