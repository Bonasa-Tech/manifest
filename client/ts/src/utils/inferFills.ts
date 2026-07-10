import bs58 from 'bs58';
import { PROGRAM_ID } from '../manifest';
import { FillLogResult } from '../types';

// The maker cannot be recovered from a transaction with truncated logs.
// Inferred fills carry an empty maker, which is how subscribers can tell a
// fill was inferred from token transfers rather than parsed from logs.
export const INFERRED_FILL_MAKER: string = '';

const TOKEN_PROGRAM_IDS: Set<string> = new Set([
  'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA',
  'TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb',
]);

const SWAP_INSTRUCTION_DISCRIMINATOR: number = 4;
const SWAP_V2_INSTRUCTION_DISCRIMINATOR: number = 13;
const TOKEN_INSTRUCTION_TRANSFER: number = 3;
const TOKEN_INSTRUCTION_TRANSFER_CHECKED: number = 12;

// Account order for the Swap and SwapV2 instructions, see
// programs/manifest/src/program/instruction.rs. SwapV2 adds a separate owner
// account after the payer (the owner of the trader token accounts, i.e. the
// taker), shifting the remaining accounts by one.
interface SwapAccountLayout {
  taker: number;
  market: number;
  traderBase: number;
  traderQuote: number;
  minAccounts: number;
}

const SWAP_LAYOUT: SwapAccountLayout = {
  taker: 0,
  market: 1,
  traderBase: 3,
  traderQuote: 4,
  minAccounts: 8,
};
const SWAP_V2_LAYOUT: SwapAccountLayout = {
  taker: 1,
  market: 2,
  traderBase: 4,
  traderQuote: 5,
  minAccounts: 9,
};

interface NormalizedInstruction {
  programId: string;
  accountKeys: string[];
  data: Uint8Array;
  stackHeight: number;
}

export interface InferFillsExtras {
  originalSigner?: string;
  aggregator?: string;
  originatingProtocol?: string;
  signers?: string[];
  blockTime?: number;
}

function toBase58(key: unknown): string {
  return typeof key === 'string'
    ? key
    : (key as { toBase58(): string }).toBase58();
}

function readU64LE(data: Uint8Array, offset: number): bigint {
  let value: bigint = 0n;
  for (let i = 7; i >= 0; i--) {
    value = (value << 8n) | BigInt(data[offset + i]);
  }
  return value;
}

/**
 * Resolve the full ordered account key list (static keys followed by keys
 * loaded from address lookup tables) as base58 strings.
 */
function resolveAccountKeys(tx: any): string[] {
  const message = tx.transaction.message;
  let keys: string[];
  if ('accountKeys' in message && message.accountKeys) {
    keys = message.accountKeys.map(toBase58);
  } else {
    keys = message.staticAccountKeys.map(toBase58);
  }
  const loadedAddresses = tx.meta?.loadedAddresses;
  if (loadedAddresses) {
    keys = keys.concat(
      (loadedAddresses.writable ?? []).map(toBase58),
      (loadedAddresses.readonly ?? []).map(toBase58),
    );
  }
  return keys;
}

/**
 * Normalize a top-level instruction. Legacy messages expose `instructions`
 * (base58 data, `accounts` indexes); v0 messages expose `compiledInstructions`
 * (Uint8Array data, `accountKeyIndexes`).
 */
function topLevelInstructions(
  tx: any,
  accountKeys: string[],
): NormalizedInstruction[] {
  const message = tx.transaction.message;
  const result: NormalizedInstruction[] = [];
  const rawInstructions: any[] =
    'instructions' in message && message.instructions
      ? message.instructions
      : message.compiledInstructions;
  for (const ix of rawInstructions) {
    const accountIndexes: number[] = ix.accounts ?? ix.accountKeyIndexes;
    const data: Uint8Array =
      typeof ix.data === 'string' ? bs58.decode(ix.data) : ix.data;
    result.push({
      programId: accountKeys[ix.programIdIndex],
      accountKeys: accountIndexes.map((i: number) => accountKeys[i]),
      data,
      stackHeight: 1,
    });
  }
  return result;
}

