// Manifest core-market defrag runner — single-file, portable.
//
// Collector-only. Sweeps accrued fees, truncates free tails, and (when asked)
// relocates resting orders to compact a market. Reads nothing machine-specific:
// all configuration comes from flags or environment.
//
// Required:
//   --rpc-url=URL        (or env RPC_URL)        Solana RPC endpoint
//   --keypair=PATH       (or env KEYPAIR_PATH)   collector keypair json; required for --execute
//
// Target (choose one for --execute):
//   --market=ADDR            one market
//   --market-list=FILE       a JSON array of addresses, or a safe-candidates file
//   --all                    every market
//
// Operation (how much work; default is full compaction):
//   --sweep-only             collect fees + truncate free tail, relocate nothing (safest)
//   --seats-only             reclaim empty seats, relocate no resting order
//   --budget=N               relocate at most N resting orders
//   --seat-budget=N          reclaim at most N empty seats
//   (omit all of the above)  unbounded full compaction
//
// Scope / safety:
//   --min-shrink=N           only markets whose free tail truncates >= N blocks
//   --quiet-only             skip markets traded within ~1 day
//   --include-active         allow active markets (warn integrators first)
//   --allow-close            permit closing emptied markets  (also needs --yes-destroy-market-addresses)
//   --max-markets=N          cap per run, highest recovery first
//
// Execution / batching:
//   --execute                submit; omit for a read-only dry run
//   --v1                     pack as v1 transactions (up to 19 markets/tx)
//   --batch / --no-batch     force or forbid batching (sweep-only batches by default)
//   --pace=MS                delay between transactions (default 3000)
//   --health-every=N         sample program health every N transactions (0 disables)
//   --state-dir=DIR          where logs/plan cache/abort state live (default ./defrag-state)
//   --use-cached-plan        reuse a recent plan cache instead of re-simulating
//   --ignore-log             do not skip markets already recorded done
//
// Dry run first, always. A sweep-only run relocates nothing and is safe on any
// market; relocation can strand a cached index in a third-party client, so test
// it one market at a time before any broad rollout.

import { Keypair, Transaction, TransactionInstruction, PublicKey, ComputeBudgetProgram } from '@solana/web3.js';
import {
  AccountRole, address, appendTransactionMessageInstructions, blockhash,
  compileTransaction, createKeyPairSignerFromBytes, createTransactionMessage,
  getBase64EncodedWireTransaction, pipe, setTransactionMessageConfig,
  setTransactionMessageFeePayer, setTransactionMessageFeePayerSigner,
  setTransactionMessageLifetimeUsingBlockhash, signTransactionMessageWithSigners,
} from '@solana/kit';
import bs58 from 'bs58';
import fs from 'node:fs';
import path from 'node:path';

// ===== rpc =====
function makeRpc(url) {
  let id = 0;
  return async function rpc(method, params, { retries = 5 } = {}) {
    for (let attempt = 0; ; attempt++) {
      try {
        const resp = await fetch(url, {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ jsonrpc: '2.0', id: ++id, method, params }),
        });
        if (!resp.ok) throw new Error(`http ${resp.status}`);
        const body = await resp.json();
        if (body.error) {
          const e = new Error(`${method}: ${body.error.message}`);
          e.rpcCode = body.error.code;
          throw e;
        }
        return body.result;
      } catch (e) {
        if (attempt >= retries) throw e;
        await new Promise((r) => setTimeout(r, 400 * (attempt + 1)));
      }
    }
  };
}

const MANIFEST = 'MNFSTqtC93rEfYHB6hF82sKdZpUDFWkViLByLd1k1Ms';
const WRAPPER = 'wMNFSTkir3HgyZTsB7uqu3i7FA73grFCptPXgrZjksL';
const COLLECTOR = 'B6dmr2UAn2wgjdm3T4N1Vjd8oPYRRTguByW7AEngkeL6';
const INSTRUCTION_NAMES = [
  'CreateMarket', 'ClaimSeat', 'Deposit', 'Withdraw', 'Swap', 'Expand',
  'BatchUpdate', 'GlobalCreate', 'GlobalAddTrader', 'GlobalDeposit',
  'GlobalWithdraw', 'GlobalEvict', 'GlobalClean', 'SwapV2', 'Defrag',
];


// ===== config =====
if (process.argv.includes('--help') || process.argv.includes('-h')) {
  console.log(fs.readFileSync(new URL(import.meta.url)).toString().split('\n')
    .filter((l) => l.startsWith('//')).map((l) => l.replace(/^\/\/ ?/, '')).join('\n'));
  process.exit(0);
}

const flag = (name) => process.argv.find((a) => a.startsWith(`--${name}=`))?.slice(name.length + 3);

// Configuration is flags-first, environment second, nothing hardcoded.
const RPC_URL = flag('rpc-url') ?? process.env.RPC_URL;
if (!RPC_URL) throw new Error('set --rpc-url=URL or RPC_URL');
const KEYPAIR_PATH = flag('keypair') ?? process.env.KEYPAIR_PATH;
const rpc = makeRpc(RPC_URL);

// State (logs, plan cache, abort record) lives here; created on demand. Keep it
// off the repo tree by default so a run never dirties a checkout.
const stateDir = flag('state-dir') ?? path.join(process.cwd(), 'defrag-state');
fs.mkdirSync(stateDir, { recursive: true });

// ===== market transaction builders =====
// Shared core-market defrag transaction construction and market enumeration.

const SYSTEM = new PublicKey('11111111111111111111111111111111');
const AUTH = new PublicKey(COLLECTOR);
const MARKET_DISC = 4859840929024028656n;

// All market accounts: pubkey, size, lamports, mints and vaults from the
// fixed header (mints at 16/48, vaults at 80/112).
async function listMarkets(rpc) {
  const disc = Buffer.alloc(8);
  disc.writeBigUInt64LE(MARKET_DISC);
  const accs = await rpc('getProgramAccounts', [MANIFEST, {
    encoding: 'base64',
    dataSlice: { offset: 0, length: 144 },
    filters: [{ memcmp: { offset: 0, bytes: bs58.encode(disc) } }],
    commitment: 'confirmed',
  }]);
  return accs.map((a) => {
    const d = Buffer.from(a.account.data[0], 'base64');
    return {
      pubkey: a.pubkey,
      space: a.account.space,
      lamports: a.account.lamports,
      baseMint: bs58.encode(d.subarray(16, 48)),
      quoteMint: bs58.encode(d.subarray(48, 80)),
      baseVault: bs58.encode(d.subarray(80, 112)),
      quoteVault: bs58.encode(d.subarray(112, 144)),
    };
  });
}

