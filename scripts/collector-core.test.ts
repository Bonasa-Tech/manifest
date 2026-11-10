/**
 * Unit tests for the core-side collection logic: how a market's shape is read
 * from its bytes, which markets are worth sweeping, which orders are
 * cleanable, and whether a batch of cleans pays for itself.
 *
 * Pure functions only: no RPC, no keypair, no validator.
 *
 * Run with: yarn test:collector-core
 */
import * as assert from 'node:assert/strict';
import { PublicKey } from '@solana/web3.js';
import {
  CLEAN_EXPIRY_BUFFER_SLOTS,
  GAS_DEPOSIT_LAMPORTS,
  MARKET_BLOCK_SIZE,
  MARKET_FIXED_SIZE,
  cleanBatchIsProfitable,
  cleanRaceErrorCode,
  createUnbackedTracker,
  findCleanableOrders,
  parseGlobalDeposits,
  planMarket,
  requiredGlobalAtoms,
  selectMarkets,
  selectStableUnbacked,
  sweepComputeUnits,
  type CleanableOrder,
  type GlobalAccount,
  type MarketPlan,
} from './collector-core';

// Mirrors the program's own constants. A copy here fails the test if either
// side moves without the other being reconsidered.
const LAMPORTS_PER_BYTE_YEAR_EXEMPT = 5080;
const ACCOUNT_OVERHEAD_BYTES = 128;
const NODE_TYPE_CLAIMED_SEAT = 1;
const NODE_TYPE_RESTING_ORDER = 2;
const MARKET = '1'.repeat(43);

const rentMinimum = (space: number): bigint =>
  BigInt((space + ACCOUNT_OVERHEAD_BYTES) * LAMPORTS_PER_BYTE_YEAR_EXEMPT);

const GLOBAL = 'G'.repeat(43);

/**
 * Write a 1:1 price and `atoms` base atoms into an order payload, so a bid for
 * it needs exactly `atoms` quote atoms of backing. Price is a u128 of quote
 * atoms per base atom scaled by 10^18, stored as two little-endian u64 words.
 */
const priceOneToOne = (data: Buffer, payload: number, atoms: bigint): void => {
  data.writeBigUInt64LE(10n ** 18n, payload);
  data.writeBigUInt64LE(0n, payload + 8);
  data.writeBigUInt64LE(atoms, payload + 16);
};

/** A global account holding `balances`; traders left out read as zero. */
const globalWith = (
  mint: string,
  balances: [string, bigint][] = [],
): GlobalAccount => ({
  pubkey: GLOBAL,
  mint,
  balances: new Map(balances),
});

/**
 * Build a market account whose dynamic area has `layout` as its block types,
 * where 0 is a free block.
 *
 * Base and quote mints must be DISTINCT. Leaving them zeroed made both sides
 * resolve to the same pubkey, so a global account mapped for the quote mint
 * also matched an ask and the side-specific check could not be tested at all.
 */
function market(
  layout: number[],
  extraLamports: bigint,
): Buffer & { lamports: bigint } {
  const data = Buffer.alloc(
    MARKET_FIXED_SIZE + layout.length * MARKET_BLOCK_SIZE,
  );
  data.fill(0xaa, 16, 48); // base mint
  data.fill(0xbb, 48, 80); // quote mint
  data.writeUInt32LE(layout.length * MARKET_BLOCK_SIZE, 152);
  layout.forEach((kind, block) => {
    data[MARKET_FIXED_SIZE + block * MARKET_BLOCK_SIZE + 13] = kind;
  });
  const result = data as Buffer & { lamports: bigint };
  result.lamports = rentMinimum(data.length) + extraLamports;
  return result;
}

const planOf = (layout: number[], extraLamports = 0n): MarketPlan => {
  const data = market(layout, extraLamports);
  const plan = planMarket({ pubkey: MARKET, lamports: data.lamports, data });
  assert.ok(plan !== null, 'expected a plan');
  return plan;
};

let failures = 0;
function test(name: string, body: () => void): void {
  try {
    body();
    console.log(`ok - ${name}`);
  } catch (error) {
    failures += 1;
    console.error(`FAIL - ${name}`);
    console.error(error);
  }
}

