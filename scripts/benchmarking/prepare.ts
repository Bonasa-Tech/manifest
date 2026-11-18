import * as bs58 from 'bs58';
import {
  AddressLookupTableAccount,
  ComputeBudgetInstruction,
  ConfirmedSignatureInfo,
  Connection,
  PublicKey,
  TransactionInstruction,
  VersionedTransactionResponse,
} from '@solana/web3.js';
import {
  MarketState,
  decodeInstructionData,
  getPhoenixEventsFromLogData,
} from 'phoenix-sdk';
import { bignum } from '@metaplex-foundation/beet';
import * as PhoenixIdl from './phoenix-idl.json';
import { parseLogs } from './parseLogs';
import { writeFileSync, existsSync, mkdirSync } from 'fs';

const phoenixMktPk = new PublicKey(
  'BKLhZ5NrFhCjViC4wyAMXBNsJFHbFfYujo3TtUmBxTH3',
);
// SOL / USDC '4DoNfFBfF7UokCC2FQzriy7yHK6DY6NVdYpuekQ5pRgg',
// BONK / SOL 'FicF181nDsEcasznMTPp9aLa5Rbpdtd11GtSEa1UUWzx'
// WIF / SOL 'BKLhZ5NrFhCjViC4wyAMXBNsJFHbFfYujo3TtUmBxTH3'
const DEFAULT_SLOTS = 100;
const GSFA_LIMIT = 1000;
const GTX_LIMIT = 50;

const { RPC_URL, SLOTS } = process.env;

export const PROGRAM = {
  COMPUTE_BUDGET: new PublicKey('ComputeBudget111111111111111111111111111111'),
  SYSTEM: PublicKey.default,
  SPL_MEMO: new PublicKey('MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr'),
  SPL_NOOP: new PublicKey('noopb9bkMVfRPU8AsbpTUg8AQkHtKwMYZiFUjNRtMmV'),
  SPL_TOKEN: new PublicKey('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA'),
  JUPITER_V3: new PublicKey('JUP3c2Uh3WA4Ng34tw6kPd2G4C5BB21Xo36Je1s32Ph'),
  JUPITER_V4: new PublicKey('JUP4Fb2cqiRUcaTHdrPC8h2gNsA2ETXiPDD33WcGuJB'),
  JUPITER_V5: new PublicKey('JUP5pEAZeHdHrLxh5UCwAbpjGwYKKoquCpda2hfP4u8'),
  JUPITER_V6: new PublicKey('JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4'),
  MANGO_V4: new PublicKey('4MangoMjqJ2firMokCjjGgoK8d4MXcrgL7XJaL3w6fVg'),
  OPENBOOK: new PublicKey('srmqPvymJeFKQ4zGQed1GFppgkRHL9kaELCbyksJtPX'),
  OPENBOOK_V2: new PublicKey('opnb2LAfJYbRMAHHvqjCwQxanZn7ReEHp1k81EohpZb'),
  ORCA_V2: new PublicKey('9W959DqEETiGZocYWCQPaJ6sBmUzgfxXfqGeTEdp3aQP'),
  ORCA_WHIRLPOOL: new PublicKey('whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc'),
  PHOENIX: new PublicKey('PhoeNiXZ8ByJGLkxNfZRnkUfjvmuYqLR89jjFHGqdXY'),
  RAYDIUM_V4: new PublicKey('675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8'),
  RAYDIUM_CLAMM: new PublicKey('CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK'),
  DUAL_DIP: new PublicKey('esdUULhoeFHro46mvkTizMzg38hKBunvErmPuUw6n9p'),
  DUAL_MARKET: new PublicKey('DiPbvUUJkDhV9jFtQsDFnMEMRJyjW5iS6NMwoySiW8ki'),
  DUAL_SO: new PublicKey('4yx1NJ4Vqf2zT1oVLk4SySBhhDJXmXFt88ncm4gPxtL7'),
  WORMHOLE: new PublicKey('wormDTUJ6AWPNvk59vGQbDvGJmqbDTdgWgAqcLBCgUb'),
  SQUADS: new PublicKey('SMPLecH534NA9acpos4G6x7uf3LWbCAwZQE9e8ZekMu'),
  SQUADS_V4: new PublicKey('SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf'),
  TBTC: new PublicKey('87MEvHZCXE3ML5rrmh5uX1FbShHmRXXS32xJDGbQ7h5t'),
  REALMS: new PublicKey('GovER5Lthms3bLBqWub97yVrMmEogzX7xNjdXpPPCVZw'),
  CCTP: new PublicKey('CCTPmbSD7gX1bxKPAmg77w8oFzNFpaQiQUWD43TKaecd'),
};

