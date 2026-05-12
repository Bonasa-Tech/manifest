import {
  Connection,
  Keypair,
  sendAndConfirmTransaction,
  PublicKey,
  Transaction,
  TransactionInstruction,
} from '@solana/web3.js';
import { ManifestClient } from '../src/client';
import {
  mintTo,
  createAssociatedTokenAccountIdempotent,
  getMint,
  createMint,
} from '@solana/spl-token';
import { createGlobal } from './createGlobal';
import { Global } from '../src/global';
import { assert } from 'chai';
import { getGlobalAddress } from '../src/utils/global';
import { airdropSol } from '../src/utils/solana';
import { waitForTokenAccount } from './helpers/tokenAccount';
import { describeIfDirectTest } from './helpers/mocha';

async function testGlobalDeposit(): Promise<void> {
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

  await global.reload(connection);
  assert(
    (await global.getGlobalBalanceTokens(connection, payerKeypair.publicKey)) ==
      10,
    'deposit global balance check',
  );
  assert(
    global.getGlobalBalanceTokensWithDecimals(payerKeypair.publicKey, 9) == 10,
    'deposit global balance with decimals check',
  );
  global.prettyPrint();
}

export async function depositGlobal(
  connection: Connection,
  traderKeypair: Keypair,
  mint: PublicKey,
  amountTokens: number,
  mintAuthorityKeypair: Keypair,
): Promise<void> {
  const globalAddTraderIx: TransactionInstruction =
    ManifestClient.createGlobalAddTraderIx(traderKeypair.publicKey, mint);

  const globalDepositIx: TransactionInstruction =
    await ManifestClient.globalDepositIx(
      connection,
      traderKeypair.publicKey,
      mint,
      amountTokens,
    );

  const traderTokenAccount: PublicKey =
    await createAssociatedTokenAccountIdempotent(
      connection,
      traderKeypair,
      mint,
      traderKeypair.publicKey,
    );
  await waitForTokenAccount(connection, traderTokenAccount);

  const mintDecimals: number = (await getMint(connection, mint)).decimals;
  const amountAtoms: number = Math.ceil(amountTokens * 10 ** mintDecimals);
  const mintSig: string = await mintTo(
    connection,
    traderKeypair,
    mint,
    traderTokenAccount,
    mintAuthorityKeypair,
    amountAtoms,
  );
  console.log(
    `Minted ${amountTokens} tokens to ${traderTokenAccount} in ${mintSig}`,
  );

  const signature: string = await sendAndConfirmTransaction(
    connection,
    new Transaction().add(globalAddTraderIx, globalDepositIx),
    [traderKeypair],
    {
      commitment: 'confirmed',
    },
  );
  console.log(
    `Global Add Trader & Deposited ${amountTokens} tokens in ${signature}`,
  );
}

describeIfDirectTest(module, 'Global Deposit test', () => {
  it('Global Deposit', async () => {
    await testGlobalDeposit();
  });
});
