// Core-program collections for the fee collector: market tail tightening and
// global order cleaning. Companion to collect-wrapper-fees.ts, which owns the
// wrapper-side Collect/Defrag; this module owns everything that touches the
// core market and global accounts.
//
// Two distinct sources of value, which are worth keeping separate because
// their economics differ:
//
//   Accrued excess lamports. Markets accumulate the advanced-order fee, and a
//   sweep moves it to the collector. Measured on mainnet at ~0.03 SOL/hour
//   program-wide (2026-10-22, four samples over 1.5h from a zeroed baseline).
//   This accrues whether or not anyone sweeps, so sweeping more often does not
//   earn more - it only shortens the lag.
//
//   Rent from truncating a market's free tail. This one DOES scale with
//   frequency, but measurements show 69 of 80 truncated markets regrew within
//   two hours and traders re-paid 87% of the rent taken: batch_update's
//   replenish path expands at the TRADER's expense. So tail tightening on an
//   active market is largely a transfer from traders to the collector rather
//   than recovered waste. It is unambiguously good on abandoned markets, where
//   the space never regrows. `--tail-tighten` is therefore opt-in and
//   separate, and `--tail-skip-active` exists to confine it to the honest case.
//
// Everything here is sweep-only (both defrag budgets zero), which relocates
// nothing: verified on mainnet to leave every resting-order and claimed-seat
// index byte-identical. Relocation is deliberately NOT exposed - it broke two
// third-party integrators on 2026-10-22 (one silently stopped quoting, one
// panicked at 737 failures per 1000 tx) and needs integrator coordination, not
// a flag.
import { AccountRole, address, type Instruction } from '@solana/kit';
import { PublicKey } from '@solana/web3.js';

export const MANIFEST_PROGRAM_ID = new PublicKey(
  'MNFSTqtC93rEfYHB6hF82sKdZpUDFWkViLByLd1k1Ms',
);
const SYSTEM_PROGRAM = '11111111111111111111111111111111';

// Instruction tags from programs/manifest/src/program/instruction.rs.
const GLOBAL_CLEAN_DISCRIMINANT = 12;
const DEFRAG_DISCRIMINANT = 14;

// Market account layout. MarketFixed is 256 bytes; the dynamic area is a
// sequence of 80-byte blocks, each a 16-byte node header followed by a 64-byte
// payload, with the payload type at byte 13 of the header.
export const MARKET_FIXED_SIZE = 256;
export const MARKET_BLOCK_SIZE = 80;
const NODE_PAYLOAD_TYPE_OFFSET = 13;
const NODE_PAYLOAD_OFFSET = 16;
const NODE_TYPE_CLAIMED_SEAT = 1;
const NODE_TYPE_RESTING_ORDER = 2;
const BASE_MINT_OFFSET = 16;
const QUOTE_MINT_OFFSET = 48;
const BASE_VAULT_OFFSET = 80;
const QUOTE_VAULT_OFFSET = 112;
const NUM_BYTES_ALLOCATED_OFFSET = 152;
// RestingOrder payload: price(16) atoms(8) seq(8) trader_index(4)
// last_valid_slot(4) is_bid(1) order_type(1) spread(2) padding(20).
const ORDER_SEQUENCE_OFFSET = 24;
const ORDER_TRADER_INDEX_OFFSET = 32;
const ORDER_LAST_VALID_SLOT_OFFSET = 36;
const ORDER_IS_BID_OFFSET = 40;
const ORDER_TYPE_OFFSET = 41;
// OrderType::Global. Only global orders can be unbacked (their collateral
// lives in a shared global account that can be withdrawn out from under them).
const ORDER_TYPE_GLOBAL = 3;
// NO_EXPIRATION_LAST_VALID_SLOT; an order with this value never expires.
const NO_EXPIRATION = 0;
// ClaimedSeat.trader is the first field of the payload, so the maker pubkey of
// an order sits at its trader_index block + the node header.
const NODE_TYPE_RESTING_ORDER_SEAT_TRADER = NODE_PAYLOAD_OFFSET;

// Rent on mainnet. Defrag leaves `keep_count + 2` blocks, never fewer: real
// allocation paths depend on two spare nodes.
const LAMPORTS_PER_BYTE_YEAR_EXEMPT = 5080;
const ACCOUNT_OVERHEAD_BYTES = 128;
const DEFRAG_SPARE_BLOCKS = 2;
// GAS_DEPOSIT_LAMPORTS: what cleaning one order pays the caller.
export const GAS_DEPOSIT_LAMPORTS = 5_000;
// One signature. A single clean exactly cancels it out, so a clean-only
// transaction must carry at least two to be worth sending.
export const SIGNATURE_FEE_LAMPORTS = 5_000;

