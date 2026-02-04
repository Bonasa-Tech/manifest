import {
  PublicKey,
  Connection,
  TransactionInstruction,
  Keypair,
  Signer,
  SystemProgram,
  RpcResponseAndContext,
  AccountInfo,
} from '@solana/web3.js';
import { bignum } from '@metaplex-foundation/beet';
import { publicKeyBeet } from './utils/beet';
import { publicKey as beetPublicKey } from '@metaplex-foundation/beet-solana';
import { deserializeRedBlackTree } from './utils/redBlackTree';
import { convertU128, toNum } from './utils/numbers';
import {
  FIXED_MANIFEST_HEADER_SIZE,
  NIL,
  NO_EXPIRATION_LAST_VALID_SLOT,
} from './constants';
import {
  claimedSeatBeet,
  ClaimedSeat as ClaimedSeatRaw,
  createCreateMarketInstruction,
  OrderType,
  PROGRAM_ID,
  restingOrderBeet,
  RestingOrder as RestingOrderRaw,
} from './manifest';
import { getVaultAddress } from './utils/market';
import { TOKEN_2022_PROGRAM_ID } from '@solana/spl-token';
import BN from 'bn.js';

/**
 * RestingOrder on the market.
 */
export type RestingOrder = {
  /** Trader public key. */
  trader: PublicKey;
  /** Number of base tokens remaining in the order. */
  numBaseTokens: bignum;
  /** Last slot before this order is invalid and will be removed. */
  lastValidSlot: bignum;
  /** Exchange defined sequenceNumber for this order, guaranteed to be unique. */
  sequenceNumber: bignum;
  /** Price as float in tokens of quote per tokens of base. */
  tokenPrice: number;
  /** OrderType: 🌎 or Limit or PostOnly */
  orderType: OrderType;
  /** Spread in bps for reverse orders. Reverse: [0.1-6553.5] bps (0.1 bps increments). ReverseTight: [0.0001-6.5535] bps (0.0001 bps increments). Only set for orderType === Reverse || ReverseTight. */
  spreadBps?: number;
};

/**
 * ClaimedSeat on the market.
 */
export type ClaimedSeat = {
  /** Public key of the trader. */
  publicKey: PublicKey;
  /** Balance of base atoms that are withdrawable (excluding in open orders). */
  baseBalance: bignum;
  /** Balance of quote atoms that are withdrawable (excluding in open orders). */
  quoteBalance: bignum;
  /** Quote volume traded by this trader in atoms. */
  quoteVolume: bignum;
};

/**
 * MarketData is all information stored on a market account.
 */
export interface MarketData {
  /** Version of the struct, included in case features are added later which use the padding. */
  version: number;
  /** Number of decimals for the baseMint (i.e. baseMintDecimals = 6 -> 1 baseAtom = .000001 baseToken). */
  baseMintDecimals: number;
  /** Number of decimals for the quoteMint (i.e. quoteMintDecimals = 6 -> 1 quoteAtom = .000001 quoteToken). */
  quoteMintDecimals: number;
  /** Public key for the base mint. */
  baseMint: PublicKey;
  /** Public key for the quote mint. */
  quoteMint: PublicKey;
  /** Current next order sequence number. */
  orderSequenceNumber: bigint;
  /** Number of bytes used in the dynamic portion of the market account. */
  numBytesAllocated: number;
  /** Sorted array of resting orders for bids currently on the orderbook. */
  bids: RestingOrder[];
  /** Sorted array of resting orders for asks currently on the orderbook. */
  asks: RestingOrder[];
  /** Array of all claimed seats. */
  claimedSeats: ClaimedSeat[];
  /** Quote volume in atoms. */
  quoteVolumeAtoms: bigint;
}

/**
 * Market object used for reading data from a manifest market.
 */
export class Market {
  /** Public key for the market account. */
  address: PublicKey;
  /** Deserialized data. */
  private data: MarketData;
  /** Last updated slot. */
  private slot: number;

  /**
   * Constructs a Market object.
   *
   * @param address The `PublicKey` of the market account
   * @param data Deserialized market data
   */
  private constructor({
    address,
    data,
    slot,
  }: {
    address: PublicKey;
    data: MarketData;
    slot: number;
  }) {
    this.address = address;
    this.data = data;
    this.slot = slot;
  }