// Token programs come from the mint owners; cached per mint.
const mintOwnerCache = new Map();
async function resolveTokenPrograms(rpc, markets) {
  const mints = [...new Set(markets.flatMap((m) => [m.baseMint, m.quoteMint]))]
    .filter((x) => !mintOwnerCache.has(x));
  for (let i = 0; i < mints.length; i += 100) {
    const chunk = mints.slice(i, i + 100);
    const resp = await rpc('getMultipleAccounts', [chunk, { encoding: 'base64', dataSlice: { offset: 0, length: 0 }, commitment: 'confirmed' }]);
    for (let j = 0; j < chunk.length; j++) {
      mintOwnerCache.set(chunk[j], resp.value[j]?.owner ?? null);
    }
  }
  for (const m of markets) {
    m.baseTokenProgram = mintOwnerCache.get(m.baseMint);
    m.quoteTokenProgram = mintOwnerCache.get(m.quoteMint);
  }
  return markets.filter((m) => m.baseTokenProgram && m.quoteTokenProgram);
}

// Instruction data: [14] unbounded, [14, relocations] legacy (all empty seats
// reclaimed), [14, relocations, seats] budgeting each step separately, and a
// trailing nonzero byte to permit retiring an emptied market. Everything past
// the legacy forms needs the per-step-budget release to be live; against an
// older program they are rejected as InvalidInstructionData.
function defragIx(m, budget, seatBudget, mayClose = false) {
  const data = mayClose
    ? (() => {
        const b = Buffer.alloc(10);
        b[0] = 14;
        b.writeUInt32LE(budget ?? 0xffffffff, 1);
        b.writeUInt32LE(seatBudget ?? 0xffffffff, 5);
        b[9] = 1;
        return b;
      })()
    : seatBudget !== undefined
    ? (() => {
        const b = Buffer.alloc(9);
        b[0] = 14;
        b.writeUInt32LE(budget ?? 0xffffffff, 1);
        b.writeUInt32LE(seatBudget, 5);
        return b;
      })()
    : budget === undefined
    ? Buffer.from([14])
    : (() => { const b = Buffer.alloc(5); b[0] = 14; b.writeUInt32LE(budget, 1); return b; })();
  return new TransactionInstruction({
    programId: new PublicKey(MANIFEST),
    keys: [
      { pubkey: AUTH, isSigner: true, isWritable: true },
      { pubkey: new PublicKey(m.pubkey), isSigner: false, isWritable: true },
      { pubkey: new PublicKey(m.baseVault), isSigner: false, isWritable: true },
      { pubkey: new PublicKey(m.quoteVault), isSigner: false, isWritable: true },
      { pubkey: new PublicKey(m.baseTokenProgram), isSigner: false, isWritable: false },
      { pubkey: new PublicKey(m.quoteTokenProgram), isSigner: false, isWritable: false },
      { pubkey: SYSTEM, isSigner: false, isWritable: false },
    ],
    data,
  });
}

// Measured: ~1 CU/byte for the 1MB market, higher per byte for small
// relocation-heavy ones. 2.5/byte with a floor keeps headroom.
const estimateCu = (space) => Math.min(1_400_000, Math.ceil(80_000 + 2.5 * space));

// `budget` is the optional u32 relocation cap: undefined = unbounded (full
// compaction), 0 = reclaim-only (empty seats are reclaimed and any free tail
// truncated, but no resting order is relocated), N = bounded run.
function buildDefragTx(m, { blockhash, budget, seatBudget, mayClose } = {}) {
  const tx = new Transaction();
  tx.feePayer = AUTH;
  tx.recentBlockhash = blockhash ?? SYSTEM.toBase58();
  // Budget by size in every mode. Disabling relocation does NOT make a run
  // cheap: the mark phase scans every allocated block whatever the budgets
  // say, and on a megabyte market that scan alone costs hundreds of
  // thousands of units. Capping sweep-only low made the two largest markets
  // fail with ProgramFailedToComplete and silently drop 3.1 SOL of fees.
  // Over-requesting costs nothing here because no priority fee is attached.
  const units = estimateCu(m.space);
  tx.add(ComputeBudgetProgram.setComputeUnitLimit({ units }), defragIx(m, budget, seatBudget, mayClose));
  return tx;
}

/// Several markets in one transaction. The compute limit is the sum of the
/// per-market estimates, capped; the caller is responsible for keeping the
/// batch within that cap and within the wire size, and for simulating it.
function buildBatchTx(markets, { blockhash, budget, seatBudget, mayClose, units } = {}) {
  const tx = new Transaction();
  tx.feePayer = AUTH;
  tx.recentBlockhash = blockhash ?? SYSTEM.toBase58();
  const limit = Math.min(1_400_000, units ?? markets.reduce((s, m) => s + estimateCu(m.space), 0));
  tx.add(ComputeBudgetProgram.setComputeUnitLimit({ units: limit }));
  for (const m of markets) tx.add(defragIx(m, budget, seatBudget, mayClose));
  return tx;
}

/// Serialized wire size of a candidate batch, which is the authoritative
/// packing limit: these are legacy transactions, so 1232 bytes is the ceiling
/// and each market adds its market plus two vault addresses.
///
/// web3.js asserts rather than reporting a size once a transaction passes
/// 1232 bytes, so a candidate that is already too large cannot be measured at
/// all. For a packer that is exactly the answer it needs - "does not fit" -
/// so the throw is translated into an infinite size instead of killing the
/// run. Returning a real number here would be a lie, and returning 1232 would
/// let a batch sit exactly on a limit the runtime rejects.
function batchWireSize(markets, opts = {}) {
  try {
    return buildBatchTx(markets, opts).serialize({ requireAllSignatures: false, verifySignatures: false }).length;
  } catch (e) {
    if (/Transaction too large/.test(e.message)) return Number.POSITIVE_INFINITY;
    throw e;
  }
}

async function buildDefragTxBase64(rpc, m, opts = {}) {
  if (!m.baseTokenProgram) await resolveTokenPrograms(rpc, [m]);
  return buildDefragTx(m, opts).serialize({ requireAllSignatures: false, verifySignatures: false }).toString('base64');
}

// Market dynamic-area census, read straight from account bytes: how many
// blocks hold resting orders, claimed seats, or are free/empty. Block layout
// is 80 bytes with the node type at offset 13 (1 = ClaimedSeat, 2 =
// RestingOrder, 0 = Empty/free-listed).
const MARKET_FIXED_SIZE = 256;
const MARKET_BLOCK_SIZE = 80;
function census(data) {
  const allocated = data.readUInt32LE(152);
  const blocks = Math.floor(allocated / MARKET_BLOCK_SIZE);
  let orders = 0, seats = 0, empty = 0;
  for (let b = 0; b < blocks; b++) {
    const at = MARKET_FIXED_SIZE + b * MARKET_BLOCK_SIZE;
    if (at + MARKET_BLOCK_SIZE > data.length) break;
    const kind = data[at + 13];
    if (kind === 2) orders++;
    else if (kind === 1) seats++;
    else empty++;
  }
  return { allocated, blocks, orders, seats, empty };
}


