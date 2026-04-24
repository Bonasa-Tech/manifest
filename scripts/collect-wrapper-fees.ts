import 'dotenv/config';

import {
  Connection,
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
  TransactionInstruction,
  sendAndConfirmTransaction,
} from '@solana/web3.js';
import { PROGRAM_ID as WRAPPER_PROGRAM_ID } from '../client/ts/src/wrapper';
import * as fs from 'fs';

const { RPC_URL, KEYPAIR_PATH } = process.env;

if (!RPC_URL) {
  throw new Error('RPC_URL missing from env');
}

// The hardcoded authorized collector from the wrapper program
const AUTHORIZED_COLLECTOR = new PublicKey(
  'B6dmr2UAn2wgjdm3T4N1Vjd8oPYRRTguByW7AEngkeL6',
);

// Collect instruction discriminant from wrapper IDL
const COLLECT_DISCRIMINANT = 7;

function createCollectInstruction(
  wrapperState: PublicKey,
  collector: PublicKey,
): TransactionInstruction {
  const keys = [
    { pubkey: wrapperState, isSigner: false, isWritable: true },
    { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    { pubkey: collector, isSigner: true, isWritable: true },
  ];

  const data = Buffer.from([COLLECT_DISCRIMINANT]);

  return new TransactionInstruction({
    keys,
    programId: WRAPPER_PROGRAM_ID,
    data,
  });
}

const run = async () => {
  const connection = new Connection(RPC_URL!);

  // Load keypair
  let keypairPath = KEYPAIR_PATH;
  if (!keypairPath) {
    keypairPath = `${process.env.HOME}/.config/solana/id.json`;
  }

  if (!fs.existsSync(keypairPath)) {
    console.error(`Keypair file not found at ${keypairPath}`);
    console.error('Set KEYPAIR_PATH env var or use default solana keypair');
    process.exit(1);
  }

  const keypairData = JSON.parse(fs.readFileSync(keypairPath, 'utf-8'));
  const collector = Keypair.fromSecretKey(Uint8Array.from(keypairData));

  console.log(`Collector: ${collector.publicKey.toBase58()}`);

  if (!collector.publicKey.equals(AUTHORIZED_COLLECTOR)) {
    console.error(`\nError: Only the authorized collector can collect fees.`);
    console.error(`Authorized: ${AUTHORIZED_COLLECTOR.toBase58()}`);
    console.error(`Your key:   ${collector.publicKey.toBase58()}`);
    process.exit(1);
  }

  console.log('\nFetching all wrapper state accounts...');

  // Get all wrapper accounts
  const wrapperAccounts =
    await connection.getProgramAccounts(WRAPPER_PROGRAM_ID);

  console.log(`Found ${wrapperAccounts.length} wrapper accounts`);

  // Filter to accounts with collectable fees
  const collectableWrappers: { pubkey: PublicKey; amount: number }[] = [];

  for (const { pubkey, account } of wrapperAccounts) {
    const rent = await connection.getMinimumBalanceForRentExemption(
      account.data.length,
    );
    const collectableAmount = account.lamports - rent;
    if (collectableAmount > 0) {
      collectableWrappers.push({ pubkey, amount: collectableAmount });
    }
  }

  console.log(`\n${collectableWrappers.length} wrappers have collectable fees`);

  if (collectableWrappers.length === 0) {
    console.log('Nothing to collect.');
    return;
  }

  // Sort by amount descending
  collectableWrappers.sort((a, b) => b.amount - a.amount);

  let totalLamports = 0;
  for (const { pubkey, amount } of collectableWrappers) {
    totalLamports += amount;
    console.log(`  ${pubkey.toBase58()}: ${(amount / 1e9).toFixed(9)} SOL`);
  }

  console.log(`\nTotal collectable: ${(totalLamports / 1e9).toFixed(9)} SOL`);

  // Batch collect instructions (max ~10 per tx to stay under size limits)
  const BATCH_SIZE = 10;
  let collected = 0;
  let txCount = 0;

  for (let i = 0; i < collectableWrappers.length; i += BATCH_SIZE) {
    const batch = collectableWrappers.slice(i, i + BATCH_SIZE);
    const tx = new Transaction();

    for (const { pubkey } of batch) {
      tx.add(createCollectInstruction(pubkey, collector.publicKey));
    }

    try {
      const sig = await sendAndConfirmTransaction(connection, tx, [collector], {
        skipPreflight: false,
        commitment: 'confirmed',
      });
      txCount++;
      collected += batch.length;
      console.log(`\nTx ${txCount}: Collected from ${batch.length} wrappers`);
      console.log(`  Signature: ${sig}`);
    } catch (e) {
      console.error(`\nTx ${txCount + 1} failed:`, e);
    }
  }

  console.log(
    `\nDone! Collected from ${collected}/${collectableWrappers.length} wrappers in ${txCount} transactions`,
  );
};

run().catch((e) => {
  console.error('Error:', e);
  process.exit(1);
});