export type MarketAccount = {
  pubkey: string;
  lamports: bigint;
  data: Buffer;
};

export type MarketPlan = {
  pubkey: string;
  space: number;
  lamports: bigint;
  baseMint: string;
  quoteMint: string;
  baseVault: string;
  quoteVault: string;
  orders: number;
  seats: number;
  /** Blocks above the highest live block: the only ones a sweep can truncate. */
  freeTailBlocks: number;
  /** Free blocks between live ones. Only relocation could gather these. */
  interiorHoles: number;
  /** Account size after a sweep, honouring the keep_count + 2 floor. */
  postSpace: number;
  /** Lamports above this market's current rent minimum. */
  excessLamports: bigint;
  /** Rent released by truncating the free tail. */
  tailRentLamports: bigint;
  /** excess + tail rent. Negative is impossible here: see expandsOnDefrag. */
  recoveredLamports: bigint;
  /**
   * The processor tops a market up to two free blocks BEFORE compacting
   * (free_blocks_short_of_n(2) -> batch_expand_market), funded by the
   * collector. On a market with no reclaimable tail that is a pure loss of
   * ~406,400 lamports, so these are never selected.
   */
  expandsOnDefrag: boolean;
};

export type CleanableOrder = {
  market: string;
  orderIndex: number;
  sequenceNumber: bigint;
  /** The global account whose mint matches this order's side. */
  global: string;
  lastValidSlot: number;
  /** The order owner, resolved from the ClaimedSeat at its trader_index. */
  maker: string;
  /**
   * 'expired' is eligible on its own. 'unbacked' is only a CANDIDATE here: a
   * global order that isn't expired might or might not be unbacked, and that
   * depends on the maker's live global balance. It is confirmed by simulating
   * the clean (the program returns InvalidClean for a still-backed order), so
   * the simulation, not this script, is the source of truth.
   */
  kind: 'expired' | 'unbacked';
};

const rentMinimum = (space: number): bigint =>
  BigInt((space + ACCOUNT_OVERHEAD_BYTES) * LAMPORTS_PER_BYTE_YEAR_EXEMPT);

/**
 * Read a market's shape straight from its bytes. Deliberately not a
 * simulateTransaction per market: that is one RPC round trip each, roughly ten
 * minutes over four thousand markets, and the same numbers are derivable
 * locally. Batches are still simulated before they are sent, which is what
 * actually guards correctness.
 */
export function planMarket(account: MarketAccount): MarketPlan | null {
  const { data, lamports, pubkey } = account;
  if (data.length < MARKET_FIXED_SIZE) return null;
  const allocated = data.readUInt32LE(NUM_BYTES_ALLOCATED_OFFSET);
  const blocks = Math.floor(allocated / MARKET_BLOCK_SIZE);

  let orders = 0;
  let seats = 0;
  let frontier = 0;
  for (let block = 0; block < blocks; block += 1) {
    const at = MARKET_FIXED_SIZE + block * MARKET_BLOCK_SIZE;
    if (at + MARKET_BLOCK_SIZE > data.length) break;
    const kind = data[at + NODE_PAYLOAD_TYPE_OFFSET];
    if (kind === NODE_TYPE_RESTING_ORDER) orders += 1;
    else if (kind === NODE_TYPE_CLAIMED_SEAT) seats += 1;
    else continue;
    frontier = block + 1;
  }
  const live = orders + seats;
  const freeTailBlocks = blocks - frontier;
  const interiorHoles = blocks - live - freeTailBlocks;

  // Mirrors `frontier.max(keep_count + 2)` in market_defrag.rs. With interior
  // holes the frontier dominates and the tail is all that can go; with none,
  // the floor binds and the market lands on exactly two free blocks.
  const newBlocks = Math.max(frontier, live + DEFRAG_SPARE_BLOCKS);
  const postSpace = MARKET_FIXED_SIZE + newBlocks * MARKET_BLOCK_SIZE;
  const excessLamports = lamports - rentMinimum(data.length);
  const tailRentLamports = BigInt(
    Math.max(0, data.length - postSpace) * LAMPORTS_PER_BYTE_YEAR_EXEMPT,
  );

  return {
    pubkey,
    space: data.length,
    lamports,
    baseMint: new PublicKey(
      data.subarray(BASE_MINT_OFFSET, BASE_MINT_OFFSET + 32),
    ).toBase58(),
    quoteMint: new PublicKey(
      data.subarray(QUOTE_MINT_OFFSET, QUOTE_MINT_OFFSET + 32),
    ).toBase58(),
    baseVault: new PublicKey(
      data.subarray(BASE_VAULT_OFFSET, BASE_VAULT_OFFSET + 32),
    ).toBase58(),
    quoteVault: new PublicKey(
      data.subarray(QUOTE_VAULT_OFFSET, QUOTE_VAULT_OFFSET + 32),
    ).toBase58(),
    orders,
    seats,
    freeTailBlocks,
    interiorHoles,
    postSpace,
    excessLamports: excessLamports > 0n ? excessLamports : 0n,
    tailRentLamports,
    recoveredLamports:
      (excessLamports > 0n ? excessLamports : 0n) + tailRentLamports,
    expandsOnDefrag: blocks - live < DEFRAG_SPARE_BLOCKS,
  };
}