const O = NODE_TYPE_RESTING_ORDER;
const S = NODE_TYPE_CLAIMED_SEAT;

test('a packed market with a 5-block tail is cut to exactly 2 free blocks', () => {
  // The keep_count + 2 floor binds because there are no interior holes, so
  // this is the 5 -> 2 case: three blocks of rent come back.
  const plan = planOf([O, O, O, O, O, 0, 0, 0, 0, 0]);
  assert.equal(plan.orders, 5);
  assert.equal(plan.freeTailBlocks, 5);
  assert.equal(plan.interiorHoles, 0);
  assert.equal(plan.postSpace, MARKET_FIXED_SIZE + 7 * MARKET_BLOCK_SIZE);
  assert.equal(
    plan.tailRentLamports,
    BigInt(3 * MARKET_BLOCK_SIZE * LAMPORTS_PER_BYTE_YEAR_EXEMPT),
  );
});

test('interior holes survive a sweep because only relocation could gather them', () => {
  // frontier dominates the floor here: the single tail block goes, the three
  // holes between live blocks stay. This is why sweeping a large, churned
  // market is almost pure fee collection.
  const plan = planOf([O, 0, O, 0, O, 0, S, 0]);
  assert.equal(plan.interiorHoles, 3);
  assert.equal(plan.freeTailBlocks, 1);
  assert.equal(plan.postSpace, MARKET_FIXED_SIZE + 7 * MARKET_BLOCK_SIZE);
  assert.equal(
    plan.tailRentLamports,
    BigInt(MARKET_BLOCK_SIZE * LAMPORTS_PER_BYTE_YEAR_EXEMPT),
  );
});

test('accrued excess above rent is counted, and a market at its minimum yields none', () => {
  assert.equal(planOf([O, O, 0, 0], 7_000n).excessLamports, 7_000n);
  assert.equal(planOf([O, O, 0, 0], 0n).excessLamports, 0n);
  // Below the minimum must not read as negative recovery.
  const underfunded = market([O, O, 0, 0], 0n);
  const plan = planMarket({ pubkey: MARKET, lamports: 1n, data: underfunded });
  assert.ok(plan !== null);
  assert.equal(plan.excessLamports, 0n);
});

test('a market with fewer than two free blocks is flagged: defrag would expand it', () => {
  // free_blocks_short_of_n(2) -> batch_expand_market runs BEFORE compaction and
  // the collector funds it, so these are a guaranteed loss of ~406,400
  // lamports. 73 such markets cost 0.0385 SOL across three mainnet sweeps
  // before this was caught.
  assert.equal(planOf([O, O, O]).expandsOnDefrag, true);
  assert.equal(planOf([O, O, 0]).expandsOnDefrag, true);
  assert.equal(planOf([O, O, 0, 0]).expandsOnDefrag, false);
});

test('selection never sends a market the processor would expand', () => {
  const expanding = planOf([O, O, O], 10_000_000n);
  const selected = selectMarkets([expanding], {
    minRecoveryLamports: 0n,
    minShrinkBlocks: 0,
  });
  assert.deepEqual(
    selected,
    [],
    'an expanding market must be excluded whatever it holds',
  );
});

test('min-shrink targets tail tightening and drops fee-only markets', () => {
  const feeOnly = planOf([O, O, 0, 0], 500_000n); // nothing to truncate
  const tail = planOf([O, O, 0, 0, 0, 0, 0]); // three blocks above the floor
  const both = [feeOnly, tail];
  assert.equal(
    selectMarkets(both, { minRecoveryLamports: 0n, minShrinkBlocks: 0 }).length,
    2,
    'at 0 every profitable market qualifies',
  );
  const tightened = selectMarkets(both, {
    minRecoveryLamports: 0n,
    minShrinkBlocks: 3,
  });
  assert.equal(tightened.length, 1);
  assert.equal(tightened[0].freeTailBlocks, 5);
});

test('selection honours the dust floor and orders by recovery', () => {
  const dust = planOf([O, O, 0, 0], 10n);
  const rich = planOf([O, O, 0, 0], 5_000_000n);
  const selected = selectMarkets([dust, rich], {
    minRecoveryLamports: 100_000n,
    minShrinkBlocks: 0,
  });
  assert.equal(selected.length, 1);
  assert.equal(selected[0].excessLamports, 5_000_000n);
});

