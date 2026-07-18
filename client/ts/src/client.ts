import { bignum } from '@metaplex-foundation/beet';
import {
  PublicKey,
  Connection,
  Keypair,
  TransactionInstruction,
  SystemProgram,
  Transaction,
  sendAndConfirmTransaction,
  AccountInfo,
  TransactionSignature,
  GetProgramAccountsResponse,
} from '@solana/web3.js';
import {
  Mint,
  TOKEN_2022_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  unpackMint,
} from '@solana/spl-token';
import {
  createCreateMarketInstruction,
  createGlobalAddTraderInstruction,
  createGlobalCreateInstruction,
  createGlobalDepositInstruction,
  createGlobalWithdrawInstruction,
  createSwapInstruction,
  createBatchUpdateInstruction as createBatchUpdateCoreInstruction,
} from './manifest/instructions';
import { OrderType, SwapParams } from './manifest/types';
import { Market, RestingOrder } from './market';
import { WrapperMarketInfo, Wrapper, WrapperData } from './wrapperObj';
import { PROGRAM_ID as MANIFEST_PROGRAM_ID, PROGRAM_ID } from './manifest';
import {
  PROGRAM_ID as WRAPPER_PROGRAM_ID,
  WrapperCancelOrderParams,
  WrapperPlaceOrderParams,
  createBatchUpdateBaseGlobalInstruction,
  createBatchUpdateInstruction,
  createBatchUpdateQuoteGlobalInstruction,
  createClaimSeatInstruction,
  createCreateWrapperInstruction,
  createDepositInstruction,
  createWithdrawInstruction,
} from './wrapper';
import { FIXED_WRAPPER_HEADER_SIZE } from './constants';
import { getVaultAddress } from './utils/market';
import { genAccDiscriminator } from './utils/discriminator';
import { getGlobalAddress, getGlobalVaultAddress } from './utils/global';
import { Global } from './global';

export interface SetupData {
  setupNeeded: boolean;
  instructions: TransactionInstruction[];
  wrapperKeypair: Keypair | null;
}

type WrapperResponse = Readonly<{
  account: AccountInfo<Buffer>;
  pubkey: PublicKey;
}>;

const marketDiscriminator: Buffer = genAccDiscriminator(
  'manifest::state::market::MarketFixed',
);

export class ManifestClient {
  public isBase22: boolean;
  public isQuote22: boolean;

  private constructor(
    public connection: Connection,
    public wrapper: Wrapper | null,
    public market: Market,
    private payer: PublicKey | null,
    private baseMint: Mint,
    private quoteMint: Mint,
    // Globals are public. The expectation is that users will directly access
    // them, similar to the market.
    public baseGlobal: Global | null,
    public quoteGlobal: Global | null,
  ) {
    // If no extension data then the mint is not Token2022
    this.isBase22 = baseMint.tlvData.length > 0;
    this.isQuote22 = quoteMint.tlvData.length > 0;
  }

  /**
   * fetches all user wrapper accounts and returns the first or null if none are found.
   * First tries the stats server API for faster lookup, falls back to getProgramAccounts.
   *
   * @param connection Connection
   * @param payerPub PublicKey of the trader
   *
   * @returns Promise<GetProgramAccountsResponse>
   */
  private static async fetchFirstUserWrapper(
    connection: Connection,
    payerPub: PublicKey,
  ): Promise<WrapperResponse | null> {
    // First try the stats server API for faster lookup
    try {
      const response = await fetch(
        `https://mfx-stats-mainnet.fly.dev/wrapper?owner=${payerPub.toBase58()}`,
      );
      if (response.ok) {
        const data = (await response.json()) as {
          owner: string;
          wrapper: string;
        };
        const wrapperPubkey = new PublicKey(data.wrapper);
        const accountInfo = await connection.getAccountInfo(wrapperPubkey);
        if (accountInfo) {
          return {
            pubkey: wrapperPubkey,
            account: accountInfo,
          };
        }
      }
    } catch {
      // API call failed, fall back to getProgramAccounts
    }

    // Fall back to getProgramAccounts
    const existingWrappers = await connection.getProgramAccounts(
      WRAPPER_PROGRAM_ID,
      {
        filters: [
          // Dont check discriminant since there is only one type of account.
          {
            memcmp: {
              offset: 8,
              encoding: 'base58',
              bytes: payerPub.toBase58(),
            },
          },
        ],
      },
    );

    return existingWrappers.length > 0 ? existingWrappers[0] : null;
  }

  /**
   * list all Manifest markets using getProgramAccounts. caution: this is a heavy call.
   *
   * @param connection Connection
   * @returns PublicKey[]
   */
  public static async listMarketPublicKeys(
    connection: Connection,
  ): Promise<PublicKey[]> {
    const accounts = await connection.getProgramAccounts(PROGRAM_ID, {
      dataSlice: { offset: 0, length: 0 },
      filters: [
        {
          memcmp: {
            offset: 0,
            bytes: marketDiscriminator.toString('base64'),
            encoding: 'base64',
          },
        },
      ],
    });

    return accounts.map((a) => a.pubkey);
  }

  /**
   * List all Manifest markets that match base and quote mint. If useApi, then
   * this call uses the manifest stats server instead of the heavy
   * getProgramAccounts RPC call.
   *
   * @param connection Connection
   * @param baseMint PublicKey
   * @param quoteMint PublicKey
   * @param useApi boolean
   * @returns PublicKey[]
   */
  public static async listMarketsForMints(
    connection: Connection,
    baseMint: PublicKey,
    quoteMint: PublicKey,
    useApi?: boolean,
  ): Promise<PublicKey[]> {
    if (useApi) {
      const responseJson = (await (
        await fetch('https://mfx-stats-mainnet.fly.dev/tickers')
      ).json()) as any[];
      const tickers: PublicKey[] = responseJson
        .filter((ticker) => {
          return (
            ticker.base_currency == baseMint.toBase58() &&
            ticker.target_currency == quoteMint.toBase58()
          );
        })
        .map((ticker) => {
          return new PublicKey(ticker.ticker_id);
        });
      return tickers;
    }
    const accounts = await connection.getProgramAccounts(PROGRAM_ID, {
      dataSlice: { offset: 0, length: 0 },
      filters: [
        {
          memcmp: {
            offset: 0,
            bytes: marketDiscriminator.toString('base64'),
            encoding: 'base64',
          },
        },
        {
          memcmp: {
            offset: 16,
            bytes: baseMint.toBase58(),
            encoding: 'base58',
          },
        },
        {
          memcmp: {
            offset: 48,
            bytes: quoteMint.toBase58(),
            encoding: 'base58',
          },
        },
      ],
    });

    return accounts.map((a) => a.pubkey);
  }

  /**
   * Get all market program accounts. This is expensive RPC load..
   *
   * @param connection Connection
   * @returns GetProgramAccountsResponse
   */
  public static async getMarketProgramAccounts(
    connection: Connection,
  ): Promise<GetProgramAccountsResponse> {
    const accounts: GetProgramAccountsResponse =
      await connection.getProgramAccounts(PROGRAM_ID, {
        filters: [
          {
            memcmp: {
              offset: 0,
              bytes: marketDiscriminator.toString('base64'),
              encoding: 'base64',
            },
          },
        ],
      });

    return accounts;
  }

