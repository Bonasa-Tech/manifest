import { WebSocketManager } from './utils/WebSocketManager';
import {
  Connection,
  ConfirmedSignatureInfo,
  VersionedTransactionResponse,
} from '@solana/web3.js';

import { FillLog } from './manifest/accounts/FillLog';
import { PROGRAM_ID } from './manifest';
import { convertU128 } from './utils/numbers';
import { genAccDiscriminator } from './utils/discriminator';
import * as promClient from 'prom-client';
import { FillLogResult } from './types';

const SIGNATURE_BATCH_SIZE = 10;

// For live monitoring of the fill feed. For a more complete look at fill
// history stats, need to index all trades.
const fills = new promClient.Counter({
  name: 'fills',
  help: 'Number of fills',
  labelNames: ['market', 'isGlobal', 'takerIsBuy'] as const,
});
const fillLag = new promClient.Gauge({
  name: 'fill_lag_seconds',
  help: 'Lag in seconds between now and the block time of the most recent fill',
});

/**
 * FillFeed example implementation.
 */
export class FillFeed {
  private wsManager: WebSocketManager;
  private shouldEnd: boolean = false;
  private ended: boolean = false;
  private lastUpdateUnix: number = Date.now();

  constructor(private connection: Connection) {
    this.wsManager = new WebSocketManager(1234, 30000);
  }

  public msSinceLastUpdate() {
    return Date.now() - this.lastUpdateUnix;
  }

  public async stopParseLogs() {
    this.shouldEnd = true;
    const start = Date.now();
    while (!this.ended) {
      const timeout = 30_000;
      const pollInterval = 500;

      if (Date.now() - start > timeout) {
        return Promise.reject(
          new Error(
            `failed to stop parseLogs after ${timeout / 1_000} seconds`,
          ),
        );
      }

      await new Promise((resolve) => setTimeout(resolve, pollInterval));
    }

    return Promise.resolve();
  }

  /**
   * Parse logs in an endless loop.
   */
  public async parseLogs(endEarly?: boolean) {
    try {
      // Start with a hopefully recent signature.
      const lastSignatureStatus = (
        await this.connection.getSignaturesForAddress(
          PROGRAM_ID,
          { limit: 1 },
          'finalized',
        )
      )[0];
      let lastSignature: string | undefined = lastSignatureStatus.signature;
      let lastSlot: number = lastSignatureStatus.slot;

      // End early is 30 seconds, used for testing.
      const endTime: Date = endEarly
        ? new Date(Date.now() + 30_000)
        : new Date(Date.now() + 1_000_000_000_000);

      // TODO: remove endTime in favor of stopParseLogs for testing
      while (!this.shouldEnd && new Date(Date.now()) < endTime) {
        // This sleep was originally implemented to wait until there was enough
        // transactions to avoid just spamming the RPC. Reduced to just
        // enough to avoid RPC spam, but not wait too long since the router
        // integrations give us steady flow.
        await new Promise((f) => setTimeout(f, 400));

        const signatures: ConfirmedSignatureInfo[] =
          await this.connection.getSignaturesForAddress(
            PROGRAM_ID,
            {
              until: lastSignature,
            },
            'finalized',
          );
        // Flip it so we do oldest first.
        signatures.reverse();

        // Process even single signatures, but handle the edge case differently
        if (signatures.length === 0) {
          continue;
        }

        // If we only got back the same signature we already processed, skip it
        if (
          signatures.length === 1 &&
          signatures[0].signature === lastSignature
        ) {
          continue;
        }
        const filteredSignatures = signatures.filter((sig) => {
          return sig.signature !== lastSignature && sig.slot >= lastSlot;
        });

        for (
          let i = 0;
          i < filteredSignatures.length;
          i += SIGNATURE_BATCH_SIZE
        ) {
          const batch = filteredSignatures.slice(i, i + SIGNATURE_BATCH_SIZE);
          await Promise.all(batch.map((sig) => this.handleSignature(sig)));
        }

        console.log(
          'New last signature:',
          signatures[signatures.length - 1].signature,
          'New last signature slot:',
          signatures[signatures.length - 1].slot,
          'num sigs',
          signatures.length,
        );
        lastSignature = signatures[signatures.length - 1].signature;
        lastSlot = signatures[signatures.length - 1].slot;

        this.lastUpdateUnix = Date.now();
      }
    } finally {
      console.log('ended loop');
      this.wsManager.close();
      this.ended = true;
    }
  }