// ===== v1 transaction builders =====
// v1 transaction construction for market defrag.
//
// Legacy transactions cap at 1232 bytes, which limits a defrag batch to 8
// markets because each one contributes three addresses. A v1 transaction
// carries up to 4096 bytes and 64 unique addresses, and for this workload the
// address count binds first: 19 markets is the maximum, at about 2478 bytes,
// so roughly 60% of the size budget goes unused. That is 2.4x fewer
// transactions for the same work.
//
// Two things are required and are easy to get wrong:
//
//   1. loadedAccountsDataSizeLimit must be set explicitly. Without it a
//      19-market batch fails with MaxLoadedAccountsDataSizeExceeded before
//      executing a single instruction, having consumed zero compute units.
//   2. web3.js cannot build this format - its VersionedTransaction is v0 - so
//      @solana/kit is used here while the rest of the kit stays on web3.js.

const V1_MAX_SIZE = 4096;
const V1_MAX_ADDRESSES = 64;
// Measured, not derived: 19 legs is where the 64-address limit bites given the
// collector, system program, and the token programs in use.
const V1_MAX_LEGS = 19;
const LOADED_ACCOUNTS_DATA_SIZE = 64 * 1024 * 1024;
// Any valid-looking blockhash works for a size probe; nothing is sent.
const PROBE_HASH = '11111111111111111111111111111111';

// Same encoding as the legacy builder: [14] unbounded, [14, relocations],
// [14, relocations, seats], plus a trailing nonzero byte to permit closure.
function defragData(budget, seatBudget, mayClose) {
  if (mayClose) {
    const b = Buffer.alloc(10);
    b[0] = 14; b.writeUInt32LE(budget ?? 0xffffffff, 1); b.writeUInt32LE(seatBudget ?? 0xffffffff, 5); b[9] = 1;
    return b;
  }
  if (seatBudget !== undefined) {
    const b = Buffer.alloc(9);
    b[0] = 14; b.writeUInt32LE(budget ?? 0xffffffff, 1); b.writeUInt32LE(seatBudget, 5);
    return b;
  }
  if (budget === undefined) return Buffer.from([14]);
  const b = Buffer.alloc(5); b[0] = 14; b.writeUInt32LE(budget, 1);
  return b;
}

const leg = (m, data) => ({
  programAddress: address(MANIFEST),
  accounts: [
    { address: address(COLLECTOR), role: AccountRole.WRITABLE_SIGNER },
    { address: address(m.pubkey), role: AccountRole.WRITABLE },
    { address: address(m.baseVault), role: AccountRole.WRITABLE },
    { address: address(m.quoteVault), role: AccountRole.WRITABLE },
    { address: address(m.baseTokenProgram), role: AccountRole.READONLY },
    { address: address(m.quoteTokenProgram), role: AccountRole.READONLY },
    { address: address(SYSTEM.toBase58()), role: AccountRole.READONLY },
  ],
  data: new Uint8Array(data),
});

function message(markets, { hash, units, budget, seatBudget, mayClose, signer }) {
  const data = defragData(budget, seatBudget, mayClose);
  return pipe(
    createTransactionMessage({ version: 1 }),
    (m) => (signer
      ? setTransactionMessageFeePayerSigner(signer, m)
      : setTransactionMessageFeePayer(address(COLLECTOR), m)),
    (m) => setTransactionMessageLifetimeUsingBlockhash(
      { blockhash: blockhash(hash), lastValidBlockHeight: 0n }, m),
    (m) => appendTransactionMessageInstructions(markets.map((x) => leg(x, data)), m),
    (m) => setTransactionMessageConfig(
      { computeUnitLimit: units, loadedAccountsDataSizeLimit: LOADED_ACCOUNTS_DATA_SIZE }, m),
  );
}

/// Wire size of a candidate batch, or Infinity when it cannot be built at all.
/// kit throws on both the address limit and the size limit, and for a packer
/// each of those means the same thing: this batch does not fit.
function v1BatchSize(markets, opts = {}) {
  if (markets.length > V1_MAX_LEGS) return Number.POSITIVE_INFINITY;
  try {
    const tx = compileTransaction(message(markets, { ...opts, hash: PROBE_HASH, units: 1_400_000 }));
    return Buffer.from(getBase64EncodedWireTransaction(tx), 'base64').length;
  } catch (e) {
    if (/too large|account addresses|maximum/i.test(e.message)) return Number.POSITIVE_INFINITY;
    throw e;
  }
}

const loadV1Signer = (keypairBytes) => createKeyPairSignerFromBytes(Uint8Array.from(keypairBytes));

/// Unsigned base64 wire transaction, for simulation.
function v1Unsigned(markets, opts) {
  const tx = compileTransaction(message(markets, opts));
  return getBase64EncodedWireTransaction(tx);
}

/// Signed base64 wire transaction, for sending.
async function v1Signed(markets, opts) {
  const signed = await signTransactionMessageWithSigners(message(markets, opts));
  return getBase64EncodedWireTransaction(signed);
}


// ===== program health gauge =====
// Program-wide health gauge, used to abort a long defrag run if it starts
// hurting anybody.
//
// The raw error ratio on the manifest program is useless on its own: one
// arbitrage bot fails well over a thousand transactions in a ten-minute window
// as a matter of course, which put the observed error rate near 50% on a
// market that was provably undisturbed. What matters is how many failures the
// manifest program itself caused, so a bounded sample of the failures is
// fetched and attributed to the program that actually failed.

async function sampleProgramHealth(rpc, { limit = 400, sampleFailures = 12 } = {}) {
  const sigs = await rpc('getSignaturesForAddress', [MANIFEST, { limit, commitment: 'confirmed' }], { retries: 2 })
    .catch(() => []);
  const failures = sigs.filter((s) => s.err !== null);
  // Spread the sample across the window rather than taking the newest N, so a
  // single burst from one bot cannot dominate it.
  const picks = [];
  const stride = Math.max(1, Math.floor(failures.length / Math.max(1, sampleFailures)));
  for (let i = 0; i < failures.length && picks.length < sampleFailures; i += stride) picks.push(failures[i]);

  let attributed = 0;
  const examples = [];
  for (const s of picks) {
    const tx = await rpc('getTransaction', [s.signature, {
      encoding: 'json', maxSupportedTransactionVersion: 1, commitment: 'confirmed',
    }], { retries: 1 }).catch(() => null);
    const line = tx ? [...(tx.meta?.logMessages ?? [])].reverse().find((l) => / failed: /.test(l)) : null;
    const prog = line ? line.split(' ')[1] : null;
    if (prog === MANIFEST) {
      attributed++;
      examples.push({ sig: s.signature, err: JSON.stringify(s.err), log: (line ?? '').slice(0, 120) });
    }
  }
  return {
    total: sigs.length,
    ok: sigs.length - failures.length,
    err: failures.length,
    sampled: picks.length,
    attributed,
    attributedRate: picks.length ? attributed / picks.length : 0,
    newestSlot: sigs[0]?.slot ?? null,
    oldestSlot: sigs.at(-1)?.slot ?? null,
    examples,
  };
}