/**
 * Expired resting orders, which GlobalClean can remove for the gas deposit.
 *
 * Only expiry is detected here. An unbacked global order is also eligible, but
 * establishing that needs the maker's balance from the global account, and
 * getting it wrong wastes a transaction on an Ineligible clean. Expiry is
 * decidable from the market bytes alone.
 *
 * `globalForMint` must contain only global accounts that exist: GlobalClean
 * requires one whose mint matches the order's side (quote for a bid, base for
 * an ask), so an order on a market with no such global cannot be cleaned.
 */
/** The order owner pubkey, read from the ClaimedSeat at trader_index. */
function readMaker(data: Buffer, traderIndex: number): string | null {
  const seat =
    MARKET_FIXED_SIZE + traderIndex + NODE_TYPE_RESTING_ORDER_SEAT_TRADER;
  if (traderIndex < 0 || seat + 32 > data.length) return null;
  return new PublicKey(data.subarray(seat, seat + 32)).toBase58();
}

/**
 * Orders GlobalClean can remove.
 *
 * Expired orders are eligible outright. With `includeUnbacked`, non-expired
 * GLOBAL orders are returned too as candidates (kind 'unbacked'): whether they
 * are actually cleanable depends on the maker's live global balance, which the
 * caller confirms by simulating the clean. Non-global, non-expired orders are
 * never cleanable and are skipped.
 *
 * `globalForMint` must contain only global accounts that exist: GlobalClean
 * needs one whose mint matches the order's side (quote for a bid, base for an
 * ask), so an order on a market with no such global cannot be cleaned.
 */
export function findCleanableOrders(
  plan: MarketPlan,
  data: Buffer,
  nowSlot: number,
  globalForMint: Map<string, string>,
  includeUnbacked = false,
): CleanableOrder[] {
  const out: CleanableOrder[] = [];
  const allocated = data.readUInt32LE(NUM_BYTES_ALLOCATED_OFFSET);
  const blocks = Math.floor(allocated / MARKET_BLOCK_SIZE);
  for (let block = 0; block < blocks; block += 1) {
    const at = MARKET_FIXED_SIZE + block * MARKET_BLOCK_SIZE;
    if (at + MARKET_BLOCK_SIZE > data.length) break;
    if (data[at + NODE_PAYLOAD_TYPE_OFFSET] !== NODE_TYPE_RESTING_ORDER)
      continue;
    const payload = at + NODE_PAYLOAD_OFFSET;
    const lastValidSlot = data.readUInt32LE(
      payload + ORDER_LAST_VALID_SLOT_OFFSET,
    );
    const expired = lastValidSlot !== NO_EXPIRATION && lastValidSlot < nowSlot;
    const isGlobal = data[payload + ORDER_TYPE_OFFSET] === ORDER_TYPE_GLOBAL;
    // Expired is eligible as-is; a non-expired global order is an unbacked
    // candidate. Everything else cannot be cleaned.
    let kind: CleanableOrder['kind'] | null = null;
    if (expired) kind = 'expired';
    else if (includeUnbacked && isGlobal) kind = 'unbacked';
    if (kind === null) continue;

    const isBid = data[payload + ORDER_IS_BID_OFFSET] === 1;
    const global = globalForMint.get(isBid ? plan.quoteMint : plan.baseMint);
    if (global === undefined) continue;
    const maker = readMaker(
      data,
      data.readUInt32LE(payload + ORDER_TRADER_INDEX_OFFSET),
    );
    if (maker === null) continue;
    out.push({
      market: plan.pubkey,
      // The instruction takes a byte offset into the dynamic area, not a
      // block number.
      orderIndex: block * MARKET_BLOCK_SIZE,
      sequenceNumber: data.readBigUInt64LE(payload + ORDER_SEQUENCE_OFFSET),
      global,
      lastValidSlot,
      maker,
      kind,
    });
  }
  return out;
}

