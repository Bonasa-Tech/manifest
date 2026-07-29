import { Connection } from '@solana/web3.js';
import { FillLogResult, FillLog } from '../../client/ts/src';
import {
  genAccDiscriminator,
  convertU128,
  hasTruncatedLogs,
} from '../../client/ts/src/utils';
import {
  detectAggregatorFromKeys,
  detectOriginatingProtocolFromKeys,
  resolveTakerFromSigners,
} from '../../client/ts/src/aggregators';

const fillDiscriminant = genAccDiscriminator('manifest::logs::FillLog');

function toFillLogResult(
  fillLog: FillLog,
  slot: number,
  signature: string,
  originalSigner?: string,
  aggregator?: string,
  originatingProtocol?: string,
  signers?: string[],
  blockTime?: number,
): FillLogResult {
  // When a delegating signer (e.g. jupui) signed on behalf of the real taker,
  // attribute the fill to the other signer instead of the on-chain taker.
  const takerFromSigner: string | undefined = resolveTakerFromSigners(
    signers,
    originalSigner,
  );

  const result: FillLogResult = {
    market: fillLog.market.toBase58(),
    maker: fillLog.maker.toBase58(),
    taker: takerFromSigner ?? fillLog.taker.toBase58(),
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

export interface ParseTransactionForFillsResult {
  fills: FillLogResult[];
  hasTruncatedLogs: boolean;
}

export const parseTransactionForFills = async (
  connection: Connection,
  signature: string,
): Promise<ParseTransactionForFillsResult> => {
  const fills: FillLogResult[] = [];

  const tx = await connection.getTransaction(signature, {
    maxSupportedTransactionVersion: 0,
  });

  if (!tx?.meta?.logMessages) {
    return { fills, hasTruncatedLogs: false };
  }

  // Truncated logs drop Program data entries, so fills can be silently missing.
  const logsTruncated: boolean = hasTruncatedLogs(tx.meta.logMessages);

  if (tx.meta.err != null) {
    return { fills, hasTruncatedLogs: logsTruncated };
  }

  const slot = tx.slot;
  const blockTime = tx.blockTime ?? undefined;

  // Extract signers
  let originalSigner: string | undefined;
  let signers: string[] | undefined;

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

  // Detect aggregator and originating protocol
  let aggregator: string | undefined;
  let originatingProtocol: string | undefined;

  if ('accountKeys' in message) {
    const accountKeysStr = message.accountKeys.map((k) => k.toBase58());
    aggregator = detectAggregatorFromKeys(accountKeysStr);
    originatingProtocol = detectOriginatingProtocolFromKeys(accountKeysStr);
  } else {
    const accountKeysStr = message.staticAccountKeys.map((k) => k.toBase58());
    aggregator = detectAggregatorFromKeys(accountKeysStr);
    originatingProtocol = detectOriginatingProtocolFromKeys(accountKeysStr);
  }

  const messages = tx.meta.logMessages;
  const programDatas = messages.filter((msg) => msg.includes('Program data:'));

  for (const programDataEntry of programDatas) {
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
        blockTime,
      );

      fills.push(fillResult);
    } catch (error) {
      console.error(`Error deserializing FillLog:`, error);
    }
  }

  return { fills, hasTruncatedLogs: logsTruncated };
};
