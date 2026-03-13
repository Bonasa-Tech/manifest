import { Connection } from '@solana/web3.js';

import { FillLog } from './manifest/accounts/FillLog';
import { PROGRAM_ID } from './manifest';
import * as promClient from 'prom-client';
import { fillDiscriminant, toFillLogResult } from './fillFeed';
import {
  detectAggregatorFromKeys,
  detectOriginatingProtocolFromKeys,
} from './aggregators';
import { WebSocketManager } from './utils/WebSocketManager';

// For live monitoring of the fill feed. For a more complete look at fill
// history stats, need to index all trades.
const fills = new promClient.Counter({
  name: 'fills_block',
  help: 'Number of fills from block processing',
  labelNames: ['market', 'isGlobal', 'takerIsBuy'] as const,
});

/**
 * FillFeedBlockSub - Processes blocks sequentially using getBlock to find Manifest program transactions
 */
export class FillFeedBlockSub {
  private wsManager: WebSocketManager;
  private shouldEnd: boolean = false;
  private ended: boolean = false;
  private lastUpdateUnix: number = Date.now();
  private currentSlot: number = 0;
  private blockProcessingDelay: number = 100; // 100ms delay between iterations

  constructor(
    private connection: Connection,
    wsPort: number = 1234,
  ) {
    this.wsManager = new WebSocketManager(wsPort, 30000);
  }

  public msSinceLastUpdate() {
    return Date.now() - this.lastUpdateUnix;
  }

  public async stop() {
    this.shouldEnd = true;

    // Wait for processing to finish gracefully
    const start = Date.now();
    while (!this.ended) {
      const timeout = 10_000;
      const pollInterval = 500;

      if (Date.now() - start > timeout) {
        console.warn('Force stopping block processing after timeout');
        break;
      }

      await new Promise((resolve) => setTimeout(resolve, pollInterval));
    }

    // Close WebSocket server
    this.wsManager.close();
    this.ended = true;
  }

  /**
   * Start processing blocks sequentially
   */
  public async start() {
    try {
      // Get the current slot to start processing from
      this.currentSlot = await this.connection.getSlot('finalized');
      console.log(`Starting block processing from slot ${this.currentSlot}`);

      while (!this.shouldEnd) {
        try {
          // TODO: If the last one had too many, dont waste time with getSlot and just get a bunch.
          // Get the latest finalized slot
          const latestSlot = await this.connection.getSlot('finalized');

          // Determine which slots need to be processed
          const slotsToProcess: number[] = [];
          for (let slot = this.currentSlot; slot <= latestSlot; slot++) {
            slotsToProcess.push(slot);
          }

          if (slotsToProcess.length === 0) {
            // No new slots to process, continue immediately
            continue;
          }

          console.log(
            `Fetching ${slotsToProcess.length} blocks in parallel (${this.currentSlot} to ${latestSlot})`,
          );

          // Fetch all blocks in parallel
          const blockPromises = slotsToProcess.map((slot) =>
            this.connection.getBlock(slot, {
              maxSupportedTransactionVersion: 0,
              transactionDetails: 'full',
              commitment: 'finalized',
            }),
          );

          const blocks = await Promise.all(blockPromises);

          // Process blocks in order
          for (let i = 0; i < blocks.length; i++) {
            const slot = slotsToProcess[i];
            const block = blocks[i];

            if (!block) {
              // Block doesn't exist or is not finalized yet
              continue;
            }

            console.log(
              `Processing block ${slot} with ${block.transactions.length} transactions`,
            );

            for (const tx of block.transactions) {
              if (tx.meta?.err !== null) {
                // Skip failed transactions
                continue;
              }

              // Check if this transaction involves the Manifest program
              const hasManifestProgram =
                this.transactionInvolvesManifestProgram(tx);
              if (!hasManifestProgram) {
                continue;
              }

              await this.processTransaction(tx, slot, block.blockTime);
            }

            this.lastUpdateUnix = Date.now();
          }

          // Update current slot to continue from the next unprocessed slot
          this.currentSlot = latestSlot + 1;
        } catch (error) {
          console.error(
            `Error processing blocks from ${this.currentSlot}:`,
            error,
          );

          // On error, move forward one slot and add a longer delay
          this.currentSlot++;

          await new Promise((resolve) =>
            setTimeout(resolve, this.blockProcessingDelay * 3),
          );
        }
      }
    } catch (error) {
      console.error('Fatal error in block processing:', error);
    } finally {
      console.log('FillFeedBlockSub ended');
      this.ended = true;
    }
  }