  /**
   * Create a new client which creates a wrapper and claims seat if needed.
   *
   * @param connection Connection
   * @param marketPk PublicKey of the market
   * @param payerKeypair Keypair of the trader
   * @param wrapperProgram PublicKey of wrapper program to use if payer doesn't have a userWrapper
   *
   * @returns ManifestClient
   */
  public static async getClientForMarket(
    connection: Connection,
    marketPk: PublicKey,
    payerKeypair: Keypair,
    wrapperProgram: PublicKey,
  ): Promise<ManifestClient> {
    const marketObject: Market = await Market.loadFromAddress({
      connection: connection,
      address: marketPk,
    });
    const baseMintPk: PublicKey = marketObject.baseMint();
    const quoteMintPk: PublicKey = marketObject.quoteMint();
    const baseMintAccountInfo: AccountInfo<Buffer> =
      (await connection.getAccountInfo(baseMintPk))!;
    const baseMint: Mint = unpackMint(
      baseMintPk,
      baseMintAccountInfo,
      baseMintAccountInfo.owner,
    );
    const quoteMintAccountInfo: AccountInfo<Buffer> =
      (await connection.getAccountInfo(quoteMintPk))!;
    const quoteMint: Mint = unpackMint(
      quoteMintPk,
      quoteMintAccountInfo,
      quoteMintAccountInfo.owner,
    );
    const baseGlobal: Global | null = await Global.loadFromAddress({
      connection,
      address: getGlobalAddress(baseMint.address),
    });
    const quoteGlobal: Global | null = await Global.loadFromAddress({
      connection,
      address: getGlobalAddress(quoteMint.address),
    });

    const userWrapper = await ManifestClient.fetchFirstUserWrapper(
      connection,
      payerKeypair.publicKey,
    );
    const transaction: Transaction = new Transaction();
    if (!userWrapper) {
      const wrapperKeypair: Keypair = Keypair.generate();
      const createAccountIx: TransactionInstruction =
        SystemProgram.createAccount({
          fromPubkey: payerKeypair.publicKey,
          newAccountPubkey: wrapperKeypair.publicKey,
          space: FIXED_WRAPPER_HEADER_SIZE,
          lamports: await connection.getMinimumBalanceForRentExemption(
            FIXED_WRAPPER_HEADER_SIZE,
          ),
          programId: WRAPPER_PROGRAM_ID,
        });
      const createWrapperIx: TransactionInstruction =
        createCreateWrapperInstruction({
          owner: payerKeypair.publicKey,
          wrapperState: wrapperKeypair.publicKey,
        });
      const claimSeatIx: TransactionInstruction = createClaimSeatInstruction(
        {
          manifestProgram: MANIFEST_PROGRAM_ID,
          owner: payerKeypair.publicKey,
          market: marketPk,
          wrapperState: wrapperKeypair.publicKey,
        },
        wrapperProgram,
      );
      transaction.add(createAccountIx);
      transaction.add(createWrapperIx);
      transaction.add(claimSeatIx);

      await sendAndConfirmTransaction(connection, transaction, [
        payerKeypair,
        wrapperKeypair,
      ]);
      const wrapper = await Wrapper.loadFromAddress({
        connection,
        address: wrapperKeypair.publicKey,
      });

      return new ManifestClient(
        connection,
        wrapper,
        marketObject,
        payerKeypair.publicKey,
        baseMint,
        quoteMint,
        baseGlobal,
        quoteGlobal,
      );
    }

    // Otherwise there is an existing wrapper
    const wrapperData: WrapperData = Wrapper.deserializeWrapperBuffer(
      userWrapper.account.data,
    );
    const existingMarketInfos: WrapperMarketInfo[] =
      wrapperData.marketInfos.filter((marketInfo: WrapperMarketInfo) => {
        return marketInfo.market.toBase58() == marketPk.toBase58();
      });
    if (existingMarketInfos.length > 0) {
      const wrapper = await Wrapper.loadFromAddress({
        connection,
        address: userWrapper.pubkey,
      });
      return new ManifestClient(
        connection,
        wrapper,
        marketObject,
        payerKeypair.publicKey,
        baseMint,
        quoteMint,
        baseGlobal,
        quoteGlobal,
      );
    }

    // There is a wrapper, but need to claim a seat.
    const claimSeatIx: TransactionInstruction = createClaimSeatInstruction({
      manifestProgram: MANIFEST_PROGRAM_ID,
      owner: payerKeypair.publicKey,
      market: marketPk,
      wrapperState: userWrapper.pubkey,
    });
    transaction.add(claimSeatIx);
    await sendAndConfirmTransaction(connection, transaction, [payerKeypair]);
    const wrapper = await Wrapper.loadFromAddress({
      connection,
      address: userWrapper.pubkey,
    });

    return new ManifestClient(
      connection,
      wrapper,
      marketObject,
      payerKeypair.publicKey,
      baseMint,
      quoteMint,
      baseGlobal,
      quoteGlobal,
    );
  }

  /**
   * generate ixs which need to be executed in order to run a manifest client for a given market. `{ setupNeeded: false }` means all good.
   * this function should be used before getClientForMarketNoPrivateKey for UI cases where `Keypair`s cannot be directly passed in.
   *
   * @param connection Connection
   * @param marketPk PublicKey of the market
   * @param trader PublicKey of the trader
   *
   * @returns Promise<SetupData>
   */
  public static async getSetupIxs(
    connection: Connection,
    marketPk: PublicKey,
    trader: PublicKey,
  ): Promise<SetupData> {
    const setupData: SetupData = {
      setupNeeded: true,
      instructions: [],
      wrapperKeypair: null,
    };
    const userWrapper = await ManifestClient.fetchFirstUserWrapper(
      connection,
      trader,
    );
    if (!userWrapper) {
      const wrapperKeypair: Keypair = Keypair.generate();
      setupData.wrapperKeypair = wrapperKeypair;

      const createAccountIx: TransactionInstruction =
        SystemProgram.createAccount({
          fromPubkey: trader,
          newAccountPubkey: wrapperKeypair.publicKey,
          space: FIXED_WRAPPER_HEADER_SIZE,
          lamports: await connection.getMinimumBalanceForRentExemption(
            FIXED_WRAPPER_HEADER_SIZE,
          ),
          programId: WRAPPER_PROGRAM_ID,
        });
      setupData.instructions.push(createAccountIx);

      const createWrapperIx: TransactionInstruction =
        createCreateWrapperInstruction({
          owner: trader,
          wrapperState: wrapperKeypair.publicKey,
        });
      setupData.instructions.push(createWrapperIx);

      const claimSeatIx: TransactionInstruction = createClaimSeatInstruction({
        manifestProgram: MANIFEST_PROGRAM_ID,
        owner: trader,
        market: marketPk,
        wrapperState: wrapperKeypair.publicKey,
      });
      setupData.instructions.push(claimSeatIx);

      return setupData;
    }

    const wrapperData: WrapperData = Wrapper.deserializeWrapperBuffer(
      userWrapper.account.data,
    );

    const existingMarketInfos: WrapperMarketInfo[] =
      wrapperData.marketInfos.filter((marketInfo: WrapperMarketInfo) => {
        return marketInfo.market.toBase58() == marketPk.toBase58();
      });
    if (existingMarketInfos.length > 0) {
      setupData.setupNeeded = false;
      return setupData;
    }

    // There is a wrapper, but need to claim a seat.
    const claimSeatIx: TransactionInstruction = createClaimSeatInstruction({
      manifestProgram: MANIFEST_PROGRAM_ID,
      owner: trader,
      market: marketPk,
      wrapperState: userWrapper.pubkey,
    });
    setupData.instructions.push(claimSeatIx);

    return setupData;
  }

  /**
   * Create a new client. throws if setup ixs are needed. Call ManifestClient.getSetupIxs to check if ixs are needed.
   * This is the way to create a client without directly passing in `Keypair` types (for example when building a UI).
   *
   * @param connection Connection
   * @param marketPk PublicKey of the market
   * @param trader PublicKey of the trader
   *
   * @returns ManifestClient
   */
  public static async getClientForMarketNoPrivateKey(
    connection: Connection,
    marketPk: PublicKey,
    trader: PublicKey,
  ): Promise<ManifestClient> {
    const { setupNeeded } = await this.getSetupIxs(
      connection,
      marketPk,
      trader,
    );
    if (setupNeeded) {
      throw new Error('setup ixs need to be executed first');
    }

    const marketObject: Market = await Market.loadFromAddress({
      connection: connection,
      address: marketPk,
    });
    const baseMintPk: PublicKey = marketObject.baseMint();
    const quoteMintPk: PublicKey = marketObject.quoteMint();
    const baseMintAccountInfo: AccountInfo<Buffer> =
      (await connection.getAccountInfo(baseMintPk))!;
    const baseMint: Mint = unpackMint(
      baseMintPk,
      baseMintAccountInfo,
      baseMintAccountInfo.owner,
    );
    const quoteMintAccountInfo: AccountInfo<Buffer> =
      (await connection.getAccountInfo(quoteMintPk))!;
    const quoteMint: Mint = unpackMint(
      quoteMintPk,
      quoteMintAccountInfo,
      quoteMintAccountInfo.owner,
    );

    const userWrapper = await ManifestClient.fetchFirstUserWrapper(
      connection,
      trader,
    );

    if (!userWrapper) {
      throw new Error(
        'userWrapper is null even though setupNeeded is false. This should never happen.',
      );
    }

    const wrapper = await Wrapper.loadFromAddress({
      connection,
      address: userWrapper.pubkey,
    });
    const baseGlobal: Global | null = await Global.loadFromAddress({
      connection,
      address: getGlobalAddress(baseMint.address),
    });
    const quoteGlobal: Global | null = await Global.loadFromAddress({
      connection,
      address: getGlobalAddress(quoteMint.address),
    });

    return new ManifestClient(
      connection,
      wrapper,
      marketObject,
      trader,
      baseMint,
      quoteMint,
      baseGlobal,
      quoteGlobal,
    );
  }

