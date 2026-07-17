import 'dotenv/config';
import { Connection, PublicKey } from '@solana/web3.js';
import { FillLogResult } from '@/../../client/ts/src/types';
import { FillLog } from '@/../../client/ts/src/manifest/accounts/FillLog';
import { getVaultAddress } from '@/../../client/ts/src/utils/market';
import { convertU128 } from '@/../../client/ts/src/utils/numbers';
import { genAccDiscriminator } from '@/../../client/ts/src/utils/discriminator';
import { hasTruncatedLogs as checkTruncatedLogs } from '@/../../client/ts/src/utils/solana';
import {
  detectAggregatorFromKeys,
  detectOriginatingProtocolFromKeys,
} from '@/../../client/ts/src/aggregators';
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
      // Versioned transaction (v0) - need to include loaded addresses from ALTs
      accountKeys = [...message.staticAccountKeys];
      if (tx.meta?.loadedAddresses) {
        accountKeys.push(
          ...tx.meta.loadedAddresses.writable.map(
            (addr) => new PublicKey(addr),
          ),
          ...tx.meta.loadedAddresses.readonly.map(
            (addr) => new PublicKey(addr),
          ),
        );
      }
    }

    // Token balance changes are the most reliable signal: any SPL transfer
    // shows up as a pre/post token balance entry, regardless of whether the
    // token program was invoked at the top level or via CPI.
    if (
      (tx.meta?.preTokenBalances?.length ?? 0) > 0 ||
      (tx.meta?.postTokenBalances?.length ?? 0) > 0
    ) {
      return true;
    }

    const involvesTokenProgram = (programIdIndex: number): boolean => {
      const programId = accountKeys[programIdIndex];
      return (
        programId != null &&
        (programId.equals(TOKEN_PROGRAM_ID) ||
          programId.equals(TOKEN_2022_PROGRAM_ID))
      );
    };

    // Check if any top-level instruction involves token programs.
    // Legacy transactions use 'instructions', versioned use 'compiledInstructions'
    const instructions =
      'instructions' in message
        ? message.instructions
        : message.compiledInstructions;
    for (const instruction of instructions) {
      if (involvesTokenProgram(instruction.programIdIndex)) {
        return true;
      }
    }

    // Aggregator/CPI swaps (e.g. Jupiter -> Manifest) invoke the token program
    // only via inner instructions, so the top-level scan alone misses them.
    for (const inner of tx.meta?.innerInstructions ?? []) {
      for (const instruction of inner.instructions) {
        if (involvesTokenProgram(instruction.programIdIndex)) {
          return true;
        }
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

interface UnparseableFill {
  market: string;
  signature: string;
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

// Tracks transactions that getTransaction failed to return during the run, for
// end-of-run reporting. Sets (keyed by signature) so a signature fetched more
// than once - e.g. once via the bulk vault scan and again via a targeted
// re-parse - is only counted once.
const txFetchStats = {
  // Not found / errored on the first getTransaction attempt (entered the retry).
  notFoundFirstAttempt: new Set<string>(),
  // Still not found / errored after the 10s retry.
  notFoundAfterRetry: new Set<string>(),
};

// Prints the not-found transaction tallies at the bottom of the report. These
// transactions yield no fills, so a high count means the report's onchain view
// is incomplete (typically RPC throttling) rather than genuinely missing fills.
const printTxFetchSummary = (): void => {
  const notFoundOverall = txFetchStats.notFoundFirstAttempt.size;
  const notFoundAfterRetry = txFetchStats.notFoundAfterRetry.size;
  console.log('');
  console.log('📊 TRANSACTION FETCH SUMMARY:');
  console.log(`Transactions not found on first attempt: ${notFoundOverall}`);
  console.log(
    `Transactions still not found after retry: ${notFoundAfterRetry}`,
  );
  console.log(`Recovered on retry: ${notFoundOverall - notFoundAfterRetry}`);
};

// Number of example truncated signatures to print at the end of the run.
const TRUNCATED_EXAMPLE_COUNT = 10;

// Prints a summary of transactions whose onchain logs were truncated. These are
// excluded from mismatch detection (their fills can't be reliably reconstructed),
// so a high count means the report is silently skipping verification for that
// many transactions.
const printTruncatedSummary = (
  truncatedSignatures: Map<string, string>,
): void => {
  console.log('');
  console.log('✂️  TRUNCATED SIGNATURE SUMMARY:');
  console.log(
    `Transactions with truncated logs (excluded from mismatch detection): ${truncatedSignatures.size}`,
  );

  if (truncatedSignatures.size === 0) {
    return;
  }

  const examples = Array.from(truncatedSignatures).slice(
    0,
    TRUNCATED_EXAMPLE_COUNT,
  );
  console.log(`Examples (up to ${TRUNCATED_EXAMPLE_COUNT}):`);
  for (const [signature, market] of examples) {
    console.log(`  ${signature} (market ${market})`);
  }
  if (truncatedSignatures.size > examples.length) {
    console.log(`  ...and ${truncatedSignatures.size - examples.length} more`);
  }
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
    let tx: Awaited<ReturnType<typeof connection.getTransaction>> | null = null;
    let fetchError: unknown = null;

    try {
      tx = await connection.getTransaction(signature, {
        maxSupportedTransactionVersion: 0,
      });
    } catch (error) {
      fetchError = error;
    }

    // Retry once after 10 seconds if transaction not found or error (transient RPC error/throttling)
    if (!tx) {
      txFetchStats.notFoundFirstAttempt.add(signature);
      const errorMsg = fetchError
        ? `: ${fetchError instanceof Error ? fetchError.message : String(fetchError)}`
        : '';
      console.log(
        logPrefix,
        `Transaction ${signature} not found${errorMsg}, retrying after 10 seconds...`,
      );
      await sleep(10000);
      try {
        tx = await connection.getTransaction(signature, {
          maxSupportedTransactionVersion: 0,
        });
      } catch (retryError) {
        // Log non-429 errors with full details
        const retryErrorMsg =
          retryError instanceof Error ? retryError.message : String(retryError);
        if (!retryErrorMsg.includes('429')) {
          console.error(
            logPrefix,
            `Error fetching transaction ${signature} after retry:`,
            retryError,
          );
        }
      }

      if (!tx) {
        txFetchStats.notFoundAfterRetry.add(signature);
      }
    }

    if (!tx?.meta?.logMessages) {
      // Log if transaction still not found after retry (and it wasn't a 429 error)
      const errorMsg = fetchError
        ? fetchError instanceof Error
          ? fetchError.message
          : String(fetchError)
        : '';
      if (!errorMsg.includes('429')) {
        console.warn(
          logPrefix,
          `Transaction ${signature} not found after retry`,
          fetchError ? `- Original error: ${errorMsg}` : '',
        );
      }
      return { fills, hasTruncatedLogs };
    }

    if (tx.meta.err != null) {
      return { fills, hasTruncatedLogs };
    }

    // Check for truncated logs
    hasTruncatedLogs = checkTruncatedLogs(tx.meta.logMessages);

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

  // Bound the query by slot so we don't scan the market's entire history.
  // /completeFills is ordered by insert timestamp (not block time), so we
  // cannot rely on block-time ordering to terminate early (see below). Instead
  // we constrain the slot range to (roughly) the requested time window with
  // generous padding, then filter precisely by block time on the client.
  let fromSlot: number | undefined;
  try {
    const APPROX_SLOT_MS = 400; // Solana targets ~400ms/slot
    const windowSlots = Math.ceil((endTime - startTime) / APPROX_SLOT_MS);
    // Pad heavily: missing a fill is a false positive, over-fetching is cheap.
    const paddedSlots = Math.ceil(windowSlots * 1.5) + 10000;
    const currentSlot = await connection.getSlot();
    fromSlot = Math.max(0, currentSlot - paddedSlots);
  } catch (error) {
    console.warn(
      logPrefix,
      'Could not determine current slot, fetching without slot bound:',
      error,
    );
  }

  while (true) {
    try {
      const params = new URLSearchParams({
        market,
        limit: limit.toString(),
        offset: offset.toString(),
      });
      if (fromSlot !== undefined) {
        params.set('fromSlot', fromSlot.toString());
      }

      // The stats server can return transient 503s when the database is
      // temporarily unavailable. Retry with backoff before giving up so a
      // brief blip doesn't abort the whole market.
      const maxFetchAttempts = 5;
      let response: Response | undefined;
      for (let attempt = 1; attempt <= maxFetchAttempts; attempt++) {
        response = await fetch(`${statsServerUrl}/completeFills?${params}`);
        if (response.ok) {
          break;
        }
        if (response.status === 503 && attempt < maxFetchAttempts) {
          const delayMs = 1000 * 2 ** (attempt - 1);
          console.warn(
            logPrefix,
            `completeFills returned 503, retrying in ${delayMs / 1000}s (attempt ${attempt}/${maxFetchAttempts})...`,
          );
          await sleep(delayMs);
          continue;
        }
        throw new Error(
          `Failed to fetch fills: ${response.status} ${response.statusText}`,
        );
      }
      const data = await response!.json();

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
        }
        // Fills outside [startTime, endTime] are skipped but we keep paginating.
        // We can't stop early here: /completeFills is ordered by insert timestamp,
        // not block time, so older-block-time fills (e.g. backfilled rows) can
        // appear before newer ones. Terminating early dropped in-window fills and
        // produced spurious "missing_in_db" mismatches. The slot bound above keeps
        // this bounded; pagination ends when a batch comes back empty.
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
  const truncatedSignatures = new Set<string>();

  while (!done) {
    try {
      const signatures = await connection.getSignaturesForAddress(baseVault, {
        before: lastSignature,
        limit: 1000,
      });

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

// Re-parse a specific signature directly and check whether a given DB fill is
// actually present in that transaction.
//
// `fetchOnchainFills` reconstructs the onchain fill set by enumerating *every*
// transaction touching the market's base vault and re-fetching each one. For
// very high-volume vaults (e.g. USDC/USDT markets whose vault sees thousands of
// txs per minute) that enumeration is unreliable: rate-limited getTransaction
// calls return null and the fill silently drops out of the reconstructed set,
// producing false "unparseable" reports for fills that are perfectly parseable.
//
// This does an O(1) targeted re-parse of the fill's own signature so we only
// flag a fill when its specific transaction genuinely cannot be parsed.
const fillExistsInTransaction = async (
  connection: Connection,
  dbFill: FillLogResult,
  logPrefix: string,
): Promise<boolean> => {
  const { fills } = await parseTransactionForFills(
    connection,
    dbFill.signature,
    dbFill.slot,
    dbFill.blockTime ?? null,
    logPrefix,
  );

  return fills.some(
    (f) =>
      f.market === dbFill.market &&
      f.maker === dbFill.maker &&
      f.taker === dbFill.taker &&
      f.baseAtoms === dbFill.baseAtoms &&
      f.quoteAtoms === dbFill.quoteAtoms &&
      f.makerSequenceNumber === dbFill.makerSequenceNumber &&
      f.takerSequenceNumber === dbFill.takerSequenceNumber,
  );
};

const compareFills = async (
  connection: Connection,
  dbFills: FillLogResult[],
  onchainFills: FillLogResult[],
  truncatedSignatures: Set<string>,
  market: string,
  dbBufferTime: number,
  logPrefix: string,
): Promise<{
  mismatches: TradeMismatch[];
  unparseableFills: UnparseableFill[];
}> => {
  const mismatches: TradeMismatch[] = [];
  const unparseableFills: UnparseableFill[] = [];

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
      // Entire transaction missing from the bulk vault enumeration. Re-parse
      // each fill's own signature directly before flagging - the enumeration is
      // unreliable for high-volume vaults (see fillExistsInTransaction).
      for (const fill of fills) {
        const verified = await fillExistsInTransaction(
          connection,
          fill,
          logPrefix,
        );
        if (verified) {
          continue;
        }

        // Targeted re-parse also failed - check if it has token transfers
        const hasTransfers = await hasTokenTransfer(connection, signature);

        if (hasTransfers) {
          // Transaction exists onchain with token transfers but we couldn't parse the fill
          // Track as unparseable rather than a mismatch
          unparseableFills.push({
            market,
            signature,
            fill,
          });
          console.log(
            logPrefix,
            `Transaction ${signature} found onchain but fill could not be parsed`,
          );
        } else {
          console.log(
            logPrefix,
            `Ignoring missing onchain fill for ${signature} - no token transfers detected`,
          );
        }
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
          // The enumerated fills for this signature didn't include this specific
          // fill. Re-parse the signature directly before flagging - the bulk
          // enumeration can return a partial result (see fillExistsInTransaction).
          const verified = await fillExistsInTransaction(
            connection,
            dbFill,
            logPrefix,
          );
          if (verified) {
            continue;
          }

          // Check if this specific transaction has token transfers before reporting
          const hasTransfers = await hasTokenTransfer(
            connection,
            dbFill.signature,
          );

          if (hasTransfers) {
            // Transaction exists onchain with token transfers but specific fill not found
            // Track as unparseable rather than a mismatch
            unparseableFills.push({
              market,
              signature: dbFill.signature,
              fill: dbFill,
            });
            console.log(
              logPrefix,
              `Transaction ${dbFill.signature} found onchain but specific fill could not be parsed`,
            );
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

  return { mismatches, unparseableFills };
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

    const verifyMarket = async (
      market: Market,
    ): Promise<{
      mismatches: TradeMismatch[];
      unparseableFills: UnparseableFill[];
      truncatedSignatures: Set<string>;
    }> => {
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
        const { mismatches, unparseableFills } = await compareFills(
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

        if (unparseableFills.length > 0) {
          console.log(
            logPrefix,
            `Found ${unparseableFills.length} fills with unparseable onchain transactions`,
          );
        }

        return { mismatches, unparseableFills, truncatedSignatures };
      } catch (error) {
        console.error(logPrefix, 'Error processing market:', error);
        return {
          mismatches: [],
          unparseableFills: [],
          truncatedSignatures: new Set<string>(),
        };
      }
    };

    // Process markets with a concurrency pool
    const allMismatches: TradeMismatch[] = [];
    const allUnparseableFills: UnparseableFill[] = [];
    // Truncated signatures across all markets, keyed by signature so a tx that
    // touches multiple markets is only counted once. Value is the market it was
    // first seen in, used for the example output at the end of the run.
    const allTruncatedSignatures = new Map<string, string>();
    const pending = new Set<Promise<void>>();
    const marketQueue = [...validMarkets];

    while (marketQueue.length > 0 || pending.size > 0) {
      while (
        marketQueue.length > 0 &&
        pending.size < MARKET_VERIFY_CONCURRENCY
      ) {
        const market = marketQueue.shift()!;
        const p = verifyMarket(market).then(
          ({ mismatches, unparseableFills, truncatedSignatures }) => {
            allMismatches.push(...mismatches);
            allUnparseableFills.push(...unparseableFills);
            for (const signature of truncatedSignatures) {
              if (!allTruncatedSignatures.has(signature)) {
                allTruncatedSignatures.set(signature, market.ticker_id);
              }
            }
            pending.delete(p);
          },
        );
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
        const maxBackfillAttempts = 3;
        for (const signature of uniqueSignaturesToBackfill) {
          for (let attempt = 1; attempt <= maxBackfillAttempts; attempt++) {
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
                break;
              } else if (
                response.status >= 500 &&
                response.status < 600 &&
                attempt < maxBackfillAttempts
              ) {
                // Retry on any 5xx error
                const errorBody = await response.text().catch(() => '');
                const errorDetail = errorBody ? `: ${errorBody}` : '';
                console.log(
                  `⏳ Backfill ${signature} returned ${response.status}${errorDetail}, retrying in 5s (attempt ${attempt}/${maxBackfillAttempts})...`,
                );
                await sleep(5000);
                continue;
              } else {
                const errorBody = await response.text().catch(() => '');
                const errorDetail = errorBody ? `: ${errorBody}` : '';
                console.log(
                  `❌ Failed to backfill ${signature}: ${response.status}${errorDetail}`,
                );
                break;
              }
            } catch (error) {
              console.log(`❌ Error backfilling ${signature}:`, error);
              break;
            }
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

    // Log unparseable fills (not failures, just informational)
    if (allUnparseableFills.length > 0) {
      console.log('\n⚠️  UNPARSEABLE ONCHAIN TRANSACTIONS ⚠️\n');
      console.log(
        'The following fills exist in the database but the onchain transaction',
      );
      console.log(
        'could not be parsed (transaction exists but fill data is not extractable):',
      );
      console.log('');

      const unparseableSignatures = new Set<string>();
      const unparseableByMarket = new Map<string, Set<string>>();

      for (const unparseable of allUnparseableFills) {
        unparseableSignatures.add(unparseable.signature);

        if (!unparseableByMarket.has(unparseable.market)) {
          unparseableByMarket.set(unparseable.market, new Set());
        }
        unparseableByMarket.get(unparseable.market)!.add(unparseable.signature);
      }

      console.log(
        `Total unparseable transactions: ${unparseableSignatures.size}`,
      );
      console.log(
        `Signatures: ${Array.from(unparseableSignatures).join(', ')}`,
      );
      console.log('');

      console.log(`📊 BREAKDOWN BY MARKET:`);
      for (const [market, signatures] of unparseableByMarket) {
        console.log(`Market ${market}: ${signatures.size} transactions`);
        console.log(`  Signatures: ${Array.from(signatures).join(', ')}`);
      }
      console.log('');
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
      printTxFetchSummary();
      printTruncatedSummary(allTruncatedSignatures);
      process.exit(1);
    } else {
      if (allUnparseableFills.length > 0) {
        console.log(
          `\n✅ All trades verified successfully! (${allUnparseableFills.length} fills had unparseable onchain transactions - see above)`,
        );
      } else {
        console.log(
          '\n✅ All trades verified successfully! No mismatches found.',
        );
      }
      printTxFetchSummary();
      printTruncatedSummary(allTruncatedSignatures);
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
