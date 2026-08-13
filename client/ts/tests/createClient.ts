import {
  Connection,
  Keypair,
  PublicKey,
  Transaction,
  sendAndConfirmTransaction,
} from '@solana/web3.js';
import { createMarket } from './createMarket';
import { ManifestClient } from '../src';
import { assert } from 'chai';
import { describeIfDirectTest } from './helpers/mocha';

async function testGetClientForMarketNoPrivateKey(
  connection: Connection,
  marketAddress: PublicKey,
  payerKeypair: Keypair,
  shouldCrash: boolean,
): Promise<void> {
  let crashed = false;
  try {
    await ManifestClient.getClientForMarketNoPrivateKey(
      connection,
      marketAddress,
      payerKeypair.publicKey,
    );
  } catch (e) {
    crashed = true;
    console.log(e);
  }

  if (shouldCrash) {
    assert(
      crashed,
      'getClientForMarketNoPrivateKey should crash if setup ixs not executed',
    );
  } else {
    assert(
      !crashed,
      'getClientForMarketNoPrivateKey should NOT crash if setup ixs executed',
    );
  }
}

async function testGetSetupIxs(
  connection: Connection,
  marketAddress: PublicKey,
  payerKeypair: Keypair,
  shouldBeNeeded: boolean,
  shouldGiveWrapperState: boolean,
) {
  const { setupNeeded, instructions, wrapperState } =
    await ManifestClient.getSetupIxs(
      connection,
      marketAddress,
      payerKeypair.publicKey,
    );
  assert(
    shouldBeNeeded === setupNeeded,
    `setupNeeded should be ${shouldBeNeeded} but was ${setupNeeded}`,
  );

  if (!setupNeeded) {
    console.log('setupIxs not needed. returning early...');
    return;
  }

  assert(
    !!wrapperState === shouldGiveWrapperState,
    `wrapperState should be ${shouldGiveWrapperState ? 'not-null' : 'null'}`,
  );

  // The wrapper account is now created with createAccountWithSeed, so the payer
  // is the only required signer (no ephemeral wrapper keypair).
  const signature = await sendAndConfirmTransaction(
    connection,
    new Transaction().add(...instructions),
    [payerKeypair],
  );

  console.log(`executed setupIxs: ${signature}`);
}

describeIfDirectTest(
  module,
  'when creating a client using getClientForMarketNoPrivateKey',
  () => {
    let connection: Connection;
    let payerKeypair: Keypair;
    let marketAddress: PublicKey;

    before(async () => {
      connection = new Connection('http://127.0.0.1:8899', 'confirmed');
      payerKeypair = Keypair.generate();
      marketAddress = await createMarket(connection, payerKeypair);
    });

    it('should crash if setupIxs NOT executed', async () => {
      await testGetClientForMarketNoPrivateKey(
        connection,
        marketAddress,
        payerKeypair,
        true,
      );
    });

    it('should get setupIxs using getSetupIxs and execute successfully', async () => {
      await testGetSetupIxs(
        connection,
        marketAddress,
        payerKeypair,
        true,
        true,
      );
    });

    it('should wait 15 seconds to let state catch up', async () => {
      await new Promise((resolve) => setTimeout(resolve, 15_000));
    });

    it('should NOT crash if setupIxs already executed', async () => {
      await testGetClientForMarketNoPrivateKey(
        connection,
        marketAddress,
        payerKeypair,
        false,
      );
    });
  },
);
