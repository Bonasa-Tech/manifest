import { Connection } from '@solana/web3.js';
import { FillLogResult, FillLog } from '@bonasa-tech/manifest-sdk';
import {
  genAccDiscriminator,
  convertU128,
} from '@bonasa-tech/manifest-sdk/utils';

const fillDiscriminant = genAccDiscriminator('manifest::logs::FillLog');

// Constants for known aggregators and protocols
const AGGREGATOR_PROGRAM_IDS = {
  MEXkeo4BPUCZuEJ4idUUwMPu4qvc9nkqtLn3yAyZLxg: 'Swissborg',
  T1TANpTeScyeqVzzgNViGDNrkQ6qHz9KrSBS4aNXvGT: 'Titan',
  '6m2CDdhRgxpH4WjvdzxAYbGxwdGUz5MziiL5jek2kBma': 'OKX',
  proVF4pMXVaYqmy4NjniPh4pqKNfMmsihgd4wdkCX3u: 'OKX',
  DF1ow4tspfHX9JwWJsAb9epbkA8hmpSEAtxXy1V27QBH: 'DFlow',
  JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4: 'Jupiter',
  SPURp82qAR9nvzy8j1gP31zmzGytrgDBKcpGzeGkka8: 'Spur',
  s7SunwrPG5SbViEKiViaDThPRJxkkTrNx2iRPN3exNC: 'Bitget',
} as const;

const ORIGINATING_PROTOCOL_IDS = {
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

function detectAggregatorFromKeys(accountKeys: string[]): string | undefined {
  for (const account of accountKeys) {
    const aggregator =
      AGGREGATOR_PROGRAM_IDS[account as keyof typeof AGGREGATOR_PROGRAM_IDS];
    if (aggregator) {
      return aggregator;
    }
  }
  return undefined;
}

function detectOriginatingProtocolFromKeys(
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

export const parseTransactionForFills = async (
  connection: Connection,
  signature: string,
): Promise<FillLogResult[]> => {
  const fills: FillLogResult[] = [];

  const tx = await connection.getTransaction(signature, {
    maxSupportedTransactionVersion: 0,
  });

  if (!tx?.meta?.logMessages) {
    return fills;
  }

  if (tx.meta.err != null) {
    return fills;
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

  return fills;
};
