import 'dotenv/config';
import { Connection, PublicKey } from '@solana/web3.js';
import { FillLogResult } from '@/../../client/ts/src/types';
import { FillLog } from '@/../../client/ts/src/manifest/accounts/FillLog';
import { getVaultAddress } from '@/../../client/ts/src/utils/market';
import { convertU128 } from '@/../../client/ts/src/utils/numbers';
import { genAccDiscriminator } from '@/../../client/ts/src/utils/discriminator';
import {
  detectAggregatorFromKeys,
  detectOriginatingProtocolFromKeys,
} from '@/../../client/ts/src/fillFeed';
import { TOKEN_PROGRAM_ID, TOKEN_2022_PROGRAM_ID } from '@solana/spl-token';

const MARKET_VERIFY_CONCURRENCY = 10;

// Helper function to sleep
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

// Helper function to check if a transaction has token program transfers
const hasTokenTransfer = async (
  connection: Connection,
  signature: string,
): Promise<boolean> => {
  try {
    const tx = await connection.getTransaction(signature, {
      maxSupportedTransactionVersion: 0,
    });

    if (!tx) {
      return false;
    }

    // Check transaction message for token program instructions
    const message = tx.transaction.message;

    // Get all account keys (handling both legacy and versioned transactions)
    let accountKeys: PublicKey[];
    if ('accountKeys' in message) {
      // Legacy transaction
      accountKeys = message.accountKeys;
    } else {
      // Versioned transaction (v0)
      accountKeys = message.staticAccountKeys;
    }

    // Check if any instruction involves token programs
    // Legacy transactions use 'instructions', versioned use 'compiledInstructions'
    const instructions =
      'instructions' in message
        ? message.instructions
        : message.compiledInstructions;
    for (const instruction of instructions) {
      const programId = accountKeys[instruction.programIdIndex];
      if (
        programId.equals(TOKEN_PROGRAM_ID) ||
        programId.equals(TOKEN_2022_PROGRAM_ID)
      ) {
        return true;
      }
    }

    return false;
  } catch (error) {
    console.warn(`Error checking token transfers for ${signature}:`, error);
    // If we can't check, assume it has transfers to be safe
    return true;
  }
};

// FillLog discriminant from the fills feed
const fillDiscriminant = genAccDiscriminator('manifest::logs::FillLog');

interface Market {
  ticker_id: string;
  base_currency: string;
  target_currency: string;
  last_price: number | null;
  base_volume: number;
  target_volume: number;
  pool_id: string;
  liquidity_in_usd: number;
  bid: number;
  ask: number;
}

interface TradeMismatch {
  market: string;
  type: 'missing_in_db' | 'missing_onchain';
  fill: FillLogResult;
}

const toFillLogResult = (
  fillLog: FillLog,
  slot: number,
  signature: string,
  originalSigner?: string,
  aggregator?: string,
  originatingProtocol?: string,
  signers?: string[],
  blockTime?: number,
): FillLogResult => {
  const result: FillLogResult = {
    market: fillLog.market.toBase58(),
    maker: fillLog.maker.toBase58(),
    taker: fillLog.taker.toBase58(),
    baseAtoms: fillLog.baseAtoms.inner.toString(),
    quoteAtoms: fillLog.quoteAtoms.inner.toString(),
    priceAtoms: convertU128(fillLog.price.inner),
    takerIsBuy: fillLog.takerIsBuy,
    isMakerGlobal: fillLog.isMakerGlobal,
    makerSequenceNumber: fillLog.makerSequenceNumber.toString(),
    takerSequenceNumber: fillLog.takerSequenceNumber.toString(),
    signature,
    slot,
  };

  if (originalSigner) {
    result.originalSigner = originalSigner;
  }
  if (aggregator) {
    result.aggregator = aggregator;
  }
  if (originatingProtocol) {
    result.originatingProtocol = originatingProtocol;
  }
  if (signers && signers.length > 0) {
    result.signers = signers;
  }
  if (blockTime !== undefined) {
    result.blockTime = blockTime;
  }

  return result;
};