/** Sweep-only Defrag: relocate nothing, reclaim no seat, collect and truncate. */
export function createMarketSweepInstruction(
  plan: MarketPlan,
  collector: PublicKey,
  baseTokenProgram: string,
  quoteTokenProgram: string,
): Instruction {
  // [tag, relocations u32 = 0, seats u32 = 0]. The two zero budgets are what
  // make this inert: no block moves, no seat is reclaimed.
  const data = new Uint8Array(9);
  data[0] = DEFRAG_DISCRIMINANT;
  return {
    accounts: [
      {
        address: address(collector.toBase58()),
        role: AccountRole.WRITABLE_SIGNER,
      },
      { address: address(plan.pubkey), role: AccountRole.WRITABLE },
      { address: address(plan.baseVault), role: AccountRole.WRITABLE },
      { address: address(plan.quoteVault), role: AccountRole.WRITABLE },
      { address: address(baseTokenProgram), role: AccountRole.READONLY },
      { address: address(quoteTokenProgram), role: AccountRole.READONLY },
      { address: address(SYSTEM_PROGRAM), role: AccountRole.READONLY },
    ],
    programAddress: address(MANIFEST_PROGRAM_ID.toBase58()),
    data,
  };
}

export function createGlobalCleanInstruction(
  order: CleanableOrder,
  collector: PublicKey,
): Instruction {
  // [tag, order_index u32] - GlobalCleanParams is a single borsh u32.
  const data = new Uint8Array(5);
  data[0] = GLOBAL_CLEAN_DISCRIMINANT;
  new DataView(data.buffer).setUint32(1, order.orderIndex, true);
  // Account order must match GlobalCleanContext::load exactly:
  // payer, market, system_program, global. Passing global and system in the
  // wrong order makes the loader validate the global account as the system
  // program and reject it with IncorrectProgramId.
  return {
    accounts: [
      {
        address: address(collector.toBase58()),
        role: AccountRole.WRITABLE_SIGNER,
      },
      { address: address(order.market), role: AccountRole.WRITABLE },
      { address: address(SYSTEM_PROGRAM), role: AccountRole.READONLY },
      { address: address(order.global), role: AccountRole.WRITABLE },
    ],
    programAddress: address(MANIFEST_PROGRAM_ID.toBase58()),
    data,
  };
}

/**
 * Which markets are worth sweeping.
 *
 * `minShrinkBlocks` targets tail tightening specifically: at 0 every
 * profitable market is taken (including fee-only ones whose size will not
 * change), and above 0 only markets whose tail actually truncates by that many
 * blocks. Markets the processor would expand are always excluded - they cost
 * the collector rent and return nothing.
 */
export function selectMarkets(
  plans: MarketPlan[],
  options: { minRecoveryLamports: bigint; minShrinkBlocks: number },
): MarketPlan[] {
  const { minRecoveryLamports, minShrinkBlocks } = options;
  return plans
    .filter((plan) => !plan.expandsOnDefrag)
    .filter((plan) => plan.recoveredLamports >= minRecoveryLamports)
    .filter((plan) => {
      if (minShrinkBlocks <= 0) return true;
      return (
        (plan.space - plan.postSpace) / MARKET_BLOCK_SIZE >= minShrinkBlocks
      );
    })
    .sort((a, b) => Number(b.recoveredLamports - a.recoveredLamports));
}

/**
 * Whether a batch of cleans pays for the transaction that carries it.
 *
 * Each clean returns one gas deposit and the transaction costs one signature,
 * so a single clean nets zero and two is the minimum that earns anything. When
 * cleans ride along with market sweeps the signature is already paid for by
 * the sweep, so the bar is just "more than nothing".
 */
export function cleanBatchIsProfitable(
  cleanCount: number,
  ridesWithOtherWork: boolean,
): boolean {
  if (cleanCount <= 0) return false;
  const revenue = BigInt(cleanCount) * BigInt(GAS_DEPOSIT_LAMPORTS);
  const cost = ridesWithOtherWork ? 0n : BigInt(SIGNATURE_FEE_LAMPORTS);
  return revenue > cost;
}

// ---------------------------------------------------------------------------
// Discovery and execution
// ---------------------------------------------------------------------------

