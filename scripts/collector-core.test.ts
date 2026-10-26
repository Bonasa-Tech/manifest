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
import {
  GAS_DEPOSIT_LAMPORTS,
  MARKET_BLOCK_SIZE,
  MARKET_FIXED_SIZE,
  cleanBatchIsProfitable,
  cleanRaceErrorCode,
  findCleanableOrders,
  planMarket,
  selectMarkets,
  sweepComputeUnits,
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

  const withGlobal = new Map([[plan.quoteMint, 'G'.repeat(43)]]);
  const found = findCleanableOrders(plan, data, 5_000, withGlobal);
  assert.equal(found.length, 1, 'only the expired bid is cleanable');
  assert.equal(found[0].orderIndex, 0);
  assert.equal(found[0].global, 'G'.repeat(43));

  // An ask needs the BASE mint's global; with only the quote one mapped it is
  // not cleanable, because the instruction would be rejected.
  data[payload(0) + 40] = 0;
  assert.deepEqual(findCleanableOrders(plan, data, 5_000, withGlobal), []);
  assert.deepEqual(findCleanableOrders(plan, data, 5_000, new Map()), []);
});

test('unbacked candidates: only non-expired GLOBAL orders, only with --clean-unbacked', () => {
  const data = market([O, O], 0n);
  const payload = (block: number): number =>
    MARKET_FIXED_SIZE + block * MARKET_BLOCK_SIZE + 16;
  // Both non-expired bids; block 0 is a Global order (type 3), block 1 a Limit.
  for (const b of [0, 1]) {
    data.writeUInt32LE(0, payload(b) + 36); // never expires
    data[payload(b) + 40] = 1; // bid -> quote mint global
  }
  data[payload(0) + 41] = 3; // OrderType::Global
  data[payload(1) + 41] = 0; // OrderType::Limit
  const plan = planMarket({ pubkey: MARKET, lamports: data.lamports, data })!;
  const withGlobal = new Map([[plan.quoteMint, 'G'.repeat(43)]]);

  // Without the flag, no non-expired order is returned.
  assert.deepEqual(findCleanableOrders(plan, data, 5_000, withGlobal), []);

  // With it, only the global order is an unbacked candidate (the limit is not).
  const cand = findCleanableOrders(plan, data, 5_000, withGlobal, true);
  assert.equal(cand.length, 1, 'only the global order is a candidate');
  assert.equal(cand[0].kind, 'unbacked');
  assert.equal(cand[0].orderIndex, 0);
  assert.equal(typeof cand[0].maker, 'string');
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