const parseTransactionForFills = async (
  connection: Connection,
  signature: string,
  slot: number,
  blockTime: number | null,
  logPrefix: string,
): Promise<{ fills: FillLogResult[]; hasTruncatedLogs: boolean }> => {
  const fills: FillLogResult[] = [];
  let hasTruncatedLogs = false;

  try {
    const tx = await connection.getTransaction(signature, {
      maxSupportedTransactionVersion: 0,
    });

    if (!tx?.meta?.logMessages) {
      return { fills, hasTruncatedLogs };
    }

    if (tx.meta.err != null) {
      return { fills, hasTruncatedLogs };
    }

    // Check for truncated logs
    hasTruncatedLogs = tx.meta.logMessages.some((log) =>
      log.toLowerCase().includes('truncated'),
    );

    // Extract signers
    let originalSigner: string | undefined;
    let signers: string[] | undefined;

    try {
      const message = tx.transaction.message;

      if ('accountKeys' in message) {
        // Legacy transaction
        originalSigner = message.accountKeys[0]?.toBase58();
        signers = message.accountKeys
          .map((key, index) => ({ key, index }))
          .filter(({ index }) => message.isAccountSigner(index))
          .map(({ key }) => key.toBase58());
      } else {
        // Versioned transaction (v0)
        originalSigner = message.staticAccountKeys[0]?.toBase58();
        signers = message.staticAccountKeys
          .map((key, index) => ({ key, index }))
          .filter(({ index }) => message.isAccountSigner(index))
          .map(({ key }) => key.toBase58());
      }
    } catch (error) {
      console.error(logPrefix, 'Error extracting signers:', error);
    }

    // Detect aggregator and originating protocol
    let aggregator: string | undefined;
    let originatingProtocol: string | undefined;

    try {
      const message = tx.transaction.message;

      if ('accountKeys' in message) {
        // Legacy transaction
        const accountKeysStr = message.accountKeys.map((k) => k.toBase58());
        aggregator = detectAggregatorFromKeys(accountKeysStr);
        originatingProtocol = detectOriginatingProtocolFromKeys(accountKeysStr);
      } else {
        // V0 transaction
        const accountKeysStr = message.staticAccountKeys.map((k) =>
          k.toBase58(),
        );
        aggregator = detectAggregatorFromKeys(accountKeysStr);
        originatingProtocol = detectOriginatingProtocolFromKeys(accountKeysStr);
      }
    } catch (error) {
      console.warn(logPrefix, 'Error detecting aggregator/protocol:', error);
    }

    const messages = tx.meta.logMessages;
    const programDatas = messages.filter((message) =>
      message.includes('Program data:'),
    );

    if (programDatas.length === 0) {
      return { fills, hasTruncatedLogs }; // No program data logs
    }

    for (let i = 0; i < programDatas.length; i++) {
      const programDataEntry = programDatas[i];
      const programData = programDataEntry.split(' ')[2];
      const byteArray = Uint8Array.from(atob(programData), (c) =>
        c.charCodeAt(0),
      );
      const buffer = Buffer.from(byteArray);

      if (!buffer.subarray(0, 8).equals(fillDiscriminant)) {
        continue;
      }

      try {
        const deserializedFillLog = FillLog.deserialize(buffer.subarray(8))[0];
        const fillResult = toFillLogResult(
          deserializedFillLog,
          slot,
          signature,
          originalSigner,
          aggregator,
          originatingProtocol,
          signers,
          blockTime ?? undefined,
        );

        fills.push(fillResult);
      } catch (error) {
        console.error(logPrefix, `Error deserializing FillLog:`, error);
      }
    }
  } catch (error) {
    console.error(logPrefix, `Error parsing transaction ${signature}:`, error);
  }

  return { fills, hasTruncatedLogs };
};