  /**
   * Create a new client that is read only. Cannot send transactions or generate instructions.
   *
   * @param connection Connection
   * @param marketPk PublicKey of the market
   * @param trader PublicKey for trader whose wrapper to fetch
   *
   * @returns ManifestClient
   */
  public static async getClientReadOnly(
    connection: Connection,
    marketPk: PublicKey,
    trader?: PublicKey,
  ): Promise<ManifestClient> {
    const marketObject: Market = await Market.loadFromAddress({
      connection: connection,
      address: marketPk,
    });
    const baseMintPk: PublicKey = marketObject.baseMint();
    const quoteMintPk: PublicKey = marketObject.quoteMint();
    const baseGlobalPk: PublicKey = getGlobalAddress(baseMintPk);
    const quoteGlobalPk: PublicKey = getGlobalAddress(quoteMintPk);

    const [
      baseMintAccountInfo,
      quoteMintAccountInfo,
      baseGlobalAccountInfo,
      quoteGlobalAccountInfo,
    ]: (AccountInfo<Buffer> | null)[] =
      await connection.getMultipleAccountsInfo([
        baseMintPk,
        quoteMintPk,
        baseGlobalPk,
        quoteGlobalPk,
      ]);

    const baseMint: Mint = unpackMint(
      baseMintPk,
      baseMintAccountInfo,
      baseMintAccountInfo!.owner,
    );
    const quoteMint: Mint = unpackMint(
      quoteMintPk,
      quoteMintAccountInfo,
      quoteMintAccountInfo!.owner,
    );

    // Global accounts are optional
    const baseGlobal: Global | null =
      baseGlobalAccountInfo &&
      Global.loadFromBuffer({
        address: baseGlobalPk,
        buffer: baseGlobalAccountInfo.data,
      });
    const quoteGlobal: Global | null =
      quoteGlobalAccountInfo &&
      Global.loadFromBuffer({
        address: quoteGlobalPk,
        buffer: quoteGlobalAccountInfo.data,
      });

    let wrapper: Wrapper | null = null;
    if (trader != null) {
      const userWrapper: WrapperResponse | null =
        await ManifestClient.fetchFirstUserWrapper(connection, trader);
      if (userWrapper) {
        wrapper = Wrapper.loadFromBuffer({
          address: userWrapper.pubkey,
          buffer: userWrapper.account.data,
        });
      }
    }

    return new ManifestClient(
      connection,
      wrapper,
      marketObject,
      null,
      baseMint,
      quoteMint,
      baseGlobal,
      quoteGlobal,
    );
  }

  /**
   * Initializes a ReadOnlyClient for each Market the trader has a seat on.
   * This has been optimized to be as light on the RPC as possible but it is
   * still using getProgramAccounts. caution: this is a heavy call.
   *
   * @param connection Connection
   * @param trader PublicKey
   * @returns ManifestClient[]
   */
  public static async getClientsReadOnlyForAllTraderSeats(
    connection: Connection,
    trader: PublicKey,
  ): Promise<ManifestClient[]> {
    const marketAccountResponse = await connection.getProgramAccounts(
      PROGRAM_ID,
      {
        filters: [
          {
            memcmp: {
              offset: 0,
              bytes: marketDiscriminator.toString('base64'),
              encoding: 'base64',
            },
          },
        ],
        withContext: true,
      },
    );

    const markets: Market[] = marketAccountResponse.value.map((m) =>
      Market.loadFromBuffer({
        address: m.pubkey,
        buffer: m.account.data,
        slot: marketAccountResponse.context.slot,
      }),
    );
    const marketsForTrader: Market[] = markets.filter((m) => m.hasSeat(trader));

    const baseMintPks: string[] = marketsForTrader.map((m) =>
      m.baseMint().toString(),
    );
    const quoteMintPks: string[] = marketsForTrader.map((m) =>
      m.quoteMint().toString(),
    );
    const baseGlobalPks: string[] = marketsForTrader.map((m) =>
      getGlobalAddress(m.baseMint()).toString(),
    );
    const quoteGlobalPks: string[] = marketsForTrader.map((m) =>
      getGlobalAddress(m.quoteMint()).toString(),
    );

    // ensure every account is only fetched once
    const allAisFetched: { [pk: string]: AccountInfo<Buffer> | null } = {};
    const allPksToFetch: string[] = [
      ...new Set([
        ...baseMintPks,
        ...quoteMintPks,
        ...baseGlobalPks,
        ...quoteGlobalPks,
      ]),
    ];
    const mutableCopy = Array.from(allPksToFetch);
    while (mutableCopy.length > 0) {
      const batchPks: string[] = mutableCopy.splice(0, 100);
      const batchAis = await connection.getMultipleAccountsInfoAndContext(
        batchPks.map((a) => new PublicKey(a)),
      );
      batchAis.value.forEach((ai, i) => (allAisFetched[batchPks[i]] = ai));
    }

    let wrapper: Wrapper | null = null;
    if (trader != null) {
      const userWrapper: WrapperResponse | null =
        await ManifestClient.fetchFirstUserWrapper(connection, trader);
      if (userWrapper) {
        wrapper = Wrapper.loadFromBuffer({
          address: userWrapper.pubkey,
          buffer: userWrapper.account.data,
        });
      }
    }

    return marketsForTrader.map((m, i) => {
      const baseMintAccountInfo = allAisFetched[baseMintPks[i]];
      const quoteMintAccountInfo = allAisFetched[quoteMintPks[i]];
      const baseGlobalAccountInfo = allAisFetched[baseGlobalPks[i]];
      const quoteGlobalAccountInfo = allAisFetched[quoteGlobalPks[i]];

      const baseMint: Mint = unpackMint(
        m.baseMint(),
        baseMintAccountInfo,
        baseMintAccountInfo!.owner,
      );
      const quoteMint: Mint = unpackMint(
        m.quoteMint(),
        quoteMintAccountInfo,
        quoteMintAccountInfo!.owner,
      );

      // Global accounts are optional
      const baseGlobal: Global | null =
        baseGlobalAccountInfo &&
        Global.loadFromBuffer({
          address: new PublicKey(baseGlobalPks[i]),
          buffer: baseGlobalAccountInfo.data,
        });
      const quoteGlobal: Global | null =
        quoteGlobalAccountInfo &&
        Global.loadFromBuffer({
          address: new PublicKey(quoteGlobalPks[i]),
          buffer: quoteGlobalAccountInfo.data,
        });

      return new ManifestClient(
        connection,
        wrapper,
        m,
        null,
        baseMint,
        quoteMint,
        baseGlobal,
        quoteGlobal,
      );
    });
  }

  /**
   * Reload the market and wrapper and global objects.
   */
  public async reload(): Promise<void> {
    await Promise.all([
      () => {
        if (this.wrapper) {
          return this.wrapper.reload(this.connection);
        }
      },
      () => {
        if (this.baseGlobal) {
          return this.baseGlobal.reload(this.connection);
        }
      },
      () => {
        if (this.quoteGlobal) {
          return this.quoteGlobal.reload(this.connection);
        }
      },
      this.market.reload(this.connection),
    ]);
  }

  /**
   * CreateMarket instruction. Assumes the account is already funded onchain.
   *
   * @param payer PublicKey of the trader
   * @param baseMint PublicKey of the baseMint
   * @param quoteMint PublicKey of the quoteMint
   * @param market PublicKey of the market that will be created. Private key
   *               will need to be a signer.
   *
   * @returns TransactionInstruction
   */
  private static createMarketIx(
    payer: PublicKey,
    baseMint: PublicKey,
    quoteMint: PublicKey,
    market: PublicKey,
  ): TransactionInstruction {
    const baseVault: PublicKey = getVaultAddress(market, baseMint);
    const quoteVault: PublicKey = getVaultAddress(market, quoteMint);
    return createCreateMarketInstruction({
      payer,
      market,
      baseVault,
      quoteVault,
      baseMint,
      quoteMint,
      tokenProgram22: TOKEN_2022_PROGRAM_ID,
    });
  }