  /**
   * Returns a `Market` for a given address, a data buffer
   *
   * @param marketAddress The `PublicKey` of the market account
   * @param buffer The buffer holding the market account data
   */
  static loadFromBuffer({
    address,
    buffer,
    slot,
  }: {
    address: PublicKey;
    buffer: Buffer;
    slot?: number;
  }): Market {
    const marketData = Market.deserializeMarketBuffer(
      buffer,
      slot ?? NO_EXPIRATION_LAST_VALID_SLOT,
    );
    // When we are not given a slot, pretend it is time zero to show everything.
    return new Market({
      address,
      data: marketData,
      slot: slot ?? NO_EXPIRATION_LAST_VALID_SLOT,
    });
  }

  /**
   * Returns a `Market` for a given address, a data buffer
   *
   * @param connection The Solana `Connection` object
   * @param address The `PublicKey` of the market account
   */
  static async loadFromAddress({
    connection,
    address,
  }: {
    connection: Connection;
    address: PublicKey;
  }): Promise<Market> {
    const [buffer, slot]: [Buffer | undefined, number] = await connection
      .getAccountInfoAndContext(address)
      .then(
        (
          getAccountInfoAndContext: RpcResponseAndContext<AccountInfo<Buffer> | null>,
        ) => {
          return [
            getAccountInfoAndContext.value?.data,
            getAccountInfoAndContext.context.slot,
          ];
        },
      );

    if (buffer === undefined) {
      throw new Error(`Failed to load ${address}`);
    }
    return Market.loadFromBuffer({ address, buffer, slot });
  }

  /**
   * Updates the data in a Market.
   *
   * @param connection The Solana `Connection` object
   */
  public async reload(connection: Connection): Promise<void> {
    const [buffer, slot]: [Buffer | undefined, number] = await connection
      .getAccountInfoAndContext(this.address)
      .then(
        (
          getAccountInfoAndContext: RpcResponseAndContext<AccountInfo<Buffer> | null>,
        ) => {
          return [
            getAccountInfoAndContext.value?.data,
            getAccountInfoAndContext.context.slot,
          ];
        },
      );
    if (buffer === undefined) {
      throw new Error(`Failed to load ${this.address}`);
    }
    this.slot = slot;
    this.data = Market.deserializeMarketBuffer(buffer, slot);
  }

  /**
   * Get the amount in tokens of balance that is deposited on this market, does
   * not include tokens currently in open orders.
   *
   * @param trader PublicKey of the trader to check balance of
   * @param isBase boolean for whether this is checking base or quote
   *
   * @returns number in tokens
   */
  public getWithdrawableBalanceTokens(
    trader: PublicKey,
    isBase: boolean,
  ): number {
    const filteredSeats = this.data.claimedSeats.filter((claimedSeat) => {
      return claimedSeat.publicKey.equals(trader);
    });
    // No seat claimed.
    if (filteredSeats.length == 0) {
      return 0;
    }
    const seat: ClaimedSeat = filteredSeats[0];
    const withdrawableBalance = isBase
      ? toNum(seat.baseBalance) / 10 ** this.baseDecimals()
      : toNum(seat.quoteBalance) / 10 ** this.quoteDecimals();
    return withdrawableBalance;
  }