export const MARKET_FIXED_DISCRIMINANT = 4859840929024028656n;
export const GLOBAL_FIXED_DISCRIMINANT = 10787423733276977665n;
const GLOBAL_MINT_OFFSET = 8;

/** Little-endian u64 discriminant as the base58 bytes a memcmp filter needs. */
export function discriminantBase58(
  value: bigint,
  bs58encode: (b: Buffer) => string,
): string {
  const buffer = Buffer.alloc(8);
  buffer.writeBigUInt64LE(value);
  return bs58encode(buffer);
}

/**
 * Mint -> global account address, for every global account that exists.
 *
 * GlobalClean needs a global whose mint matches the order's side, so an order
 * whose mint has no global account is simply not cleanable. Reading the set
 * that exists is cheaper and less error-prone than deriving PDAs and probing.
 */
export async function fetchGlobalAccounts(
  rpc: <T>(method: string, params: unknown[]) => Promise<T>,
  bs58encode: (b: Buffer) => string,
): Promise<Map<string, string>> {
  const accounts = await rpc<
    { pubkey: string; account: { data: [string, 'base64'] } }[]
  >('getProgramAccounts', [
    MANIFEST_PROGRAM_ID.toBase58(),
    {
      commitment: 'confirmed',
      encoding: 'base64',
      dataSlice: { offset: 0, length: GLOBAL_MINT_OFFSET + 32 },
      filters: [
        {
          memcmp: {
            offset: 0,
            bytes: discriminantBase58(GLOBAL_FIXED_DISCRIMINANT, bs58encode),
          },
        },
      ],
    },
  ]);
  const byMint = new Map<string, string>();
  for (const { pubkey, account } of accounts) {
    const data = Buffer.from(account.data[0], 'base64');
    if (data.length < GLOBAL_MINT_OFFSET + 32) continue;
    const mint = new PublicKey(
      data.subarray(GLOBAL_MINT_OFFSET, GLOBAL_MINT_OFFSET + 32),
    ).toBase58();
    byMint.set(mint, pubkey);
  }
  return byMint;
}

/** Every market account, with full data: the shape read needs every block. */
export async function fetchMarketAccounts(
  rpc: <T>(method: string, params: unknown[]) => Promise<T>,
  bs58encode: (b: Buffer) => string,
): Promise<MarketAccount[]> {
  const accounts = await rpc<
    {
      pubkey: string;
      account: { data: [string, 'base64']; lamports: number };
    }[]
  >('getProgramAccounts', [
    MANIFEST_PROGRAM_ID.toBase58(),
    {
      commitment: 'confirmed',
      encoding: 'base64',
      filters: [
        {
          memcmp: {
            offset: 0,
            bytes: discriminantBase58(MARKET_FIXED_DISCRIMINANT, bs58encode),
          },
        },
      ],
    },
  ]);
  return accounts.map(({ pubkey, account }) => ({
    pubkey,
    lamports: BigInt(account.lamports),
    data: Buffer.from(account.data[0], 'base64'),
  }));
}

/**
 * Token programs come from the mint owners, which a sweep instruction must
 * name. Cached per mint so a few thousand markets cost a handful of calls.
 */
export async function resolveTokenPrograms(
  rpc: <T>(method: string, params: unknown[]) => Promise<T>,
  mints: string[],
): Promise<Map<string, string>> {
  const owners = new Map<string, string>();
  const unique = [...new Set(mints)];
  for (let i = 0; i < unique.length; i += 100) {
    const chunk = unique.slice(i, i + 100);
    const response = await rpc<{
      value: ({ owner: string } | null)[];
    }>('getMultipleAccounts', [
      chunk,
      {
        commitment: 'confirmed',
        encoding: 'base64',
        dataSlice: { offset: 0, length: 0 },
      },
    ]);
    chunk.forEach((mint, index) => {
      const owner = response.value[index]?.owner;
      if (owner !== undefined) owners.set(mint, owner);
    });
  }
  return owners;
}

// ---------------------------------------------------------------------------
// Batching and submission
// ---------------------------------------------------------------------------

// v1 transaction limits. For market sweeps the address count binds first:
// three addresses per market (market + two vaults) against 64 total means 19
// legs, measured, at roughly 2.5KB of the 4KB size budget. Cleans are cheaper
// at two addresses each (market + global).
const V1_MAX_SIZE = 4096;
export const MAX_SWEEPS_PER_TX = 19;
export const MAX_CLEANS_PER_TX = 24;
const MAX_COMPUTE_UNITS = 1_400_000;
// Mandatory. Without it a multi-leg batch fails MaxLoadedAccountsDataSizeExceeded
// having consumed zero compute units, which looks like a program bug.
const MAX_LOADED_ACCOUNTS_DATA_SIZE = 64 * 1024 * 1024;
const SWEEP_CU_BASE = 3_000;
const SWEEP_CU_PER_BYTE = 2.5;