const describeHealth = (h) =>
  `${h.ok}/${h.total} ok, ${h.err} err, ${h.attributed}/${h.sampled} sampled failures attributed to manifest` +
  (h.newestSlot ? ` (slots ${h.oldestSlot}-${h.newestSlot})` : '');


// ===== runner =====
const execute = process.argv.includes('--execute');
const quietOnly = process.argv.includes('--quiet-only');
const includeActive = process.argv.includes('--include-active');
const all = process.argv.includes('--all');
// Market closure is irreversible and destroys the address; opt in per run.
const allowClose = process.argv.includes('--allow-close');
// Closing a market destroys its address forever: vaults closed, all lamports
// swept, account gone. Some closable markets may have traded recently, so
// closure is deliberately double-gated: --allow-close is not enough on its
// own, a second explicit flag is required, so a market is never closed as a
// side effect of a sweep or a copied command line.
if (allowClose && !process.argv.includes('--yes-destroy-market-addresses')) {
  throw new Error(
    '--allow-close is gated: closing destroys market addresses irreversibly.\n' +
    '  Some closable markets may have traded recently.\n' +
    '  If closures are genuinely intended now, re-run with --yes-destroy-market-addresses as well.',
  );
}
const seatsOnly = process.argv.includes('--seats-only');
// Sweep excess lamports and truncate the existing free tail, touching no
// entry: both budgets zero. Needs the two-budget program release to be live.
const sweepOnly = process.argv.includes('--sweep-only');
const budgetArg = process.argv.find((a) => a.startsWith('--budget='))?.slice(9);
const seatBudgetArg = process.argv.find((a) => a.startsWith('--seat-budget='))?.slice(14);
if ([seatsOnly, sweepOnly, budgetArg !== undefined].filter(Boolean).length > 1) {
  throw new Error('pass only one of --seats-only, --sweep-only, --budget=N');
}
const budget = seatsOnly || sweepOnly ? 0 : budgetArg === undefined ? undefined : Number(budgetArg);
if (budget !== undefined && (!Number.isInteger(budget) || budget < 0)) throw new Error(`invalid --budget: ${budgetArg}`);
const seatBudget = sweepOnly
  ? 0
  : seatBudgetArg === undefined
  ? undefined
  : Number(seatBudgetArg);
if (seatBudget !== undefined && (!Number.isInteger(seatBudget) || seatBudget < 0)) {
  throw new Error(`invalid --seat-budget: ${seatBudgetArg}`);
}
const marketArg = process.argv.find((a) => a.startsWith('--market='))?.slice(9);
// Process at most this many markets this run, highest recovery first. The
// confirmed-market log makes a later run resume rather than repeat, so a
// paced rollout is just the same command again.
const maxMarketsArg = process.argv.find((a) => a.startsWith('--max-markets='))?.slice(14);
const maxMarkets = maxMarketsArg === undefined ? undefined : Number(maxMarketsArg);
if (maxMarkets !== undefined && (!Number.isInteger(maxMarkets) || maxMarkets <= 0)) {
  throw new Error(`invalid --max-markets: ${maxMarketsArg}`);
}
// Batching. Default on for a run that relocates nothing and reclaims nothing,
// because such a run is idempotent and structurally inert: a batch that fails
// costs nothing and can be retried or split. Off by default for anything that
// moves or removes state, where one bad market in a batch would take the
// others down with it and blur which one was at fault.
const noBatch = process.argv.includes('--no-batch');
const forceBatch = process.argv.includes('--batch');
const useV1 = process.argv.includes('--v1');
const batchCapArg = process.argv.find((a) => a.startsWith('--batch-size='))?.slice(13);
const batchCap = batchCapArg === undefined ? (useV1 ? V1_MAX_LEGS : 12) : Number(batchCapArg);
if (!Number.isInteger(batchCap) || batchCap <= 0) throw new Error(`invalid --batch-size: ${batchCapArg}`);
// Legacy transactions cap at 1232 bytes (PACKET_DATA_SIZE), and that is a hard
// runtime limit, not a guideline. The size used for packing is the real
// serialization of the candidate batch and already counts the signature, so
// only a small cushion is needed for compact-u16 array headers.
const WIRE_BUDGET = 1210;
// v1 raises the ceiling to 4096 bytes and 64 addresses. For this workload the
// address count binds first at 19 markets, so the size budget is never the
// constraint and no cushion is needed below it.
const CU_BUDGET = 1_300_000;
const closures = [];
const QUIET_SLOTS = 216_000; // ~1 day
const paceArg = process.argv.find((a) => a.startsWith('--pace='))?.slice(7);
const DELAY_MS = paceArg === undefined ? 3_000 : Number(paceArg);
if (!Number.isFinite(DELAY_MS) || DELAY_MS < 0) throw new Error(`invalid --pace: ${paceArg}`);
// How many transactions between health samples, and what counts as bad enough
// to stop. A single sample is noisy, so two consecutive bad ones are required;
// an absolute floor keeps a 1-of-12 blip from tripping it.
const healthEveryArg = process.argv.find((a) => a.startsWith('--health-every='))?.slice(15);
const HEALTH_EVERY = healthEveryArg === undefined ? 10 : Number(healthEveryArg);
const HEALTH_MIN_ATTRIBUTED = 3;
const HEALTH_RATE_MARGIN = 0.2;
const HEALTH_BAD_STREAK = 2;
const MAX_SEND_ATTEMPTS = 3;
const MIN_RECOVERY_LAMPORTS = 100_000; // skip dust
// Only take markets whose free tail is at least this many blocks, i.e. where
// the sweep genuinely shrinks the account rather than just collecting fees.
const minShrinkArg = process.argv.find((a) => a.startsWith('--min-shrink='))?.slice(13);
const minShrinkBlocks = minShrinkArg === undefined ? 0 : Number(minShrinkArg);
if (!Number.isInteger(minShrinkBlocks) || minShrinkBlocks < 0) throw new Error(`invalid --min-shrink: ${minShrinkArg}`);

// The resume log records a market as done, but says nothing about WHICH
// operation was done to it. Stage 2 (sweep-only) and Stage 3 (relocation and
// seat reclamation) act on the same markets with different parameters, so
// sharing one log makes every market swept in Stage 2 invisible to Stage 3 -
// a full-compaction estimate run against the Stage 2 log saw only 82 of 4230
// markets and reported 0.487 SOL, which was pure artifact. Each stage gets its
// own log file. Entries now also record the mode they were performed under, so
// a log can be audited for what it actually represents.
const logArg = process.argv.find((a) => a.startsWith('--log='))?.slice(6);
const logPath = path.join(stateDir, logArg ?? 'defrag-markets-log.jsonl');
const modeKey = JSON.stringify({ budget, seatBudget, allowClose });
// Mode-aware resume. "Done" has to mean "done under THESE parameters": round
// 3a (budget 1, seats 1) and round 3b (budget 50, seats 10) act on the same
// market, so a per-market filter made 3b silently select zero markets after 3a
// had logged it. Entries written before mode was recorded have no mode field
// and are treated as done for any mode, which preserves the Stage 2 resume
// behaviour its log was built under.
const ignoreLog = process.argv.includes('--ignore-log');
const done = new Set(
  fs.existsSync(logPath) && !ignoreLog
    ? fs.readFileSync(logPath, 'utf8').trim().split('\n').filter(Boolean)
        .map((l) => JSON.parse(l))
        .filter((e) => e.status === 'confirmed' && (e.mode === undefined || e.mode === modeKey))
        .map((e) => e.market)
    : [],
);

