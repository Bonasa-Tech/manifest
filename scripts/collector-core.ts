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
// price is a u128 of quote atoms per base atom scaled by 10^18, stored as two
// little-endian u64 words (QuoteAtomsPerBaseAtom keeps 8-byte alignment).
const ORDER_PRICE_OFFSET = 0;
const ORDER_BASE_ATOMS_OFFSET = 16;
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
// Only treat an order as expired once it has been expired this many slots at
// the time of the read. A freshly expired order on an active market is gone
// within seconds — the matching engine cancels expired orders on touch and the
// block gets recycled — so cleaning it races nearly every time: on mainnet,
// every failed clean batch traced to orders expired 1-6 slots before the read
// (WrongIndexHint when the block emptied, InvalidClean "Wrong global" when it
// was recycled to the other side), while orders expired for tens of thousands
// of slots simulated clean. Deferring a fresh order costs only its
// 5,000-lamport deposit for a pass; one that persists is caught next pass.
export const CLEAN_EXPIRY_BUFFER_SLOTS = 1_500;
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

/** A u128 stored as two little-endian u64 words, as the program stores price. */
function readU128LE(data: Buffer, at: number): bigint {
  return data.readBigUInt64LE(at) | (data.readBigUInt64LE(at + 8) << 64n);
}

/** The order owner pubkey, read from the ClaimedSeat at trader_index. */
function readMaker(data: Buffer, traderIndex: number): string | null {
  const seat =
    MARKET_FIXED_SIZE + traderIndex + NODE_TYPE_RESTING_ORDER_SEAT_TRADER;
  if (traderIndex < 0 || seat + 32 > data.length) return null;
  return new PublicKey(data.subarray(seat, seat + 32)).toBase58();
}

const U64_MAX = (1n << 64n) - 1n;
const U128_MAX = (1n << 128n) - 1n;
const PRICE_SCALE = 10n ** 18n;

/**
 * Global atoms an order needs to be backed, mirroring `required_global_atoms`
 * in global_clean.rs: an ask is backed in base atoms one-for-one, a bid in
 * quote atoms at `ceil(price * base_atoms / 10^18)`.
 *
 * `null` is the program's `None`: the quote does not fit, which makes the order
 * eligible regardless of balance (`required.map_or(true, ...)`). The program
 * reaches that through a u128 overflow or a quote above u64, both checked here
 * because BigInt has neither limit of its own.
 */
export function requiredGlobalAtoms(
  price: bigint,
  baseAtoms: bigint,
  isBid: boolean,
): bigint | null {
  if (!isBid) return baseAtoms;
  const product = price * baseAtoms;
  if (product > U128_MAX) return null;
  const quote = (product + PRICE_SCALE - 1n) / PRICE_SCALE;
  return quote > U64_MAX ? null : quote;
}

/**
 * Orders GlobalClean can remove.
 *
 * Expired orders are eligible outright. With `includeUnbacked`, a non-expired
 * GLOBAL order is returned (kind 'unbacked') only when the maker's balance in
 * the global for that side is short of what the order needs — the program's own
 * eligibility test, evaluated against the same bytes the program would read.
 * Every other order is not cleanable and is skipped.
 *
 * Testing it here rather than leaving it to simulation is the difference
 * between a candidate set of every global order on the program (40,580 on
 * mainnet, almost all of them perfectly backed) and the few that are actually
 * cleanable. The caller still simulates what survives, but as a final check on
 * the read -> send race, not as a search.
 *
 * `globalForMint` must contain only global accounts that exist: GlobalClean
 * needs one whose mint matches the order's side (quote for a bid, base for an
 * ask), so an order on a market with no such global cannot be cleaned.
 */