export const sweepComputeUnits = (plan: MarketPlan): number =>
  Math.ceil(SWEEP_CU_BASE + SWEEP_CU_PER_BYTE * plan.space);

/**
 * Greedily pack instructions into transactions, bounded by a leg cap and by
 * the real serialized size of the candidate. Size is measured rather than
 * estimated because a transaction that exceeds the limit is rejected outright.
 */
export function packInstructions<T>(
  items: T[],
  toInstruction: (item: T) => Instruction,
  maxLegs: number,
  measure: (instructions: Instruction[]) => number | null,
): T[][] {
  const batches: T[][] = [];
  let current: T[] = [];
  for (const item of items) {
    if (current.length === 0) {
      current = [item];
      continue;
    }
    const candidate = [...current, item];
    const size =
      candidate.length <= maxLegs
        ? measure(candidate.map(toInstruction))
        : null;
    if (size !== null && size <= V1_MAX_SIZE) {
      current = candidate;
    } else {
      batches.push(current);
      current = [item];
    }
  }
  if (current.length > 0) batches.push(current);
  return batches;
}

export type CoreCollectionOptions = {
  execute: boolean;
  /** Sweep markets for accrued fees (and any already-free tail). */
  sweepMarkets: boolean;
  /**
   * Require the sweep to truncate at least this many blocks, i.e. target tail
   * tightening specifically rather than fee collection. See the note at the
   * top of this file: on active markets that rent is largely re-paid by
   * traders, so this is opt-in.
   */
  minShrinkBlocks: number;
  /** Also clean expired orders, batched, and only when they pay for themselves. */
  globalClean: boolean;
  /**
   * In addition to expired orders, clean unbacked global orders. Each candidate
   * is confirmed by simulation before it is included, so a still-backed order is
   * never submitted.
   */
  cleanUnbacked: boolean;
  /** Maker pubkeys whose orders must never be cleaned. */
  skipTraders: Set<string>;
  minRecoveryLamports: bigint;
  maxMarkets?: number;
  delayMs: number;
  shouldStop: () => boolean;
};

export type CoreCollectionResult = {
  marketsPlanned: number;
  marketsSelected: number;
  cleanableOrders: number;
  transactionsSent: number;
  recoveredLamports: bigint;
  cleanRewardLamports: bigint;
  signatures: string[];
};

/**
 * One core-side collection pass. Read-only unless `execute` is set.
 *
 * Ordering matters: cleans go first in a transaction that also sweeps, so the
 * gas deposits land before the sweep measures the market's excess. A clean
 * credits the collector directly rather than the market, so this is only about
 * not leaving a freed block behind for the next pass.
 */