async function scrapeConfirmedSignatures(
  conn: Connection,
  address: PublicKey,
  startSlot: number,
  endSlot: number,
): Promise<ConfirmedSignatureInfo[]> {
  let maxSlot = 0;
  let allSigs: ConfirmedSignatureInfo[] = [];

  while (allSigs.length == 0) {
    const initSigs = await conn.getSignaturesForAddress(address, {
      limit: GSFA_LIMIT,
    });
    console.log('initSigs', initSigs.length, maxSlot);

    initSigs.reverse();
    allSigs = initSigs.filter((s) => s.slot >= startSlot);
  }

  while (maxSlot <= endSlot) {
    const sigUntil = allSigs[allSigs.length - 1].signature;

    let sigBefore;
    let concatSigs: ConfirmedSignatureInfo[] = [];
    // eslint-disable-next-line no-constant-condition
    while (true) {
      const fetchSigs = await conn.getSignaturesForAddress(address, {
        before: sigBefore,
        until: sigUntil,
        limit: GSFA_LIMIT,
      });

      concatSigs = concatSigs.concat(fetchSigs);

      console.log('fetchSigs', fetchSigs.length, maxSlot);

      if (fetchSigs.length < GSFA_LIMIT) {
        break;
      }

      sigBefore = fetchSigs[fetchSigs.length - 1].signature;
    }

    console.log('concatSigs', concatSigs.length, maxSlot);

    // reverse order so items are sorted in ascending time order
    concatSigs.reverse();
    if (concatSigs.length > 0) {
      allSigs = allSigs.concat(concatSigs.filter((s) => s.slot >= startSlot));
      maxSlot = concatSigs[0].slot;
    }
  }

  console.log('allSigs', allSigs.length, maxSlot);

  const uniqueSigs = new Set(allSigs.map((s) => s.signature));

  console.log('uniqueSigs', uniqueSigs.size, maxSlot);

  return allSigs.filter((s) => uniqueSigs.delete(s.signature));
}

async function fetchTxBatch(
  conn: Connection,
  batch: ConfirmedSignatureInfo[],
): Promise<{
  txs: VersionedTransactionResponse[];
  alts: AddressLookupTableAccount[];
}> {
  const txs: VersionedTransactionResponse[] = (
    await conn.getTransactions(
      batch.map((t) => t.signature),
      { commitment: 'confirmed', maxSupportedTransactionVersion: 0 },
    )
  ).filter((tx) => tx !== null) as VersionedTransactionResponse[];

  const altPks = [
    ...new Set(
      txs.flatMap((tx) =>
        tx!.transaction.message.addressTableLookups.map((l) =>
          l.accountKey.toString(),
        ),
      ),
    ),
  ];

  const altAis = await conn.getMultipleAccountsInfo(
    altPks.map((pk) => new PublicKey(pk)),
  );

  const alts = altPks.map(
    (key, i) =>
      new AddressLookupTableAccount({
        key: new PublicKey(key),
        state: AddressLookupTableAccount.deserialize(altAis[i]!.data),
      }),
  );

  return { txs, alts };
}