  /**
   * Get the total amount in atoms of balance that is deposited on this market, split
   * by base, quote, and whether in orders or not for the whole market.
   *
   * @returns {
   *    baseWithdrawableBalanceAtoms: number,
   *    quoteWithdrawableBalanceAtoms: number,
   *    baseOpenOrdersBalanceAtoms: number,
   *    quoteOpenOrdersBalanceAtoms: number
   * }
   */
  public getMarketBalances(): {
    baseWithdrawableBalanceAtoms: number;
    quoteWithdrawableBalanceAtoms: number;
    baseOpenOrdersBalanceAtoms: number;
    quoteOpenOrdersBalanceAtoms: number;
  } {
    const asks: RestingOrder[] = this.asks();
    const bids: RestingOrder[] = this.bids();

    const quoteOpenOrdersBalanceAtoms: number = bids
      .filter((restingOrder: RestingOrder) => {
        return restingOrder.orderType != OrderType.Global;
      })
      .map((restingOrder: RestingOrder) => {
        return Math.ceil(
          Number(restingOrder.numBaseTokens) *
            restingOrder.tokenPrice *
            10 ** this.data.quoteMintDecimals -
            // Force float precision to not round up on an integer.
            0.00001,
        );
      })
      .reduce((sum, current) => sum + current, 0);
    const baseOpenOrdersBalanceAtoms: number = asks
      .filter((restingOrder: RestingOrder) => {
        return restingOrder.orderType != OrderType.Global;
      })
      .map((restingOrder: RestingOrder) => {
        return (
          Number(restingOrder.numBaseTokens) * 10 ** this.data.baseMintDecimals
        );
      })
      .reduce((sum, current) => sum + current, 0);

    const quoteWithdrawableBalanceAtoms: number = this.data.claimedSeats
      .map((claimedSeat: ClaimedSeat) => {
        return Number(claimedSeat.quoteBalance);
      })
      .reduce((sum, current) => sum + current, 0);
    const baseWithdrawableBalanceAtoms: number = this.data.claimedSeats
      .map((claimedSeat: ClaimedSeat) => {
        return Number(claimedSeat.baseBalance);
      })
      .reduce((sum, current) => sum + current, 0);

    return {
      baseWithdrawableBalanceAtoms,
      quoteWithdrawableBalanceAtoms,
      baseOpenOrdersBalanceAtoms,
      quoteOpenOrdersBalanceAtoms,
    };
  }

  /**
   * Get the amount in tokens of balance that is deposited on this market for a trader, split
   * by base, quote, and whether in orders or not.
   *
   * @param trader PublicKey of the trader to check balance of
   *
   * @returns {
   *    baseWithdrawableBalanceTokens: number,
   *    quoteWithdrawableBalanceTokens: number,
   *    baseOpenOrdersBalanceTokens: number,
   *    quoteOpenOrdersBalanceTokens: number
   * }
   */
  public getBalances(trader: PublicKey): {
    baseWithdrawableBalanceTokens: number;
    quoteWithdrawableBalanceTokens: number;
    baseOpenOrdersBalanceTokens: number;
    quoteOpenOrdersBalanceTokens: number;
  } {
    const filteredSeats = this.data.claimedSeats.filter((claimedSeat) => {
      return claimedSeat.publicKey.equals(trader);
    });
    // No seat claimed.
    if (filteredSeats.length == 0) {
      return {
        baseWithdrawableBalanceTokens: 0,
        quoteWithdrawableBalanceTokens: 0,
        baseOpenOrdersBalanceTokens: 0,
        quoteOpenOrdersBalanceTokens: 0,
      };
    }
    const seat: ClaimedSeat = filteredSeats[0];

    const asks: RestingOrder[] = this.asks();
    const bids: RestingOrder[] = this.bids();
    const baseOpenOrdersBalanceTokens: number = asks
      .filter((ask) => ask.trader.equals(trader))
      .reduce((sum, ask) => sum + Number(ask.numBaseTokens), 0);
    const quoteOpenOrdersBalanceTokens: number = bids
      .filter((bid) => bid.trader.equals(trader))
      .reduce(
        (sum, bid) => sum + Number(bid.numBaseTokens) * Number(bid.tokenPrice),
        0,
      );

    const quoteWithdrawableBalanceTokens: number =
      toNum(seat.quoteBalance) / 10 ** this.quoteDecimals();
    const baseWithdrawableBalanceTokens: number =
      toNum(seat.baseBalance) / 10 ** this.baseDecimals();
    return {
      baseWithdrawableBalanceTokens,
      quoteWithdrawableBalanceTokens,
      baseOpenOrdersBalanceTokens,
      quoteOpenOrdersBalanceTokens,
    };
  }

