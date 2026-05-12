import {
  Connection,
  Keypair,
  sendAndConfirmTransaction,
  PublicKey,
  SystemProgram,
  TransactionInstruction,
  Transaction,
} from '@solana/web3.js';
import { ManifestClient } from '../src/client';
import { PROGRAM_ID } from '../src/manifest';
import { Market } from '../src/market';
import { airdropSol, getClusterFromConnection } from '../src/utils/solana';
import { createMint } from '@solana/spl-token';
import { FIXED_MANIFEST_HEADER_SIZE } from '../src/constants';
import { describeIfDirectTest } from './helpers/mocha';

async function testCreateMarket(): Promise<void> {
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
}

export async function createMarket(
  connection: Connection,
  payerKeypair: Keypair,
): Promise<PublicKey> {
  const marketKeypair: Keypair = Keypair.generate();
  console.log(`Cluster is ${await getClusterFromConnection(connection)}`);

  // Get SOL for rent and make airdrop states.
  await airdropSol(connection, payerKeypair.publicKey);
  const baseMint: PublicKey = await createMint(
    connection,
    payerKeypair,
    payerKeypair.publicKey,
    payerKeypair.publicKey,
    9,
  );
  const quoteMint: PublicKey = await createMint(
    connection,
    payerKeypair,
    payerKeypair.publicKey,
    payerKeypair.publicKey,
    6,
  );
  console.log(`Created baseMint ${baseMint} quoteMint ${quoteMint}`);

  const createAccountIx: TransactionInstruction = SystemProgram.createAccount({
    fromPubkey: payerKeypair.publicKey,
    newAccountPubkey: marketKeypair.publicKey,
    space: FIXED_MANIFEST_HEADER_SIZE,
    lamports: await connection.getMinimumBalanceForRentExemption(
      FIXED_MANIFEST_HEADER_SIZE,
    ),
    programId: PROGRAM_ID,
  });

  const createMarketIx = ManifestClient['createMarketIx'](
    payerKeypair.publicKey,
    baseMint,
    quoteMint,
    marketKeypair.publicKey,
  );

  const tx: Transaction = new Transaction();
  tx.add(createAccountIx);
  tx.add(createMarketIx);
  const signature = await sendAndConfirmTransaction(connection, tx, [
    payerKeypair,
    marketKeypair,
  ]);
  console.log(`Created market at ${marketKeypair.publicKey} in ${signature}`);
  return marketKeypair.publicKey;
}

describeIfDirectTest(module, 'Create Market test', () => {
  it('Create Market', async () => {
    await testCreateMarket();
  });
});