let markets = await resolveTokenPrograms(rpc, await listMarkets(rpc));
const modeLabel = budget === undefined
  ? `FULL compaction (unbounded${seatBudget === undefined ? '' : `, seats capped at ${seatBudget}`})`
  : budget === 0 && seatBudget === 0
  ? 'SWEEP-ONLY (collect excess and truncate the free tail, touch nothing)'
  : budget === 0
  ? `SEATS-ONLY (relocate nothing, reclaim ${seatBudget === undefined ? 'every' : seatBudget} empty seat${seatBudget === 1 ? '' : 's'})`
  : `BOUNDED run (relocations ${budget}${seatBudget === undefined ? '' : `, seats ${seatBudget}`})`;
console.log(`mode: ${modeLabel} | ${execute ? 'EXECUTE' : 'DRY RUN'} | closures ${allowClose ? 'INCLUDED' : 'excluded'}`);
console.log(`markets on mainnet: ${markets.length}${done.size ? ` (${done.size} already done in log)` : ''}`);
// An explicit market set, so a screened list can be run without pasting
// hundreds of addresses. Accepts either a bare JSON array of addresses or the
// output of 09-safe-candidates.mjs, in which case only TIER A is taken: tier B
// holds resting orders that compaction would relocate, and feeding it here by
// accident is exactly the mistake this flag should not enable.
const listArg = process.argv.find((a) => a.startsWith('--market-list='))?.slice(14);
if (listArg) {
  const raw = JSON.parse(fs.readFileSync(path.isAbsolute(listArg) ? listArg : path.join(process.cwd(), listArg), 'utf8'));
  const wanted = new Set(
    Array.isArray(raw)
      ? raw.map((e) => (typeof e === 'string' ? e : e.pubkey))
      : (raw.tierA ?? []).map((e) => e.pubkey),
  );
  if (!wanted.size) throw new Error(`${listArg} contained no markets (tier A empty?)`);
  const before = markets.length;
  markets = markets.filter((m) => wanted.has(m.pubkey));
  console.log(`--market-list: ${wanted.size} requested, ${markets.length} matched of ${before} on chain`
    + (markets.length < wanted.size ? ` (${wanted.size - markets.length} missing - closed or renamed since the screen)` : ''));
  if (!markets.length) throw new Error('no requested market exists on chain');
} else if (marketArg) {
  markets = markets.filter((m) => m.pubkey === marketArg);
  if (!markets.length) throw new Error(`${marketArg} is not a market`);
} else if (!all && execute) {
  throw new Error('choose --market=ADDR, --market-list=FILE or --all for execute');
}
markets = markets.filter((m) => !done.has(m.pubkey));

const currentSlot = await rpc('getSlot', [{ commitment: 'confirmed' }]);

// Simulating every candidate costs one RPC round trip per market, which is
// around nine minutes over four thousand markets with nothing printed. That is
// indistinguishable from a hang, and it is paid again on every resume, so the
// result is cached and progress is reported.
//
// A cached plan is only safe because each batch is simulated again immediately
// before it is sent: the cache can go stale in the direction of wrong recovery
// figures or wrong compute estimates, and the pre-send simulation catches both.
// It is still refused once it is older than the freshness window, and it is
// keyed by the mode, since postSpace and recovered depend on the budgets.
const planCachePath = path.join(stateDir, 'plan-cache.json');
const PLAN_CACHE_MAX_AGE_MS = 60 * 60_000;
const useCachedPlan = process.argv.includes('--use-cached-plan');
const plan = [];
let planFromCache = false;
if (useCachedPlan) {
  if (!fs.existsSync(planCachePath)) throw new Error(`--use-cached-plan given but ${planCachePath} does not exist`);
  const cache = JSON.parse(fs.readFileSync(planCachePath, 'utf8'));
  const ageMs = Date.now() - Date.parse(cache.at);
  if (cache.modeKey !== modeKey) throw new Error(`cached plan is for mode ${cache.modeKey}, this run is ${modeKey}`);
  if (ageMs > PLAN_CACHE_MAX_AGE_MS) {
    throw new Error(`cached plan is ${(ageMs / 60_000).toFixed(0)} min old (limit ${PLAN_CACHE_MAX_AGE_MS / 60_000}); re-run without --use-cached-plan`);
  }
  // Re-filter against the log, so markets swept since the cache was written
  // are not attempted twice, and drop anything excluded by this run's flags.
  const fresh = cache.plan.filter((p) => !done.has(p.pubkey) && (!marketArg || p.pubkey === marketArg));
  plan.push(...fresh);
  planFromCache = true;
  console.log(
    `using cached plan from ${cache.at} (slot ${cache.slot}, ${(ageMs / 60_000).toFixed(1)} min old): ` +
      `${cache.plan.length} markets, ${cache.plan.length - fresh.length} dropped as already done or filtered out`,
  );
}

