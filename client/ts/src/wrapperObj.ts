import { Connection, PublicKey } from '@solana/web3.js';
import { bignum } from '@metaplex-foundation/beet';
import { publicKey as beetPublicKey } from '@metaplex-foundation/beet-solana';
import { FIXED_WRAPPER_HEADER_SIZE, NIL } from './constants';
import { OrderType } from './manifest';
import { deserializeRedBlackTree } from './utils/redBlackTree';
import {
  MarketInfo,
  WrapperOpenOrder as WrapperOpenOrderRaw,
  marketInfoBeet,
  wrapperOpenOrderBeet,
} from './wrapper/types';
import { convertU128 } from './utils/numbers';
import BN from 'bn.js';

/**
 * All data stored on a wrapper account.
 */
export interface WrapperData {
  /** Public key for the trader that owns this wrapper. */
  trader: PublicKey;
  /** Array of market infos that have been parsed. */
  marketInfos: WrapperMarketInfo[];
}

/**
 * Parsed market info on a wrapper. Accurate to the last sync.
 */
export interface WrapperMarketInfo {
  /** Public key for market. */
  market: PublicKey;
  /** Location of the traders seat in the market dynamic data. */
  traderIndex: number;
  /** Base balance in atoms. */
  baseBalanceAtoms: bignum;
  /** Quote balance in atoms. */
  quoteBalanceAtoms: bignum;
  /** Quote volume in atoms. */
  quoteVolumeAtoms: bignum;
  /** Open orders. */
  orders: WrapperOpenOrder[];
  /** Last update slot number. */
  lastUpdatedSlot: number;
}

/**
 * OpenOrder on a wrapper. Accurate as of the latest sync.
 */
export interface WrapperOpenOrder {
  /** Price as float in atoms of quote per atoms of base. */
  price: number;
  /** Client order id used for cancelling orders. Does not need to be unique. */
  clientOrderId: bignum;
  /** Exchange defined id for an order. */
  orderSequenceNumber: bignum;
  /** Number of base atoms in the order. */
  numBaseAtoms: bignum;
  /** Hint for the location of the order in the manifest dynamic data. */
  marketDataIndex: number;
  /** Last slot before this order is invalid and will be removed. */
  lastValidSlot: number;
  /** Boolean for whether this order is on the bid side. */
  isBid: boolean;
  /** Type of order (Limit, PostOnly, ...). */
  orderType: OrderType;
  /** unused */
  padding: number[];
}

/**
 * Wrapper object used for reading data from a wrapper for manifest markets.
 */
export class Wrapper {
  /** Public key for the market account. */
  address: PublicKey;
  /** Deserialized data. */
  private data: WrapperData;

  /**
   * Constructs a Wrapper object.
   *
   * @param address The `PublicKey` of the wrapper account
   * @param data Deserialized wrapper data
   */
  private constructor({
    address,
    data,
  }: {
    address: PublicKey;
    data: WrapperData;
  }) {
    this.address = address;
    this.data = data;
  }

  /**
   * Returns a `Wrapper` for a given address, a data buffer
   *
   * @param marketAddress The `PublicKey` of the wrapper account
   * @param buffer The buffer holding the wrapper account data
   */
  static loadFromBuffer({
    address,
    buffer,
  }: {
    address: PublicKey;
    buffer: Buffer;
  }): Wrapper {
    const wrapperData = Wrapper.deserializeWrapperBuffer(buffer);
    return new Wrapper({ address, data: wrapperData });
  }

  /**
   * Returns a `Wrapper` for a given address, a data buffer
   *
   * @param connection The Solana `Connection` object
   * @param address The `PublicKey` of the wrapper account
   */
  static async loadFromAddress({
    connection,
    address,
  }: {
    connection: Connection;
    address: PublicKey;
  }): Promise<Wrapper> {
    const buffer = await connection
      .getAccountInfo(address)
      .then((accountInfo) => accountInfo?.data);

    if (buffer === undefined) {
      throw new Error(`Failed to load ${address}`);
    }
    return Wrapper.loadFromBuffer({ address, buffer });
  }

  /**
   * Updates the data in a Wrapper.
   *
   * @param connection The Solana `Connection` object
   */
  public async reload(connection: Connection): Promise<void> {
    const buffer = await connection
      .getAccountInfo(this.address)
      .then((accountInfo) => accountInfo?.data);
    if (buffer === undefined) {
      throw new Error(`Failed to load ${this.address}`);
    }
    this.data = Wrapper.deserializeWrapperBuffer(buffer);
  }