  /**
   * Deposit instruction
   *
   * @param payer PublicKey of the trader
   * @param mint PublicKey for deposit mint. Must be either the base or quote
   * @param amountTokens Number of tokens to deposit.
   *
   * @returns TransactionInstruction
   */
  public depositIx(
    payer: PublicKey,
    mint: PublicKey,
    amountTokens: number,
  ): TransactionInstruction {
    if (!this.wrapper || !this.payer) {
      throw new Error('Read only');
    }
    const vault: PublicKey = getVaultAddress(this.market.address, mint);
    const is22: boolean =
      (mint.equals(this.baseMint.address) && this.isBase22) ||
      (mint.equals(this.quoteMint.address) && this.isQuote22);
    const traderTokenAccount: PublicKey = getAssociatedTokenAddressSync(
      mint,
      payer,
      true,
      is22 ? TOKEN_2022_PROGRAM_ID : TOKEN_PROGRAM_ID,
    );
    const mintDecimals =
      this.market.quoteMint().toBase58() === mint.toBase58()
        ? this.market.quoteDecimals()
        : this.market.baseDecimals();
    const amountAtoms = Math.round(amountTokens * 10 ** mintDecimals);

    return createDepositInstruction(
      {
        market: this.market.address,
        traderTokenAccount,
        vault,
        manifestProgram: MANIFEST_PROGRAM_ID,
        owner: this.payer,
        wrapperState: this.wrapper.address,
        mint,
        tokenProgram: is22 ? TOKEN_2022_PROGRAM_ID : TOKEN_PROGRAM_ID,
      },
      {
        params: {
          amountAtoms,
        },
      },
    );
  }

  /**
   * Withdraw instruction
   *
   * @param payer PublicKey of the trader
   * @param mint PublicKey for withdraw mint. Must be either the base or quote
   * @param amountTokens Number of tokens to withdraw.
   *
   * @returns TransactionInstruction
   */
  public withdrawIx(
    payer: PublicKey,
    mint: PublicKey,
    amountTokens: number,
  ): TransactionInstruction {
    if (!this.wrapper || !this.payer) {
      throw new Error('Read only');
    }
    const vault: PublicKey = getVaultAddress(this.market.address, mint);
    const is22: boolean =
      (mint.equals(this.baseMint.address) && this.isBase22) ||
      (mint.equals(this.quoteMint.address) && this.isQuote22);
    const traderTokenAccount: PublicKey = getAssociatedTokenAddressSync(
      mint,
      payer,
      true,
      is22 ? TOKEN_2022_PROGRAM_ID : TOKEN_PROGRAM_ID,
    );
    const mintDecimals =
      this.market.quoteMint().toBase58() === mint.toBase58()
        ? this.market.quoteDecimals()
        : this.market.baseDecimals();
    const amountAtoms = Math.floor(amountTokens * 10 ** mintDecimals);

    return createWithdrawInstruction(
      {
        market: this.market.address,
        traderTokenAccount,
        vault,
        manifestProgram: MANIFEST_PROGRAM_ID,
        owner: this.payer,
        wrapperState: this.wrapper.address,
        mint,
        tokenProgram: is22 ? TOKEN_2022_PROGRAM_ID : TOKEN_PROGRAM_ID,
      },
      {
        params: {
          amountAtoms,
        },
      },
    );
  }

  /**
   * Withdraw All instruction. Withdraws all available base and quote tokens
   *
   * @returns TransactionInstruction[]
   */
  public withdrawAllIx(): TransactionInstruction[] {
    if (!this.wrapper || !this.payer) {
      throw new Error('Read only');
    }
    const withdrawInstructions: TransactionInstruction[] = [];

    const baseBalance = this.market.getWithdrawableBalanceTokens(
      this.payer,
      true,
    );
    if (baseBalance > 0) {
      const baseWithdrawIx = this.withdrawIx(
        this.payer,
        this.market.baseMint(),
        baseBalance,
      );
      withdrawInstructions.push(baseWithdrawIx);
    }

    const quoteBalance = this.market.getWithdrawableBalanceTokens(
      this.payer,
      false,
    );
    if (quoteBalance > 0) {
      const quoteWithdrawIx = this.withdrawIx(
        this.payer,
        this.market.quoteMint(),
        quoteBalance,
      );
      withdrawInstructions.push(quoteWithdrawIx);
    }

    return withdrawInstructions;
  }

  /**
   * PlaceOrder instruction
   *
   * @param params WrapperPlaceOrderParamsExternal | WrapperPlaceOrderReverseParamsExternal
   * including all the information for placing an order like amount, price,
   * ordertype, ... This is called external because to avoid conflicts with the
   * autogenerated version which has problems with expressing some of the
   * parameters. The reverse type has a spreadBps field instead of lastValidSlot.
   *
   * @returns TransactionInstruction
   */
  public placeOrderIx(
    params:
      | WrapperPlaceOrderParamsExternal
      | WrapperPlaceOrderReverseParamsExternal,
  ): TransactionInstruction {
    if (!this.wrapper || !this.payer) {
      throw new Error('Read only');
    }

    // Check if global accounts exist for this market
    const hasQuoteGlobal = this.quoteGlobal !== null;
    const hasBaseGlobal = this.baseGlobal !== null;

    // For non-Global order types, we might still need to include global accounts
    // because the counterparty (maker) might be using a global order
    if (params.orderType != OrderType.Global) {
      // Bid (buy) orders need base global (sellers might use base global)
      if (params.isBid && hasBaseGlobal) {
        const global: PublicKey = getGlobalAddress(this.baseMint.address);
        const globalVault: PublicKey = getGlobalVaultAddress(
          this.baseMint.address,
        );
        const vault: PublicKey = getVaultAddress(
          this.market.address,
          this.baseMint.address,
        );
        return createBatchUpdateBaseGlobalInstruction(
          {
            market: this.market.address,
            manifestProgram: MANIFEST_PROGRAM_ID,
            owner: this.payer,
            wrapperState: this.wrapper.address,
            baseMint: this.baseMint.address,
            baseGlobal: global,
            baseGlobalVault: globalVault,
            baseMarketVault: vault,
            baseTokenProgram: this.isBase22
              ? TOKEN_2022_PROGRAM_ID
              : TOKEN_PROGRAM_ID,
          },
          {
            params: {
              cancels: [],
              cancelAll: false,
              orders: [toWrapperPlaceOrderParams(this.market, params)],
            },
          },
        );
      }

      // Ask (sell) orders need quote global (buyers might use quote global)
      if (!params.isBid && hasQuoteGlobal) {
        const global: PublicKey = getGlobalAddress(this.quoteMint.address);
        const globalVault: PublicKey = getGlobalVaultAddress(
          this.quoteMint.address,
        );
        const vault: PublicKey = getVaultAddress(
          this.market.address,
          this.quoteMint.address,
        );
        return createBatchUpdateQuoteGlobalInstruction(
          {
            market: this.market.address,
            manifestProgram: MANIFEST_PROGRAM_ID,
            owner: this.payer,
            wrapperState: this.wrapper.address,
            quoteMint: this.quoteMint.address,
            quoteGlobal: global,
            quoteGlobalVault: globalVault,
            quoteMarketVault: vault,
            quoteTokenProgram: this.isQuote22
              ? TOKEN_2022_PROGRAM_ID
              : TOKEN_PROGRAM_ID,
          },
          {
            params: {
              cancels: [],
              cancelAll: false,
              orders: [toWrapperPlaceOrderParams(this.market, params)],
            },
          },
        );
      }

      // No global accounts exist or not needed - use regular batch update
      return createBatchUpdateInstruction(
        {
          market: this.market.address,
          manifestProgram: MANIFEST_PROGRAM_ID,
          owner: this.payer,
          wrapperState: this.wrapper.address,
        },
        {
          params: {
            cancels: [],
            cancelAll: false,
            orders: [toWrapperPlaceOrderParams(this.market, params)],
          },
        },
      );
    }
    if (params.isBid) {
      const global: PublicKey = getGlobalAddress(this.quoteMint.address);
      const globalVault: PublicKey = getGlobalVaultAddress(
        this.quoteMint.address,
      );
      const vault: PublicKey = getVaultAddress(
        this.market.address,
        this.quoteMint.address,
      );
      return createBatchUpdateQuoteGlobalInstruction(
        {
          market: this.market.address,
          manifestProgram: MANIFEST_PROGRAM_ID,
          owner: this.payer,
          wrapperState: this.wrapper.address,
          quoteMint: this.quoteMint.address,
          quoteGlobal: global,
          quoteGlobalVault: globalVault,
          quoteMarketVault: vault,
          quoteTokenProgram: this.isQuote22
            ? TOKEN_2022_PROGRAM_ID
            : TOKEN_PROGRAM_ID,
        },
        {
          params: {
            cancels: [],
            cancelAll: false,
            orders: [toWrapperPlaceOrderParams(this.market, params)],
          },
        },
      );
    } else {
      const global: PublicKey = getGlobalAddress(this.baseMint.address);
      const globalVault: PublicKey = getGlobalVaultAddress(
        this.baseMint.address,
      );
      const vault: PublicKey = getVaultAddress(
        this.market.address,
        this.baseMint.address,
      );
      return createBatchUpdateBaseGlobalInstruction(
        {
          market: this.market.address,
          manifestProgram: MANIFEST_PROGRAM_ID,
          owner: this.payer,
          wrapperState: this.wrapper.address,
          baseMint: this.baseMint.address,
          baseGlobal: global,
          baseGlobalVault: globalVault,
          baseMarketVault: vault,
          baseTokenProgram: this.isBase22
            ? TOKEN_2022_PROGRAM_ID
            : TOKEN_PROGRAM_ID,
        },
        {
          params: {
            cancels: [],
            cancelAll: false,
            orders: [toWrapperPlaceOrderParams(this.market, params)],
          },
        },
      );
    }
  }