test('only expired orders are cleanable, and only where a global account exists', () => {
  const data = market([O, O, O], 0n);
  const payload = (block: number): number =>
    MARKET_FIXED_SIZE + block * MARKET_BLOCK_SIZE + 16;
  // block 0 expired, block 1 never expires, block 2 expires in the future.
  data.writeUInt32LE(100, payload(0) + 36);
  data.writeUInt32LE(0, payload(1) + 36);
  data.writeUInt32LE(9_000, payload(2) + 36);
  data[payload(0) + 40] = 1; // a bid, so the QUOTE mint's global applies
  const plan = planMarket({ pubkey: MARKET, lamports: data.lamports, data })!;

  const withGlobal = new Map([[plan.quoteMint, globalWith(plan.quoteMint)]]);
  const found = findCleanableOrders(plan, data, 5_000, withGlobal);
  assert.equal(found.length, 1, 'only the expired bid is cleanable');
  assert.equal(found[0].orderIndex, 0);
  assert.equal(found[0].global, GLOBAL);

  // An ask needs the BASE mint's global; with only the quote one mapped it is
  // not cleanable, because the instruction would be rejected.
  data[payload(0) + 40] = 0;
  assert.deepEqual(findCleanableOrders(plan, data, 5_000, withGlobal), []);
  assert.deepEqual(findCleanableOrders(plan, data, 5_000, new Map()), []);
});

test('a freshly expired order is deferred until past the expiry buffer', () => {
  const data = market([O, O], 0n);
  const payload = (block: number): number =>
    MARKET_FIXED_SIZE + block * MARKET_BLOCK_SIZE + 16;
  // Both bids. Block 0 expired but within the buffer (cleaning it would race
  // the book: on an active market the block empties or is recycled within
  // seconds of expiry); block 1 expired one slot past the buffer.
  const nowSlot = 100_000;
  data.writeUInt32LE(nowSlot - CLEAN_EXPIRY_BUFFER_SLOTS, payload(0) + 36);
  data.writeUInt32LE(nowSlot - CLEAN_EXPIRY_BUFFER_SLOTS - 1, payload(1) + 36);
  for (const b of [0, 1]) data[payload(b) + 40] = 1;
  const plan = planMarket({ pubkey: MARKET, lamports: data.lamports, data })!;
  const withGlobal = new Map([[plan.quoteMint, globalWith(plan.quoteMint)]]);

  const found = findCleanableOrders(plan, data, nowSlot, withGlobal);
  assert.equal(found.length, 1, 'only the long-expired order is cleanable');
  assert.equal(found[0].orderIndex, MARKET_BLOCK_SIZE);
});

test('unbacked candidates: only non-expired GLOBAL orders, only with --clean-unbacked', () => {
  const data = market([O, O], 0n);
  const payload = (block: number): number =>
    MARKET_FIXED_SIZE + block * MARKET_BLOCK_SIZE + 16;
  // Both non-expired bids; block 0 is a Global order (type 3), block 1 a Limit.
  for (const b of [0, 1]) {
    data.writeUInt32LE(0, payload(b) + 36); // never expires
    data[payload(b) + 40] = 1; // bid -> quote mint global
    priceOneToOne(data, payload(b), 1_000n);
  }
  data[payload(0) + 41] = 3; // OrderType::Global
  data[payload(1) + 41] = 0; // OrderType::Limit
  const plan = planMarket({ pubkey: MARKET, lamports: data.lamports, data })!;
  // The maker has nothing deposited, so the global order's 1,000 quote atoms
  // are unbacked.
  const withGlobal = new Map([[plan.quoteMint, globalWith(plan.quoteMint)]]);

  // Without the flag, no non-expired order is returned.
  assert.deepEqual(findCleanableOrders(plan, data, 5_000, withGlobal), []);

  // With it, only the global order is an unbacked candidate (the limit is not).
  const cand = findCleanableOrders(plan, data, 5_000, withGlobal, true);
  assert.equal(cand.length, 1, 'only the global order is a candidate');
  assert.equal(cand[0].kind, 'unbacked');
  assert.equal(cand[0].orderIndex, 0);
  assert.equal(typeof cand[0].maker, 'string');
});