  /**
   * Handle a signature by fetching the tx onchain and possibly sending a fill
   * notification.
   */
  private async handleSignature(signature: ConfirmedSignatureInfo) {
    console.log('Handling', signature.signature, 'slot', signature.slot);
    const tx = await this.connection.getTransaction(signature.signature, {
      maxSupportedTransactionVersion: 0,
    });
    if (!tx?.meta?.logMessages) {
      console.log('No log messages');
      return;
    }
    if (tx.meta.err != null) {
      console.log('Skipping failed tx', signature.signature);
      return;
    }

    // Extract the original signer (fee payer/first signer) and all signers
    let originalSigner: string | undefined;
    let signers: string[] | undefined;
    try {
      const message = tx.transaction.message;

      if ('accountKeys' in message) {
        // Legacy transaction
        originalSigner = message.accountKeys[0]?.toBase58();
        // Extract all signers using isAccountSigner method
        signers = message.accountKeys
          .map((key, index) => ({ key, index }))
          .filter(({ index }) => message.isAccountSigner(index))
          .map(({ key }) => key.toBase58());
      } else {
        // Versioned transaction (v0) - use staticAccountKeys for the main accounts
        originalSigner = message.staticAccountKeys[0]?.toBase58();
        // Extract all signers using isAccountSigner method
        signers = message.staticAccountKeys
          .map((key, index) => ({ key, index }))
          .filter(({ index }) => message.isAccountSigner(index))
          .map(({ key }) => key.toBase58());
      }
    } catch (error) {
      console.error('Error extracting signers:', error);
    }

    const aggregator: string | undefined = detectAggregator(tx);
    const originatingProtocol: string | undefined =
      detectOriginatingProtocol(tx);

    const messages: string[] = tx?.meta?.logMessages!;
    const programDatas: string[] = messages.filter((message) => {
      return message.includes('Program data:');
    });

    if (programDatas.length == 0) {
      console.log('No program datas');
      return;
    }

    for (const programDataEntry of programDatas) {
      const programData = programDataEntry.split(' ')[2];
      const byteArray: Uint8Array = Uint8Array.from(atob(programData), (c) =>
        c.charCodeAt(0),
      );
      const buffer = Buffer.from(byteArray);
      if (!buffer.subarray(0, 8).equals(fillDiscriminant)) {
        continue;
      }
      const deserializedFillLog: FillLog = FillLog.deserialize(
        buffer.subarray(8),
      )[0];
      const fillResult = toFillLogResult(
        deserializedFillLog,
        signature.slot,
        signature.signature,
        originalSigner,
        aggregator,
        originatingProtocol,
        signers,
        // ?? undefined because can be null or undefined
        signature.blockTime ?? undefined,
      );
      const resultString: string = JSON.stringify(fillResult);
      console.log('Got a fill', resultString);
      fills.inc({
        market: deserializedFillLog.market.toString(),
        isGlobal: deserializedFillLog.isMakerGlobal.toString(),
        takerIsBuy: deserializedFillLog.takerIsBuy.toString(),
      });
      this.wsManager.broadcast(JSON.stringify(fillResult));
      if (signature.blockTime) {
        fillLag.set(Date.now() / 1000 - signature.blockTime);
      }
    }
  }
}

// Constants for known aggregators and protocols
export const AGGREGATOR_PROGRAM_IDS = {
  MEXkeo4BPUCZuEJ4idUUwMPu4qvc9nkqtLn3yAyZLxg: 'Swissborg',
  T1TANpTeScyeqVzzgNViGDNrkQ6qHz9KrSBS4aNXvGT: 'Titan',
  '6m2CDdhRgxpH4WjvdzxAYbGxwdGUz5MziiL5jek2kBma': 'OKX',
  proVF4pMXVaYqmy4NjniPh4pqKNfMmsihgd4wdkCX3u: 'OKX',
  DF1ow4tspfHX9JwWJsAb9epbkA8hmpSEAtxXy1V27QBH: 'DFlow',
  JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4: 'Jupiter',
  SPURp82qAR9nvzy8j1gP31zmzGytrgDBKcpGzeGkka8: 'Spur',
  s7SunwrPG5SbViEKiViaDThPRJxkkTrNx2iRPN3exNC: 'Bitget',
} as const;