// Simulate every candidate; collect recovery and activity.
if (!planFromCache) for (const [n, m] of markets.entries()) {
  if (n > 0 && n % 500 === 0) {
    console.log(`  simulated ${n}/${markets.length} markets, ${plan.length} with something to recover`);
  }
  const tx = buildDefragTx(m, { budget, seatBudget, mayClose: allowClose });
  const b64 = tx.serialize({ requireAllSignatures: false, verifySignatures: false }).toString('base64');
  const sim = await rpc('simulateTransaction', [b64, {
    encoding: 'base64', sigVerify: false, replaceRecentBlockhash: true, commitment: 'confirmed',
    accounts: { encoding: 'base64', addresses: [m.pubkey] },
  }], { retries: 2 }).catch((e) => ({ value: { err: `rpc-error: ${e.message}` } }));
  if (sim.value.err) {
    console.log(`  SIM FAIL ${m.pubkey} (${m.space}B): ${JSON.stringify(sim.value.err)}`);
    continue;
  }
  const post = sim.value.accounts[0];
  const postSpace = Buffer.from(post.data[0], 'base64').length;
  const recovered = m.lamports - post.lamports;
  // Defrag CLOSES a market outright when nothing survives compaction and both
  // vaults are empty: vaults closed, all lamports swept, market account gone,
  // address destroyed forever. The program's test is emptiness right now, not
  // whether the market is in use, so a live market whose maker has pulled
  // quotes qualifies. Never in a sweep unless the operator opts in per run.
  // With --allow-close the program is told it may retire the market, so a
  // simulated closure is what the run would really do. Without it the program
  // leaves the market standing, and this only records which ones qualified.
  if (postSpace === 0 && post.lamports === 0) {
    closures.push({ pubkey: m.pubkey, space: m.space, lamports: m.lamports });
    if (!allowClose) continue;
  }
  // Never pay to defrag. The processor expands a market that has fewer than
  // two free blocks before compacting it (`free_blocks_short_of_n(2)` ->
  // `batch_expand_market`), because `defragment_with` asserts two spare nodes.
  // On a tightly packed market there is then no free tail to truncate, so the
  // account just stays one block bigger and the COLLECTOR funds the rent:
  // about 406,400 lamports a time. The old filter let these through because it
  // only skipped markets whose size was unchanged, and these do change size -
  // upward. 73 markets across three sweeps cost 0.0385 SOL this way.
  if (recovered <= 0) continue;
  // Target only markets where the run actually truncates a free tail. Those
  // are the deterministic wins: the account shrinks, the rent comes back, and
  // because a sweep never relocates, nothing a client references moves. A
  // market with no free tail yields only its accrued fees, which is still
  // profit but a different (smaller, size-neutral) proposition. The floor is
  // keep_count+2, so a densely packed market with a 5-block tail lands on
  // exactly 2 free blocks; one with interior holes keeps them, because
  // `frontier` dominates the floor and only relocation could gather them.
  if (minShrinkBlocks > 0 && (m.space - postSpace) / MARKET_BLOCK_SIZE < minShrinkBlocks) continue;
  if (recovered < MIN_RECOVERY_LAMPORTS && postSpace === m.space) continue;
  const sigs = await rpc('getSignaturesForAddress', [m.pubkey, { limit: 1, commitment: 'confirmed' }]);
  const lastSlot = sigs[0]?.slot ?? 0;
  const active = currentSlot - lastSlot < QUIET_SLOTS;
  plan.push({ ...m, postSpace, recovered, cu: sim.value.unitsConsumed, active });
}
plan.sort((a, b) => b.recovered - a.recovered);
if (!planFromCache) {
  fs.writeFileSync(planCachePath, JSON.stringify({ at: new Date().toISOString(), slot: currentSlot, modeKey, plan }, null, 2));
  console.log(`wrote ${path.join(stateDir, 'plan-cache.json')} (${plan.length} markets); a resume within ${PLAN_CACHE_MAX_AGE_MS / 60_000} min can skip this phase with --use-cached-plan`);
}

if (closures.length) {
  const closureSol = closures.reduce((s, c) => s + c.lamports, 0) / 1e9;
  console.log(`\n${closures.length} markets would be CLOSED by defrag (${closureSol.toFixed(3)} SOL, addresses destroyed irreversibly)`);
  console.log(allowClose
    ? '  --allow-close given: these ARE included below.'
    : '  excluded (default). Re-run with --allow-close to include them, after confirming none are in use.');
}
const eligible = plan.filter((p) => (quietOnly ? !p.active : true));
const selected = maxMarkets === undefined ? eligible : eligible.slice(0, maxMarkets);
const totalRecovery = selected.reduce((s, p) => s + p.recovered, 0);
console.log(`\n${plan.length} markets have something to recover; selected ${selected.length}${quietOnly ? ' (quiet only)' : ''}`);
if (maxMarkets !== undefined && eligible.length > selected.length) {
  const deferred = eligible.slice(maxMarkets);
  console.log(
    `  --max-markets=${maxMarkets}: deferring ${deferred.length} markets holding ` +
      `${(deferred.reduce((s, p) => s + p.recovered, 0) / 1e9).toFixed(3)} SOL. ` +
      `Re-run to continue; the log skips what already landed.`,
  );
}
console.log(`cumulative recovery (market lamports): ${(totalRecovery / 1e9).toFixed(3)} SOL`);
console.table(selected.slice(0, 25).map((p) => ({
  market: p.pubkey, bytes: p.space, defragged: p.postSpace,
  recoverSOL: (p.recovered / 1e9).toFixed(4), cu: p.cu, active: p.active ? 'ACTIVE' : 'quiet',
})));
if (selected.length > 25) console.log(`  (${selected.length - 25} more omitted)`);
const activeCount = selected.filter((p) => p.active).length;
// The active-market gate exists for the two things that can disturb a live
// maker: reclaiming a seat, and relocating a node out from under a cached
// index. A run that does neither - both budgets zero - only collects excess
// lamports and truncates an already-free tail, so whether a market is busy is
// irrelevant and demanding --include-active would just train the operator to
// pass it reflexively where it does matter.
const canDisturb = !(budget === 0 && seatBudget === 0);
if (activeCount && execute && !canDisturb) {
  console.log(`\n${activeCount} selected markets are ACTIVE, which does not gate this mode: it relocates nothing and reclaims nothing.`);
}
if (activeCount && execute && !includeActive && !marketArg && canDisturb) {
  console.log(`\n${activeCount} selected markets are ACTIVE. Re-run with --quiet-only to skip them or --include-active to proceed (warn MM operators first).`);
  process.exit(1);
}

// Pack markets into transactions. The compute cost used is the measured cost
// from each market's own simulation above, not an estimate, with a margin
// because the real run happens a few slots later against slightly different
// state. A market whose own cost exceeds the budget still gets a transaction
// to itself: that is the isolated-big-defrag case, and it is accepted rather
// than skipped. Wire size is checked by serializing the candidate, because
// these are legacy transactions capped at 1232 bytes and each market
// contributes three addresses, so size binds well before the account limit.
const batchingOn = forceBatch || (!noBatch && !canDisturb);
const CU_MARGIN = 1.3;
const cuOf = (arr) => arr.reduce((s, p) => s + (p.cu ?? estimateCu(p.space)), 0);
const unitsFor = (arr) => Math.min(1_400_000, Math.ceil(cuOf(arr) * CU_MARGIN) + 5_000);
const txOpts = { budget, seatBudget, mayClose: allowClose };
const wireSize = (arr) => (useV1 ? v1BatchSize(arr, txOpts) : batchWireSize(arr, txOpts));
const wireLimit = useV1 ? V1_MAX_SIZE : WIRE_BUDGET;
const legCap = useV1 ? Math.min(batchCap, V1_MAX_LEGS) : batchCap;
function packBatches(list) {
  const out = [];
  let cur = [];
  for (const p of list) {
    if (cur.length === 0) { cur = [p]; continue; }
    const cand = [...cur, p];
    const fits = cand.length <= legCap
      && unitsFor(cand) <= CU_BUDGET
      && wireSize(cand) <= wireLimit;
    if (fits) cur = cand;
    else { out.push(cur); cur = [p]; }
  }
  if (cur.length) out.push(cur);
  return out;
}
const batches = batchingOn ? packBatches(selected) : selected.map((p) => [p]);
if (batchingOn && selected.length) {
  const solo = batches.filter((b) => b.length === 1).length;
  console.log(
    `\nbatching ON (${useV1 ? 'v1' : 'legacy'} transactions): ${selected.length} markets in ${batches.length} transactions ` +
      `(largest ${Math.max(...batches.map((b) => b.length))} markets, ${solo} isolated), ` +
      `caps ${legCap}/tx, ${wireLimit}B wire, ${(CU_BUDGET / 1e6).toFixed(1)}M CU. ` +
      `Each batch is simulated before it is sent; a batch that fails is split in half and retried, not abandoned.`,
  );
  console.table(batches.slice(0, 10).map((b, n) => ({
    tx: n + 1, markets: b.length, cuMeasured: cuOf(b), cuRequested: unitsFor(b),
    wireBytes: wireSize(b),
    recoverSOL: (b.reduce((s, p) => s + p.recovered, 0) / 1e9).toFixed(4),
  })));
  if (batches.length > 10) console.log(`  (${batches.length - 10} more transactions omitted)`);
} else if (!noBatch && selected.length) {
  console.log('\nbatching OFF for this mode: it relocates or reclaims state, so a failure must name one market. Pass --batch to override.');
}

