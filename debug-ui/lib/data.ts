import { Market } from '@bonasa-tech/manifest-sdk';
import { Connection, PublicKey } from '@solana/web3.js';

export const fetchWithTimeout = <T>(
  promise: Promise<T>,
  timeout: number,
): Promise<T> => {
  return Promise.race([
    promise,
    new Promise<T>((_, reject) =>
      setTimeout(() => reject(new Error('Request timed out')), timeout),
    ),
  ]);
};

export const fetchMarket = async (
  conn: Connection,
  marketAddress: PublicKey,
): Promise<Market> => {
  const market: Market = await Market.loadFromAddress({
    connection: conn,
    address: new PublicKey(marketAddress),
  });
  return market;
};