const fetchDatabaseFills = async (
  connection: Connection,
  statsServerUrl: string,
  market: string,
  startTime: number,
  endTime: number,
  logPrefix: string,
): Promise<FillLogResult[]> => {
  const fills: FillLogResult[] = [];
  let offset = 0;
  const limit = 1000;

  console.log(logPrefix, `Fetching fills from database...`);

  // Cache for block times to avoid repeated RPC calls
  const blockTimeCache = new Map<number, number>();

  while (true) {
    try {
      const params = new URLSearchParams({
        market,
        limit: limit.toString(),
        offset: offset.toString(),
      });

      const response = await fetch(`${statsServerUrl}/completeFills?${params}`);
      if (!response.ok) {
        throw new Error(
          `Failed to fetch fills: ${response.status} ${response.statusText}`,
        );
      }
      const data = await response.json();

      const { fills: batchFills, hasMore } = data;

      if (!batchFills || batchFills.length === 0) {
        break;
      }

      // Filter fills by time, fetching block times as needed
      for (const fill of batchFills) {
        let fillTime: number;

        if (fill.blockTime) {
          // Use existing block time
          fillTime = fill.blockTime * 1000;
        } else {
          // Fetch block time from RPC using slot
          if (!blockTimeCache.has(fill.slot)) {
            try {
              const blockTime = await connection.getBlockTime(fill.slot);
              if (blockTime) {
                blockTimeCache.set(fill.slot, blockTime);
                fillTime = blockTime * 1000;
              } else {
                // If we can't get block time, assume it's old and skip
                continue;
              }
            } catch (error) {
              // Check if this is a "cleaned up" block error
              const errorMessage =
                error instanceof Error ? error.message : String(error);
              if (
                errorMessage.includes('cleaned up') ||
                errorMessage.includes('does not exist on node')
              ) {
                // Block is too old and has been cleaned up, skip without logging
                continue;
              } else {
                // Other error, log it
                console.warn(
                  logPrefix,
                  `Error fetching block time for slot ${fill.slot}:`,
                  error,
                );
                continue;
              }
            }
          } else {
            fillTime = blockTimeCache.get(fill.slot)! * 1000;
          }
        }

        // Only include fills within our time window
        if (fillTime >= startTime && fillTime <= endTime) {
          // Update the fill with the block time if it was missing
          if (!fill.blockTime && blockTimeCache.has(fill.slot)) {
            fill.blockTime = blockTimeCache.get(fill.slot);
          }
          fills.push(fill);
        } else if (fillTime < startTime) {
          // Once we hit fills older than our time window, we're done
          console.log(
            logPrefix,
            `Reached fills older than time window, stopping at slot ${fill.slot}`,
          );
          return fills;
        }
        // If fillTime > endTime, skip this fill but continue (it's newer than our window)
      }

      if (!hasMore) {
        break;
      }

      offset += limit;
    } catch (error) {
      console.error(logPrefix, 'Error fetching fills from database:', error);
      throw error;
    }
  }

  return fills;
};