  /**
   * PlaceOrderWithRequiredDeposit instruction. Only deposits the appropriate base
   * or quote tokens if not in the withdrawable balances.
   *
   * @param payer PublicKey of the trader
   * @param params WrapperPlaceOrderParamsExternal | WrapperPlaceOrderReverseParamsExternal
   * including all the information for placing an order like amount, price,
   * ordertype, ... This is called external because to avoid conflicts with the
   * autogenerated version which has problems with expressing some of the
   * parameters. The reverse type has a spreadBps field instead of lastValidSlot.
   *
   * @returns TransactionInstruction[]
   */
  public async placeOrderWithRequiredDepositIxs(
    payer: PublicKey,
    params:
      | WrapperPlaceOrderParamsExternal
      | WrapperPlaceOrderReverseParamsExternal,
  ): Promise<TransactionInstruction[]> {
    const placeOrderIx: TransactionInstruction = this.placeOrderIx(params);

    if (params.orderType != OrderType.Global) {
      const currentBalanceTokens: number =
        this.market.getWithdrawableBalanceTokens(payer, !params.isBid);
      let depositMint: PublicKey;
      let depositAmountTokens: number = 0;

      if (params.isBid) {
        depositMint = this.market.quoteMint();
        depositAmountTokens =
          params.numBaseTokens * params.tokenPrice - currentBalanceTokens;
      } else {
        depositMint = this.market.baseMint();
        depositAmountTokens = params.numBaseTokens - currentBalanceTokens;
      }

      if (depositAmountTokens <= 0) {
        return [placeOrderIx];
      }
      const depositIx = this.depositIx(payer, depositMint, depositAmountTokens);

      return [depositIx, placeOrderIx];
    } else {
      const global: Global = (
        params.isBid ? this.quoteGlobal : this.baseGlobal
      )!;
      const currentBalanceTokens: number = await global.getGlobalBalanceTokens(
        this.connection,
        payer,
      );

      let depositMint: PublicKey;
      let depositAmountTokens: number = 0;

      if (params.isBid) {
        depositMint = this.market.quoteMint();
        depositAmountTokens =
          params.numBaseTokens * params.tokenPrice - currentBalanceTokens;
      } else {
        depositMint = this.market.baseMint();
        depositAmountTokens = params.numBaseTokens - currentBalanceTokens;
      }

      if (depositAmountTokens <= 0) {
        return [placeOrderIx];
      }
      const depositIx = await ManifestClient.globalDepositIx(
        this.connection,
        payer!,
        depositMint,
        depositAmountTokens,
      );

      return [depositIx, placeOrderIx];
    }
  }

  /**
   * Swap instruction
   *
   * Optimized swap for routers and arb bots. Normal traders should compose
   * depost/withdraw/placeOrder to get limit orders. Does not go through the
   * wrapper.
   *
   * @param payer PublicKey of the trader
   * @param params SwapParams
   *
   * @returns TransactionInstruction
   */
  public swapIx(payer: PublicKey, params: SwapParams): TransactionInstruction {
    const traderBase: PublicKey = getAssociatedTokenAddressSync(
      this.baseMint.address,
      payer,
      true,
      this.isBase22 ? TOKEN_2022_PROGRAM_ID : TOKEN_PROGRAM_ID,
    );
    const traderQuote: PublicKey = getAssociatedTokenAddressSync(
      this.quoteMint.address,
      payer,
      true,
      this.isQuote22 ? TOKEN_2022_PROGRAM_ID : TOKEN_PROGRAM_ID,
    );
    const baseVault: PublicKey = getVaultAddress(
      this.market.address,
      this.baseMint.address,
    );
    const quoteVault: PublicKey = getVaultAddress(
      this.market.address,
      this.quoteMint.address,
    );

    const global: PublicKey = getGlobalAddress(
      params.isBaseIn ? this.quoteMint.address : this.baseMint.address,
    );
    const globalVault: PublicKey = getGlobalVaultAddress(
      params.isBaseIn ? this.quoteMint.address : this.baseMint.address,
    );

    // Assumes just normal token program for now.
    // No Token22 support here in sdk yet, but includes programs and mints as
    // though it was.

    // No support for the case where global are not needed. That is an
    // optimization that needs to be made when looking at the orderbook and
    // deciding if it is worthwhile to lock the accounts.
    return createSwapInstruction(
      {
        payer,
        market: this.market.address,
        traderBase,
        traderQuote,
        baseVault,
        quoteVault,
        tokenProgramBase: this.isBase22
          ? TOKEN_2022_PROGRAM_ID
          : TOKEN_PROGRAM_ID,
        baseMint: this.baseMint.address,
        tokenProgramQuote: this.isQuote22
          ? TOKEN_2022_PROGRAM_ID
          : TOKEN_PROGRAM_ID,
        quoteMint: this.quoteMint.address,
        global,
        globalVault,
      },
      {
        params,
      },
    );
  }

  public getSwapAltPks(): Set<string> {
    const pks = new Set<string>();

    pks.add(MANIFEST_PROGRAM_ID.toString());
    pks.add(SystemProgram.programId.toString());
    pks.add(this.market.address.toString());
    if (this.isBase22) {
      pks.add(this.baseMint.address.toString());
      pks.add(TOKEN_2022_PROGRAM_ID.toString());
    } else {
      pks.add(TOKEN_PROGRAM_ID.toString());
    }
    if (this.isQuote22) {
      pks.add(this.quoteMint.address.toString());
      pks.add(TOKEN_2022_PROGRAM_ID.toString());
    } else {
      pks.add(TOKEN_PROGRAM_ID.toString());
    }

    const baseVault: PublicKey = getVaultAddress(
      this.market.address,
      this.baseMint.address,
    );
    pks.add(baseVault.toString());

    const quoteVault: PublicKey = getVaultAddress(
      this.market.address,
      this.quoteMint.address,
    );
    pks.add(quoteVault.toString());

    const baseGlobal: PublicKey = getGlobalAddress(this.baseMint.address);
    pks.add(baseGlobal.toString());

    const quoteGlobal: PublicKey = getGlobalAddress(this.quoteMint.address);
    pks.add(quoteGlobal.toString());

    const baseGlobalVault: PublicKey = getGlobalVaultAddress(
      this.baseMint.address,
    );
    pks.add(baseGlobalVault.toString());

    const quoteGlobalVault: PublicKey = getGlobalVaultAddress(
      this.baseMint.address,
    );
    pks.add(quoteGlobalVault.toString());

    return pks;
  }

  /**
   * CancelOrder instruction
   *
   * @param params WrapperCancelOrderParams includes the clientOrderId of the
   * order to cancel.
   *
   * @returns TransactionInstruction
   */
  public cancelOrderIx(
    params: WrapperCancelOrderParams,
  ): TransactionInstruction {
    if (!this.wrapper || !this.payer) {
      throw new Error('Read only');
    }

    // Global not required for cancels. If we do cancel a global, then our gas
    // prepayment is abandoned.
    return createBatchUpdateInstruction(
      {
        market: this.market.address,
        manifestProgram: MANIFEST_PROGRAM_ID,
        owner: this.payer,
        wrapperState: this.wrapper.address,
      },
      {
        params: {
          cancels: [params],
          cancelAll: false,
          orders: [],
        },
      },
    );
  }

