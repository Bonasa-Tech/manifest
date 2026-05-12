import {
  Connection,
  Keypair,
  sendAndConfirmTransaction,
  PublicKey,
  Transaction,
} from '@solana/web3.js';
import { ManifestClient } from '../src/client';
import { createGlobal } from './createGlobal';
import { depositGlobal } from './globalDeposit';
import { Global } from '../src/global';
import { assert } from 'chai';
import { getGlobalAddress } from '../src/utils/global';
import { createMint } from '@solana/spl-token';
import { airdropSol } from '../src/utils/solana';
import { describeIfDirectTest } from './helpers/mocha';

async function testGlobalWithdraw(): Promise<void> {
  const connection: Connection = new Connection(
    'http://127.0.0.1:8899',
    'confirmed',
  );
  const payerKeypair: Keypair = Keypair.generate();
  // Get SOL for rent.
  await airdropSol(connection, payerKeypair.publicKey);

  const tokenMint: PublicKey = await createMint(
    connection,
    payerKeypair,
    payerKeypair.publicKey,
    payerKeypair.publicKey,
    9,
  );
  await createGlobal(connection, payerKeypair, tokenMint);

  const global: Global = (await Global.loadFromAddress({
    connection,
    address: getGlobalAddress(tokenMint),
  }))!;

  await depositGlobal(
    connection,
    payerKeypair,
    global.tokenMint(),
    10,
    payerKeypair,
  );
  await withdrawGlobal(connection, payerKeypair, global.tokenMint(), 5);

  await global.reload(connection);
  assert(
    (await global.getGlobalBalanceTokens(connection, payerKeypair.publicKey)) ==
      5,
    'global withdraw balance check base',
  );
  global.prettyPrint();
}

export async function withdrawGlobal(
  connection: Connection,
  payerKeypair: Keypair,
  mint: PublicKey,
  amountTokens: number,
): Promise<void> {
  const globalwithdrawIx = await ManifestClient.globalWithdrawIx(
    connection,
    payerKeypair.publicKey,
    mint,
    amountTokens,
  );

  const signature = await sendAndConfirmTransaction(
    connection,
    new Transaction().add(globalwithdrawIx),
    [payerKeypair],
    {
      commitment: 'confirmed',
    },
  );
  console.log(`Global Withdrew ${amountTokens} tokens in ${signature}`);
}

describeIfDirectTest(module, 'Global Withdraw test', () => {
  it('Global Withdraw', async () => {
    await testGlobalWithdraw();
  });
});