const fetchOnchainFills = async (
  connection: Connection,
  marketPubkey: PublicKey,
  baseMint: PublicKey,
  startTime: number,
  endTime: number,
  logPrefix: string,
): Promise<{ fills: FillLogResult[]; truncatedSignatures: Set<string> }> => {
  const fills: FillLogResult[] = [];
  const baseVault = getVaultAddress(marketPubkey, baseMint);

  console.log(
    logPrefix,
    `Fetching onchain fills, base vault: ${baseVault.toString()}`,
  );

  let lastSignature: string | undefined;
  let done = false;
  let totalSignatures = 0;
  const truncatedSignatures = new Set<string>();

  while (!done) {
    try {
      const signatures = await connection.getSignaturesForAddress(baseVault, {
        before: lastSignature,
        limit: 1000,
      });

      totalSignatures += signatures.length;

      if (signatures.length === 0) {
        break;
      }

      // Process signatures sequentially
      const fillBatches: FillLogResult[][] = [];
      for (const sig of signatures) {
        const { fills: sigFills, hasTruncatedLogs } =
          await parseTransactionForFills(
            connection,
            sig.signature,
            sig.slot,
            sig.blockTime!,
            logPrefix,
          );
        fillBatches.push(sigFills);

        // Track truncated signatures
        if (hasTruncatedLogs) {
          truncatedSignatures.add(sig.signature);
        }
      }

      for (let i = 0; i < signatures.length; i++) {
        const sig = signatures[i];
        const sigTime = (sig.blockTime ?? 0) * 1000;

        if (sigTime < startTime) {
          done = true;
          break;
        }

        const sigFills = fillBatches[i];
        // Only add fills for this specific market and within our time window
        const marketFills = sigFills.filter((f) => {
          const fillMarketMatches = f.market === marketPubkey.toString();
          const fillTime = (f.blockTime ?? 0) * 1000;
          const fillInTimeWindow = fillTime >= startTime && fillTime <= endTime;
          return fillMarketMatches && fillInTimeWindow;
        });
        fills.push(...marketFills);
      }

      lastSignature = signatures[signatures.length - 1].signature;
    } catch (error) {
      console.error(logPrefix, 'Error fetching onchain signatures:', error);
      throw error;
    }
  }

  if (truncatedSignatures.size > 0) {
    console.log(
      logPrefix,
      `Found ${truncatedSignatures.size} truncated signatures that will be excluded from mismatch detection`,
    );
  }

  return { fills, truncatedSignatures };
};

const compareFills = async (
  connection: Connection,
  dbFills: FillLogResult[],
  onchainFills: FillLogResult[],
  truncatedSignatures: Set<string>,
  market: string,
  dbBufferTime: number,
  logPrefix: string,
): Promise<TradeMismatch[]> => {
  const mismatches: TradeMismatch[] = [];

  // Create maps keyed by signature for easy lookup
  const dbFillsMap = new Map<string, FillLogResult[]>();
  for (const fill of dbFills) {
    if (!dbFillsMap.has(fill.signature)) {
      dbFillsMap.set(fill.signature, []);
    }
    dbFillsMap.get(fill.signature)!.push(fill);
  }

  const onchainFillsMap = new Map<string, FillLogResult[]>();
  for (const fill of onchainFills) {
    if (!onchainFillsMap.has(fill.signature)) {
      onchainFillsMap.set(fill.signature, []);
    }
    onchainFillsMap.get(fill.signature)!.push(fill);
  }

  // Check for fills in database but not onchain
  for (const [signature, fills] of Array.from(dbFillsMap)) {
    // Skip checking fills with truncated signatures - we can't verify them reliably
    if (truncatedSignatures.has(signature)) {
      continue;
    }

    const onchainFills = onchainFillsMap.get(signature);

    if (!onchainFills) {
      // Entire transaction missing from onchain - check if it has token transfers
      const hasTransfers = await hasTokenTransfer(connection, signature);

      if (hasTransfers) {
        // Only report as missing if the transaction actually has token transfers
        for (const fill of fills) {
          mismatches.push({
            market,
            type: 'missing_onchain',
            fill,
          });
        }
      } else {
        console.log(
          logPrefix,
          `Ignoring missing onchain fill for ${signature} - no token transfers detected`,
        );
      }
    } else {
      // Check if specific fills within the transaction match
      for (const dbFill of fills) {
        const matchingFill = onchainFills.find(
          (f) =>
            f.maker === dbFill.maker &&
            f.taker === dbFill.taker &&
            f.baseAtoms === dbFill.baseAtoms &&
            f.quoteAtoms === dbFill.quoteAtoms &&
            f.makerSequenceNumber === dbFill.makerSequenceNumber &&
            f.takerSequenceNumber === dbFill.takerSequenceNumber,
        );

        if (!matchingFill) {
          // Check if this specific transaction has token transfers before reporting
          const hasTransfers = await hasTokenTransfer(
            connection,
            dbFill.signature,
          );

          if (hasTransfers) {
            mismatches.push({
              market,
              type: 'missing_onchain',
              fill: dbFill,
            });
          } else {
            console.log(
              logPrefix,
              `Ignoring missing onchain fill for ${dbFill.signature} - no token transfers detected`,
            );
          }
        }
      }
    }
  }

  // Check for fills onchain but not in database
  for (const [signature, fills] of Array.from(onchainFillsMap)) {
    const dbFills = dbFillsMap.get(signature);

    if (!dbFills) {
      // Entire transaction missing from database - check if it's within buffer time
      for (const fill of fills) {
        const fillTime = (fill.blockTime ?? 0) * 1000;

        if (fillTime > dbBufferTime) {
          // Fill is too recent, might not be in DB yet - ignore
          console.log(
            logPrefix,
            `Ignoring missing DB fill for ${signature} - within buffer time (${new Date(fillTime).toISOString()})`,
          );
        } else {
          // Fill is old enough that it should be in DB
          mismatches.push({
            market,
            type: 'missing_in_db',
            fill,
          });
        }
      }
    } else {
      // Check if specific fills within the transaction match
      for (const onchainFill of fills) {
        const matchingFill = dbFills.find(
          (f) =>
            f.maker === onchainFill.maker &&
            f.taker === onchainFill.taker &&
            f.baseAtoms === onchainFill.baseAtoms &&
            f.quoteAtoms === onchainFill.quoteAtoms &&
            f.makerSequenceNumber === onchainFill.makerSequenceNumber &&
            f.takerSequenceNumber === onchainFill.takerSequenceNumber,
        );

        if (!matchingFill) {
          const fillTime = (onchainFill.blockTime ?? 0) * 1000;

          if (fillTime > dbBufferTime) {
            // Fill is too recent, might not be in DB yet - ignore
            console.log(
              logPrefix,
              `Ignoring missing DB fill for ${onchainFill.signature} - within buffer time (${new Date(fillTime).toISOString()})`,
            );
          } else {
            // Fill is old enough that it should be in DB
            mismatches.push({
              market,
              type: 'missing_in_db',
              fill: onchainFill,
            });
          }
        }
      }
    }
  }

  return mismatches;
};

