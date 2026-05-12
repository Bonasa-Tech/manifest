import {
  Connection,
  Keypair,
  sendAndConfirmTransaction,
  PublicKey,
  Transaction,
  TransactionInstruction,
} from '@solana/web3.js';
import { ManifestClient } from '../src/client';
import { OrderType } from '../src/manifest/types';
import { createMarket } from './createMarket';
import { deposit } from './deposit';
import { Market } from '../src/market';
import { assert } from 'chai';
import {
  createAssociatedTokenAccountIdempotent,
  mintTo,
} from '@solana/spl-token';
import { waitForTokenAccount } from './helpers/tokenAccount';
import { describeIfDirectTest } from './helpers/mocha';

async function testDepositPlaceOrder(): Promise<void> {
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
  await market.reload(connection);
  // No deposit should be needed here
  await depositPlaceOrder(
    connection,
    payerKeypair,
    marketAddress,
    4,
    5,
    false,
    OrderType.Limit,
    0,
  );
  const traderTokenAccount = await createAssociatedTokenAccountIdempotent(
    connection,
    payerKeypair,
    market.quoteMint(),
    payerKeypair.publicKey,
  );
  await waitForTokenAccount(connection, traderTokenAccount);
  const quoteSize = 3;
  const quotePrice = 3;
  const quoteNotional = quotePrice * quoteSize;
  const amountAtoms = Math.ceil(quoteNotional * 10 ** market.quoteDecimals());
  const mintSig = await mintTo(
    connection,
    payerKeypair,
    market.quoteMint(),
    traderTokenAccount,
    payerKeypair.publicKey,
    amountAtoms,
  );
  console.log('Minted quote tokens', mintSig);
  await depositPlaceOrder(
    connection,
    payerKeypair,
    marketAddress,
    quoteSize,
    quotePrice,
    true,
    OrderType.Limit,
    1,
  );

  await market.reload(connection);
  market.prettyPrint();

  // Asks are sorted worst to best.
  assert(market.asks().length == 1, 'place asks did not work');
  assert(
    Number(market.asks()[0].numBaseTokens) == 4,
    'ask top of book wrong size',
  );
  assert(
    market.asks()[0].tokenPrice == 5,
    `ask top of book wrong price ${market.asks()[0].tokenPrice}`,
  );
  assert(market.bids().length == 1, 'place bids did not work');
  assert(
    market.getWithdrawableBalanceTokens(payerKeypair.publicKey, true) == 6,
    'withdraw withdrawable balance check base',
  );
  assert(
    market.getWithdrawableBalanceTokens(payerKeypair.publicKey, false) == 0,
    'withdraw withdrawable balance check quote',
  );

  assert(
    JSON.stringify(market.getBalances(payerKeypair.publicKey)) ==
      JSON.stringify({
        baseWithdrawableBalanceTokens: 6,
        quoteWithdrawableBalanceTokens: 0,
        baseOpenOrdersBalanceTokens: 4,
        quoteOpenOrdersBalanceTokens: 9,
      }),
    `getBalances failed expected ${JSON.stringify({
      baseWithdrawableBalanceTokens: 6,
      quoteWithdrawableBalanceTokens: 0,
      baseOpenOrdersBalanceTokens: 4,
      quoteOpenOrdersBalanceTokens: 9,
    })} actual ${JSON.stringify(market.getBalances(payerKeypair.publicKey))}`,
  );
}

export async function depositPlaceOrder(
  connection: Connection,
  payerKeypair: Keypair,
  marketAddress: PublicKey,
  numBaseTokens: number,
  tokenPrice: number,
  isBid: boolean,
  orderType: OrderType,
  clientOrderId: number,
  lastValidSlot: number = 0,
): Promise<void> {
  const client: ManifestClient = await ManifestClient.getClientForMarket(
    connection,
    marketAddress,
    payerKeypair,
  );

  const depositPlaceOrderIx: TransactionInstruction[] =
    await client.placeOrderWithRequiredDepositIxs(payerKeypair.publicKey, {
      numBaseTokens,
      tokenPrice,
      isBid,
      lastValidSlot: lastValidSlot,
      orderType: orderType,
      clientOrderId,
    });

  const signature = await sendAndConfirmTransaction(
    connection,
    new Transaction().add(...depositPlaceOrderIx),
    [payerKeypair],
  );
  console.log(`Required Deposit and Placed order in ${signature}`);
}

describeIfDirectTest(module, 'Deposit Place Order test', () => {
  it('Deposit Place Order', async () => {
    await testDepositPlaceOrder();
  });
});