if (!execute) {
  // Emit each selected defrag as an inspectable transaction so it can be
  // decoded and simulated independently in Solana Explorer. A real recent
  // blockhash is used so the explorer's own simulation runs; the transaction
  // is unsigned, which is fine because inspector simulation does not verify
  // signatures.
  const { blockhash } = (await rpc('getLatestBlockhash', [{ commitment: 'confirmed' }])).value;

  // Prove the packing before anything is signed: simulate the largest batch by
  // market count and the most expensive one by measured compute. If those two
  // fit, the rest do by construction, and the execute path re-simulates every
  // batch anyway.
  if (batchingOn && batches.length) {
    const widest = batches.reduce((a, b) => (b.length > a.length ? b : a));
    const priciest = batches.reduce((a, b) => (cuOf(b) > cuOf(a) ? b : a));
    for (const [why, b] of [['widest', widest], ['most expensive', priciest]]) {
      if (why === 'most expensive' && b === widest) continue;
      const units = unitsFor(b);
      const b64 = useV1
        ? v1Unsigned(b, { ...txOpts, hash: blockhash, units })
        : buildBatchTx(b, { ...txOpts, blockhash, units })
            .serialize({ requireAllSignatures: false, verifySignatures: false }).toString('base64');
      const sim = await rpc('simulateTransaction', [b64, {
        encoding: 'base64', sigVerify: false, replaceRecentBlockhash: true, commitment: 'confirmed',
      }], { retries: 2 }).catch((e) => ({ value: { err: `rpc-error: ${e.message}` } }));
      console.log(
        `\n${why} batch: ${b.length} markets, ${Buffer.from(b64, 'base64').length}B wire, ${units} CU requested -> ` +
          (sim.value.err
            ? `SIMULATION FAILED ${JSON.stringify(sim.value.err)} (execute would split this batch and retry)`
            : `OK, ${sim.value.unitsConsumed} CU consumed`),
      );
      if (!sim.value.err && b.length > 1 && !useV1) {
        const tx = buildBatchTx(b, { ...txOpts, blockhash, units });
        console.log(`  inspector: https://explorer.solana.com/tx/inspector?message=${encodeURIComponent(tx.serializeMessage().toString('base64'))}`);
      }
    }
  }

  const INSPECT_CAP = 5;
  const show = selected.slice(0, INSPECT_CAP);
  const payloads = [];
  for (const p of show) {
    const tx = buildDefragTx(p, { blockhash, budget, seatBudget, mayClose: allowClose });
    const message = tx.serializeMessage().toString('base64');
    const transaction = tx.serialize({ requireAllSignatures: false, verifySignatures: false }).toString('base64');
    const url = `https://explorer.solana.com/tx/inspector?message=${encodeURIComponent(message)}`;
    payloads.push({ market: p.pubkey, mode: budget === undefined ? 'full' : `budget ${budget}`, blockhash, message, transaction, url });
    console.log(`\n--- ${p.pubkey} (${modeLabel.split(' (')[0]}${allowClose ? ', MAY CLOSE' : ''}) ---`);
    console.log('Explorer inspector link:');
    console.log(url);
    console.log('\nIf that link does not load, open https://explorer.solana.com/tx/inspector');
    console.log('and paste this base64 transaction:');
    console.log(transaction);
  }
  if (selected.length > INSPECT_CAP) {
    console.log(`\n(${selected.length - INSPECT_CAP} further selected markets not printed; use --market=ADDR for one at a time)`);
  }
  fs.writeFileSync(path.join(stateDir, 'inspect-payloads.json'), JSON.stringify({ blockhash, payloads }, null, 2));
  console.log(`\nwrote ${path.join(stateDir, 'inspect-payloads.json')}`);
  console.log('Dry run only. Add --execute to submit.');
  process.exit(0);
}

if (!KEYPAIR_PATH) throw new Error('set --keypair=PATH or KEYPAIR_PATH; the collector keypair is required to execute');
const keypairBytes = Uint8Array.from(JSON.parse(fs.readFileSync(KEYPAIR_PATH, 'utf8')));
const signer = Keypair.fromSecretKey(keypairBytes);
if (signer.publicKey.toBase58() !== COLLECTOR) {
  throw new Error(`${KEYPAIR_PATH} is ${signer.publicKey.toBase58()}, not the collector ${COLLECTOR}`);
}
console.log(`signing with ${signer.publicKey.toBase58()} (${KEYPAIR_PATH})`);
// kit needs its own signer object; the web3.js keypair above still gates the
// collector check, so both paths verify the same key before anything is sent.
const v1Signer = useV1 ? await loadV1Signer(keypairBytes) : null;
if (useV1 && v1Signer.address !== COLLECTOR) {
  throw new Error(`v1 signer resolved to ${v1Signer.address}, not the collector ${COLLECTOR}`);
}

// Baseline the program's health before touching anything, so the comparison
// during the run is against how mainnet actually looked beforehand rather than
// against an assumption that it was quiet.
const baseline = await sampleProgramHealth(rpc);
console.log(`\nhealth baseline: ${describeHealth(baseline)}`);
if (baseline.attributed > 0) {
  console.log('  NOTE: manifest-attributed failures exist before this run; the abort gate compares against that level.');
  for (const e of baseline.examples.slice(0, 3)) console.log(`    ${e.sig} ${e.err}`);
}
const abortPath = path.join(stateDir, 'defrag-abort.json');
let badStreak = 0;