  /**
   * Check if a transaction involves the Manifest program
   * This checks account keys and addresses loaded from lookup tables
   */
  private transactionInvolvesManifestProgram(tx: any): boolean {
    if (!tx.transaction?.message) {
      return false;
    }

    const message = tx.transaction.message;
    const programId = PROGRAM_ID.toBase58();

    // Check legacy transaction format
    if ('accountKeys' in message) {
      const inAccountKeys = message.accountKeys.some(
        (key: any) => key.toBase58() === programId,
      );
      if (inAccountKeys) {
        return true;
      }
    }

    // Check versioned transaction format
    if ('staticAccountKeys' in message) {
      const inAccountKeys = message.staticAccountKeys.some(
        (key: any) => key.toBase58() === programId,
      );
      if (inAccountKeys) {
        return true;
      }
    }

    // Check addresses loaded from address lookup tables (ALTs)
    if (tx.meta?.loadedAddresses) {
      const loadedAddresses = tx.meta.loadedAddresses;

      if (loadedAddresses.writable) {
        const inWritable = loadedAddresses.writable.some(
          (key: any) =>
            (typeof key === 'string' ? key : key.toBase58()) === programId,
        );
        if (inWritable) {
          return true;
        }
      }

      if (loadedAddresses.readonly) {
        const inReadonly = loadedAddresses.readonly.some(
          (key: any) =>
            (typeof key === 'string' ? key : key.toBase58()) === programId,
        );
        if (inReadonly) {
          return true;
        }
      }
    }

    return false;
  }

  /**
   * Process a single transaction from a block
   */
  private async processTransaction(
    tx: any,
    slot: number,
    blockTime?: number | null,
  ): Promise<void> {
    const signature = tx.transaction.signatures[0];
    console.log('Handling transaction', signature, 'slot', slot);

    if (!tx.meta?.logMessages) {
      console.log('No log messages');
      return;
    }

    // Extract signers from the transaction
    let originalSigner: string | undefined;
    let signers: string[] = [];
    let accountKeysStr: string[] = [];

    try {
      const message = tx.transaction.message;

      if ('accountKeys' in message) {
        // Legacy transaction
        accountKeysStr = message.accountKeys.map((key: any) => key.toBase58());
        originalSigner = accountKeysStr[0];
        // Extract all signers using isAccountSigner method
        signers = message.accountKeys
          .map((key: any, index: number) => ({ key, index }))
          .filter(({ index }: any) => message.isAccountSigner(index))
          .map(({ key }: any) => key.toBase58());
      } else {
        // Versioned transaction (v0) - use staticAccountKeys
        accountKeysStr = message.staticAccountKeys.map((key: any) =>
          key.toBase58(),
        );
        originalSigner = accountKeysStr[0];
        // Extract all signers using isAccountSigner method
        signers = message.staticAccountKeys
          .map((key: any, index: number) => ({ key, index }))
          .filter(({ index }: any) => message.isAccountSigner(index))
          .map(({ key }: any) => key.toBase58());
      }
    } catch (error) {
      console.error('Error extracting signers:', error);
      return;
    }

    const aggregator = detectAggregatorFromKeys(accountKeysStr);
    const originatingProtocol =
      detectOriginatingProtocolFromKeys(accountKeysStr);

    const messages: string[] = tx.meta.logMessages;
    const programDatas: string[] = messages.filter((message) => {
      return message.includes('Program data:');
    });

    if (programDatas.length === 0) {
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
        slot,
        signature,
        originalSigner,
        aggregator,
        originatingProtocol,
        signers,
        blockTime ?? undefined,
      );
      const resultString: string = JSON.stringify(fillResult);
      console.log('Got a fill', resultString);
      fills.inc({
        market: deserializedFillLog.market.toString(),
        isGlobal: deserializedFillLog.isMakerGlobal.toString(),
        takerIsBuy: deserializedFillLog.takerIsBuy.toString(),
      });

      // Send to all connected clients
      this.wsManager.broadcast(JSON.stringify(fillResult));
    }
  }
}