  /**
   * Get the amount in tokens of balance that is deposited on this market for a trader, split
   * by base, quote, and whether in orders or not but ignoring orders that use
   * global balances.
   *
   * @param trader PublicKey of the trader to check balance of
   *
   * @returns {
   *    baseWithdrawableBalanceTokens: number,
   *    quoteWithdrawableBalanceTokens: number,
   *    baseOpenOrdersBalanceTokens: number,
   *    quoteOpenOrdersBalanceTokens: number
   * }
   */
  public getMarketBalancesForTrader(trader: PublicKey): {
    baseWithdrawableBalanceTokens: number;
    quoteWithdrawableBalanceTokens: number;
    baseOpenOrdersBalanceTokens: number;
    quoteOpenOrdersBalanceTokens: number;
  } {
    const filteredSeats = this.data.claimedSeats.filter((claimedSeat) => {
      return claimedSeat.publicKey.equals(trader);
    });
    // No seat claimed.
    if (filteredSeats.length == 0) {
      return {
        baseWithdrawableBalanceTokens: 0,
        quoteWithdrawableBalanceTokens: 0,
        baseOpenOrdersBalanceTokens: 0,
        quoteOpenOrdersBalanceTokens: 0,
      };
    }
    const seat: ClaimedSeat = filteredSeats[0];

    const asks: RestingOrder[] = this.asks();
    const bids: RestingOrder[] = this.bids();
    const baseOpenOrdersBalanceTokens: number = asks
      .filter(
        (ask) => ask.trader.equals(trader) && ask.orderType != OrderType.Global,
      )
      .reduce((sum, ask) => sum + Number(ask.numBaseTokens), 0);
    const quoteOpenOrdersBalanceTokens: number = bids
      .filter(
        (bid) => bid.trader.equals(trader) && bid.orderType != OrderType.Global,
      )
      .reduce(
        (sum, bid) => sum + Number(bid.numBaseTokens) * Number(bid.tokenPrice),
        0,
      );

    const quoteWithdrawableBalanceTokens: number =
      toNum(seat.quoteBalance) / 10 ** this.quoteDecimals();
    const baseWithdrawableBalanceTokens: number =
      toNum(seat.baseBalance) / 10 ** this.baseDecimals();
    return {
      baseWithdrawableBalanceTokens,
      quoteWithdrawableBalanceTokens,
      baseOpenOrdersBalanceTokens,
      quoteOpenOrdersBalanceTokens,
    };
  }

  /**
   * Gets the base mint of the market
   *
   * @returns PublicKey
   */
  public baseMint(): PublicKey {
    return this.data.baseMint;
  }

  /**
   * Gets the quote mint of the market
   *
   * @returns PublicKey
   */
  public quoteMint(): PublicKey {
    return this.data.quoteMint;
  }

  /**
   * Gets the base decimals of the market
   *
   * @returns number
   */
  public baseDecimals(): number {
    return this.data.baseMintDecimals;
  }

  /**
   * Gets the base decimals of the market
   *
   * @returns number
   */
  public quoteDecimals(): number {
    return this.data.quoteMintDecimals;
  }

  /**
   * Check whether a given public key has a claimed seat on the market
   *
   * @param trader PublicKey of the trader
   *
   * @returns boolean
   */
  public hasSeat(trader: PublicKey): boolean {
    const filteredSeats = this.data.claimedSeats.filter((claimedSeat) => {
      return claimedSeat.publicKey.equals(trader);
    });
    return filteredSeats.length > 0;
  }

  /**
   * Get all open bids on the market.
   *
   * @returns RestingOrder[]
   */
  public bids(): RestingOrder[] {
    return this.data.bids;
  }

  /**
   * Get all open asks on the market.
   *
   * @returns RestingOrder[]
   */
  public asks(): RestingOrder[] {
    return this.data.asks;
  }

  /**
   * Get the most competitive bid price
   *
   * @returns number | undefined
   */
  public bestBidPrice(): number | undefined {
    return this.data.bids[this.data.bids.length - 1]?.tokenPrice;
  }

  /**
   * Get the most competitive ask price.
   *
   * @returns number | undefined
   */
  public bestAskPrice(): number | undefined {
    return this.data.asks[this.data.asks.length - 1]?.tokenPrice;
  }

