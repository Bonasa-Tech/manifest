import 'dotenv/config';

import {
  PublicKey,
  Connection,
  Keypair,
  sendAndConfirmTransaction,
  Transaction,
  TransactionInstruction,
  SystemProgram,
  SendTransactionError,
} from '@solana/web3.js';
import { ManifestClient } from '@bonasa-tech/manifest-sdk';
import { Market } from '@bonasa-tech/manifest-sdk/market';
import {
  createAssociatedTokenAccountIdempotent,
  createMint,
  getAssociatedTokenAddressSync,
  mintTo,
} from '@solana/spl-token';
import {
  airdropSol,
  getClusterFromConnection,
} from '@bonasa-tech/manifest-sdk/utils/solana';
import { OrderType, PROGRAM_ID } from '@bonasa-tech/manifest-sdk/manifest';
import { FIXED_MANIFEST_HEADER_SIZE } from '@bonasa-tech/manifest-sdk/constants';

const {
  RPC_URL,
  MARKET_CREATOR_PRIVATE_KEY,
  MARKET_ADDRESS,
  ALICE_PRIVATE_KEY,
  BOB_PRIVATE_KEY,
  CHARLIE_PRIVATE_KEY,
} = process.env;

if (!RPC_URL) {
  throw new Error('RPC_URL missing from env');
}

if (!MARKET_CREATOR_PRIVATE_KEY) {
  console.warn('MARKET_CREATOR_PRIVATE_KEY missing from env. using random.');
}

if (!MARKET_ADDRESS) {
  console.warn('MARKET_ADDRESS missing from env');
}

if (!ALICE_PRIVATE_KEY) {
  console.warn('ALICE_PRIVATE_KEY missing from env. using random.');
}

if (!BOB_PRIVATE_KEY) {
  console.warn('BOB_PRIVATE_KEY missing from env. using random.');
}

if (!CHARLIE_PRIVATE_KEY) {
  console.warn('CHARLIE_PRIVATE_KEY missing from env. using random.');
}

class DummyTrader {
  constructor(
    public connection: Connection,
    public keypair: Keypair,
    public marketAddress: PublicKey,
    public name: string,
    private commands: string[],
    private client: ManifestClient,
  ) {}

  public static async create(
    connection: Connection,
    keypair: Keypair,
    marketAddress: PublicKey,
    name: string,
    commands: string[],
  ) {
    const client: ManifestClient = await ManifestClient.getClientForMarket(
      connection,
      marketAddress,
      keypair,
    );
    return new DummyTrader(
      connection,
      keypair,
      marketAddress,
      name,
      commands,
      client,
    );
  }

  public async run(): Promise<void> {
    // Randomly take an action.
    console.log('Running');
    // eslint-disable-next-line no-constant-condition
    while (true) {
      await new Promise((f) => setTimeout(f, 1000));
      const market: Market = await Market.loadFromAddress({
        connection: this.connection,
        address: this.marketAddress,
      });
      market.prettyPrint();

      const randomFunction =
        this.commands[Math.floor(Math.random() * this.commands.length)];

      // Try catch all of the functions because they could fail in
      // getAccountInfo due to transient RPC errors.
      switch (randomFunction) {
        case 'DEPOSIT': {
          console.log(`${this.name} trying to deposit`);
          try {
            await this.deposit();
          } catch (err) {}
          break;
        }
        case 'WITHDRAW': {
          console.log(`${this.name} trying to withdraw`);
          try {
            await this.withdraw();
          } catch (err) {}
          break;
        }
        case 'PLACE_ORDER': {
          console.log(`${this.name} trying to place order`);
          try {
            await this.placeOrder();
          } catch (err) {}
          break;
        }
        case 'SWAP': {
          console.log(`${this.name} trying to swap`);
          try {
            await this.swap();
          } catch (err) {}
          break;
        }
        case 'CANCEL_ORDER': {
          console.log(`${this.name} trying to cancel order`);
          try {
            await this.cancelOrder();
          } catch (err) {}
          break;
        }
      }
    }
  }