function normalizeInner(ix: any, accountKeys: string[]): NormalizedInstruction {
  return {
    programId: accountKeys[ix.programIdIndex],
    accountKeys: (ix.accounts as number[]).map((i: number) => accountKeys[i]),
    data: bs58.decode(ix.data as string),
    // Inner instructions are at least stack height 2. Old RPC responses may
    // omit stackHeight; default to 2 so direct CPIs are still attributed.
    stackHeight: ix.stackHeight ?? 2,
  };
}

interface TokenTransfer {
  source: string;
  destination: string;
  amount: bigint;
}

function parseTokenTransfer(
  ix: NormalizedInstruction,
): TokenTransfer | undefined {
  if (!TOKEN_PROGRAM_IDS.has(ix.programId) || ix.data.length < 9) {
    return undefined;
  }
  const instructionType: number = ix.data[0];
  if (
    instructionType === TOKEN_INSTRUCTION_TRANSFER &&
    ix.accountKeys.length >= 3
  ) {
    return {
      source: ix.accountKeys[0],
      destination: ix.accountKeys[1],
      amount: readU64LE(ix.data, 1),
    };
  }
  if (
    instructionType === TOKEN_INSTRUCTION_TRANSFER_CHECKED &&
    ix.accountKeys.length >= 4
  ) {
    return {
      source: ix.accountKeys[0],
      destination: ix.accountKeys[2],
      amount: readU64LE(ix.data, 1),
    };
  }
  return undefined;
}

interface SwapSite {
  instruction: NormalizedInstruction;
  layout: SwapAccountLayout;
  cpiInstructions: NormalizedInstruction[];
}

function swapAccountLayout(
  ix: NormalizedInstruction,
): SwapAccountLayout | undefined {
  if (ix.programId !== PROGRAM_ID.toBase58() || ix.data.length === 0) {
    return undefined;
  }
  const layout: SwapAccountLayout | undefined =
    ix.data[0] === SWAP_INSTRUCTION_DISCRIMINATOR
      ? SWAP_LAYOUT
      : ix.data[0] === SWAP_V2_INSTRUCTION_DISCRIMINATOR
        ? SWAP_V2_LAYOUT
        : undefined;
  return layout && ix.accountKeys.length >= layout.minAccounts
    ? layout
    : undefined;
}

/**
 * Find every Manifest Swap invocation (top-level or CPI) along with the
 * instructions it invoked, using inner-instruction stack heights.
 */
function findSwapSites(tx: any, accountKeys: string[]): SwapSite[] {
  const sites: SwapSite[] = [];
  const innerGroups: any[] = tx.meta?.innerInstructions ?? [];
  const innerByTopIndex: Map<number, NormalizedInstruction[]> = new Map();
  for (const group of innerGroups) {
    innerByTopIndex.set(
      group.index,
      group.instructions.map((ix: any) => normalizeInner(ix, accountKeys)),
    );
  }

  topLevelInstructions(tx, accountKeys).forEach(
    (ix: NormalizedInstruction, topIndex: number) => {
      const layout: SwapAccountLayout | undefined = swapAccountLayout(ix);
      if (layout) {
        sites.push({
          instruction: ix,
          layout,
          cpiInstructions: innerByTopIndex.get(topIndex) ?? [],
        });
      }
    },
  );

  for (const group of innerByTopIndex.values()) {
    for (let i = 0; i < group.length; i++) {
      const ix: NormalizedInstruction = group[i];
      const layout: SwapAccountLayout | undefined = swapAccountLayout(ix);
      if (!layout) {
        continue;
      }
      const cpiInstructions: NormalizedInstruction[] = [];
      for (let j = i + 1; j < group.length; j++) {
        if (group[j].stackHeight <= ix.stackHeight) {
          break;
        }
        cpiInstructions.push(group[j]);
      }
      sites.push({ instruction: ix, layout, cpiInstructions });
    }
  }
  return sites;
}

/**
 * Infer combined fills from a transaction whose logs were truncated, using
 * the Manifest Swap/SwapV2 CPIs into the token programs. Token movements
 * between the trader token accounts and the vaults give exact taker amounts
 * even when the FillLog Program data entries were dropped from the logs.
 *
 * The maker, sequence numbers, and isMakerGlobal cannot be recovered; the
 * result is one combined fill per swap instruction with an empty maker.
 */