test('a global order the maker has deposited for is not a candidate at all', () => {
  // The whole point of reading the balances: a backed global order must never
  // reach the simulation queue. Before this check every global order on the
  // program was a candidate (40,580 on mainnet), the per-pass cap was spent
  // re-simulating backed ones, and nothing was ever cleaned.
  const data = market([O], 0n);
  const payload = MARKET_FIXED_SIZE + 16;
  data.writeUInt32LE(0, payload + 36); // never expires
  data[payload + 40] = 1; // bid -> quote mint global
  data[payload + 41] = 3; // OrderType::Global
  priceOneToOne(data, payload, 1_000n);
  const plan = planMarket({ pubkey: MARKET, lamports: data.lamports, data })!;
  const maker = new PublicKey(
    data.subarray(MARKET_FIXED_SIZE + 16, MARKET_FIXED_SIZE + 48),
  ).toBase58();

  const fully = new Map([
    [plan.quoteMint, globalWith(plan.quoteMint, [[maker, 1_000n]])],
  ]);
  assert.deepEqual(
    findCleanableOrders(plan, data, 5_000, fully, true),
    [],
    'balance exactly covers the order: ineligible, as the program requires',
  );

  const short = new Map([
    [plan.quoteMint, globalWith(plan.quoteMint, [[maker, 999n]])],
  ]);
  assert.equal(
    findCleanableOrders(plan, data, 5_000, short, true).length,
    1,
    'one atom short is unbacked',
  );

  // An ask is backed in base atoms one-for-one, and reads the BASE global.
  data[payload + 40] = 0;
  const asks = new Map([
    [plan.baseMint, globalWith(plan.baseMint, [[maker, 1_000n]])],
  ]);
  assert.deepEqual(findCleanableOrders(plan, data, 5_000, asks, true), []);
  const asksShort = new Map([
    [plan.baseMint, globalWith(plan.baseMint, [[maker, 1n]])],
  ]);
  assert.equal(
    findCleanableOrders(plan, data, 5_000, asksShort, true).length,
    1,
  );
});

test('global balances come from the deposits tree, not a block scan', () => {
  // GlobalFixed is 96 bytes, then 64-byte blocks: a 16-byte RBNode header
  // (left, right, parent, color, payload_type, padding) and a GlobalDeposit of
  // trader(32) balance_atoms(8) padding(8).
  const GLOBAL_FIXED_SIZE = 96;
  const GLOBAL_BLOCK_SIZE = 64;
  const NIL = 0xffff_ffff;
  const trader = (c: string): Buffer => Buffer.alloc(32, c.charCodeAt(0));
  const key = (c: string): string => new PublicKey(trader(c)).toBase58();

  const data = Buffer.alloc(GLOBAL_FIXED_SIZE + 3 * GLOBAL_BLOCK_SIZE);
  data.writeUInt32LE(0, 76); // deposits_root_index -> block 0
  const node = (index: number, left: number, right: number): number => {
    const at = GLOBAL_FIXED_SIZE + index;
    data.writeUInt32LE(left, at);
    data.writeUInt32LE(right, at + 4);
    data.writeUInt32LE(NIL, at + 8);
    return at + 16;
  };
  // Root holds A, its only child holds B.
  let payload = node(0, GLOBAL_BLOCK_SIZE, NIL);
  trader('A').copy(data, payload);
  data.writeBigUInt64LE(10n, payload + 32);
  payload = node(GLOBAL_BLOCK_SIZE, NIL, NIL);
  trader('B').copy(data, payload);
  data.writeBigUInt64LE(20n, payload + 32);
  // A freed block whose old deposit bytes survive. A linear scan would read C
  // as a live 99-atom balance and call every order of its long-gone maker
  // backed; only the tree walk knows it is unreachable.
  payload = node(2 * GLOBAL_BLOCK_SIZE, NIL, NIL);
  trader('C').copy(data, payload);
  data.writeBigUInt64LE(99n, payload + 32);

  const balances = parseGlobalDeposits(data);
  assert.deepEqual(
    [...balances.entries()].sort(),
    [
      [key('A'), 10n],
      [key('B'), 20n],
    ].sort(),
  );

  // Untrusted RPC bytes: a cycle must terminate, and a bogus root must not
  // throw or read out of bounds.
  data.writeUInt32LE(0, GLOBAL_FIXED_SIZE + GLOBAL_BLOCK_SIZE); // B.left -> root
  assert.equal(parseGlobalDeposits(data).size, 2, 'a cycle terminates');
  data.writeUInt32LE(9_999, 76);
  assert.equal(
    parseGlobalDeposits(data).size,
    0,
    'an out-of-range root is empty',
  );
  data.writeUInt32LE(NIL, 76);
  assert.equal(parseGlobalDeposits(data).size, 0, 'an empty tree is empty');
  assert.equal(
    parseGlobalDeposits(Buffer.alloc(8)).size,
    0,
    'truncated is empty',
  );
});