  async deposit(): Promise<void> {
    const market: Market = await Market.loadFromAddress({
      connection: this.connection,
      address: this.marketAddress,
    });
    if (!market.hasSeat(this.keypair.publicKey)) {
      console.log('Cannot deposit because does not have seat');
      return;
    }

    const mints = [market.baseMint(), market.quoteMint()];
    const randomMint = mints[Math.floor(Math.random() * mints.length)];

    const traderTokenAccount = getAssociatedTokenAddressSync(
      randomMint,
      this.keypair.publicKey,
    );
    const walletTokens: number = Number(
      (await this.connection.getTokenAccountBalance(traderTokenAccount)).value
        .uiAmount,
    );
    // Deposit at most half
    const depositAmountTokens: number = Math.floor(
      (Math.random() * walletTokens) / 2,
    );
    const depositIx = this.client.depositIx(
      this.keypair.publicKey,
      randomMint,
      depositAmountTokens,
    );
    try {
      const signature = await sendAndConfirmTransaction(
        this.connection,
        new Transaction().add(depositIx),
        [this.keypair],
        {
          skipPreflight: true,
        },
      );
      console.log(
        `${this.name} deposited ${depositAmountTokens} ${randomMint.toBase58()} tokens in ${signature}`,
      );
    } catch (err) {
      // Wait in case block takes time to propagate.
      await new Promise((f) => setTimeout(f, 1_000));
      const logs = await (err as SendTransactionError).getLogs(this.connection);
      console.log('SendTransactionError logs', logs);
      throw err;
    }
  }

  async withdraw(): Promise<void> {
    await new Promise((f) => setTimeout(f, 5_000));
    const market: Market = await Market.loadFromAddress({
      connection: this.connection,
      address: this.marketAddress,
    });

    const mints = [market.baseMint(), market.quoteMint()];
    const randomMint = mints[Math.floor(Math.random() * mints.length)];

    const balanceTokens = market.getWithdrawableBalanceTokens(
      this.keypair.publicKey,
      randomMint.toBase58() == market.baseMint().toBase58(),
    );
    if (balanceTokens == 0) {
      return;
    }

    const withdrawAmountTokens: number = Math.floor(
      (Math.random() * balanceTokens) / 2,
    );
    const withdrawIx = this.client.withdrawIx(
      this.keypair.publicKey,
      randomMint,
      withdrawAmountTokens,
    );
    try {
      const signature = await sendAndConfirmTransaction(
        this.connection,
        new Transaction().add(withdrawIx),
        [this.keypair],
        {
          skipPreflight: true,
        },
      );
      console.log(
        `${this.name} withdrew ${withdrawAmountTokens} ${randomMint.toBase58()} tokens in ${signature}`,
      );
    } catch (err) {
      console.log('Failed to withdraw, likely stale read of balance');
    }
  }

  async cancelOrder(): Promise<void> {
    const market: Market = await Market.loadFromAddress({
      connection: this.connection,
      address: this.marketAddress,
    });
    const orders = [...market.bids(), ...market.asks()];
    const myOrders = orders.filter((restingOrder) => {
      return (
        restingOrder.trader.toBase58() == this.keypair.publicKey.toBase58()
      );
    });
    if (myOrders.length == 0) {
      console.log('No orders to cancel');
      return;
    }
    const orderToCancel = myOrders[Math.floor(Math.random() * myOrders.length)];

    const cancelOrderIx: TransactionInstruction = this.client.cancelOrderIx({
      clientOrderId: 0,
    });
    const signature = await sendAndConfirmTransaction(
      this.connection,
      new Transaction().add(cancelOrderIx),
      [this.keypair],
      {
        skipPreflight: true,
      },
    );
    console.log(
      `${this.name} cancelled ${Number(orderToCancel.sequenceNumber)} in ${signature}`,
    );
  }

  async placeOrder(): Promise<void> {
    const market: Market = await Market.loadFromAddress({
      connection: this.connection,
      address: this.marketAddress,
    });

    const mints: PublicKey[] = [market.baseMint(), market.quoteMint()];
    const randomMint: PublicKey =
      mints[Math.floor(Math.random() * mints.length)];

    const isBid: boolean =
      randomMint.toBase58() == market.quoteMint().toBase58();
    const balanceTokens: number = market.getWithdrawableBalanceTokens(
      this.keypair.publicKey,
      !isBid,
    );
    if (balanceTokens == 0) {
      console.log('No funds on exchange to place order');
      return;
    }

    const priceTokens: number = 100.0 * (1 + (0.5 - Math.random()) / 10);
    const maxTokens: number = isBid
      ? balanceTokens / priceTokens
      : balanceTokens;
    const baseTokensInOrder: number = Math.floor(
      (Math.random() * maxTokens) / 2,
    );

    let orderType = OrderType.Limit;
    switch (this.name) {
      case 'bob': {
        orderType = OrderType.PostOnly;
        break;
      }
      case 'charlie': {
        orderType = OrderType.ImmediateOrCancel;
        break;
      }
    }
    const placeOrderIx: TransactionInstruction = this.client.placeOrderIx({
      isBid,
      lastValidSlot: 0,
      orderType,
      clientOrderId: 0,
      numBaseTokens: baseTokensInOrder,
      tokenPrice: priceTokens,
    });
    const signature: string = await sendAndConfirmTransaction(
      this.connection,
      new Transaction().add(placeOrderIx),
      [this.keypair],
      {
        skipPreflight: true,
      },
    );
    console.log(
      `${this.name} placed order for ${baseTokensInOrder}@${priceTokens} in ${signature}`,
    );
  }

