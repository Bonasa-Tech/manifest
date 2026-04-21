import 'dotenv/config';

import { Keypair } from '@solana/web3.js';
import bs58 from 'bs58';

const {
  MARKET_CREATOR_PRIVATE_KEY,
  ALICE_PRIVATE_KEY,
  BOB_PRIVATE_KEY,
  CHARLIE_PRIVATE_KEY,
} = process.env;

if (!MARKET_CREATOR_PRIVATE_KEY) {
  throw new Error('MARKET_CREATOR_PRIVATE_KEY missing from env');
}

if (!ALICE_PRIVATE_KEY) {
  throw new Error('ALICE_PRIVATE_KEY missing from env');
}

if (!BOB_PRIVATE_KEY) {
  throw new Error('BOB_PRIVATE_KEY missing from env');
}

if (!CHARLIE_PRIVATE_KEY) {
  throw new Error('CHARLIE_PRIVATE_KEY missing from env');
}

const run = async () => {
  // iAhWd4nDdrzj1jtkceFZXhQCqH8QZF2YrVPktoLVV6ifoFqg7QqZR97mg4HwBwGWpu89uhqVr8E1VH27gsAv7ey
  const marketCreatorPk = Keypair.fromSecretKey(
    Uint8Array.from(MARKET_CREATOR_PRIVATE_KEY.split(',').map(Number)),
  );

  const marketCreatorB58 = bs58.encode(marketCreatorPk.secretKey);
  console.log('marketCreator: ');
  console.log(marketCreatorB58);

  const alicePk = Keypair.fromSecretKey(
    Uint8Array.from(ALICE_PRIVATE_KEY.split(',').map(Number)),
  );

  const aliceB58 = bs58.encode(alicePk.secretKey);
  console.log('alice: ');
  console.log(aliceB58);

  const bobPk = Keypair.fromSecretKey(
    Uint8Array.from(BOB_PRIVATE_KEY.split(',').map(Number)),
  );

  const bobB58 = bs58.encode(bobPk.secretKey);
  console.log('bob: ');
  console.log(bobB58);

  const charliePk = Keypair.fromSecretKey(
    Uint8Array.from(CHARLIE_PRIVATE_KEY.split(',').map(Number)),
  );

  const charlieB58 = bs58.encode(charliePk.secretKey);
  console.log('charlie: ');
  console.log(charlieB58);
};

run().catch(console.error);