export async function runCoreCollections(
  deps: {
    rpc: <T>(method: string, params: unknown[]) => Promise<T>;
    bs58encode: (b: Buffer) => string;
    collector: PublicKey;
    buildAndSend: (
      instructions: Instruction[],
      computeUnits: number,
      loadedAccountsDataSize: number,
    ) => Promise<string>;
    simulate: (
      instructions: Instruction[],
      computeUnits: number,
      loadedAccountsDataSize: number,
    ) => Promise<{ err: unknown; unitsConsumed?: number }>;
    measure: (instructions: Instruction[]) => number | null;
    sleep: (ms: number) => Promise<void>;
  },
  options: CoreCollectionOptions,
): Promise<CoreCollectionResult> {
  const { rpc, bs58encode, collector } = deps;
  const result: CoreCollectionResult = {
    marketsPlanned: 0,
    marketsSelected: 0,
    cleanableOrders: 0,
    transactionsSent: 0,
    recoveredLamports: 0n,
    cleanRewardLamports: 0n,
    signatures: [],
  };

  console.log('Fetching core market accounts...');
  const accounts = await fetchMarketAccounts(rpc, bs58encode);
  const plans = accounts
    .map(planMarket)
    .filter((plan): plan is MarketPlan => plan !== null);
  result.marketsPlanned = plans.length;
  const dataByMarket = new Map(accounts.map((a) => [a.pubkey, a.data]));

  const expanders = plans.filter((plan) => plan.expandsOnDefrag).length;
  console.log(
    `${plans.length} markets read; ${expanders} excluded because defrag would expand them at the collector's expense`,
  );

  let selected = options.sweepMarkets
    ? selectMarkets(plans, {
        minRecoveryLamports: options.minRecoveryLamports,
        minShrinkBlocks: options.minShrinkBlocks,
      })
    : [];
  if (options.maxMarkets !== undefined)
    selected = selected.slice(0, options.maxMarkets);
  result.marketsSelected = selected.length;
  const sweepTotal = selected.reduce(
    (sum, plan) => sum + plan.recoveredLamports,
    0n,
  );
  const tailTotal = selected.reduce(
    (sum, plan) => sum + plan.tailRentLamports,
    0n,
  );
  if (options.sweepMarkets) {
    console.log(
      `Sweeping ${selected.length} markets: ${Number(sweepTotal) / 1e9} SOL ` +
        `(${Number(sweepTotal - tailTotal) / 1e9} accrued fees, ${Number(tailTotal) / 1e9} tail rent` +
        `${options.minShrinkBlocks > 0 ? `, min-shrink ${options.minShrinkBlocks} blocks` : ''})`,
    );
  }

  let cleans: CleanableOrder[] = [];
  if (options.globalClean) {
    const globals = await fetchGlobalAccounts(rpc, bs58encode);
    const slot = await rpc<number>('getSlot', [{ commitment: 'confirmed' }]);
    // Scan EVERY market, not just the ones selected for sweeping. Whether a
    // market is worth sweeping says nothing about whether it holds expired
    // orders, and restricting the search to the selection lost 178 of 179
    // cleanable orders as soon as --min-shrink narrowed it.
    const candidates: CleanableOrder[] = [];
    for (const plan of plans) {
      const data = dataByMarket.get(plan.pubkey);
      if (data === undefined) continue;
      candidates.push(
        ...findCleanableOrders(
          plan,
          data,
          slot,
          globals,
          options.cleanUnbacked,
        ),
      );
    }

    // Never touch a whitelisted maker's orders.
    const beforeSkip = candidates.length;
    let kept = candidates.filter((c) => !options.skipTraders.has(c.maker));
    const skipped = beforeSkip - kept.length;

    // Expired orders are eligible outright. Unbacked candidates are only
    // eligible if the maker's global balance has fallen short, which the
    // program alone can judge: simulate each one and keep only those the
    // program would accept. One dummy blockhash serves every sim
    // (replaceRecentBlockhash), so this is N cheap simulateTransaction calls.
    const expired = kept.filter((c) => c.kind === 'expired');
    const unbackedCandidates = kept.filter((c) => c.kind === 'unbacked');
    const unbacked: CleanableOrder[] = [];
    if (unbackedCandidates.length > 0) {
      console.log(
        `confirming ${unbackedCandidates.length} unbacked global-order candidates by simulation...`,
      );
      for (const c of unbackedCandidates) {
        if (options.shouldStop()) break;
        const sim = await deps.simulate(
          [createGlobalCleanInstruction(c, collector)],
          40_000,
          MAX_LOADED_ACCOUNTS_DATA_SIZE,
        );
        if (sim.err === null || sim.err === undefined) unbacked.push(c);
      }
    }
    kept = [...expired, ...unbacked];

    const byMaker = new Map<string, { expired: number; unbacked: number }>();
    for (const c of kept) {
      const m = byMaker.get(c.maker) ?? { expired: 0, unbacked: 0 };
      m[c.kind] += 1;
      byMaker.set(c.maker, m);
    }
    result.cleanableOrders = kept.length;
    console.log(
      `${globals.size} global accounts; ${expired.length} expired + ${unbacked.length} unbacked ` +
        `= ${kept.length} cleanable (${Number(BigInt(kept.length) * BigInt(GAS_DEPOSIT_LAMPORTS)) / 1e9} SOL of gas deposits)` +
        (skipped > 0 ? `; ${skipped} skipped by trader whitelist` : ''),
    );
    // Preview: which traders have orders to clean (the unbacked ones especially).
    const unbackedByMaker = [...byMaker.entries()]
      .filter(([, v]) => v.unbacked > 0)
      .sort((a, b) => b[1].unbacked - a[1].unbacked);
    if (unbackedByMaker.length > 0) {
      console.log('  traders with unbacked orders:');
      for (const [maker, v] of unbackedByMaker) {
        console.log(
          `    ${maker}  ${v.unbacked} unbacked${v.expired ? `, ${v.expired} expired` : ''}`,
        );
      }
    }

    cleans = kept;
    if (!cleanBatchIsProfitable(cleans.length, options.sweepMarkets)) {
      console.log(
        '  skipping global clean: the deposits would not cover the signature it needs',
      );
      cleans = [];
    }
  }

  const tokenPrograms = await resolveTokenPrograms(
    rpc,
    selected.flatMap((plan) => [plan.baseMint, plan.quoteMint]),
  );
  const sweepable = selected.filter(
    (plan) =>
      tokenPrograms.has(plan.baseMint) && tokenPrograms.has(plan.quoteMint),
  );
  if (sweepable.length < selected.length) {
    console.warn(
      `Excluding ${selected.length - sweepable.length} markets whose mint owner could not be read`,
    );
  }

  const sweepInstruction = (plan: MarketPlan): Instruction =>
    createMarketSweepInstruction(
      plan,
      collector,
      tokenPrograms.get(plan.baseMint)!,
      tokenPrograms.get(plan.quoteMint)!,
    );
  const cleanInstruction = (order: CleanableOrder): Instruction =>
    createGlobalCleanInstruction(order, collector);

  const sweepBatches = packInstructions(
    sweepable,
    sweepInstruction,
    MAX_SWEEPS_PER_TX,
    deps.measure,
  );
  const cleanBatches = packInstructions(
    cleans,
    cleanInstruction,
    MAX_CLEANS_PER_TX,
    deps.measure,
  );
  console.log(
    `Plan: ${sweepBatches.length} sweep transactions, ${cleanBatches.length} clean transactions`,
  );

  type Work = {
    instructions: Instruction[];
    units: number;
    label: string;
    recovered: bigint;
    reward: bigint;
    /**
     * Whether a simulation failure should abort the run. Cleans are
     * opportunistic: an expired order can be cancelled by anyone between the
     * read and the send, which invalidates its index and fails the batch
     * through no fault of ours. On an hourly schedule that must not take the
     * sweep down with it. A sweep failing is a real problem and still stops.
     */
    optional: boolean;
  };
  const work: Work[] = [
    // Cleans first: cheap, and they free blocks a later sweep can truncate.
    ...cleanBatches.map((batch) => ({
      instructions: batch.map(cleanInstruction),
      units: Math.min(MAX_COMPUTE_UNITS, 40_000 * batch.length + 10_000),
      label: `clean ${batch.length} expired orders`,
      recovered: 0n,
      reward: BigInt(batch.length) * BigInt(GAS_DEPOSIT_LAMPORTS),
      optional: true,
    })),
    ...sweepBatches.map((batch) => ({
      instructions: batch.map(sweepInstruction),
      units: Math.min(
        MAX_COMPUTE_UNITS,
        batch.reduce((sum, plan) => sum + sweepComputeUnits(plan), 0),
      ),
      label: `sweep ${batch.length} markets`,
      recovered: batch.reduce((sum, plan) => sum + plan.recoveredLamports, 0n),
      reward: 0n,
      optional: false,
    })),
  ];

  if (!options.execute) {
    console.log(
      `Dry run: ${work.length} transactions would be sent. Add --execute to submit.`,
    );
    return result;
  }

  for (const [index, item] of work.entries()) {
    if (options.shouldStop()) {
      console.warn('Stopping before the next transaction.');
      break;
    }
    const simulation = await deps.simulate(
      item.instructions,
      item.units,
      MAX_LOADED_ACCOUNTS_DATA_SIZE,
    );
    if (simulation.err !== null && simulation.err !== undefined) {
      const detail = `${item.label}: ${JSON.stringify(simulation.err)}`;
      if (!item.optional) throw new Error(`Simulation failed for ${detail}`);
      // A clean batch can fail for a transient reason (an order was cancelled
      // between the read and the send), which is fine to skip. But a batch
      // failing EVERY time points at a malformed instruction, not a race, so
      // name the error rather than presuming a cancel.
      console.warn(`  skipping ${detail}`);
      continue;
    }
    const signature = await deps.buildAndSend(
      item.instructions,
      item.units,
      MAX_LOADED_ACCOUNTS_DATA_SIZE,
    );
    result.transactionsSent += 1;
    result.recoveredLamports += item.recovered;
    result.cleanRewardLamports += item.reward;
    result.signatures.push(signature);
    console.log(
      `[${index + 1}/${work.length}] ${item.label}: ${Number(item.recovered + item.reward) / 1e9} SOL, ${signature}`,
    );
    if (index + 1 < work.length) await deps.sleep(options.delayMs);
  }
  return result;
}