  async swap(): Promise<void> {
    const market: Market = await Market.loadFromAddress({
      connection: this.connection,
      address: this.marketAddress,
    });

    const mints: PublicKey[] = [market.baseMint(), market.quoteMint()];
    const randomMint: PublicKey =
      mints[Math.floor(Math.random() * mints.length)];

    const isBid: boolean =
      randomMint.toBase58() == market.quoteMint().toBase58();
    const traderTokenAccount = getAssociatedTokenAddressSync(
      randomMint,
      this.keypair.publicKey,
    );
    const balanceTokens: number = Number(
      (await this.connection.getTokenAccountBalance(traderTokenAccount)).value
        .uiAmount,
    );
    if (balanceTokens == 0) {
      console.log('No balance for placing order from wallet');
      return;
    }

    const priceTokens: number = 100.0 * (1 + (0.5 - Math.random()) / 10);
    const maxBaseTokens: number = isBid
      ? balanceTokens / priceTokens
      : balanceTokens;
    const baseTokensInOrder: number = Math.floor(
      (Math.random() * maxBaseTokens) / 4,
    );
    const quoteTokensInOrder: number = Math.floor(
      baseTokensInOrder * priceTokens,
    );

    const swapIx: TransactionInstruction = this.client.swapIx(
      this.keypair.publicKey,
      {
        inAtoms: isBid ? quoteTokensInOrder : baseTokensInOrder,
        outAtoms: 0,
        isBaseIn: isBid,
        isExactIn: true,
      },
    );
    const signature: string = await sendAndConfirmTransaction(
      this.connection,
      new Transaction().add(swapIx),
      [this.keypair],
      {
        skipPreflight: true,
      },
    );
    console.log(
      `${this.name} swap for ${baseTokensInOrder}@${priceTokens} in ${signature}`,
    );
  }
}

async function fundWallet(
  connection: Connection,
  recipientKeypair: Keypair,
  mintAuthorityKeypair: Keypair,
  mint: PublicKey,
) {
  console.log('Funding wallet');
  // Get SOL for gas.
  const solBal = await connection.getBalance(recipientKeypair.publicKey);
  if (solBal == 0) {
    console.log('requesting airdrop');
    await airdropSol(connection, recipientKeypair.publicKey);
  }

  const traderTokenAccount: PublicKey =
    await createAssociatedTokenAccountIdempotent(
      connection,
      recipientKeypair,
      mint,
      recipientKeypair.publicKey,
    );
  const amountAtoms: number = 1_000_000_000_000;

  console.log('minting tokens');

  const mintSig: string = await mintTo(
    connection,
    mintAuthorityKeypair,
    mint,
    traderTokenAccount,
    mintAuthorityKeypair.publicKey,
    amountAtoms,
    undefined,
    { skipPreflight: true },
  );
  console.log(`Minted ${amountAtoms} to ${traderTokenAccount} in ${mintSig}`);
}

async function createMarket(
  connection: Connection,
  keypair: Keypair,
  baseMint: PublicKey,
  quoteMint: PublicKey,
) {
  const marketKeypair: Keypair = Keypair.generate();
  const createAccountIx: TransactionInstruction = SystemProgram.createAccount({
    fromPubkey: keypair.publicKey,
    newAccountPubkey: marketKeypair.publicKey,
    space: FIXED_MANIFEST_HEADER_SIZE,
    lamports: await connection.getMinimumBalanceForRentExemption(
      FIXED_MANIFEST_HEADER_SIZE,
    ),
    programId: PROGRAM_ID,
  });
  const createMarketIx = ManifestClient['createMarketIx'](
    keypair.publicKey,
    baseMint,
    quoteMint,
    marketKeypair.publicKey,
  );

  const tx: Transaction = new Transaction();
  tx.add(createAccountIx);
  tx.add(createMarketIx);
  console.log(
    'Creating market with signers, payer',
    keypair.publicKey.toBase58(),
    'market',
    marketKeypair.publicKey.toBase58(),
  );
  const signature = await sendAndConfirmTransaction(connection, tx, [
    keypair,
    marketKeypair,
  ]);
  console.log(`Created market at ${marketKeypair.publicKey} in ${signature}`);
  return marketKeypair.publicKey;
}