  /**
   * BatchUpdate instruction
   *
   * @param placeParams (WrapperPlaceOrderParamsExternal | WrapperPlaceOrderReverseParamsExternal)[]
   * including all the information for placing an order like amount, price,
   * ordertype, ... This is called external because to avoid conflicts with the
   * autogenerated version which has problems with expressing some of the
   * parameters. The reverse type has a spreadBps field instead of lastValidSlot.
   * @param params WrapperCancelOrderParams[] includes the clientOrderId of the
   * order to cancel.
   *
   * @returns TransactionInstruction
   */
  public batchUpdateIx(
    placeParams: (
      | WrapperPlaceOrderParamsExternal
      | WrapperPlaceOrderReverseParamsExternal
    )[],
    cancelParams: WrapperCancelOrderParams[],
    cancelAll: boolean,
  ): TransactionInstruction {
    if (!this.wrapper || !this.payer) {
      throw new Error('Read only');
    }
    // Check if base global is needed:
    // 1. Bid orders (non-Global) might match with makers using base global
    // 2. Ask orders with Global orderType use their own base global
    const baseGlobalRequired: boolean =
      this.baseGlobal !== null &&
      placeParams.some(
        (
          placeParams:
            | WrapperPlaceOrderParamsExternal
            | WrapperPlaceOrderReverseParamsExternal,
        ) => {
          return (
            placeParams.isBid ||
            (!placeParams.isBid && placeParams.orderType === OrderType.Global)
          );
        },
      );
    // Check if quote global is needed:
    // 1. Ask orders (non-Global) might match with makers using quote global
    // 2. Bid orders with Global orderType use their own quote global
    const quoteGlobalRequired: boolean =
      this.quoteGlobal !== null &&
      placeParams.some(
        (
          placeParams:
            | WrapperPlaceOrderParamsExternal
            | WrapperPlaceOrderReverseParamsExternal,
        ) => {
          return (
            !placeParams.isBid ||
            (placeParams.isBid && placeParams.orderType === OrderType.Global)
          );
        },
      );
    if (!baseGlobalRequired && !quoteGlobalRequired) {
      return createBatchUpdateInstruction(
        {
          market: this.market.address,
          manifestProgram: MANIFEST_PROGRAM_ID,
          owner: this.payer,
          wrapperState: this.wrapper.address,
        },
        {
          params: {
            cancels: cancelParams,
            cancelAll,
            orders: placeParams.map(
              (
                params:
                  | WrapperPlaceOrderParamsExternal
                  | WrapperPlaceOrderReverseParamsExternal,
              ) => toWrapperPlaceOrderParams(this.market, params),
            ),
          },
        },
      );
    }
    if (!baseGlobalRequired && quoteGlobalRequired) {
      const global: PublicKey = getGlobalAddress(this.quoteMint.address);
      const globalVault: PublicKey = getGlobalVaultAddress(
        this.quoteMint.address,
      );
      const vault: PublicKey = getVaultAddress(
        this.market.address,
        this.quoteMint.address,
      );
      return createBatchUpdateQuoteGlobalInstruction(
        {
          market: this.market.address,
          manifestProgram: MANIFEST_PROGRAM_ID,
          owner: this.payer,
          wrapperState: this.wrapper.address,
          quoteMint: this.quoteMint.address,
          quoteGlobal: global,
          quoteGlobalVault: globalVault,
          quoteTokenProgram: this.isQuote22
            ? TOKEN_2022_PROGRAM_ID
            : TOKEN_PROGRAM_ID,
          quoteMarketVault: vault,
        },
        {
          params: {
            cancels: cancelParams,
            cancelAll,
            orders: placeParams.map(
              (
                params:
                  | WrapperPlaceOrderParamsExternal
                  | WrapperPlaceOrderReverseParamsExternal,
              ) => toWrapperPlaceOrderParams(this.market, params),
            ),
          },
        },
      );
    }
    if (baseGlobalRequired && !quoteGlobalRequired) {
      const global: PublicKey = getGlobalAddress(this.baseMint.address);
      const globalVault: PublicKey = getGlobalVaultAddress(
        this.baseMint.address,
      );
      const vault: PublicKey = getVaultAddress(
        this.market.address,
        this.baseMint.address,
      );
      return createBatchUpdateBaseGlobalInstruction(
        {
          market: this.market.address,
          manifestProgram: MANIFEST_PROGRAM_ID,
          owner: this.payer,
          wrapperState: this.wrapper.address,
          baseMint: this.baseMint.address,
          baseGlobal: global,
          baseGlobalVault: globalVault,
          baseTokenProgram: this.isBase22
            ? TOKEN_2022_PROGRAM_ID
            : TOKEN_PROGRAM_ID,
          baseMarketVault: vault,
        },
        {
          params: {
            cancels: cancelParams,
            cancelAll,
            orders: placeParams.map(
              (
                params:
                  | WrapperPlaceOrderParamsExternal
                  | WrapperPlaceOrderReverseParamsExternal,
              ) => toWrapperPlaceOrderParams(this.market, params),
            ),
          },
        },
      );
    }

    const baseGlobal: PublicKey = getGlobalAddress(this.baseMint.address);
    const baseGlobalVault: PublicKey = getGlobalVaultAddress(
      this.baseMint.address,
    );
    const baseMarketVault: PublicKey = getVaultAddress(
      this.market.address,
      this.baseMint.address,
    );
    const quoteGlobal: PublicKey = getGlobalAddress(this.quoteMint.address);
    const quoteGlobalVault: PublicKey = getGlobalVaultAddress(
      this.quoteMint.address,
    );
    const quoteMarketVault: PublicKey = getVaultAddress(
      this.market.address,
      this.quoteMint.address,
    );
    return createBatchUpdateInstruction(
      {
        market: this.market.address,
        manifestProgram: MANIFEST_PROGRAM_ID,
        owner: this.payer,
        wrapperState: this.wrapper.address,
        baseMint: this.baseMint.address,
        baseGlobal,
        baseGlobalVault,
        baseTokenProgram: this.isBase22
          ? TOKEN_2022_PROGRAM_ID
          : TOKEN_PROGRAM_ID,
        baseMarketVault,
        quoteMint: this.quoteMint.address,
        quoteGlobal,
        quoteGlobalVault,
        quoteTokenProgram: this.isQuote22
          ? TOKEN_2022_PROGRAM_ID
          : TOKEN_PROGRAM_ID,
        quoteMarketVault,
      },
      {
        params: {
          cancels: cancelParams,
          cancelAll,
          orders: placeParams.map(
            (
              params:
                | WrapperPlaceOrderParamsExternal
                | WrapperPlaceOrderReverseParamsExternal,
            ) => toWrapperPlaceOrderParams(this.market, params),
          ),
        },
      },
    );
  }

  /**
   * CancelAll instruction. Cancels all orders on a market. This is discouraged
   * outside of circuit breaker usage because it is less efficient and does not
   * cancel global cleanly. Use batchUpdate instead. This also does not cancel
   * any orders not placed through the wrapper, which includes reverse orders
   * that were reversed.
   *
   * @returns TransactionInstruction
   */
  public cancelAllIx(): TransactionInstruction {
    if (!this.wrapper || !this.payer) {
      throw new Error('Read only');
    }

    // Global not required for cancelAll. If we do cancel a global, then our gas
    // prepayment is abandoned.
    return createBatchUpdateInstruction(
      {
        market: this.market.address,
        manifestProgram: MANIFEST_PROGRAM_ID,
        owner: this.payer,
        wrapperState: this.wrapper.address,
      },
      {
        params: {
          cancels: [],
          cancelAll: true,
          orders: [],
        },
      },
    );
  }

  /**
   * CancelAllOnCore instruction. Cancels all orders on a market directly on the core program,
   * including reverse orders and global orders with rent prepayment.
   *
   * @returns TransactionInstruction[]
   */
  public async cancelAllOnCoreIx(): Promise<TransactionInstruction[]> {
    if (!this.payer) {
      throw new Error('Read only');
    }

    const openOrders: RestingOrder[] = this.market.openOrders();
    const ordersToCancel: {
      orderSequenceNumber: bignum;
      orderIndexHint: null;
    }[] = [];

    for (const openOrder of openOrders) {
      if (openOrder.trader.toBase58() === this.payer.toBase58()) {
        const seqNum: bignum = openOrder.sequenceNumber;
        ordersToCancel.push({
          orderSequenceNumber: seqNum,
          orderIndexHint: null,
        });
      }
    }

    if (ordersToCancel.length === 0) {
      return [];
    }

    const MAX_CANCELS_PER_BATCH = 20;
    const cancelInstructions: TransactionInstruction[] = [];

    for (let i = 0; i < ordersToCancel.length; i += MAX_CANCELS_PER_BATCH) {
      const batchOfCancels = ordersToCancel.slice(i, i + MAX_CANCELS_PER_BATCH);

      const batchedCancelInstruction: TransactionInstruction =
        createBatchUpdateCoreInstruction(
          {
            payer: this.payer,
            market: this.market.address,
            baseMint: this.baseMint.address,
            baseGlobal: getGlobalAddress(this.baseMint.address),
            baseGlobalVault: getGlobalVaultAddress(this.baseMint.address),
            baseTokenProgram: this.isBase22
              ? TOKEN_2022_PROGRAM_ID
              : TOKEN_PROGRAM_ID,
            baseMarketVault: getVaultAddress(
              this.market.address,
              this.baseMint.address,
            ),
            quoteMint: this.quoteMint.address,
            quoteGlobal: getGlobalAddress(this.quoteMint.address),
            quoteGlobalVault: getGlobalVaultAddress(this.quoteMint.address),
            quoteTokenProgram: this.isQuote22
              ? TOKEN_2022_PROGRAM_ID
              : TOKEN_PROGRAM_ID,
            quoteMarketVault: getVaultAddress(
              this.market.address,
              this.quoteMint.address,
            ),
          },
          {
            params: {
              cancels: batchOfCancels,
              orders: [],
              traderIndexHint: null,
            },
          },
        );

      cancelInstructions.push(batchedCancelInstruction);
    }

    return cancelInstructions;
  }

