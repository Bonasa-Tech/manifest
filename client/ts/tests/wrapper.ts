import {
  Connection,
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
  TransactionInstruction,
  sendAndConfirmTransaction,
} from '@solana/web3.js';
import { Market } from '../src/market';
import { createMarket } from './createMarket';
import { ManifestClient } from '../src';
import { assert } from 'chai';
import { placeOrder } from './placeOrder';
import { OrderType } from '../src/manifest';
import { deposit } from './deposit';
import {
  createCreateWrapperInstruction,
  PROGRAM_ID as WRAPPER_PROGRAM_ID,
} from '../src/wrapper';
import { Wrapper } from '../src/wrapperObj';
import { FIXED_WRAPPER_HEADER_SIZE } from '../src/constants';
import { airdropSol } from '../src/utils/solana';
import { describeIfDirectTest } from './helpers/mocha';

async function testWrapper(): Promise<void> {
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

  // Test loading fails on bad address
  try {
    await Wrapper.loadFromAddress({
      connection,
      address: Keypair.generate().publicKey,
    });
    assert(false, 'expected load from address fail');
  } catch (err) {
    assert(true, 'expected load from address fail');
  }

  // Test reloading successful.
  await wrapper.reload(connection);

  // Test reloading fail.
  try {
    await wrapper.reload(new Connection('https://api.devnet.solana.com'));
    assert(false, 'expected reload fail');
  } catch (err) {
    assert(true, 'expected reload fail');
  }

  // Wrapper successfully find market info
  assert(
    wrapper.marketInfoForMarket(marketAddress) != null,
    'expected non null market info for market',
  );
  // Wrapper fail find market info
  assert(
    wrapper.marketInfoForMarket(Keypair.generate().publicKey) == null,
    'expected null market info for market',
  );

  // Place an order to get more coverage on the pretty print.
  await deposit(connection, payerKeypair, marketAddress, market.baseMint(), 10);
  await placeOrder(
    connection,
    payerKeypair,
    marketAddress,
    5,
    5,
    false,
    OrderType.Limit,
    0,
  );

  await wrapper.reload(connection);
  // Wrapper successfully find open orders
  assert(
    wrapper.openOrdersForMarket(marketAddress) != null,
    'expected non null open orders for market',
  );
  // Wrapper fail find open orders
  assert(
    wrapper.openOrdersForMarket(Keypair.generate().publicKey) == null,
    'expected null open orders for market',
  );

  wrapper.prettyPrint();

  // Wrapper without a market for coverage.
  const payerKeypair2: Keypair = Keypair.generate();
  await airdropSol(connection, payerKeypair2.publicKey);
  const wrapperKeypair2: Keypair = Keypair.generate();
  const createAccountIx: TransactionInstruction = SystemProgram.createAccount({
    fromPubkey: payerKeypair2.publicKey,
    newAccountPubkey: wrapperKeypair2.publicKey,
    space: FIXED_WRAPPER_HEADER_SIZE,
    lamports: await connection.getMinimumBalanceForRentExemption(
      FIXED_WRAPPER_HEADER_SIZE,
    ),
    programId: WRAPPER_PROGRAM_ID,
  });
  const createWrapperIx: TransactionInstruction =
    createCreateWrapperInstruction({
      owner: payerKeypair2.publicKey,
      wrapperState: wrapperKeypair2.publicKey,
    });

  await sendAndConfirmTransaction(
    connection,
    new Transaction().add(createAccountIx).add(createWrapperIx),
    [payerKeypair2, wrapperKeypair2],
  );
  const wrapper2 = await Wrapper.loadFromAddress({
    connection,
    address: wrapperKeypair2.publicKey,
  });
  wrapper2.prettyPrint();

  // Check loading all markets with active seats for coverage.
  const allClients = await ManifestClient.getClientsReadOnlyForAllTraderSeats(
    connection,
    payerKeypair.publicKey,
  );
  assert(
    allClients.find((c) => c.market.address.equals(market.address)) !=
      undefined,
    'expected user to have a seat on new market',
  );
}

describeIfDirectTest(module, 'Wrapper test', () => {
  it('Wrapper', async () => {
    await testWrapper();
  });
});