async function main() {
  const connection: Connection = new Connection(RPC_URL as string, 'confirmed');

  const creatorKeypair: Keypair = process.env.MARKET_CREATOR_PRIVATE_KEY
    ? Keypair.fromSecretKey(
        Uint8Array.from(
          process.env.MARKET_CREATOR_PRIVATE_KEY.split(',').map(Number),
        ),
      )
    : Keypair.generate();
  if (!process.env.MARKET_CREATOR_PRIVATE_KEY) {
    console.log('Requesting airdrop for rent/gas');
    await airdropSol(connection, creatorKeypair.publicKey);
  }

  let baseMint: PublicKey;
  let quoteMint: PublicKey;
  let marketAddress: PublicKey;
  if (process.env.MARKET_ADDRESS) {
    marketAddress = new PublicKey(process.env.MARKET_ADDRESS);
    console.log('loading market...');
    const market: Market = await Market.loadFromAddress({
      connection: connection,
      address: marketAddress,
    });
    baseMint = market.baseMint();
    quoteMint = market.quoteMint();
  } else {
    console.log('creating market...');
    baseMint = await createMint(
      connection,
      creatorKeypair,
      creatorKeypair.publicKey,
      creatorKeypair.publicKey,
      9,
    );
    quoteMint = await createMint(
      connection,
      creatorKeypair,
      creatorKeypair.publicKey,
      creatorKeypair.publicKey,
      6,
    );
    console.log('Base mint', baseMint.toBase58());
    console.log('Quote mint', quoteMint.toBase58());
    marketAddress = await createMarket(
      connection,
      creatorKeypair,
      baseMint,
      quoteMint,
    );
    console.log('Market address', marketAddress.toBase58());
  }

  console.log(`Cluster is ${await getClusterFromConnection(connection)}`);

  if (process.env.ALICE_PRIVATE_KEY) {
    const aliceKeypair: Keypair = process.env.ALICE_PRIVATE_KEY
      ? Keypair.fromSecretKey(
          Uint8Array.from(process.env.ALICE_PRIVATE_KEY.split(',').map(Number)),
        )
      : Keypair.generate();
    const alice: DummyTrader = await DummyTrader.create(
      connection,
      aliceKeypair,
      marketAddress,
      'alice',
      ['DEPOSIT', 'WITHDRAW', 'PLACE_ORDER', 'SWAP', 'CANCEL_ORDER'],
    );
    await fundWallet(connection, aliceKeypair, creatorKeypair, baseMint);
    await fundWallet(connection, aliceKeypair, creatorKeypair, quoteMint);
    await Promise.all([alice.run()]);
  }

  if (process.env.BOB_PRIVATE_KEY) {
    const bobKeypair: Keypair = process.env.BOB_PRIVATE_KEY
      ? Keypair.fromSecretKey(
          Uint8Array.from(process.env.BOB_PRIVATE_KEY.split(',').map(Number)),
        )
      : Keypair.generate();
    const bob: DummyTrader = await DummyTrader.create(
      connection,
      bobKeypair,
      marketAddress,
      'bob',
      [
        'DEPOSIT',
        'WITHDRAW',
        'PLACE_ORDER',
        //'SWAP',
        'CANCEL_ORDER',
      ],
    );
    await fundWallet(connection, bobKeypair, creatorKeypair, baseMint);
    await fundWallet(connection, bobKeypair, creatorKeypair, quoteMint);
    await Promise.all([bob.run()]);
  }

  if (process.env.CHARLIE_PRIVATE_KEY) {
    const charlieKeypair: Keypair = process.env.CHARLIE_PRIVATE_KEY
      ? Keypair.fromSecretKey(
          Uint8Array.from(
            process.env.CHARLIE_PRIVATE_KEY.split(',').map(Number),
          ),
        )
      : Keypair.generate();
    const charlie: DummyTrader = await DummyTrader.create(
      connection,
      charlieKeypair,
      marketAddress,
      'charlie',
      [
        //'DEPOSIT',
        //'WITHDRAW',
        //'PLACE_ORDER',
        'SWAP',
        //'CANCEL_ORDER',
      ],
    );
    await fundWallet(connection, charlieKeypair, creatorKeypair, baseMint);
    await fundWallet(connection, charlieKeypair, creatorKeypair, quoteMint);
    await Promise.all([charlie.run()]);
  }
}

main();