test('required global atoms mirrors the program, None included', () => {
  // Ask: base atoms one for one.
  assert.equal(requiredGlobalAtoms(0n, 7n, false), 7n);
  // Bid: ceil(price * atoms / 10^18). 1.5 quote per base over 3 base atoms.
  assert.equal(requiredGlobalAtoms(15n * 10n ** 17n, 3n, true), 5n);
  // Rounding is up, as the program's round_up=true demands: anything above an
  // exact atom costs the next one.
  assert.equal(requiredGlobalAtoms(1n, 1n, true), 1n);
  assert.equal(requiredGlobalAtoms(0n, 1n, true), 0n);
  // The largest quote that still fits u64 is not None: 1:1 over a full u64.
  assert.equal(
    requiredGlobalAtoms(10n ** 18n, 2n ** 64n - 1n, true),
    2n ** 64n - 1n,
  );
  // One atom past it is the program's None: eligible whatever the balance.
  assert.equal(
    requiredGlobalAtoms(2n * 10n ** 18n, 2n ** 64n - 1n, true),
    null,
  );
  // So is a u128 overflow in the product itself.
  assert.equal(requiredGlobalAtoms((1n << 127n) + 1n, 4n, true), null);
});

const unbackedCandidate = (
  market: string,
  sequenceNumber: bigint,
): CleanableOrder => ({
  market,
  orderIndex: 0,
  sequenceNumber,
  global: 'G'.repeat(43),
  lastValidSlot: 0,
  maker: 'M'.repeat(43),
  kind: 'unbacked',
});

test('unbacked cleaning waits for an order to rest unchanged across a pass', () => {
  const tracker = createUnbackedTracker();
  const a = unbackedCandidate('A'.repeat(43), 1n);
  const b = unbackedCandidate('B'.repeat(43), 2n);

  // First sighting: baseline only, nothing is confirmed yet.
  const first = selectStableUnbacked([a, b], tracker, 1_000);
  assert.deepEqual(first.toConfirm, []);
  assert.equal(first.stable, 0);
  assert.equal(first.freshlySeen, 2);

  // Same two orders a pass later: both have rested unchanged, so both qualify.
  const second = selectStableUnbacked([a, b], tracker, 1_000);
  assert.equal(second.toConfirm.length, 2);
  assert.equal(second.stable, 2);
  assert.equal(second.freshlySeen, 0);
});

test('a replaced order (new sequence number) starts its wait over', () => {
  const tracker = createUnbackedTracker();
  const market = 'A'.repeat(43);
  selectStableUnbacked([unbackedCandidate(market, 1n)], tracker, 1_000);
  // The slot now holds a different order: same market, new sequence number.
  const next = selectStableUnbacked(
    [unbackedCandidate(market, 2n)],
    tracker,
    1_000,
  );
  assert.deepEqual(next.toConfirm, [], 'the new order is not yet old enough');
  assert.equal(next.freshlySeen, 1);
});