  /**
   * CancelBidsOnCore instruction. Cancels all bid orders on a market directly on the core program,
   * including reverse orders and global orders with rent prepayment.
   *
   * @returns TransactionInstruction[]
   */
  public async cancelBidsOnCoreIx(): Promise<TransactionInstruction[]> {
    if (!this.payer) {
      throw new Error('Read only');
    }

    const bidOrders: RestingOrder[] = this.market.bidsL2();
    const ordersToCancel: {
      orderSequenceNumber: bignum;
      orderIndexHint: null;
    }[] = [];

    for (const bidOrder of bidOrders) {
      if (bidOrder.trader.toBase58() === this.payer.toBase58()) {
        const seqNum: bignum = bidOrder.sequenceNumber;
        ordersToCancel.push({
          orderSequenceNumber: seqNum,
          orderIndexHint: null,
        });
      }
    }

    if (ordersToCancel.length === 0) {
      return [];
    }

    const MAX_CANCELS_PER_BATCH = 25;
    const cancelInstructions: TransactionInstruction[] = [];

    for (let i = 0; i < ordersToCancel.length; i += MAX_CANCELS_PER_BATCH) {
      const batchOfCancels = ordersToCancel.slice(i, i + MAX_CANCELS_PER_BATCH);

      const batchedCancelInstruction: TransactionInstruction =
        createBatchUpdateCoreInstruction(
          {
            payer: this.payer,
            market: this.market.address,
            baseMint: this.baseMint.address,
            baseGlobal: getGlobalAddress(this.baseMint.address),
            baseGlobalVault: getGlobalVaultAddress(this.baseMint.address),
            baseTokenProgram: this.isBase22
              ? TOKEN_2022_PROGRAM_ID
              : TOKEN_PROGRAM_ID,
            baseMarketVault: getVaultAddress(
              this.market.address,
              this.baseMint.address,
            ),
            quoteMint: this.quoteMint.address,
            quoteGlobal: getGlobalAddress(this.quoteMint.address),
            quoteGlobalVault: getGlobalVaultAddress(this.quoteMint.address),
            quoteTokenProgram: this.isQuote22
              ? TOKEN_2022_PROGRAM_ID
              : TOKEN_PROGRAM_ID,
            quoteMarketVault: getVaultAddress(
              this.market.address,
              this.quoteMint.address,
            ),
          },
          {
            params: {
              cancels: batchOfCancels,
              orders: [],
              traderIndexHint: null,
            },
          },
        );

      cancelInstructions.push(batchedCancelInstruction);
    }

    return cancelInstructions;
  }

  /**
   * CancelAsksOnCore instruction. Cancels all ask orders on a market directly on the core program,
   * including reverse orders and global orders with rent prepayment.
   *
   * @returns TransactionInstruction[]
   */
  public async cancelAsksOnCoreIx(): Promise<TransactionInstruction[]> {
    if (!this.payer) {
      throw new Error('Read only');
    }

    const askOrders: RestingOrder[] = this.market.asksL2();
    const ordersToCancel: {
      orderSequenceNumber: bignum;
      orderIndexHint: null;
    }[] = [];

    for (const askOrder of askOrders) {
      if (askOrder.trader.toBase58() === this.payer.toBase58()) {
        const seqNum: bignum = askOrder.sequenceNumber;
        ordersToCancel.push({
          orderSequenceNumber: seqNum,
          orderIndexHint: null,
        });
      }
    }

    if (ordersToCancel.length === 0) {
      return [];
    }

    const MAX_CANCELS_PER_BATCH = 25;
    const cancelInstructions: TransactionInstruction[] = [];

    for (let i = 0; i < ordersToCancel.length; i += MAX_CANCELS_PER_BATCH) {
      const batchOfCancels = ordersToCancel.slice(i, i + MAX_CANCELS_PER_BATCH);

      const batchedCancelInstruction: TransactionInstruction =
        createBatchUpdateCoreInstruction(
          {
            payer: this.payer,
            market: this.market.address,
            baseMint: this.baseMint.address,
            baseGlobal: getGlobalAddress(this.baseMint.address),
            baseGlobalVault: getGlobalVaultAddress(this.baseMint.address),
            baseTokenProgram: this.isBase22
              ? TOKEN_2022_PROGRAM_ID
              : TOKEN_PROGRAM_ID,
            baseMarketVault: getVaultAddress(
              this.market.address,
              this.baseMint.address,
            ),
            quoteMint: this.quoteMint.address,
            quoteGlobal: getGlobalAddress(this.quoteMint.address),
            quoteGlobalVault: getGlobalVaultAddress(this.quoteMint.address),
            quoteTokenProgram: this.isQuote22
              ? TOKEN_2022_PROGRAM_ID
              : TOKEN_PROGRAM_ID,
            quoteMarketVault: getVaultAddress(
              this.market.address,
              this.quoteMint.address,
            ),
          },
          {
            params: {
              cancels: batchOfCancels,
              orders: [],
              traderIndexHint: null,
            },
          },
        );

      cancelInstructions.push(batchedCancelInstruction);
    }

    return cancelInstructions;
  }

  /**
   * killSwitchMarket transactions. Pulls all orders
   * and withdraws all balances from the market in two transactions
   *
   * @param payer PublicKey of the trader
   *
   * @returns TransactionSignatures[]
   */
  public async killSwitchMarket(
    payerKeypair: Keypair,
  ): Promise<TransactionSignature[]> {
    await this.market.reload(this.connection);
    const cancelAllIx = this.cancelAllIx();
    const cancelAllTx = new Transaction();
    const cancelAllSig = await sendAndConfirmTransaction(
      this.connection,
      cancelAllTx.add(cancelAllIx),
      [payerKeypair],
      {
        skipPreflight: true,
        commitment: 'confirmed',
      },
    );
    // TOOD: Merge this into one transaction
    await this.market.reload(this.connection);
    const withdrawAllIx = this.withdrawAllIx();
    const withdrawAllTx = new Transaction();
    const withdrawAllSig = await sendAndConfirmTransaction(
      this.connection,
      withdrawAllTx.add(...withdrawAllIx),
      [payerKeypair],
      {
        skipPreflight: true,
        commitment: 'confirmed',
      },
    );
    return [cancelAllSig, withdrawAllSig];
  }

  /**
   * CreateGlobalCreate instruction. Creates the global account. Should be used only once per mint.
   *
   * @param connection Connection to pull mint info
   * @param payer PublicKey of the trader
   * @param globalMint PublicKey of the globalMint
   *
   * @returns Promise<TransactionInstruction>
   */
  private static async createGlobalCreateIx(
    connection: Connection,
    payer: PublicKey,
    globalMint: PublicKey,
  ): Promise<TransactionInstruction> {
    const global: PublicKey = getGlobalAddress(globalMint);
    const globalVault: PublicKey = getGlobalVaultAddress(globalMint);
    const globalMintAccountInfo: AccountInfo<Buffer> =
      (await connection.getAccountInfo(globalMint))!;
    const mint: Mint = unpackMint(
      globalMint,
      globalMintAccountInfo,
      globalMintAccountInfo.owner,
    );
    const is22: boolean = mint.tlvData.length > 0;
    return createGlobalCreateInstruction({
      payer,
      global,
      mint: globalMint,
      globalVault,
      tokenProgram: is22 ? TOKEN_2022_PROGRAM_ID : TOKEN_PROGRAM_ID,
    });
  }