export const ORIGINATING_PROTOCOL_IDS = {
  LiMoM9rMhrdYrfzUCxQppvxCSG1FcrUK9G8uLq4A1GF: 'kamino',
  UMnFStVeG1ecZFc2gc5K3vFy3sMpotq8C91mXBQDGwh: 'cabana',
  BQ72nSv9f3PRyRKCBnHLVrerrv37CYTHm5h3s9VSGQDV: 'jupiter', // JUP 1
  '2MFoS3MPtvyQ4Wh4M9pdfPjz6UhVoNbFbGJAskCPCj3h': 'jupiter', // JUP 2
  HU23r7UoZbqTUuh3vA7emAGztFtqwTeVips789vqxxBw: 'jupiter', // JUP 3
  '6LXutJvKUw8Q5ue2gCgKHQdAN4suWW8awzFVC6XCguFx': 'jupiter', // JUP 5
  CapuXNQoDviLvU1PxFiizLgPNQCxrsag1uMeyk6zLVps: 'jupiter', // JUP 6
  GGztQqQ6pCPaJQnNpXBgELr5cs3WwDakRbh1iEMzjgSJ: 'jupiter', // JUP 7
  '9nnLbotNTcUhvbrsA6Mdkx45Sm82G35zo28AqUvjExn8': 'jupiter', // JUP 8
  '6U91aKa8pmMxkJwBCfPTmUEfZi6dHe7DcFq2ALvB2tbB': 'jupiter', // JUP 12
  '4xDsmeTWPNjgSVSS1VTfzFq3iHZhp77ffPkAmkZkdu71': 'jupiter', // JUP 14
  HFqp6ErWHY6Uzhj8rFyjYuDya2mXUpYEk8VW75K9PSiY: 'jupiter', // JUP 16
  '9yj3zvLS3fDMqi1F8zhkaWfq8TZpZWHe6cz1Sgt7djXf': 'phantom',
  '8psNvWTrdNTiVRNzAgsou9kETXNJm2SXZyaKuJraVRtf': 'phantom',
  B3111yJCeHBcA1bizdJjUFPALfhAfSRnAbJzGUtnt56A: 'binance',
} as const;

// Helper function to detect aggregator from account keys
export function detectAggregatorFromKeys(
  accountKeys: string[],
): string | undefined {
  for (const account of accountKeys) {
    const aggregator =
      AGGREGATOR_PROGRAM_IDS[account as keyof typeof AGGREGATOR_PROGRAM_IDS];
    if (aggregator) {
      return aggregator;
    }
  }
  return undefined;
}

// Helper function to detect originating protocol from account keys
export function detectOriginatingProtocolFromKeys(
  accountKeys: string[],
): string | undefined {
  for (const accountKey of accountKeys) {
    const protocol =
      ORIGINATING_PROTOCOL_IDS[
        accountKey as keyof typeof ORIGINATING_PROTOCOL_IDS
      ];
    if (protocol) {
      return protocol;
    }
  }
  return undefined;
}

function detectAggregator(
  tx: VersionedTransactionResponse,
): string | undefined {
  // Look for the aggregator program id from a list of known ids.
  try {
    // For versioned transactions, we need to handle both static and resolved account keys
    const message = tx.transaction.message;

    // Handle both legacy and versioned transactions
    if ('accountKeys' in message) {
      // Legacy transaction
      const accountKeysStr = message.accountKeys.map((k) => k.toBase58());
      return detectAggregatorFromKeys(accountKeysStr);
    } else {
      // V0 transaction - use staticAccountKeys directly to avoid lookup resolution issues
      const accountKeysStr = message.staticAccountKeys.map((k) => k.toBase58());
      return detectAggregatorFromKeys(accountKeysStr);
    }
  } catch (error) {
    console.warn('Error detecting aggregator:', error);
    // Fall back to undefined if we can't detect the aggregator
  }
  return undefined;
}

function detectOriginatingProtocol(
  tx: VersionedTransactionResponse,
): string | undefined {
  try {
    const message = tx.transaction.message;

    // Handle both legacy and versioned transactions
    if ('accountKeys' in message) {
      // Legacy transaction
      const accountKeysStr = message.accountKeys.map((k) => k.toBase58());
      return detectOriginatingProtocolFromKeys(accountKeysStr);
    } else {
      // V0 transaction - use staticAccountKeys directly to avoid lookup resolution issues
      const accountKeysStr = message.staticAccountKeys.map((k) => k.toBase58());
      return detectOriginatingProtocolFromKeys(accountKeysStr);
    }
  } catch (error) {
    console.warn('Error detecting originating protocol:', error);
    // Fall back to undefined if we can't detect the originating protocol
  }
  return undefined;
}

export const fillDiscriminant = genAccDiscriminator('manifest::logs::FillLog');

export function toFillLogResult(
  fillLog: FillLog,
  slot: number,
  signature: string,
  originalSigner?: string,
  aggregator?: string,
  originatingProtocol?: string,
  signers?: string[],
  blockTime?: number,
): FillLogResult {
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
}