export function inferFillsFromTransaction(
  tx: any,
  signature: string,
  slot: number,
  extras: InferFillsExtras = {},
): FillLogResult[] {
  const accountKeys: string[] = resolveAccountKeys(tx);
  const results: FillLogResult[] = [];

  for (const site of findSwapSites(tx, accountKeys)) {
    const swapAccounts: string[] = site.instruction.accountKeys;
    const taker: string = swapAccounts[site.layout.taker];
    const market: string = swapAccounts[site.layout.market];
    const traderBase: string = swapAccounts[site.layout.traderBase];
    const traderQuote: string = swapAccounts[site.layout.traderQuote];

    let basePaid: bigint = 0n;
    let baseReceived: bigint = 0n;
    let quotePaid: bigint = 0n;
    let quoteReceived: bigint = 0n;

    for (const cpi of site.cpiInstructions) {
      const transfer: TokenTransfer | undefined = parseTokenTransfer(cpi);
      if (!transfer) {
        continue;
      }
      if (transfer.source === traderBase) {
        basePaid += transfer.amount;
      }
      if (transfer.destination === traderBase) {
        baseReceived += transfer.amount;
      }
      if (transfer.source === traderQuote) {
        quotePaid += transfer.amount;
      }
      if (transfer.destination === traderQuote) {
        quoteReceived += transfer.amount;
      }
    }

    let takerIsBuy: boolean;
    let baseAtoms: bigint;
    let quoteAtoms: bigint;
    if (baseReceived > 0n && quotePaid > 0n) {
      takerIsBuy = true;
      baseAtoms = baseReceived;
      quoteAtoms = quotePaid;
    } else if (basePaid > 0n && quoteReceived > 0n) {
      takerIsBuy = false;
      baseAtoms = basePaid;
      quoteAtoms = quoteReceived;
    } else {
      // Swap instruction with no vault movement means no fills happened.
      continue;
    }

    const result: FillLogResult = {
      market,
      maker: INFERRED_FILL_MAKER,
      taker,
      baseAtoms: baseAtoms.toString(),
      quoteAtoms: quoteAtoms.toString(),
      priceAtoms: Number(quoteAtoms) / Number(baseAtoms),
      takerIsBuy,
      isMakerGlobal: false,
      makerSequenceNumber: '0',
      takerSequenceNumber: '0',
      signature,
      slot,
    };
    if (extras.originalSigner) {
      result.originalSigner = extras.originalSigner;
    }
    if (extras.aggregator) {
      result.aggregator = extras.aggregator;
    }
    if (extras.originatingProtocol) {
      result.originatingProtocol = extras.originatingProtocol;
    }
    if (extras.signers && extras.signers.length > 0) {
      result.signers = extras.signers;
    }
    if (extras.blockTime !== undefined) {
      result.blockTime = extras.blockTime;
    }
    results.push(result);
  }

  return results;
}

/**
 * Subtract fills that survived truncation from the inferred combined fills so
 * that emitting both does not double count. Parsed fills are matched to an
 * inferred fill by market and taker; whatever taker amounts they do not
 * account for is returned as a remainder inferred fill.
 */
export function computeInferredRemainders(
  inferred: FillLogResult[],
  parsed: FillLogResult[],
): FillLogResult[] {
  const remainders: FillLogResult[] = [];
  for (const fill of inferred) {
    let baseAtoms: bigint = BigInt(fill.baseAtoms);
    let quoteAtoms: bigint = BigInt(fill.quoteAtoms);
    for (const parsedFill of parsed) {
      if (
        parsedFill.market === fill.market &&
        parsedFill.taker === fill.taker &&
        parsedFill.takerIsBuy === fill.takerIsBuy
      ) {
        baseAtoms -= BigInt(parsedFill.baseAtoms);
        quoteAtoms -= BigInt(parsedFill.quoteAtoms);
      }
    }
    if (baseAtoms > 0n && quoteAtoms > 0n) {
      remainders.push({
        ...fill,
        baseAtoms: baseAtoms.toString(),
        quoteAtoms: quoteAtoms.toString(),
        priceAtoms: Number(quoteAtoms) / Number(baseAtoms),
      });
    }
  }
  return remainders;
}