  /**
   * CreateGlobalAddTrader instruction. Adds a new trader to the global account.
   * Static because it does not require a wrapper.
   *
   * @param payer PublicKey of the trader
   * @param globalMint PublicKey of the globalMint
   *
   * @returns TransactionInstruction
   */
  public static createGlobalAddTraderIx(
    payer: PublicKey,
    globalMint: PublicKey,
  ): TransactionInstruction {
    const global: PublicKey = getGlobalAddress(globalMint);
    return createGlobalAddTraderInstruction({
      payer,
      global,
    });
  }

  /**
   * Global deposit instruction. Static because it does not require a wrapper.
   *
   * @param connection Connection to pull mint info
   * @param payer PublicKey of the trader
   * @param globalMint PublicKey for global mint deposit.
   * @param amountTokens Number of tokens to deposit.
   *
   * @returns Promise<TransactionInstruction>
   */
  public static async globalDepositIx(
    connection: Connection,
    payer: PublicKey,
    globalMint: PublicKey,
    amountTokens: number,
  ): Promise<TransactionInstruction> {
    const globalAddress: PublicKey = getGlobalAddress(globalMint);
    const globalVault: PublicKey = getGlobalVaultAddress(globalMint);
    const globalMintAccountInfo: AccountInfo<Buffer> =
      (await connection.getAccountInfo(globalMint))!;
    const mint: Mint = unpackMint(
      globalMint,
      globalMintAccountInfo,
      globalMintAccountInfo.owner,
    );
    const is22: boolean = mint.tlvData.length > 0;
    const traderTokenAccount: PublicKey = getAssociatedTokenAddressSync(
      globalMint,
      payer,
      true,
      is22 ? TOKEN_2022_PROGRAM_ID : TOKEN_PROGRAM_ID,
    );
    const mintDecimals = mint.decimals;
    const amountAtoms = Math.round(amountTokens * 10 ** mintDecimals);

    return createGlobalDepositInstruction(
      {
        payer: payer,
        global: globalAddress,
        mint: globalMint,
        globalVault: globalVault,
        traderToken: traderTokenAccount,
        tokenProgram: is22 ? TOKEN_2022_PROGRAM_ID : TOKEN_PROGRAM_ID,
      },
      {
        params: {
          amountAtoms,
        },
      },
    );
  }

  /**
   * Global withdraw instruction. Static because it does not require a wrapper.
   *
   * @param connection Connection to pull mint info
   * @param payer PublicKey of the trader
   * @param globalMint PublicKey for global mint withdraw.
   * @param amountTokens Number of tokens to withdraw.
   *
   * @returns Promise<TransactionInstruction>
   */
  public static async globalWithdrawIx(
    connection: Connection,
    payer: PublicKey,
    globalMint: PublicKey,
    amountTokens: number,
  ): Promise<TransactionInstruction> {
    const globalAddress: PublicKey = getGlobalAddress(globalMint);
    const globalVault: PublicKey = getGlobalVaultAddress(globalMint);
    const globalMintAccountInfo: AccountInfo<Buffer> =
      (await connection.getAccountInfo(globalMint))!;
    const mint: Mint = unpackMint(
      globalMint,
      globalMintAccountInfo,
      globalMintAccountInfo.owner,
    );
    const is22: boolean = mint.tlvData.length > 0;
    const traderTokenAccount: PublicKey = getAssociatedTokenAddressSync(
      globalMint,
      payer,
      true,
      is22 ? TOKEN_2022_PROGRAM_ID : TOKEN_PROGRAM_ID,
    );
    const mintDecimals = mint.decimals;
    const amountAtoms = Math.round(amountTokens * 10 ** mintDecimals);

    return createGlobalWithdrawInstruction(
      {
        payer: payer,
        global: globalAddress,
        mint: globalMint,
        globalVault: globalVault,
        traderToken: traderTokenAccount,
        tokenProgram: is22 ? TOKEN_2022_PROGRAM_ID : TOKEN_PROGRAM_ID,
      },
      {
        params: {
          amountAtoms,
        },
      },
    );
  }
}

/**
 * Same as the autogenerated WrapperPlaceOrderParams except price here is a number.
 */
export type WrapperPlaceOrderParamsExternal = {
  /** Number of base tokens in the order. */
  numBaseTokens: number;
  /** Price as float in quote tokens per base tokens. */
  tokenPrice: number;
  /** Boolean for whether this order is on the bid side. */
  isBid: boolean;
  /** Last slot before this order is invalid and will be removed. If below
   * 10_000_000, then will be treated as slots in force when it lands in the
   * wrapper onchain.
   */
  lastValidSlot: number;
  /** Type of order (Limit, PostOnly, ...). */
  orderType: OrderType;
  /** Client order id used for cancelling orders. Does not need to be unique. */
  clientOrderId: bignum;
};

/**
 * Same as the autogenerated WrapperPlaceOrderParamsExternal except lastValidSlot is spread.
 */
export type WrapperPlaceOrderReverseParamsExternal = {
  /** Number of base tokens in the order. */
  numBaseTokens: number;
  /** Price as float in quote tokens per base tokens. */
  tokenPrice: number;
  /** Boolean for whether this order is on the bid side. */
  isBid: boolean;
  /** Spread in bps. Can be between 0 and 6553 in increments of .1 */
  spreadBps: number;
  /** Type of order (Limit, PostOnly, ...). */
  orderType: OrderType;
  /** Client order id used for cancelling orders. Does not need to be unique. */
  clientOrderId: bignum;
};

function toWrapperPlaceOrderParams(
  market: Market,
  wrapperPlaceOrderParamsExternal:
    | WrapperPlaceOrderParamsExternal
    | WrapperPlaceOrderReverseParamsExternal,
): WrapperPlaceOrderParams {
  // Convert spread bps based on order type
  // ReverseTight uses 10^-8 precision, Reverse uses 10^-5 precision
  if ('spreadBps' in wrapperPlaceOrderParamsExternal) {
    const originalSpreadBps = wrapperPlaceOrderParamsExternal['spreadBps'];
    const multiplier =
      wrapperPlaceOrderParamsExternal['orderType'] === OrderType.ReverseTight
        ? 10000
        : 10;
    wrapperPlaceOrderParamsExternal['lastValidSlot'] = Math.floor(
      originalSpreadBps * multiplier,
    );
  }

  // Choose max exponent based on ordertype
  // TODO: warn if the exponent causes resolution issues
  //       with the desired price
  let maxExponent = 8;
  if (wrapperPlaceOrderParamsExternal['orderType'] == OrderType.Reverse) {
    maxExponent -= 5;
  } else if (
    wrapperPlaceOrderParamsExternal['orderType'] == OrderType.ReverseTight
  ) {
    maxExponent -= 8;
  }

  const quoteAtomsPerToken = 10 ** market.quoteDecimals();
  const baseAtomsPerToken = 10 ** market.baseDecimals();
  // Converts token price to atom price since not always equal
  // Ex. BONK/USDC = 0.00001854 USDC tokens/BONK tokens -> 0.0001854 USDC Atoms/BONK Atoms
  const priceQuoteAtomsPerBaseAtoms =
    wrapperPlaceOrderParamsExternal.tokenPrice *
    (quoteAtomsPerToken / baseAtomsPerToken);
  const { priceMantissa, priceExponent } = toMantissaAndExponent(
    priceQuoteAtomsPerBaseAtoms,
    maxExponent,
  );
  const numBaseAtoms: bignum = Math.floor(
    wrapperPlaceOrderParamsExternal.numBaseTokens * baseAtomsPerToken,
  );

  return {
    ...(wrapperPlaceOrderParamsExternal as WrapperPlaceOrderParamsExternal),
    baseAtoms: numBaseAtoms,
    priceMantissa,
    priceExponent,
  };
}

function calculateMantissa(value: number, exp: number): number {
  return Math.round(value * Math.pow(10, -exp));
}

export function toMantissaAndExponent(
  input: number,
  maxExponent: number,
): {
  priceMantissa: number;
  priceExponent: number;
} {
  let exponent = 0;
  const uInt32Max = 4_294_967_296;

  // prevent overflow when casting to u32
  while (
    exponent < maxExponent &&
    calculateMantissa(input, exponent) > uInt32Max
  ) {
    exponent += 1;
  }

  // prevent underflow and maximize precision available
  while (exponent > -20 && calculateMantissa(input, exponent - 1) < uInt32Max) {
    exponent -= 1;
  }

  return {
    priceMantissa: calculateMantissa(input, exponent),
    priceExponent: exponent,
  };
}