export function findCleanableOrders(
  plan: MarketPlan,
  data: Buffer,
  nowSlot: number,
  globalForMint: Map<string, GlobalAccount>,
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
    const expired =
      lastValidSlot !== NO_EXPIRATION &&
      lastValidSlot < nowSlot - CLEAN_EXPIRY_BUFFER_SLOTS;
    const isGlobal = data[payload + ORDER_TYPE_OFFSET] === ORDER_TYPE_GLOBAL;
    // Expired is eligible as-is; a non-expired global order is an unbacked
    // candidate. Everything else cannot be cleaned.
    let kind: CleanableOrder['kind'] | null = null;
    if (expired) kind = 'expired';
    else if (includeUnbacked && isGlobal) kind = 'unbacked';
    if (kind === null) continue;

    // The program checks the global account against the order's side: a bid is
    // backed in the quote mint, an ask in the base mint (global_clean.rs). This
    // mirrors that exactly. The side is read from a live snapshot, though, and
    // the slot can be cancelled or recycled to the other side before the clean
    // lands — at which point this global is "wrong" for the new order. That
    // race is caught at send time (a failed clean stops global clean for the
    // pass), not avoidable here: the selection is correct for the state we
    // read.
    const isBid = data[payload + ORDER_IS_BID_OFFSET] === 1;
    const global = globalForMint.get(isBid ? plan.quoteMint : plan.baseMint);
    if (global === undefined) continue;
    const maker = readMaker(
      data,
      data.readUInt32LE(payload + ORDER_TRADER_INDEX_OFFSET),
    );
    if (maker === null) continue;

    // An expired order is eligible whatever the maker holds. A global one is
    // eligible only while its backing is short, so apply the program's test
    // now: a trader with no deposit reads as zero, exactly as the program's
    // get_balance_atoms does for an evicted one.
    if (kind === 'unbacked') {
      const required = requiredGlobalAtoms(
        readU128LE(data, payload + ORDER_PRICE_OFFSET),
        data.readBigUInt64LE(payload + ORDER_BASE_ATOMS_OFFSET),
        isBid,
      );
      const balance = global.balances.get(maker) ?? 0n;
      if (required !== null && balance >= required) continue;
    }

    out.push({
      market: plan.pubkey,
      // The instruction takes a byte offset into the dynamic area, not a
      // block number.
      orderIndex: block * MARKET_BLOCK_SIZE,
      sequenceNumber: data.readBigUInt64LE(payload + ORDER_SEQUENCE_OFFSET),
      global: global.pubkey,
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
// GlobalFixed: discriminant(8) mint(32) vault(32) traders_root(4)
// deposits_root(4) deposits_max(4) free_list_head(4) num_bytes_allocated(4)
// vault_bump(1) global_bump(1) num_seats_claimed(2) = 96.
const GLOBAL_FIXED_SIZE = 96;
const GLOBAL_DEPOSITS_ROOT_OFFSET = 76;
// The dynamic area is 64-byte blocks: the same 16-byte RBNode header the
// market uses, then a 48-byte payload. GlobalDeposit is trader(32)
// balance_atoms(8) padding(8).
const GLOBAL_BLOCK_SIZE = 64;
const DEPOSIT_BALANCE_OFFSET = 32;
// DataIndex::MAX. The certora build uses 0x7FFFFFFF instead, so the walk below
// treats any out-of-bounds index as a terminator rather than matching on one
// sentinel value.
const NIL = 0xffff_ffff;

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
 * One global account: its address and every trader's deposited balance.
 *
 * The balances are what make an unbacked order decidable off-chain. GlobalClean
 * accepts a non-expired global order only when the maker's balance in the
 * global for that side has fallen below what the order needs (global_clean.rs),
 * so holding the balance map here turns "candidate" from "any global order" —
 * the whole global book, tens of thousands of orders — into the handful that
 * the program would actually accept.
 */
export type GlobalAccount = {
  pubkey: string;
  mint: string;
  /** trader pubkey -> balance_atoms. Absent means zero: an evicted trader. */
  balances: Map<string, bigint>;
};

/**
 * Every trader's balance, read by walking the global deposits red-black tree.
 *
 * The tree is walked rather than the blocks scanned linearly because a freed
 * block keeps its old bytes: a linear scan would read stale deposits as live
 * ones and conclude an order is backed when its collateral is long gone. RPC
 * bytes are untrusted, so a bad index or a cycle has to terminate the walk
 * instead of looping or reading out of bounds.
 */
export function parseGlobalDeposits(data: Buffer): Map<string, bigint> {
  const balances = new Map<string, bigint>();
  if (data.length < GLOBAL_FIXED_SIZE) return balances;
  const root = data.readUInt32LE(GLOBAL_DEPOSITS_ROOT_OFFSET);
  const dynamic = data.length - GLOBAL_FIXED_SIZE;
  const seen = new Set<number>();
  const stack: number[] = [root];
  while (stack.length > 0) {
    const index = stack.pop() as number;
    if (index === NIL) continue;
    if (index % GLOBAL_BLOCK_SIZE !== 0) continue;
    if (index + GLOBAL_BLOCK_SIZE > dynamic) continue;
    if (seen.has(index)) continue;
    seen.add(index);
    const node = GLOBAL_FIXED_SIZE + index;
    stack.push(data.readUInt32LE(node), data.readUInt32LE(node + 4));
    const payload = node + NODE_PAYLOAD_OFFSET;
    balances.set(
      new PublicKey(data.subarray(payload, payload + 32)).toBase58(),
      data.readBigUInt64LE(payload + DEPOSIT_BALANCE_OFFSET),
    );
  }
  return balances;
}

/**
 * Mint -> global account, for every global account that exists.
 *
 * GlobalClean needs a global whose mint matches the order's side, so an order
 * whose mint has no global account is simply not cleanable. Reading the set
 * that exists is cheaper and less error-prone than deriving PDAs and probing.
 *
 * Full account data, not just the mint: the deposits tree is what decides
 * whether a global order is unbacked. There are a few hundred of these and
 * each is a few KB, so this is far cheaper than the market fetch beside it —
 * and vastly cheaper than the simulation it replaces.
 */
export async function fetchGlobalAccounts(
  rpc: <T>(method: string, params: unknown[]) => Promise<T>,
  bs58encode: (b: Buffer) => string,
): Promise<Map<string, GlobalAccount>> {
  const accounts = await rpc<
    { pubkey: string; account: { data: [string, 'base64'] } }[]
  >('getProgramAccounts', [
    MANIFEST_PROGRAM_ID.toBase58(),
    {
      commitment: 'confirmed',
      encoding: 'base64',
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
  const byMint = new Map<string, GlobalAccount>();
  for (const { pubkey, account } of accounts) {
    const data = Buffer.from(account.data[0], 'base64');
    if (data.length < GLOBAL_FIXED_SIZE) continue;
    const mint = new PublicKey(
      data.subarray(GLOBAL_MINT_OFFSET, GLOBAL_MINT_OFFSET + 32),
    ).toBase58();
    byMint.set(mint, {
      pubkey,
      mint,
      balances: parseGlobalDeposits(data),
    });
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
// A clean contributes two unique addresses (market + global; payer and system
// are shared across the whole transaction), so the 64-address v1 limit allows
// ~31 cleans on distinct markets and more when several orders share a market
// and its global - up to the ~63-instruction ceiling. CU does not bind: one
// GlobalClean measures ~1,650 CU on mainnet, so even 60 fit inside 1.4M many
// times over. The packer measures the real serialized size and stops at the
// address/size limit on its own; this cap just needs to sit above it rather
// than throttle packing, with a little headroom under the instruction ceiling.
export const MAX_CLEANS_PER_TX = 55;
// Measured ~1,650 CU per clean on mainnet; 3,000 leaves generous margin.
const CLEAN_CU_EACH = 3_000;
// Default ceiling on per-pass unbacked confirmation simulations. Each is one
// RPC round trip, so the cap is what keeps a pass short. 50 is small because
// the candidates reaching it are already confirmed unbacked from the global
// balances: the simulation is a last check on the read -> send race, not a
// search through the book. Overridable via the maxUnbackedSims option.
export const DEFAULT_MAX_UNBACKED_SIMS = 50;

/**
 * Carried across passes to tell an old, quiet unbacked order from a fast
 * churning one. Maps an order's identity (`market:sequence_number`, stable
 * because a resting order is immutable) to the number of consecutive passes it
 * has been seen as an unbacked candidate. Only an order seen in the previous
 * pass too — i.e. unchanged since the last run — is cleaned. The caller owns
 * one tracker and passes it in every pass; the first pass sets the baseline.
 */
export type UnbackedTracker = {
  streaks: Map<string, number>;
  /**
   * Identity the last pass stopped on; the next resumes after it. Sorting by
   * how long an order has rested and taking the first `cap` looks fair but is
   * not: a streak only grows, so the same prefix is picked every pass and
   * anything behind it is never reached. Resuming past the last one examined
   * covers the whole set in `ceil(n / cap)` passes whatever the simulations
   * return. An identity rather than an index because candidates come and go
   * between passes, which would shift a positional cursor onto an unrelated
   * order.
   */
  cursor: string;
};

export const createUnbackedTracker = (): UnbackedTracker => ({
  streaks: new Map(),
  cursor: '',
});

/**
 * Choose which unbacked candidates to confirm this pass, and advance the
 * tracker. Only orders that have rested UNCHANGED since the previous pass are
 * chosen: a resting order is immutable, so the same `market:sequence_number`
 * across passes is literally the same order, and one that survived a full
 * interval is old and quiet rather than a fast-churning slot. This both targets
 * stale orders and sidesteps the read->send race, which only bites orders that
 * turn over quickly.
 *
 * When more are eligible than the `cap` allows, the pass takes a `cap`-sized
 * window starting where the last one stopped, so consecutive passes walk the
 * whole set rather than re-examining one end of it. The tracker is rebuilt from
 * only this pass's candidates, so an order that was cancelled or replaced drops
 * out and its successor (a new sequence number) starts over at one. With no
 * tracker, nothing is ever chosen — there is no prior pass to compare against —
 * which is why a single run only sets the baseline.
 */
export function selectStableUnbacked(
  candidates: CleanableOrder[],
  tracker: UnbackedTracker | undefined,
  cap: number,
): {
  toConfirm: CleanableOrder[];
  /** Candidates present in the previous pass too, so eligible this pass. */
  stable: number;
  /** Candidates seen for the first time (or after a change): not yet eligible. */
  freshlySeen: number;
  /** Eligible candidates left for a later pass because of the cap. */
  overCap: number;
} {
  const identity = (order: CleanableOrder): string =>
    `${order.market}:${order.sequenceNumber}`;
  const prior = tracker?.streaks ?? new Map<string, number>();
  const withStreak = candidates.map((order) => {
    const id = identity(order);
    return { order, id, streak: (prior.get(id) ?? 0) + 1 };
  });
  // Ordered by identity, not by streak: a stable ordering is what makes the
  // cursor below cover every candidate. Streak order would re-pick the same
  // prefix forever, since resting one more pass only lengthens a streak.
  const stable = withStreak
    .filter((entry) => entry.streak >= 2)
    .sort((a, b) => (a.id < b.id ? -1 : 1));
  const window = Math.max(0, Math.min(cap, stable.length));
  // Resume after the identity the last pass stopped on, wrapping to the start
  // once past the end. A first pass, or a cursor whose order is gone, begins at
  // the first candidate that sorts after it.
  const resumeAt = tracker?.cursor ?? '';
  const after = stable.findIndex((entry) => entry.id > resumeAt);
  const start = after === -1 ? 0 : after;
  const toConfirm = Array.from(
    { length: window },
    (_unused, offset) => stable[(start + offset) % stable.length].order,
  );
  if (tracker) {
    tracker.streaks = new Map(
      withStreak.map((entry) => [entry.id, entry.streak]),
    );
    if (window > 0)
      tracker.cursor = stable[(start + window - 1) % stable.length].id;
  }
  return {
    toConfirm,
    stable: stable.length,
    freshlySeen: withStreak.length - stable.length,
    overCap: stable.length - toConfirm.length,
  };
}
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
   * When present, only these markets are eligible for a sweep. Other core
   * phases still inspect every market; this filter is intentionally limited
   * to the zero-relocation market defrag.
   */
  sweepMarketPubkeys?: ReadonlySet<string>;
  /** Called immediately after a market sweep transaction is confirmed. */
  onMarketsSwept?: (markets: readonly string[]) => void;
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
  /**
   * Most unbacked candidates to confirm by simulation in one pass. Defaults to
   * {@link DEFAULT_MAX_UNBACKED_SIMS}. The oldest (longest-resting) candidates
   * are confirmed first; the rest wait for a later pass.
   */
  maxUnbackedSims?: number;
  /**
   * Tracks which unbacked orders have been resting unchanged across passes, so
   * only old, quiet ones are cleaned (see {@link UnbackedTracker}). Without it,
   * no unbacked order is ever cleaned — there is no "last run" to compare to.
   */
  unbackedTracker?: UnbackedTracker;
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
  /** Markets whose sweep transaction was confirmed during this pass. */
  sweptMarkets: string[];
  /**
   * A clean transaction failed and global clean was stopped for the rest of
   * the pass (the next pass retries from a fresh read). Expected now and then
   * on a live book; surfaced so a pass that cleaned little can be told apart
   * from one that simply had little to clean.
   */
  cleanStoppedEarly: boolean;
};

/**
 * Error codes the program returns when a clean's target order has moved since
 * we read the market: `InvalidClean` (21 — the global no longer matches the
 * order's side, logged "Wrong global provided", or the order is no longer
 * eligible) and `WrongIndexHintParams` (10 — the slot now holds a different
 * node, or defrag shrank the account past the index). Both mean the same
 * thing operationally: skip this order, the next pass reads fresh state.
 */
const CLEAN_RACE_ERROR_CODES: ReadonlyMap<number, string> = new Map([
  [21, 'InvalidClean'],
  [10, 'WrongIndexHint'],
]);

const safeStringify = (value: unknown): string => {
  if (typeof value === 'string') return value;
  // A thrown send error carries its detail in the message (the `rpc` helper
  // formats `${method} RPC failed: ${JSON.stringify(error)}`), which
  // JSON.stringify of the Error object would drop — it has no enumerable
  // fields. Read the message directly so the `Custom` code survives.
  if (value instanceof Error) return value.stack ?? value.message;
  try {
    return JSON.stringify(value) ?? String(value);
  } catch {
    return String(value);
  }
};

/**
 * The program error code, if `value` is a race on a clean's target order (see
 * {@link CLEAN_RACE_ERROR_CODES}). Reads both the structured `err` a
 * simulation returns and the error a send throws, by matching the `Custom`
 * code either carries, so a failed clean can be reported as an expected race
 * ("an order moved since the read") rather than an opaque fault before global
 * clean backs off for the pass.
 */
export const cleanRaceErrorCode = (value: unknown): number | null => {
  const match = safeStringify(value).match(/"Custom"\s*:\s*(\d+)/);
  if (match === null) return null;
  const code = Number(match[1]);
  return CLEAN_RACE_ERROR_CODES.has(code) ? code : null;
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
    sweptMarkets: [],
    cleanStoppedEarly: false,
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

  const sweepCandidates =
    options.sweepMarketPubkeys === undefined
      ? plans
      : plans.filter((plan) => options.sweepMarketPubkeys!.has(plan.pubkey));
  if (options.sweepMarketPubkeys !== undefined) {
    console.log(
      `Fill filter: ${sweepCandidates.length}/${plans.length} markets filled since their last confirmed sweep`,
    );
  }
  let selected = options.sweepMarkets
    ? selectMarkets(sweepCandidates, {
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

    // Expired orders are eligible outright. An unbacked one has already been
    // measured against the maker's global balance in findCleanableOrders, so
    // what arrives here is what the program would accept as of the read. The
    // simulation is the guard on the gap between read and send: the maker can
    // top up, or the order can be cancelled and its block recycled, in which
    // case the clean would fail and take its whole batch down. One dummy
    // blockhash serves every sim (replaceRecentBlockhash), so these are cheap
    // simulateTransaction calls — but each is a round trip, hence the cap.
    const expired = kept.filter((c) => c.kind === 'expired');
    const unbackedCandidates = kept.filter((c) => c.kind === 'unbacked');
    const unbacked: CleanableOrder[] = [];
    if (unbackedCandidates.length > 0) {
      const cap = options.maxUnbackedSims ?? DEFAULT_MAX_UNBACKED_SIMS;
      const selection = selectStableUnbacked(
        unbackedCandidates,
        options.unbackedTracker,
        cap,
      );
      console.log(
        `${unbackedCandidates.length} unbacked orders: ${selection.stable} unchanged since last pass` +
          (selection.freshlySeen > 0
            ? `, ${selection.freshlySeen} new or changed (not cleaned this pass)`
            : '') +
          (selection.overCap > 0
            ? `, ${selection.overCap} past the ${cap}-sim cap (next pass resumes there)`
            : '') +
          `; confirming ${selection.toConfirm.length} by simulation...`,
      );
      for (const order of selection.toConfirm) {
        if (options.shouldStop()) break;
        const sim = await deps.simulate(
          [createGlobalCleanInstruction(order, collector)],
          40_000,
          MAX_LOADED_ACCOUNTS_DATA_SIZE,
        );
        if (sim.err === null || sim.err === undefined) unbacked.push(order);
      }
    }
    // Expired first, and the packer preserves this order, so expired orders
    // fill the earliest clean transactions. That matters because the first
    // clean batch that fails stops global clean for the whole pass: the
    // unambiguous orders (expiry is decidable from the market bytes, with a
    // 1,500-slot buffer behind it) should be banked before the ones whose
    // eligibility depends on a balance that a maker can top up mid-pass.
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
     * Whether a simulation failure should abort the run. A sweep failing is a
     * real problem and stops the run. Clean batches are handled separately (see
     * cleanItems): a failed clean backs off global clean for the pass but never
     * aborts, so they do not rely on this flag.
     */
    optional: boolean;
    /**
     * The orders behind a clean batch. Present only on clean work items, which
     * are sent through sendCleanBatch; the first failure stops global clean for
     * the rest of the pass.
     */
    cleanItems?: CleanableOrder[];
    /** The markets behind a sweep work item. */
    sweepItems?: MarketPlan[];
  };
  const work: Work[] = [
    // Cleans first: cheap, and they free blocks a later sweep can truncate.
    ...cleanBatches.map((batch) => ({
      instructions: batch.map(cleanInstruction),
      units: Math.min(MAX_COMPUTE_UNITS, CLEAN_CU_EACH * batch.length + 10_000),
      label: `clean ${batch.length} orders`,
      recovered: 0n,
      reward: BigInt(batch.length) * BigInt(GAS_DEPOSIT_LAMPORTS),
      optional: true,
      cleanItems: batch,
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
      sweepItems: batch,
    })),
  ];

  if (!options.execute) {
    console.log(
      `Dry run: ${work.length} transactions would be sent. Add --execute to submit.`,
    );
    return result;
  }

  // Send one clean batch, which is exactly one transaction. Returns false if it
  // did not land - the pre-send simulation rejected it, or the send threw (a
  // raced order, a churn on the book, an RPC hiccup). On the first such failure
  // the caller stops cleaning for the rest of this pass and waits for the next
  // one, which reads fresh state and tries again. Both the simulation and the
  // send are guarded, so a failed clean backs cleaning off rather than aborting
  // the pass and taking the sweep down with it.
  const describeCleanFailure = (reason: unknown): string => {
    const raceCode = cleanRaceErrorCode(reason);
    return raceCode !== null
      ? `${CLEAN_RACE_ERROR_CODES.get(raceCode)}: an order moved since the read`
      : safeStringify(reason);
  };
  const simulateAndSendCleans = async (
    orders: CleanableOrder[],
  ): Promise<boolean> => {
    const units = Math.min(
      MAX_COMPUTE_UNITS,
      CLEAN_CU_EACH * orders.length + 10_000,
    );
    const ixs = orders.map(cleanInstruction);
    const sim = await deps.simulate(ixs, units, MAX_LOADED_ACCOUNTS_DATA_SIZE);
    if (sim.err !== null && sim.err !== undefined) {
      console.warn(
        `  clean batch of ${orders.length} did not simulate (${describeCleanFailure(sim.err)})`,
      );
      return false;
    }
    try {
      const sig = await deps.buildAndSend(
        ixs,
        units,
        MAX_LOADED_ACCOUNTS_DATA_SIZE,
      );
      result.transactionsSent += 1;
      result.cleanRewardLamports +=
        BigInt(orders.length) * BigInt(GAS_DEPOSIT_LAMPORTS);
      result.signatures.push(sig);
      console.log(`clean ${orders.length} orders: ${sig}`);
      await deps.sleep(options.delayMs);
      return true;
    } catch (error) {
      console.warn(
        `  clean batch of ${orders.length} failed to send (${describeCleanFailure(error)})`,
      );
      return false;
    }
  };
  const sendCleanBatch = async (orders: CleanableOrder[]): Promise<boolean> => {
    if (await simulateAndSendCleans(orders)) return true;
    if (orders.length <= 1) return false;
    // One raced order fails the whole batch, and it kept failing every pass:
    // the batch always contained at least one order that moved between the
    // read and the simulation, so the stable orders behind it were never
    // collected. Isolate instead: simulate each clean on its own, drop the
    // movers, and send the survivors together.
    const survivors: CleanableOrder[] = [];
    for (const order of orders) {
      if (options.shouldStop()) return false;
      const sim = await deps.simulate(
        [cleanInstruction(order)],
        Math.min(MAX_COMPUTE_UNITS, CLEAN_CU_EACH + 10_000),
        MAX_LOADED_ACCOUNTS_DATA_SIZE,
      );
      if (sim.err === null || sim.err === undefined) survivors.push(order);
    }
    console.warn(
      `  ${survivors.length}/${orders.length} cleans survived individual simulation`,
    );
    if (survivors.length === 0) return false;
    return simulateAndSendCleans(survivors);
  };

  // Once a clean transaction fails, stop trying to clean for the rest of this
  // pass. The book is churning or an order raced; rather than burn signatures
  // retrying within the same pass, back off and let the next pass start over
  // from a fresh read. Only global clean stops - the sweep still runs.
  let cleanPhaseStopped = false;

  for (const [index, item] of work.entries()) {
    if (options.shouldStop()) {
      console.warn('Stopping before the next transaction.');
      break;
    }
    if (item.cleanItems) {
      if (cleanPhaseStopped) continue;
      const sent = await sendCleanBatch(item.cleanItems);
      if (!sent) {
        cleanPhaseStopped = true;
        result.cleanStoppedEarly = true;
        console.warn(
          '  a clean transaction failed; stopping global clean for this pass (the next pass retries from a fresh read)',
        );
      }
      continue;
    }
    const simulation = await deps.simulate(
      item.instructions,
      item.units,
      MAX_LOADED_ACCOUNTS_DATA_SIZE,
    );
    if (simulation.err !== null && simulation.err !== undefined) {
      const detail = `${item.label}: ${safeStringify(simulation.err)}`;
      if (!item.optional) throw new Error(`Simulation failed for ${detail}`);
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
    if (item.sweepItems !== undefined) {
      const sweptMarkets = item.sweepItems.map((plan) => plan.pubkey);
      result.sweptMarkets.push(...sweptMarkets);
      options.onMarketsSwept?.(sweptMarkets);
    }
    console.log(
      `[${index + 1}/${work.length}] ${item.label}: ${Number(item.recovered + item.reward) / 1e9} SOL, ${signature}`,
    );
    if (index + 1 < work.length) await deps.sleep(options.delayMs);
  }
  return result;
}