  /**
   * Get all open bids on the market ordered from most competitive to least.
   *
   * @returns RestingOrder[]
   */
  public bidsL2(): RestingOrder[] {
    return this.data.bids.slice().reverse();
  }

  /**
   * Get all open asks on the market ordered from most competitive to least.
   *
   * @returns RestingOrder[]
   */
  public asksL2(): RestingOrder[] {
    return this.data.asks.slice().reverse();
  }

  /**
   * Get all open orders on the market.
   *
   * @returns RestingOrder[]
   */
  public openOrders(): RestingOrder[] {
    return [...this.data.bids, ...this.data.asks];
  }

  /**
   * Get all claimed seats on the market.
   *
   * @returns ClaimedSeat[]
   */
  public claimedSeats(): ClaimedSeat[] {
    return this.data.claimedSeats;
  }

  /**
   * Gets the quote volume traded over the lifetime of the market.
   *
   * @returns bigint
   */
  public quoteVolume(): bigint {
    return this.data.quoteVolumeAtoms;
  }

  /**
   * Print all information loaded about the market in a human readable format.
   */
  public prettyPrint(): void {
    console.log('');
    console.log(`Market: ${this.address}`);
    console.log(`========================`);
    console.log(`Version: ${this.data.version}`);
    console.log(`BaseMint: ${this.data.baseMint.toBase58()}`);
    console.log(`QuoteMint: ${this.data.quoteMint.toBase58()}`);
    console.log(`OrderSequenceNumber: ${this.data.orderSequenceNumber}`);
    console.log(`NumBytesAllocated: ${this.data.numBytesAllocated}`);
    console.log('Bids:');
    this.data.bids.forEach((bid) => {
      console.log(
        `trader: ${bid.trader} numBaseTokens: ${bid.numBaseTokens} token price: ${bid.tokenPrice} lastValidSlot: ${bid.lastValidSlot} sequenceNumber: ${bid.sequenceNumber}`,
      );
    });
    console.log('Asks:');
    this.data.asks.forEach((ask) => {
      console.log(
        `trader: ${ask.trader} numBaseTokens: ${ask.numBaseTokens} token price: ${ask.tokenPrice} lastValidSlot: ${ask.lastValidSlot} sequenceNumber: ${ask.sequenceNumber}`,
      );
    });
    console.log('ClaimedSeats:');
    this.data.claimedSeats.forEach((claimedSeat) => {
      console.log(
        `publicKey: ${claimedSeat.publicKey.toBase58()} baseBalance: ${claimedSeat.baseBalance} quoteBalance: ${claimedSeat.quoteBalance} quoteVolume: ${claimedSeat.quoteVolume}`,
      );
    });
    console.log(`========================`);
  }