function parseTransaction(
  tx: VersionedTransactionResponse,
  alts: AddressLookupTableAccount[],
) {
  // Skip error transactions.
  if (tx.meta?.err) {
    return;
  }
  const signature = tx.transaction.signatures[0];
  const txAlts = tx!.transaction.message.addressTableLookups.map(
    (l) => alts.find((alt) => alt.key.equals(l.accountKey))!,
  );
  const txAccountKeys = tx!.transaction.message.getAccountKeys({
    addressLookupTableAccounts: txAlts,
  });

  function accountMeta(accountIndex) {
    const { header } = tx.transaction.message;
    const pubkey = txAccountKeys.get(accountIndex)!;
    if (
      accountIndex <
      header.numRequiredSignatures - header.numReadonlySignedAccounts
    ) {
      return { pubkey, isSigner: true, isWritable: true };
    } else if (accountIndex < header.numRequiredSignatures) {
      return { pubkey, isSigner: true, isWritable: false };
    } else if (
      accountIndex <
      txAccountKeys.length - header.numReadonlyUnsignedAccounts
    ) {
      return { pubkey, isSigner: false, isWritable: true };
    } else {
      return { pubkey, isSigner: false, isWritable: false };
    }
  }

  const parsedIxs = (tx?.transaction.message.compiledInstructions ?? []).map(
    (cix, messageIndex) => {
      const programId = txAccountKeys.get(cix.programIdIndex)!;
      const instruction: TransactionInstruction = {
        data: Buffer.from(cix.data),
        programId,
        keys: cix.accountKeyIndexes.map((ix) => accountMeta(ix)),
      };
      return { programId, instruction, messageIndex };
    },
  );

  const mergedIxs: {
    programId: PublicKey;
    instruction: TransactionInstruction;
    messageIndex: number;
    cpiIndex?: number;
  }[] = [];

  for (let messageIndex = 0; messageIndex < parsedIxs.length; ++messageIndex) {
    mergedIxs.push(parsedIxs[messageIndex]);
    const innerIxs =
      (tx?.meta?.innerInstructions ?? []).find(
        (iix) => iix.index === messageIndex,
      )?.instructions ?? [];
    const parsedInnerIx = innerIxs.map((ciix, cpiIndex) => {
      const programId = txAccountKeys.get(ciix.programIdIndex)!;
      const instruction: TransactionInstruction = {
        data: Buffer.from(bs58.default.decode(ciix.data.toString())),
        programId,
        keys: ciix.accounts.map((ix) => accountMeta(ix)),
      };
      return { programId, instruction, messageIndex, cpiIndex };
    });
    mergedIxs.push(...parsedInnerIx);
  }

  const parsedLogs = parseLogs(tx.meta?.logMessages || []);
  const signer = txAccountKeys.staticAccountKeys[0].toString();
  const slot = tx.slot;
  const blockTime = tx.blockTime;
  let computeUnitLimit, computeUnitPrice;
  let isJupFlow = false;
  const phxInstructions: {
    instructionName: string;
    decodedData: any;
    cuConsumed?: number;
  }[] = [];
  const phxLogs: any[] = [];

  for (let i = 0; i < mergedIxs.length; ++i) {
    const { programId, instruction } = mergedIxs[i];
    const logs = parsedLogs[i];

    if (PROGRAM.COMPUTE_BUDGET.equals(programId)) {
      const ixType =
        ComputeBudgetInstruction.decodeInstructionType(instruction);

      switch (ixType) {
        case 'RequestUnits': {
          const parsed =
            ComputeBudgetInstruction.decodeRequestUnits(instruction);
          computeUnitLimit = parsed.units;
          computeUnitPrice = parsed.additionalFee / parsed.units;
          break;
        }
        case 'RequestHeapFrame': {
          break;
        }
        case 'SetComputeUnitLimit': {
          const parsed =
            ComputeBudgetInstruction.decodeSetComputeUnitLimit(instruction);
          computeUnitLimit = parsed.units;
          break;
        }
        case 'SetComputeUnitPrice': {
          const parsed =
            ComputeBudgetInstruction.decodeSetComputeUnitPrice(instruction);
          computeUnitPrice =
            (parsed.microLamports.toString() as any) / 1_000_000;
          break;
        }
      }
    }

    if (
      PROGRAM.JUPITER_V6.equals(programId) ||
      PROGRAM.JUPITER_V5.equals(programId) ||
      PROGRAM.JUPITER_V4.equals(programId) ||
      PROGRAM.JUPITER_V3.equals(programId)
    ) {
      isJupFlow = true;
    }

    if (PROGRAM.PHOENIX.equals(programId)) {
      const { instructionName, decodedData } = decodeInstructionData(
        instruction.data,
        PhoenixIdl,
      );
      if (
        instruction.keys.length < 3 ||
        instruction.keys[2].pubkey.toBase58() != phoenixMktPk.toBase58()
      ) {
        continue;
      }
      const cuConsumed = logs?.cuConsumed;

      switch (instructionName) {
        case 'CancelAllOrders':
        case 'CancelAllOrdersWithFreeFunds':
        case 'CancelUpTo':
        case 'CancelUpToWithFreeFunds':
        case 'CancelMultipleOrdersById':
        case 'CancelMultipleOrdersByIdWithFreeFunds':
        case 'Swap':
        case 'SwapWithFreeFunds':
        case 'PlaceLimitOrder':
        case 'PlaceLimitOrderWithFreeFunds':
        case 'PlaceMultiplePostOnlyOrders':
        case 'PlaceMultiplePostOnlyOrdersWithFreeFunds': {
          phxInstructions.push({ instructionName, decodedData, cuConsumed });
          break;
        }
        case 'Log': {
          phxLogs.push(getPhoenixEventsFromLogData(instruction.data.slice(1)));
          break;
        }
        // case "ReduceOrder":
        // case "ReduceOrderWithFreeFunds":
        // case "ForceCancelOrders"
        case 'ChangeFeeRecipient':
        case 'CollectFees':
        case 'RequestSeat':
        case 'RequestSeatAuthorized':
        case 'ChangeSeatStatus':
        case 'EvictSeat':
        case 'DepositFunds':
        case 'WithdrawFunds':
        case 'ClaimAuthority':
        case 'NameSuccessor':
        case 'ChangeMarketStatus':
        case 'InitializeMarket':
          // ignored known tx
          break;
        default:
          console.log('ignored unknown instruction', instructionName);
      }
    }
  }

  const phxTape = phxInstructions
    .map((instruction) => instruction.instructionName)
    .join(',');

  if (phxTape.length === 0) {
    return;
  }

  return {
    signature,
    signer,
    slot,
    blockTime,
    computeUnitLimit,
    computeUnitPrice,
    isJupFlow,
    instructions: phxInstructions,
    logs: phxLogs.flatMap((l) => l.events),
  };
}