let doneCount = 0, recoveredTotal = 0, sent = 0;
const queue = batches.slice();
while (queue.length) {
  const batch = queue.shift();
  const label = batch.length === 1 ? batch[0].pubkey : `batch of ${batch.length}`;
  const units = unitsFor(batch);

  // Simulate before sending. A batch that cannot fit is split in half and both
  // halves requeued, which isolates the offending market in a few rounds
  // instead of dropping the whole batch. A lone market that fails simulation
  // is a real problem and stops the run.
  const { blockhash: simHash } = (await rpc('getLatestBlockhash', [{ commitment: 'confirmed' }])).value;
  const probeB64 = useV1
    ? v1Unsigned(batch, { ...txOpts, hash: simHash, units })
    : buildBatchTx(batch, { ...txOpts, blockhash: simHash, units })
        .serialize({ requireAllSignatures: false, verifySignatures: false }).toString('base64');
  const probeSim = await rpc('simulateTransaction', [probeB64,
    { encoding: 'base64', sigVerify: false, replaceRecentBlockhash: true, commitment: 'confirmed' },
  ], { retries: 2 }).catch((e) => ({ value: { err: `rpc-error: ${e.message}` } }));
  if (probeSim.value.err) {
    if (batch.length > 1) {
      const mid = Math.ceil(batch.length / 2);
      console.warn(`  ${label} failed simulation (${JSON.stringify(probeSim.value.err)}); splitting ${mid}+${batch.length - mid} and retrying`);
      queue.unshift(batch.slice(0, mid), batch.slice(mid));
      continue;
    }
    console.error(`STOP at ${batch[0].pubkey}: simulation failed ${JSON.stringify(probeSim.value.err)}`);
    fs.appendFileSync(logPath, JSON.stringify({ market: batch[0].pubkey, status: 'failed', error: `simulation: ${JSON.stringify(probeSim.value.err)}`, at: new Date().toISOString() }) + '\n');
    process.exit(1);
  }
  if (batch.length > 1) {
    console.log(`  ${label} simulated clean: ${probeSim.value.unitsConsumed} CU of ${units} requested, ${Buffer.from(probeB64, 'base64').length}B wire`);
  }

  let landed = false, sig;
  for (let attempt = 1; attempt <= MAX_SEND_ATTEMPTS && !landed; attempt++) {
    const { blockhash, lastValidBlockHeight } = (await rpc('getLatestBlockhash', [{ commitment: 'confirmed' }])).value;
    let b64;
    if (useV1) {
      b64 = await v1Signed(batch, { ...txOpts, hash: blockhash, units, signer: v1Signer });
    } else {
      const tx = buildBatchTx(batch, { ...txOpts, blockhash, units });
      tx.sign(signer);
      b64 = tx.serialize().toString('base64');
    }
    try {
      sig = await rpc('sendTransaction', [b64, { encoding: 'base64', maxRetries: 3, preflightCommitment: 'confirmed', skipPreflight: false }]);
      for (let w = 0; w < 40; w++) {
        const st = (await rpc('getSignatureStatuses', [[sig]])).value[0];
        if (st && (st.confirmationStatus === 'confirmed' || st.confirmationStatus === 'finalized')) {
          if (st.err) throw new Error(`landed with error: ${JSON.stringify(st.err)}`);
          landed = true;
          break;
        }
        const h = (await rpc('getBlockHeight', [{ commitment: 'confirmed' }]));
        if (h > lastValidBlockHeight) break;
        await new Promise((r) => setTimeout(r, 1500));
      }
      if (!landed && attempt < MAX_SEND_ATTEMPTS) {
        const st = (await rpc('getSignatureStatuses', [[sig], { searchTransactionHistory: true }])).value[0];
        if (st && !st.err) { landed = true; break; }
        if (st && st.err) throw new Error(`landed with error: ${JSON.stringify(st.err)}`);
        console.warn(`  ${label}: expired without landing, retrying (${attempt + 1}/${MAX_SEND_ATTEMPTS})`);
        continue;
      }
    } catch (e) {
      console.error(`STOP at ${label}: ${e.message}`);
      for (const p of batch) {
        fs.appendFileSync(logPath, JSON.stringify({ market: p.pubkey, status: 'failed', error: e.message, batch: batch.length, at: new Date().toISOString() }) + '\n');
      }
      process.exit(1);
    }
  }
  if (!landed) { console.error(`STOP: ${label} never landed after ${MAX_SEND_ATTEMPTS} attempts`); process.exit(1); }

  sent++;
  const batchRecovery = batch.reduce((s, p) => s + p.recovered, 0);
  for (const p of batch) {
    doneCount++; recoveredTotal += p.recovered;
    fs.appendFileSync(logPath, JSON.stringify({ market: p.pubkey, status: 'confirmed', sig, pre: p.space, post: p.postSpace, recovered: p.recovered, batch: batch.length, mode: modeKey, at: new Date().toISOString() }) + '\n');
  }
  console.log(
    `confirmed ${doneCount}/${selected.length} markets (tx ${sent}): ` +
      (batch.length === 1
        ? `${batch[0].pubkey} ${batch[0].space}->${batch[0].postSpace}B, `
        : `${batch.length} markets, `) +
      `${(batchRecovery / 1e9).toFixed(4)} SOL, ${sig}`,
  );
  // Periodic health gate. Stopping a sweep-only run costs nothing: every
  // market already swept keeps its recovery, the log makes the rest
  // resumable, and nothing is left half-done because each transaction is
  // atomic. So the gate is allowed to be twitchy in the safe direction.
  if (HEALTH_EVERY > 0 && sent % HEALTH_EVERY === 0 && queue.length) {
    const h = await sampleProgramHealth(rpc);
    const worse = h.attributed >= HEALTH_MIN_ATTRIBUTED
      && h.attributedRate > baseline.attributedRate + HEALTH_RATE_MARGIN;
    badStreak = worse ? badStreak + 1 : 0;
    console.log(`  health after ${sent} tx: ${describeHealth(h)}${worse ? `  <-- WORSE than baseline (streak ${badStreak}/${HEALTH_BAD_STREAK})` : ''}`);
    if (worse) for (const e of h.examples.slice(0, 3)) console.log(`    ${e.sig} ${e.err} ${e.log}`);
    if (badStreak >= HEALTH_BAD_STREAK) {
      const remaining = queue.reduce((s, b) => s + b.reduce((t, p) => t + p.recovered, 0), 0);
      fs.writeFileSync(abortPath, JSON.stringify({
        stoppedAfterTx: sent, marketsDone: doneCount, recovered: recoveredTotal,
        remainingMarkets: queue.flat().length, remainingLamports: remaining,
        baseline, latest: h, at: new Date().toISOString(),
      }, null, 2));
      console.error(
        `\nABORTING: manifest-attributed failures rose above baseline on ${HEALTH_BAD_STREAK} consecutive checks.\n` +
          `  baseline ${baseline.attributed}/${baseline.sampled}, now ${h.attributed}/${h.sampled}.\n` +
          `  ${doneCount} markets swept for ${(recoveredTotal / 1e9).toFixed(3)} SOL; ` +
          `${queue.flat().length} markets holding ${(remaining / 1e9).toFixed(3)} SOL left untouched.\n` +
          `  wrote ${abortPath}. Investigate before resuming; the log skips what already landed.`,
      );
      process.exit(2);
    }
  }
  if (queue.length) await new Promise((r) => setTimeout(r, DELAY_MS));
}
console.log(`\nDefragged ${doneCount}/${selected.length} markets in ${sent} transactions, recovered ${(recoveredTotal / 1e9).toFixed(3)} SOL (market lamports; vault excess sweeps additionally to collector token accounts).`);