  /**
   * Deserializes market data from a given buffer and returns a `Market` object
   *
   * This includes both the fixed and dynamic parts of the market.
   * https://github.com/Bonasa-Tech/manifest/blob/main/programs/manifest/src/state/market.rs
   *
   * @param data The data buffer to deserialize
   * @param currentSlot Number that is the cutoff for order expiration.
   */
  static deserializeMarketBuffer(
    data: Buffer,
    currentSlot: number,
  ): MarketData {
    let offset = 0;
    // Deserialize the market header
    const _discriminant = data.readBigUInt64LE(0);
    offset += 8;

    const version = data.readUInt8(offset);
    offset += 1;
    const baseMintDecimals = data.readUInt8(offset);
    offset += 1;
    const quoteMintDecimals = data.readUInt8(offset);
    offset += 1;
    const _baseVaultBump = data.readUInt8(offset);
    offset += 1;
    const _quoteVaultBump = data.readUInt8(offset);
    offset += 1;
    // 3 bytes of unused padding.
    offset += 3;

    const baseMint = beetPublicKey.read(data, offset);
    offset += beetPublicKey.byteSize;
    const quoteMint = beetPublicKey.read(data, offset);
    offset += beetPublicKey.byteSize;
    const _baseVault = beetPublicKey.read(data, offset);
    offset += beetPublicKey.byteSize;
    const _quoteVault = beetPublicKey.read(data, offset);
    offset += beetPublicKey.byteSize;

    const orderSequenceNumber = data.readBigUInt64LE(offset);
    offset += 8;

    const numBytesAllocated = data.readUInt32LE(offset);
    offset += 4;

    const bidsRootIndex = data.readUInt32LE(offset);
    offset += 4;
    const _bidsBestIndex = data.readUInt32LE(offset);
    offset += 4;

    const asksRootIndex = data.readUInt32LE(offset);
    offset += 4;
    const _askBestIndex = data.readUInt32LE(offset);
    offset += 4;

    const claimedSeatsRootIndex = data.readUInt32LE(offset);
    offset += 4;

    const _freeListHeadIndex = data.readUInt32LE(offset);
    offset += 4;

    const _padding2 = data.readUInt32LE(offset);
    offset += 4;

    const quoteVolumeAtoms: bigint = data.readBigUInt64LE(offset);
    offset += 8;

    // _padding3: [u64; 8],

    const bids: RestingOrder[] =
      bidsRootIndex != NIL
        ? deserializeRedBlackTree(
            data.subarray(FIXED_MANIFEST_HEADER_SIZE),
            bidsRootIndex,
            restingOrderBeet,
          )
            .map((restingOrderInternal: RestingOrderRaw) => {
              const baseOrder = {
                trader: publicKeyBeet.deserialize(
                  data.subarray(
                    Number(restingOrderInternal.traderIndex) +
                      16 +
                      FIXED_MANIFEST_HEADER_SIZE,
                    Number(restingOrderInternal.traderIndex) +
                      48 +
                      FIXED_MANIFEST_HEADER_SIZE,
                  ),
                )[0].publicKey,
                numBaseTokens:
                  toNum(restingOrderInternal.numBaseAtoms) /
                  10 ** baseMintDecimals,
                tokenPrice:
                  convertU128(restingOrderInternal.price) *
                  10 ** (baseMintDecimals - quoteMintDecimals),
                ...restingOrderInternal,
              };

              if (
                restingOrderInternal.orderType === OrderType.Reverse ||
                restingOrderInternal.orderType === OrderType.ReverseTight
              ) {
                const paddingBytes = restingOrderInternal.padding;
                const spreadRaw =
                  paddingBytes[0] |
                  (paddingBytes[1] << 8) |
                  (paddingBytes[2] << 16) |
                  (paddingBytes[3] << 24);

                // Convert spreadRaw to bps based on order type
                // Reverse: spreadRaw is in units of 1/100,000 (base), divide by 10 to get bps
                // ReverseTight: spreadRaw is in units of 1/100,000,000 (base), divide by 10,000 to get bps
                const spreadBps =
                  restingOrderInternal.orderType === OrderType.ReverseTight
                    ? spreadRaw / 10000
                    : spreadRaw / 10;

                return {
                  ...baseOrder,
                  spreadBps,
                };
              }

              return baseOrder;
            })
            .filter((bid: RestingOrder) => {
              return (
                bid.lastValidSlot == NO_EXPIRATION_LAST_VALID_SLOT ||
                Number(bid.lastValidSlot) > currentSlot
              );
            })
        : [];

    const asks: RestingOrder[] =
      asksRootIndex != NIL
        ? deserializeRedBlackTree(
            data.subarray(FIXED_MANIFEST_HEADER_SIZE),
            asksRootIndex,
            restingOrderBeet,
          )
            .map((restingOrderInternal: RestingOrderRaw) => {
              const baseOrder = {
                trader: publicKeyBeet.deserialize(
                  data.subarray(
                    Number(restingOrderInternal.traderIndex) +
                      16 +
                      FIXED_MANIFEST_HEADER_SIZE,
                    Number(restingOrderInternal.traderIndex) +
                      48 +
                      FIXED_MANIFEST_HEADER_SIZE,
                  ),
                )[0].publicKey,
                numBaseTokens:
                  toNum(restingOrderInternal.numBaseAtoms) /
                  10 ** baseMintDecimals,
                tokenPrice:
                  convertU128(restingOrderInternal.price) *
                  10 ** (baseMintDecimals - quoteMintDecimals),
                ...restingOrderInternal,
              };

              if (
                restingOrderInternal.orderType === OrderType.Reverse ||
                restingOrderInternal.orderType === OrderType.ReverseTight
              ) {
                const paddingBytes = restingOrderInternal.padding;
                const spreadRaw =
                  paddingBytes[0] |
                  (paddingBytes[1] << 8) |
                  (paddingBytes[2] << 16) |
                  (paddingBytes[3] << 24);

                // Convert spreadRaw to bps based on order type
                // Reverse: spreadRaw is in units of 1/100,000 (base), divide by 10 to get bps
                // ReverseTight: spreadRaw is in units of 1/100,000,000 (base), divide by 10,000 to get bps
                const spreadBps =
                  restingOrderInternal.orderType === OrderType.ReverseTight
                    ? spreadRaw / 10000
                    : spreadRaw / 10;

                return {
                  ...baseOrder,
                  spreadBps,
                };
              }

              return baseOrder;
            })
            .filter((ask: RestingOrder) => {
              return (
                ask.lastValidSlot == NO_EXPIRATION_LAST_VALID_SLOT ||
                Number(ask.lastValidSlot) > currentSlot
              );
            })
        : [];

    const claimedSeats: ClaimedSeat[] =
      claimedSeatsRootIndex != NIL
        ? deserializeRedBlackTree(
            data.subarray(FIXED_MANIFEST_HEADER_SIZE),
            claimedSeatsRootIndex,
            claimedSeatBeet,
          ).map((claimedSeatInternal: ClaimedSeatRaw) => {
            return {
              publicKey: claimedSeatInternal.trader,
              baseBalance: claimedSeatInternal.baseWithdrawableBalance,
              quoteBalance: claimedSeatInternal.quoteWithdrawableBalance,
              quoteVolume: claimedSeatInternal.quoteVolume,
            };
          })
        : [];

    return {
      version,
      baseMintDecimals,
      quoteMintDecimals,
      baseMint,
      quoteMint,
      orderSequenceNumber,
      numBytesAllocated,
      bids,
      asks,
      claimedSeats,
      quoteVolumeAtoms,
    };
  }