  /**
   * Get the parsed market info from the wrapper.
   *
   * @param marketPk PublicKey for the market
   *
   * @return MarketInfoParsed
   */
  public marketInfoForMarket(marketPk: PublicKey): WrapperMarketInfo | null {
    const filtered: WrapperMarketInfo[] = this.data.marketInfos.filter(
      (marketInfo: WrapperMarketInfo) => {
        return marketInfo.market.toBase58() == marketPk.toBase58();
      },
    );
    if (filtered.length == 0) {
      return null;
    }
    return filtered[0];
  }

  /**
   * Get the open orders from the wrapper.
   *
   * @param marketPk PublicKey for the market
   *
   * @return WrapperOpenOrder[]
   */
  public openOrdersForMarket(marketPk: PublicKey): WrapperOpenOrder[] | null {
    const filtered: WrapperMarketInfo[] = this.data.marketInfos.filter(
      (marketInfo: WrapperMarketInfo) => {
        return marketInfo.market.toBase58() == marketPk.toBase58();
      },
    );
    if (filtered.length == 0) {
      return null;
    }
    return filtered[0].orders;
  }

  // Do not include getters for the balances because those can be retrieved from
  // the market and that will be fresher data or the same always.

  /**
   * Print all information loaded about the wrapper in a human readable format.
   */
  public prettyPrint() {
    console.log('');
    console.log(`Wrapper: ${this.address.toBase58()}`);
    console.log(`========================`);
    console.log(`Trader: ${this.data.trader.toBase58()}`);
    this.data.marketInfos.forEach((marketInfo: WrapperMarketInfo) => {
      console.log(`------------------------`);
      console.log(`Market: ${marketInfo.market}`);
      console.log(`Last updated slot: ${marketInfo.lastUpdatedSlot}`);
      console.log(
        `BaseAtoms: ${marketInfo.baseBalanceAtoms} QuoteAtoms: ${marketInfo.quoteBalanceAtoms}`,
      );
      marketInfo.orders.forEach((order: WrapperOpenOrder) => {
        console.log(
          `OpenOrder: ClientOrderId: ${order.clientOrderId} ${order.numBaseAtoms}@${order.price} SeqNum: ${order.orderSequenceNumber} LastValidSlot: ${order.lastValidSlot} IsBid: ${order.isBid}`,
        );
      });
    });
    console.log(`------------------------`);
  }

  /**
   * Deserializes wrapper data from a given buffer and returns a `Wrapper` object
   *
   * This includes both the fixed and dynamic parts of the market.
   * https://github.com/Bonasa-Tech/manifest/blob/main/programs/wrapper/src/wrapper_state.rs
   *
   * @param data The data buffer to deserialize
   *
   * @returns WrapperData
   */
  public static deserializeWrapperBuffer(data: Buffer): WrapperData {
    let offset = 0;
    // Deserialize the market header
    const _discriminant = data.readBigUInt64LE(0);
    offset += 8;

    const trader = beetPublicKey.read(data, offset);

    offset += beetPublicKey.byteSize;

    const _numBytesAllocated = data.readUInt32LE(offset);
    offset += 4;

    const _freeListHeadIndex = data.readUInt32LE(offset);
    offset += 4;

    const marketInfosRootIndex = data.readUInt32LE(offset);
    offset += 4;

    const _padding = data.readUInt32LE(offset);
    offset += 12;

    const marketInfos: MarketInfo[] =
      marketInfosRootIndex != NIL
        ? deserializeRedBlackTree(
            data.subarray(FIXED_WRAPPER_HEADER_SIZE),
            marketInfosRootIndex,
            marketInfoBeet,
          )
        : [];

    const parsedMarketInfos: WrapperMarketInfo[] = marketInfos.map(
      (marketInfoRaw: MarketInfo) => {
        const rootIndex: number = marketInfoRaw.ordersRootIndex;
        const rawOpenOrders: WrapperOpenOrderRaw[] =
          rootIndex != NIL
            ? deserializeRedBlackTree(
                data.subarray(FIXED_WRAPPER_HEADER_SIZE),
                rootIndex,
                wrapperOpenOrderBeet,
              )
            : [];

        const parsedOpenOrdersWithPrice: WrapperOpenOrder[] = rawOpenOrders.map(
          (openOrder: WrapperOpenOrderRaw) => {
            return {
              ...openOrder,
              price: convertU128(new BN(openOrder.price, 10, 'le')),
            };
          },
        );

        return {
          market: marketInfoRaw.market,
          traderIndex: marketInfoRaw.traderIndex,
          baseBalanceAtoms: marketInfoRaw.baseBalance,
          quoteBalanceAtoms: marketInfoRaw.quoteBalance,
          quoteVolumeAtoms: marketInfoRaw.quoteVolume,
          orders: parsedOpenOrdersWithPrice,
          lastUpdatedSlot: marketInfoRaw.lastUpdatedSlot,
        };
      },
    );

    return {
      trader,
      marketInfos: parsedMarketInfos,
    };
  }
}