function parsePhoenixMarket(mktPk: PublicKey, buffer: Buffer) {
  const phxMkt = MarketState.load({
    address: mktPk,
    buffer: buffer,
  });
  const tradersByIndex = {};
  for (const traderKey of phxMkt.data.traders.keys()) {
    const trader = phxMkt.data.traders.get(traderKey)!;
    const traderQuoteAtoms: number = phxMkt.quoteLotsToQuoteAtoms(
      Number(trader.quoteLotsFree!) + Number(trader.quoteLotsLocked!),
    );
    const traderBaseAtoms: number = phxMkt.baseLotsToBaseAtoms(
      Number(trader.baseLotsFree!) + Number(trader.baseLotsLocked!),
    );
    const traderIndex: number =
      phxMkt.data.traderPubkeyToTraderIndex.get(traderKey)!;

    tradersByIndex[traderIndex] = {
      baseAtomsDeposited: traderBaseAtoms,
      quoteAtomsDeposited: traderQuoteAtoms,
      quoteLotsFree: Number(trader.quoteLotsFree!),
      baseLotsFree: Number(trader.baseLotsFree!),
      quoteLotsLocked: Number(trader.quoteLotsLocked!),
      baseLotsLocked: Number(trader.baseLotsLocked!),
      key: new PublicKey(traderKey),
    };
  }

  const bids: {
    priceFloat: number;
    orderSequenceNumber: bignum;
    traderIndex: number;
    numBaseAtoms: number;
    lastValidSlot: number;
    maker: string;
  }[] = [];
  for (const [orderId, restingOrder] of phxMkt.data.bids) {
    const priceFloat = phxMkt.ticksToFloatPrice(Number(orderId.priceInTicks));
    const orderSequenceNumber = orderId.orderSequenceNumber;
    const traderIndex = Number(restingOrder.traderIndex);
    const numBaseAtoms = phxMkt.baseLotsToBaseAtoms(
      Number(restingOrder.numBaseLots),
    );
    const lastValidSlot = Number(restingOrder.lastValidSlot);
    const _lastValidUnixTimestampInSeconds = Number(
      restingOrder.lastValidUnixTimestampInSeconds,
    );
    bids.push({
      priceFloat,
      orderSequenceNumber,
      traderIndex,
      numBaseAtoms,
      lastValidSlot,
      maker: tradersByIndex[traderIndex].key.toBase58(),
    });
  }

  const asks: {
    priceFloat: number;
    orderSequenceNumber: bignum;
    traderIndex: number;
    numBaseAtoms: number;
    lastValidSlot: number;
    maker: string;
  }[] = [];
  for (const [orderId, restingOrder] of phxMkt.data.asks) {
    const priceFloat = phxMkt.ticksToFloatPrice(Number(orderId.priceInTicks));
    const orderSequenceNumber = orderId.orderSequenceNumber;
    const traderIndex = Number(restingOrder.traderIndex);
    const numBaseAtoms = phxMkt.baseLotsToBaseAtoms(
      Number(restingOrder.numBaseLots),
    );
    const lastValidSlot = Number(restingOrder.lastValidSlot);
    const _lastValidUnixTimestampInSeconds = Number(
      restingOrder.lastValidUnixTimestampInSeconds,
    );
    asks.push({
      priceFloat,
      orderSequenceNumber,
      traderIndex,
      numBaseAtoms,
      lastValidSlot,
      maker: tradersByIndex[traderIndex].key.toBase58(),
    });
  }

  return {
    market: mktPk.toBase58(),
    baseMint: phxMkt.data.header.baseParams.mintKey.toBase58(),
    quoteMint: phxMkt.data.header.quoteParams.mintKey.toBase58(),
    baseLotSize: Number(phxMkt.data.header.baseLotSize),
    quoteLotSize: Number(phxMkt.data.header.quoteLotSize),
    baseDecimals: phxMkt.data.header.baseParams.decimals,
    quoteDecimals: phxMkt.data.header.quoteParams.decimals,
    tradersByIndex,
    bids,
    asks,
  };
}