  static async findByMints(
    connection: Connection,
    baseMint: PublicKey,
    quoteMint: PublicKey,
  ): Promise<Market[]> {
    // Based on the MarketFixed struct
    const baseMintOffset = 16;
    const quoteMintOffset = 48;

    const filters = [
      {
        memcmp: {
          offset: baseMintOffset,
          bytes: baseMint.toBase58(),
        },
      },
      {
        memcmp: {
          offset: quoteMintOffset,
          bytes: quoteMint.toBase58(),
        },
      },
    ];

    const accounts = await connection.getProgramAccounts(PROGRAM_ID, {
      filters,
    });

    return accounts
      .map(({ account, pubkey }) =>
        Market.loadFromBuffer({ address: pubkey, buffer: account.data }),
      )
      .sort((a, b) =>
        new BN(b.quoteVolume().toString())
          .sub(new BN(a.quoteVolume().toString()))
          .toNumber(),
      );
  }

  static async setupIxs(
    connection: Connection,
    baseMint: PublicKey,
    quoteMint: PublicKey,
    payer: PublicKey,
  ): Promise<{ ixs: TransactionInstruction[]; signers: Signer[] }> {
    const marketKeypair: Keypair = Keypair.generate();
    const createAccountIx: TransactionInstruction = SystemProgram.createAccount(
      {
        fromPubkey: payer,
        newAccountPubkey: marketKeypair.publicKey,
        space: FIXED_MANIFEST_HEADER_SIZE,
        lamports: await connection.getMinimumBalanceForRentExemption(
          FIXED_MANIFEST_HEADER_SIZE,
        ),
        programId: PROGRAM_ID,
      },
    );

    const market = marketKeypair.publicKey;
    const baseVault = getVaultAddress(market, baseMint);
    const quoteVault = getVaultAddress(market, quoteMint);
    const createMarketIx = createCreateMarketInstruction({
      payer,
      baseMint,
      quoteMint,
      market,
      baseVault,
      quoteVault,
      tokenProgram22: TOKEN_2022_PROGRAM_ID,
    });
    return { ixs: [createAccountIx, createMarketIx], signers: [marketKeypair] };
  }
}