const run = async () => {
  const { RPC_URL, STATS_SERVER_URL } = process.env;

  if (!RPC_URL) {
    console.error(
      'RPC_URL is required. Set it like: RPC_URL="your-rpc-url" npx tsx scripts/verify-trades.ts',
    );
    throw new Error('RPC_URL missing from env');
  }

  const statsServerUrl = (STATS_SERVER_URL || 'http://localhost:5000').replace(
    /\/$/,
    '',
  ); // Remove trailing slash
  const connection = new Connection(RPC_URL);

  console.log('Fetching market tickers from stats server...');
  console.log(`Using stats server: ${statsServerUrl}`);

  try {
    // Fetch all market tickers
    const tickersResponse = await fetch(`${statsServerUrl}/tickers`);
    if (!tickersResponse.ok) {
      throw new Error(
        `Failed to fetch tickers: ${tickersResponse.status} ${tickersResponse.statusText}`,
      );
    }
    const tickersData = await tickersResponse.json();

    // Check if the response is an array or has a different structure
    let markets: Market[];
    if (Array.isArray(tickersData)) {
      markets = tickersData;
    } else if (tickersData.markets) {
      markets = tickersData.markets;
    } else if (tickersData.data) {
      markets = tickersData.data;
    } else {
      // If it's an object with market addresses as keys
      markets = Object.entries(tickersData).map(
        ([key, value]: [string, any]) => ({
          ticker_id: key,
          base_currency: value.base_currency,
          target_currency: value.target_currency,
          ...value,
        }),
      );
    }

    console.log(`Found ${markets.length} markets to verify`);

    const validMarkets = markets.filter((market) => {
      if (!market.ticker_id || !market.base_currency) {
        console.log('Skipping invalid market:', market);
        return false;
      }
      return true;
    });

    const verifyMarket = async (market: Market): Promise<TradeMismatch[]> => {
      const logPrefix = `[${market.ticker_id}]`;
      console.log(logPrefix, 'Verifying market');

      try {
        // Set fixed time window for this market analysis
        const endTime = Date.now();
        const startTime = endTime - 6 * 60 * 60 * 1000; // 6 hours ago
        const dbFetchStartTime = Date.now(); // When we start fetching from DB

        console.log(
          logPrefix,
          `Time window: ${new Date(startTime).toISOString()} to ${new Date(endTime).toISOString()}`,
        );

        // Fetch fills from database
        const dbFills = await fetchDatabaseFills(
          connection,
          statsServerUrl,
          market.ticker_id,
          startTime,
          endTime,
          logPrefix,
        );
        console.log(logPrefix, `Found ${dbFills.length} fills in database`);

        // Fetch fills from onchain
        const marketPubkey = new PublicKey(market.ticker_id);
        const baseMint = new PublicKey(market.base_currency);
        const { fills: onchainFills, truncatedSignatures } =
          await fetchOnchainFills(
            connection,
            marketPubkey,
            baseMint,
            startTime,
            endTime,
            logPrefix,
          );
        console.log(logPrefix, `Found ${onchainFills.length} fills onchain`);

        // Compare fills with DB fetch buffer
        const dbBufferTime = dbFetchStartTime - 60 * 1000; // 60 seconds before we started fetching from DB
        const mismatches = await compareFills(
          connection,
          dbFills,
          onchainFills,
          truncatedSignatures,
          market.ticker_id,
          dbBufferTime,
          logPrefix,
        );

        if (mismatches.length > 0) {
          console.log(logPrefix, `Found ${mismatches.length} mismatches`);

          // Log unique transaction signatures for this market
          const uniqueSignatures = new Set<string>();
          for (const mismatch of mismatches) {
            uniqueSignatures.add(mismatch.fill.signature);
          }
          console.log(
            logPrefix,
            `Mismatch transaction IDs: ${Array.from(uniqueSignatures).join(', ')}`,
          );
        } else {
          console.log(logPrefix, 'All fills match');
        }

        return mismatches;
      } catch (error) {
        console.error(logPrefix, 'Error processing market:', error);
        return [];
      }
    };

    // Process markets with a concurrency pool
    const allMismatches: TradeMismatch[] = [];
    const pending = new Set<Promise<void>>();
    const marketQueue = [...validMarkets];

    while (marketQueue.length > 0 || pending.size > 0) {
      while (
        marketQueue.length > 0 &&
        pending.size < MARKET_VERIFY_CONCURRENCY
      ) {
        const market = marketQueue.shift()!;
        const p = verifyMarket(market).then((mismatches) => {
          allMismatches.push(...mismatches);
          pending.delete(p);
        });
        pending.add(p);
      }
      if (pending.size > 0) {
        await Promise.race(pending);
      }
    }

    // Attempt to backfill any missing_in_db mismatches
    if (allMismatches.length > 0) {
      const missingInDbMismatches = allMismatches.filter(
        (m) => m.type === 'missing_in_db',
      );
      const uniqueSignaturesToBackfill = new Set<string>();
      for (const mismatch of missingInDbMismatches) {
        uniqueSignaturesToBackfill.add(mismatch.fill.signature);
      }

      if (uniqueSignaturesToBackfill.size > 0) {
        console.log(
          `\n🔄 Attempting to backfill ${uniqueSignaturesToBackfill.size} missing transactions...`,
        );

        const backfilledSignatures = new Set<string>();
        for (const signature of uniqueSignaturesToBackfill) {
          try {
            const response = await fetch(
              `${statsServerUrl}/backfill?signature=${signature}`,
            );
            if (response.ok) {
              const result = await response.json();
              if (result.success) {
                console.log(
                  `✅ Backfilled ${signature}: ${result.backfilled} new, ${result.alreadyExisted} existed`,
                );
                backfilledSignatures.add(signature);
              }
            } else {
              console.log(
                `❌ Failed to backfill ${signature}: ${response.status}`,
              );
            }
          } catch (error) {
            console.log(`❌ Error backfilling ${signature}:`, error);
          }
        }

        // Remove successfully backfilled mismatches from the list
        if (backfilledSignatures.size > 0) {
          const remainingMismatches = allMismatches.filter(
            (m) =>
              m.type !== 'missing_in_db' ||
              !backfilledSignatures.has(m.fill.signature),
          );
          console.log(
            `\n📊 After backfill: ${allMismatches.length - remainingMismatches.length} mismatches resolved`,
          );
          allMismatches.length = 0;
          allMismatches.push(...remainingMismatches);
        }
      }
    }

    // Log all mismatches
    if (allMismatches.length > 0) {
      console.log('\n🚨 MISMATCHES FOUND 🚨\n');

      // First, show summary of all mismatch transaction IDs
      const allMismatchSignatures = new Set<string>();
      const mismatchesByMarket = new Map<string, Set<string>>();

      for (const mismatch of allMismatches) {
        allMismatchSignatures.add(mismatch.fill.signature);

        if (!mismatchesByMarket.has(mismatch.market)) {
          mismatchesByMarket.set(mismatch.market, new Set());
        }
        mismatchesByMarket.get(mismatch.market)!.add(mismatch.fill.signature);
      }

      console.log(`📋 SUMMARY OF ALL MISMATCH TRANSACTION IDs:`);
      console.log(
        `Total unique transaction signatures: ${allMismatchSignatures.size}`,
      );
      console.log(
        `All mismatch signatures: ${Array.from(allMismatchSignatures).join(', ')}`,
      );
      console.log('');

      console.log(`📊 BREAKDOWN BY MARKET:`);
      for (const [market, signatures] of mismatchesByMarket) {
        console.log(`Market ${market}: ${signatures.size} unique transactions`);
        console.log(`  Signatures: ${Array.from(signatures).join(', ')}`);
      }
      console.log('');

      console.log(`📄 DETAILED FILL INFORMATION:`);
      for (const mismatch of allMismatches) {
        console.log(`Market: ${mismatch.market}`);
        console.log(`Type: ${mismatch.type}`);
        console.log(`Fill Details:`);
        console.log(`  Signature: ${mismatch.fill.signature}`);
        console.log(`  Slot: ${mismatch.fill.slot}`);
        console.log(`  Maker: ${mismatch.fill.maker}`);
        console.log(`  Taker: ${mismatch.fill.taker}`);
        console.log(`  Base Atoms: ${mismatch.fill.baseAtoms}`);
        console.log(`  Quote Atoms: ${mismatch.fill.quoteAtoms}`);
        console.log(`  Price: ${mismatch.fill.priceAtoms}`);
        console.log(`  Taker is Buy: ${mismatch.fill.takerIsBuy}`);
        console.log(`  Is Maker Global: ${mismatch.fill.isMakerGlobal}`);
        console.log(`  Maker Seq: ${mismatch.fill.makerSequenceNumber}`);
        console.log(`  Taker Seq: ${mismatch.fill.takerSequenceNumber}`);
        if (mismatch.fill.originalSigner) {
          console.log(`  Original Signer: ${mismatch.fill.originalSigner}`);
        }
        if (mismatch.fill.aggregator) {
          console.log(`  Aggregator: ${mismatch.fill.aggregator}`);
        }
        if (mismatch.fill.originatingProtocol) {
          console.log(
            `  Originating Protocol: ${mismatch.fill.originatingProtocol}`,
          );
        }
        console.log('---');
      }

      console.log(`\nTotal mismatches: ${allMismatches.length}`);
      console.log(`Unique transactions: ${allMismatchSignatures.size}`);
      process.exit(1);
    } else {
      console.log(
        '\n✅ All trades verified successfully! No mismatches found.',
      );
    }
  } catch (error) {
    console.error('Fatal error:', error);
    throw error;
  }
};

run().catch((e) => {
  console.error('fatal error', e);
  throw e;
});