test('the sim cap rotates, so every eligible candidate is reached', () => {
  // The bug this guards: ordering by how long an order has rested and taking
  // the first `cap` picks the same prefix every pass, because resting one more
  // pass only lengthens a streak. On mainnet that spent all 1,000 sims of
  // every pass on the same 1,000 orders and never reached the other 37,378.
  const tracker = createUnbackedTracker();
  const markets = ['A', 'B', 'C', 'D'].map((c) => c.repeat(43));
  const all = markets.map((m, i) => unbackedCandidate(m, BigInt(i)));
  selectStableUnbacked(all, tracker, 2); // baseline pass: nothing eligible yet

  const seen: string[] = [];
  for (let pass = 0; pass < 2; pass += 1) {
    const chosen = selectStableUnbacked(all, tracker, 2);
    assert.equal(chosen.stable, 4);
    assert.equal(chosen.overCap, 2);
    seen.push(...chosen.toConfirm.map((o) => o.market));
  }
  assert.deepEqual(
    [...seen].sort(),
    [...markets].sort(),
    'two passes at a cap of 2 cover all four, with no repeats',
  );

  // And it keeps going round rather than stopping at the end of the list.
  const wrapped = selectStableUnbacked(all, tracker, 2);
  assert.deepEqual(
    wrapped.toConfirm.map((o) => o.market),
    seen.slice(0, 2),
    'the third pass starts the cycle again',
  );
});

test('a cap at or above the candidate count confirms every one of them', () => {
  const tracker = createUnbackedTracker();
  const all = ['A', 'B', 'C'].map((c, i) =>
    unbackedCandidate(c.repeat(43), BigInt(i)),
  );
  selectStableUnbacked(all, tracker, 50);
  const chosen = selectStableUnbacked(all, tracker, 50);
  assert.equal(chosen.toConfirm.length, 3);
  assert.equal(chosen.overCap, 0);
});

test('without a tracker there is no last run to compare, so nothing is chosen', () => {
  const chosen = selectStableUnbacked(
    [unbackedCandidate('A'.repeat(43), 1n)],
    undefined,
    1_000,
  );
  assert.deepEqual(chosen.toConfirm, []);
  assert.equal(chosen.stable, 0);
});

test('a raced clean is recognised from both a sim error and a thrown send error', () => {
  // The order's slot was cancelled or recycled since we read it: the program
  // returns InvalidClean (21, "Wrong global provided" or "Ineligible") or
  // WrongIndexHint (10). Both must read as an expected skip, whether they
  // arrive as the structured err a simulation returns or buried in the string
  // of an error a send throws.
  assert.equal(
    cleanRaceErrorCode({ InstructionError: [2, { Custom: 21 }] }),
    21,
  );
  assert.equal(
    cleanRaceErrorCode({ InstructionError: [0, { Custom: 10 }] }),
    10,
  );
  assert.equal(
    cleanRaceErrorCode(
      new Error('failed to send: {"InstructionError":[2,{"Custom":21}]}'),
    ),
    21,
  );
  // A different program error is a real fault, not a race: not classified.
  assert.equal(
    cleanRaceErrorCode({ InstructionError: [0, { Custom: 15 }] }),
    null,
  );
  assert.equal(cleanRaceErrorCode('BlockhashNotFound'), null);
  assert.equal(cleanRaceErrorCode(null), null);
});

test('a clean batch must out-earn the signature it needs', () => {
  assert.equal(
    GAS_DEPOSIT_LAMPORTS,
    5_000,
    'one deposit exactly equals one signature',
  );
  assert.equal(cleanBatchIsProfitable(0, false), false);
  assert.equal(
    cleanBatchIsProfitable(1, false),
    false,
    'one clean alone only breaks even',
  );
  assert.equal(cleanBatchIsProfitable(2, false), true);
  // Riding with a sweep, the signature is already paid for.
  assert.equal(cleanBatchIsProfitable(1, true), true);
  assert.equal(cleanBatchIsProfitable(0, true), false);
});

test('sweep compute scales with account size', () => {
  const small = sweepComputeUnits(planOf([O, O, 0, 0]));
  const large = sweepComputeUnits(
    planOf(new Array(200).fill(O).concat([0, 0])),
  );
  assert.ok(large > small, `${large} should exceed ${small}`);
});

if (failures > 0) {
  console.error(`${failures} test(s) failed`);
  process.exit(1);
}
console.log('all core collection tests passed');
