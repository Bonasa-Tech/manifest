import { PublicKey } from '@solana/web3.js';
import { genAccDiscriminator } from '@bonasa-tech/manifest-sdk/utils';

// Stores volume checkpoints every 5 minutes
export const VOLUME_CHECKPOINT_DURATION_SEC: number = 5 * 60;
export const DATABASE_CHECKPOINT_DURATION_SEC: number = 60 * 60;
export const ONE_DAY_SEC: number = 24 * 60 * 60;
export const PORT: number = 3000;
export const DEPTHS_BPS: number[] = [50, 100, 200];

// Manifest Program ID
export const MANIFEST_PROGRAM_ID = new PublicKey(
  'MNFSTqtC93rEfYHB6hF82sKdZpUDFWkViLByLd1k1Ms',
);

// Market discriminator for filtering accounts
export const MARKET_DISCRIMINATOR: Buffer = genAccDiscriminator(
  'manifest::state::market::MarketFixed',
);

// Market and mint addresses
export const SOL_USDC_MARKET = 'ENhU8LsaR7vDD2G1CsWcsuSGNrih9Cv5WZEk7q9kPapQ';
export const CBBTC_USDC_MARKET = 'Bey9vLee8CrC8S7iqNseb146upQCnSTbJQbu6vLiBRpD';
export const WBTC_USDC_MARKET = '77WgoACGnKG98WdusJeaYbjdu6NKgqXF3B85NBMcSzJk';
export const USDC_MINT = 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v';
export const SOL_MINT = 'So11111111111111111111111111111111111111112';
export const CBBTC_MINT = 'cbbtcf3aa214zXHbiAZQwf4122FBYbraNdFqgw4iMij';
export const WBTC_MINT = '3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh';

// Stablecoin mints (treated as 1:1 USD equivalent)
export const USDT_MINT = 'Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB';
export const PYUSD_MINT = '2b1kV6DkPAnxd5ixfnxCpjxmKwqjjaYmCZfHsFu24GXo';
export const USDS_MINT = 'USDSwr9ApdHk5bvJKMjzff41FfuX8bSxdKcR81vTwcA';
export const USD1_MINT = 'USD1ttGY1N17NEEHLmELoaybftRBUSErhqYiQzvEmuB';
export const CASH_MINT = 'CASHx9KJUStyftLFWGvEVf59SGeG9sh5FfcnZMVPCASH';
export const USDG_MINT = 'USDGmN7zGcsB2sE3W9i13tqM4j4t2UaB5s1M3m8UaU';

export const STABLECOIN_MINTS = new Set([
  USDC_MINT,
  USDT_MINT,
  PYUSD_MINT,
  USDS_MINT,
  USD1_MINT,
  CASH_MINT,
  USDG_MINT,
]);

// Known aggregators
export const KNOWN_AGGREGATORS = new Set([
  'D5YqVMoSxnqeZAKAUUE1Dm3bmjtdxQ5DCF356ozqN9cM', // Titan
  'HV1KXxWFaSeriyFvXyx48FqG9BoFbfinB8njCJonqP7K', // OKX
  'ARu4n5mFdZogZAravu7CcizaojWnS6oqka37gdLT5SZn', // OKX v1
]);