async function main() {
  const conn = new Connection(RPC_URL!, 'finalized');

  console.log('Loading initial state');
  const phoenixMktResInitial =
    await conn.getAccountInfoAndContext(phoenixMktPk);
  const initialState = parsePhoenixMarket(
    phoenixMktPk,
    phoenixMktResInitial.value?.data!,
  );

  console.log('Waiting for slots to pass');
  await new Promise((f) => setTimeout(f, Number(SLOTS ?? DEFAULT_SLOTS) * 400));

  console.log('Loading ending slot');
  const phoenixMktResFinal = await conn.getAccountInfoAndContext(phoenixMktPk);

  const startSlot = phoenixMktResInitial.context.slot;
  const endSlot = phoenixMktResFinal.context.slot;

  console.log('Scraping confirmed signatures');
  const signatures = await scrapeConfirmedSignatures(
    conn,
    phoenixMktPk,
    startSlot,
    endSlot,
  );

  const batches: ConfirmedSignatureInfo[][] = [];
  while (signatures.length > 0) {
    batches.push(signatures.splice(0, GTX_LIMIT));
  }

  const results: any[] = [];
  for (const batch of batches) {
    const { txs, alts } = await fetchTxBatch(conn, batch);
    results.push(
      ...txs.map((tx) => parseTransaction(tx, alts)).filter((tx) => !!tx),
    );
  }

  console.log('Writing results');
  const dir = `scripts/benchmarking/tests/data/${startSlot}`;
  if (!existsSync(dir)) {
    mkdirSync(dir, { recursive: true });
  }
  writeFileSync(`${dir}/transactions.json`, JSON.stringify(results, null, 2));
  writeFileSync(
    `${dir}/initial.json`,
    JSON.stringify({ initialState }, null, 2),
  );
}

main();
