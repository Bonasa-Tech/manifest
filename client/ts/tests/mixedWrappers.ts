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
import { assert } from 'chai';
import { OrderType } from '../src/manifest';
import { deposit } from './deposit';
import {
  FIXED_WRAPPER_HEADER_SIZE,
  NO_EXPIRATION_LAST_VALID_SLOT,
} from '../src/constants';
import { airdropSol } from '../src/utils/solana';
import { ManifestClient } from '../src/client';
import {
  mintTo,
  createAssociatedTokenAccountIdempotent,
  getMint,
} from '@solana/spl-token';

// Import UI wrapper and Market from old SDK version
import {
  createCreateWrapperInstruction as createCreateUIWrapperInstruction,
  PROGRAM_ID as UI_WRAPPER_PROGRAM_ID,
} from '@bonasa-tech/manifest-sdk-old/dist/cjs/ui_wrapper';
import { UiWrapper } from '@bonasa-tech/manifest-sdk-old/dist/cjs/uiWrapperObj';
import { Market as OldMarket } from '@bonasa-tech/manifest-sdk-old/dist/cjs/market';

async function testMixedWrappers(): Promise<void> {
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
  console.log('Created market:', marketAddress.toBase58());
  market.prettyPrint();

  // Step 1: Create a UI wrapper using the old SDK
  console.log('\n=== Step 1: Creating UI wrapper and claiming seat ===');

  const uiWrapperKeypair: Keypair = Keypair.generate();
  await airdropSol(connection, payerKeypair.publicKey);

  const createUIWrapperAccountIx: TransactionInstruction =
    SystemProgram.createAccount({
      fromPubkey: payerKeypair.publicKey,
      newAccountPubkey: uiWrapperKeypair.publicKey,
      space: FIXED_WRAPPER_HEADER_SIZE,
      lamports: await connection.getMinimumBalanceForRentExemption(
        FIXED_WRAPPER_HEADER_SIZE,
      ),
      programId: UI_WRAPPER_PROGRAM_ID,
    });

  const createUIWrapperIx: TransactionInstruction =
    createCreateUIWrapperInstruction({
      owner: payerKeypair.publicKey,
      payer: payerKeypair.publicKey,
      wrapperState: uiWrapperKeypair.publicKey,
    });

  await sendAndConfirmTransaction(
    connection,
    new Transaction().add(createUIWrapperAccountIx).add(createUIWrapperIx),
    [payerKeypair, uiWrapperKeypair],
  );

  console.log('UI Wrapper created at:', uiWrapperKeypair.publicKey.toBase58());

  // Load the UI wrapper
  const uiWrapperAccountInfo = await connection.getAccountInfo(
    uiWrapperKeypair.publicKey,
  );
  if (!uiWrapperAccountInfo) {
    throw new Error('UI wrapper account not found');
  }
  const uiWrapper = UiWrapper.loadFromBuffer({
    address: uiWrapperKeypair.publicKey,
    buffer: uiWrapperAccountInfo.data,
  });

  // Load market using old SDK's Market class for compatibility with UiWrapper.placeOrderIx
  const oldMarket = await OldMarket.loadFromAddress({
    connection,
    address: marketAddress,
  });

  // Mint base tokens to the trader's associated token account
  // (UI wrapper will auto-deposit from this account when placing the sell order)
  const baseMint = market.baseMint();
  const traderBaseTokenAccount = await createAssociatedTokenAccountIdempotent(
    connection,
    payerKeypair,
    baseMint,
    payerKeypair.publicKey,
  );
  const baseMintDecimals = (await getMint(connection, baseMint)).decimals;
  const amountBaseAtoms = Math.ceil(100 * 10 ** baseMintDecimals);
  await mintTo(
    connection,
    payerKeypair,
    baseMint,
    traderBaseTokenAccount,
    payerKeypair.publicKey,
    amountBaseAtoms,
  );
  console.log('Minted base tokens to trader');

  // Place an order using UI wrapper's placeOrderIx method
  // This will claim the seat and auto-deposit base tokens
  const placeOrderUIIx = uiWrapper.placeOrderIx(
    oldMarket,
    {
      payer: payerKeypair.publicKey,
    },
    {
      isBid: false,
      amount: 10,
      price: 5,
      orderId: 0,
    },
  );

  await sendAndConfirmTransaction(
    connection,
    new Transaction().add(placeOrderUIIx),
    [payerKeypair],
  );

  console.log(
    'Successfully placed order using UI wrapper (seat claimed automatically)',
  );

  // Reload wrapper to see the order
  await uiWrapper.reload(connection);
  const uiWrapperMarketInfo = uiWrapper.marketInfoForMarket(marketAddress);
  assert(
    uiWrapperMarketInfo != null,
    'Expected to find market info for UI wrapper',
  );
  assert(
    uiWrapperMarketInfo!.orders.length === 1,
    'Expected 1 order from UI wrapper',
  );

  console.log('\n=== Step 2: Creating normal wrapper with same wallet ===');

  // Step 2: Use ManifestClient which will create a normal wrapper automatically
  // (since it only searches WRAPPER_PROGRAM_ID, not UI_WRAPPER_PROGRAM_ID)
  const client = await ManifestClient.getClientForMarket(
    connection,
    marketAddress,
    payerKeypair,
  );

  const normalWrapper = client.wrapper!;
  console.log('Normal Wrapper created at:', normalWrapper.address.toBase58());

  // Deposit to the market through the normal wrapper
  await deposit(
    connection,
    payerKeypair,
    marketAddress,
    market.baseMint(),
    100,
  );

  // Place an order using the normal wrapper via ManifestClient
  const placeOrderNormalIx = client.placeOrderIx({
    numBaseTokens: 15,
    tokenPrice: 6,
    isBid: false,
    lastValidSlot: NO_EXPIRATION_LAST_VALID_SLOT,
    orderType: OrderType.Limit,
    clientOrderId: 1,
  });

  await sendAndConfirmTransaction(
    connection,
    new Transaction().add(placeOrderNormalIx),
    [payerKeypair],
  );

  console.log('Successfully placed order using normal wrapper');

  // Reload both wrappers to see all orders
  await normalWrapper.reload(connection);
  await uiWrapper.reload(connection);

  const normalWrapperOrders = normalWrapper.openOrdersForMarket(marketAddress);
  await uiWrapper.reload(connection);
  const uiWrapperMarketInfoFinal = uiWrapper.marketInfoForMarket(marketAddress);

  // Verify results
  console.log('\n=== Test Results ===');
  console.log(
    'UI Wrapper orders:',
    uiWrapperMarketInfoFinal?.orders.length || 0,
  );
  console.log('Normal Wrapper orders:', normalWrapperOrders?.length || 0);

  // The test demonstrates that the same wallet can use both UI wrapper and normal wrapper
  assert(
    uiWrapperMarketInfoFinal != null &&
      uiWrapperMarketInfoFinal.orders.length >= 1,
    'Expected UI wrapper to have at least 1 order',
  );

  assert(
    normalWrapperOrders != null && normalWrapperOrders.length >= 1,
    'Expected normal wrapper to have at least 1 order',
  );

  console.log(
    '\n✅ Test passed: Same wallet successfully used both UI wrapper and normal wrapper',
  );

  // Pretty print both wrappers
  console.log('\n--- UI Wrapper State ---');
  uiWrapper.prettyPrint();

  console.log('\n--- Normal Wrapper State ---');
  normalWrapper.prettyPrint();

  // Reload and print the market to show both orders from the same wallet
  await market.reload(connection);
  console.log('\n--- Market State (showing both orders from same wallet) ---');
  market.prettyPrint();
}

describe('Mixed Wrappers test', () => {
  it('Should allow same wallet to use both UI wrapper and normal wrapper', async () => {
    await testMixedWrappers();
  });
});
