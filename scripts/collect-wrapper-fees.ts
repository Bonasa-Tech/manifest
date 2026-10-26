import 'dotenv/config';

import {
  AccountRole,
  address,
  appendTransactionMessageInstructions,
  blockhash,
  compileTransaction,
  createKeyPairSignerFromBytes,
  createTransactionMessage,
  getBase64EncodedWireTransaction,
  pipe,
  setTransactionMessageConfig,
  setTransactionMessageFeePayer,
  setTransactionMessageFeePayerSigner,
  setTransactionMessageLifetimeUsingBlockhash,
  signTransactionMessageWithSigners,
  type Instruction,
  type KeyPairSigner,
} from '@solana/kit';
import { Connection, PublicKey } from '@solana/web3.js';
import bs58 from 'bs58';
import * as fs from 'fs';
import { PROGRAM_ID as WRAPPER_PROGRAM_ID } from '../client/ts/src/wrapper';
import {
  DEFAULT_MAX_UNBACKED_SIMS,
  createUnbackedTracker,
  runCoreCollections,
} from './collector-core';

const { RPC_URL, KEYPAIR_PATH } = process.env;

// One tracker for the whole process, so --loop passes can tell an order that
// has rested unchanged since the last pass from a fast-churning one. Only
// orders seen across passes are cleaned (see collector-core's UnbackedTracker).
// A single (non-loop) invocation has only one pass, so it sets the baseline and
// cleans no unbacked orders; use --loop for unbacked cleaning. Set
// UNBACKED_STATE_PATH to persist the baseline across restarts.
const unbackedTracker = createUnbackedTracker();
const UNBACKED_STATE_PATH = process.env.UNBACKED_STATE_PATH;

// Best effort: a missing or unreadable state file just means a fresh baseline.
function loadUnbackedState(): void {
  if (UNBACKED_STATE_PATH === undefined) return;
  try {
    const raw = fs.readFileSync(UNBACKED_STATE_PATH, 'utf8');
    const parsed: unknown = JSON.parse(raw);
    if (parsed !== null && typeof parsed === 'object') {
      unbackedTracker.streaks = new Map(
        Object.entries(parsed as Record<string, number>).filter(
          ([, n]) => typeof n === 'number' && Number.isFinite(n),
        ),
      );
      console.log(
        `Loaded ${unbackedTracker.streaks.size} unbacked-order streaks from ${UNBACKED_STATE_PATH}`,
      );
    }
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ENOENT') {
      console.warn(
        `Could not read ${UNBACKED_STATE_PATH}: ${error instanceof Error ? error.message : error}`,
      );
    }
  }
}

function saveUnbackedState(): void {
  if (UNBACKED_STATE_PATH === undefined) return;
  try {
    fs.writeFileSync(
      UNBACKED_STATE_PATH,
      JSON.stringify(Object.fromEntries(unbackedTracker.streaks)),
    );
  } catch (error) {
    console.warn(
      `Could not write ${UNBACKED_STATE_PATH}: ${error instanceof Error ? error.message : error}`,
    );
  }
}

const AUTHORIZED_COLLECTOR = new PublicKey(
  'B6dmr2UAn2wgjdm3T4N1Vjd8oPYRRTguByW7AEngkeL6',
);
const SYSTEM_PROGRAM = '11111111111111111111111111111111';
const COLLECT_DISCRIMINANT = 7;
const DEFRAG_DISCRIMINANT = 8;
// Wrapper state layout: fixed header then 96-byte blocks. Free-list nodes
// store their next index in the first four bytes of the block.
const WRAPPER_FIXED_SIZE = 64;
const WRAPPER_BLOCK_SIZE = 96;
const NIL = 0xffffffff;
// Full-defrag compute scales with account size: the mark walk, the relocation
// scan and the free-list rebuild each touch every block once, and relocations
// are bounded by the program's own per-run clamp. The per-byte term carries
// the whole estimate; the base only covers per-instruction overhead.
//
// Calibrated against the 2026-10-22 mainnet sweep (421 wrappers, 28 txs, every
// batch simulated). Worst observed cost was 1.46 CU/byte, on the 3-wrapper
// batch of 30,144 total bytes; the 618,304-byte wrapper came in at 0.92
// CU/byte, and the smallest already-packed wrappers at 1,155 CU each. 2.5/byte
// keeps a 1.7x margin over the worst case, and because the estimate is at
// least DEFRAG_CU_PER_BYTE per byte while real cost stays below it, a batch
// that fits CU_BUDGET_PER_TRANSACTION cannot exceed the MAX_COMPUTE_UNITS the
// transaction requests.
//
// The base was 60,000 through the 2026-10-22 sweep, which is 50x the measured
// cost of an already-packed wrapper. Since 1_300_000/60_000 = 21, it capped
// every batch of small wrappers at 21 regardless of real cost, short of the 32
// the 64-instruction limit allows in --defrag mode. 2,500 covers the measured
// 1,155 floor with margin and lets the per-byte term decide the batch size.
const DEFRAG_CU_BASE = 2_500;
const DEFRAG_CU_PER_BYTE = 2.5;
const CU_BUDGET_PER_TRANSACTION = 1_300_000;
// Wrapper state discriminator: little-endian u64 value 1.
const WRAPPER_STATE_DISCRIMINATOR_BASE58 = 'Ahg1opVcGX';
const SIZE_CHECK_BLOCKHASH = '11111111111111111111111111111111';
const V1_MAX_TRANSACTION_SIZE = 4096;
const V1_MAX_ACCOUNT_ADDRESSES = 64;
// collector + System Program + wrapper program are shared by every instruction.
const V1_MAX_WRAPPERS = V1_MAX_ACCOUNT_ADDRESSES - 3;
const MAX_COMPUTE_UNITS = 1_400_000;
const MAX_LOADED_ACCOUNTS_DATA_SIZE = 64 * 1024 * 1024;
const DEFAULT_DELAY_MS = 1_000;
// How many fresh-blockhash attempts a batch gets when its transaction expires
// without landing. Expiry means the transaction can never land, so resending
// the batch is safe; a landed-check runs before each retry to catch the
// last-valid-block race. No priority fee: landing is best effort, retried.
const MAX_SEND_ATTEMPTS = 3;

type CollectableWrapper = {
  pubkey: PublicKey;
  trader: PublicKey;
  amount: bigint;
  space: number;
  // Set in --defrag mode: the size defrag packs the account down to
  // (live blocks only), and the compute estimate for the defrag run.
  predictedSpace?: number;
  defragCu?: number;
};

type RpcResponse<T> = {
  id: number;
  result?: T;
  error?: unknown;
};

type SimulationResult = {
  value: {
    err: unknown;
    logs?: string[] | null;
    unitsConsumed?: number;
    loadedAccountsDataSize?: number;
  };
};

async function rpc<T>(method: string, params: unknown[]): Promise<T> {
  const response = await fetch(RPC_URL!, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
  });
  if (!response.ok) {
    throw new Error(
      `${method} RPC failed with HTTP ${response.status}: ${await response.text()}`,
    );
  }
  const json = (await response.json()) as RpcResponse<T>;
  if (json.error !== undefined || json.result === undefined) {
    throw new Error(`${method} RPC failed: ${JSON.stringify(json.error)}`);
  }
  return json.result;
}

async function getRentExemptBalances(
  accountLengths: number[],
): Promise<Map<number, bigint>> {
  const requests = accountLengths.map((length, index) => ({
    jsonrpc: '2.0',
    id: index + 1,
    method: 'getMinimumBalanceForRentExemption',
    params: [length, { commitment: 'confirmed' }],
  }));
  const response = await fetch(RPC_URL!, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(requests),
  });
  if (!response.ok) {
    throw new Error(
      `Rent-exemption RPC failed with HTTP ${response.status}: ${await response.text()}`,
    );
  }
  const json = (await response.json()) as RpcResponse<number>[];
  if (!Array.isArray(json)) {
    throw new Error('Rent-exemption RPC returned a non-batch response');
  }
  const rentById = new Map<number, bigint>();
  for (const item of json) {
    if (item.error !== undefined || item.result === undefined) {
      throw new Error(
        `Rent-exemption RPC item ${item.id} failed: ${JSON.stringify(item.error)}`,
      );
    }
    rentById.set(item.id, BigInt(item.result));
  }
  return new Map(
    accountLengths.map((length, index) => {
      const rent = rentById.get(index + 1);
      if (rent === undefined) {
        throw new Error(`Rent-exemption RPC omitted account length ${length}`);
      }
      return [length, rent];
    }),
  );
}

function createCollectInstruction(
  wrapperState: PublicKey,
  collector: PublicKey,
): Instruction {
  return {
    accounts: [
      {
        address: address(wrapperState.toBase58()),
        role: AccountRole.WRITABLE,
      },
      { address: address(SYSTEM_PROGRAM), role: AccountRole.READONLY },
      {
        address: address(collector.toBase58()),
        role: AccountRole.WRITABLE_SIGNER,
      },
    ],
    programAddress: address(WRAPPER_PROGRAM_ID.toBase58()),
    data: new Uint8Array([COLLECT_DISCRIMINANT]),
  };
}

function createDefragInstruction(
  wrapperState: PublicKey,
  collector: PublicKey,
): Instruction {
  return {
    accounts: [
      {
        address: address(wrapperState.toBase58()),
        role: AccountRole.WRITABLE,
      },
      {
        address: address(collector.toBase58()),
        role: AccountRole.READONLY_SIGNER,
      },
    ],
    programAddress: address(WRAPPER_PROGRAM_ID.toBase58()),
    data: new Uint8Array([DEFRAG_DISCRIMINANT]),
  };
}

/** Count the wrapper's free-list blocks; null if the list is malformed. */
function freeListLength(data: Buffer): number | null {
  if (data.length < WRAPPER_FIXED_SIZE) return null;
  const allocated: number = data.readUInt32LE(40);
  const blocks: number = Math.floor(allocated / WRAPPER_BLOCK_SIZE);
  let next: number = data.readUInt32LE(44);
  let count: number = 0;
  while (next !== NIL) {
    if (
      next % WRAPPER_BLOCK_SIZE !== 0 ||
      next + WRAPPER_BLOCK_SIZE > allocated ||
      WRAPPER_FIXED_SIZE + next + 4 > data.length ||
      count > blocks
    ) {
      return null;
    }
    count += 1;
    next = data.readUInt32LE(WRAPPER_FIXED_SIZE + next);
  }
  return count;
}

/** The size a full defrag packs the wrapper down to, or null when the
 *  free list cannot be walked (defrag itself validates and would reject
 *  deeper corruption; a leaked block only makes this an overestimate). */
function predictDefraggedSpace(data: Buffer): number | null {
  const freeBlocks: number | null = freeListLength(data);
  if (freeBlocks === null) return null;
  const allocated: number = data.readUInt32LE(40);
  const liveBlocks: number =
    Math.floor(allocated / WRAPPER_BLOCK_SIZE) - freeBlocks;
  if (liveBlocks < 0) return null;
  return WRAPPER_FIXED_SIZE + liveBlocks * WRAPPER_BLOCK_SIZE;
}

export function estimateDefragCu(space: number): number {
  return Math.min(
    CU_BUDGET_PER_TRANSACTION,
    Math.ceil(DEFRAG_CU_BASE + DEFRAG_CU_PER_BYTE * space),
  );
}

/** What the batcher budgeted for a batch, so simulation can be compared
 *  against it. The estimate drives packing, so a run that reports both keeps
 *  DEFRAG_CU_BASE and DEFRAG_CU_PER_BYTE honest against mainnet state. */
export function estimateBatchDefragCu(
  batch: CollectableWrapper[],
  withDefrag: boolean,
): number {
  if (!withDefrag) return 0;
  return batch.reduce(
    (total, wrapper) =>
      total + (wrapper.defragCu ?? estimateDefragCu(wrapper.space)),
    0,
  );
}

export function createCollectMessage(
  batch: CollectableWrapper[],
  collector: PublicKey,
  recentBlockhash: string,
  lastValidBlockHeight: bigint,
  withDefrag: boolean = false,
) {
  const instructions = batch.flatMap(({ pubkey }) =>
    withDefrag
      ? [
          createDefragInstruction(pubkey, collector),
          createCollectInstruction(pubkey, collector),
        ]
      : [createCollectInstruction(pubkey, collector)],
  );
  return pipe(
    createTransactionMessage({ version: 1 }),
    (message) =>
      setTransactionMessageFeePayer(address(collector.toBase58()), message),
    (message) =>
      setTransactionMessageLifetimeUsingBlockhash(
        {
          blockhash: blockhash(recentBlockhash),
          lastValidBlockHeight,
        },
        message,
      ),
    (message) => appendTransactionMessageInstructions(instructions, message),
    (message) =>
      setTransactionMessageConfig(
        {
          computeUnitLimit: MAX_COMPUTE_UNITS,
          loadedAccountsDataSizeLimit: MAX_LOADED_ACCOUNTS_DATA_SIZE,
        },
        message,
      ),
  );
}

function serializedSize(
  batch: CollectableWrapper[],
  collector: PublicKey,
  withDefrag: boolean = false,
): number | null {
  if (batch.length > V1_MAX_WRAPPERS) return null;
  try {
    const message = createCollectMessage(
      batch,
      collector,
      SIZE_CHECK_BLOCKHASH,
      0n,
      withDefrag,
    );
    const transaction = compileTransaction(message);
    return Buffer.from(getBase64EncodedWireTransaction(transaction), 'base64')
      .length;
  } catch (error) {
    // Every way a batch can fail to compile is "this batch does not fit", and
    // the caller's job is to close the batch and start a new one. Besides the
    // wire size and the 64-address limit, --defrag mode runs into the
    // 64-instruction limit first: at two instructions per wrapper that caps a
    // defrag batch at 32 wrappers, half the 61 addresses a v1 transaction can
    // name and the reason --defrag cannot reach collect-only's batch size.
    if (
      error instanceof Error &&
      (error.message.includes('too large') ||
        error.message.includes('64 account') ||
        (error.message.includes('instructions') &&
          error.message.includes('maximum')))
    ) {
      return null;
    }
    throw error;
  }
}

/** Greedily packs the maximum number of Collect instructions into v1 transactions. */
export function packCollectTransactions(
  wrappers: CollectableWrapper[],
  collector: PublicKey,
  maxWrappersPerTransaction?: number,
  withDefrag: boolean = false,
): CollectableWrapper[][] {
  if (
    maxWrappersPerTransaction !== undefined &&
    (!Number.isInteger(maxWrappersPerTransaction) ||
      maxWrappersPerTransaction <= 0)
  ) {
    throw new Error('max wrappers per transaction must be a positive integer');
  }

  const configuredCap = Math.min(
    maxWrappersPerTransaction ?? V1_MAX_WRAPPERS,
    V1_MAX_WRAPPERS,
  );
  const batches: CollectableWrapper[][] = [];
  let current: CollectableWrapper[] = [];
  let currentCu = 0;

  for (const wrapper of wrappers) {
    const wrapperCu: number = withDefrag
      ? (wrapper.defragCu ?? estimateDefragCu(wrapper.space))
      : 0;
    const candidate = [...current, wrapper];
    if (
      candidate.length <= configuredCap &&
      (!withDefrag || currentCu + wrapperCu <= CU_BUDGET_PER_TRANSACTION) &&
      (serializedSize(candidate, collector, withDefrag) ?? Infinity) <=
        V1_MAX_TRANSACTION_SIZE
    ) {
      current = candidate;
      currentCu += wrapperCu;
      continue;
    }

    if (current.length === 0) {
      throw new Error(
        `A single Collect instruction exceeds ${V1_MAX_TRANSACTION_SIZE} bytes`,
      );
    }
    batches.push(current);
    current = [wrapper];
    currentCu = wrapperCu;
  }

  if (current.length > 0) batches.push(current);
  return batches;
}

function optionValue(name: string): string | undefined {
  const prefix = `--${name}=`;
  return process.argv
    .find((value) => value.startsWith(prefix))
    ?.slice(prefix.length);
}

function parsePositiveInteger(
  optionName: string,
  environmentName: string,
): number | undefined {
  const raw = optionValue(optionName) ?? process.env[environmentName];
  if (raw === undefined) return undefined;
  const parsed = Number(raw);
  if (!Number.isInteger(parsed) || parsed <= 0) {
    throw new Error(`Invalid --${optionName}: ${JSON.stringify(raw)}`);
  }
  return parsed;
}

async function loadCollector(): Promise<{
  publicKey: PublicKey;
  signer: KeyPairSigner;
}> {
  const keypairPath =
    KEYPAIR_PATH ?? `${process.env.HOME}/.config/solana/id.json`;
  if (!fs.existsSync(keypairPath)) {
    throw new Error(
      `Keypair file not found at ${keypairPath}. Set KEYPAIR_PATH or install the authorized keypair there.`,
    );
  }
  const keypairBytes = Uint8Array.from(
    JSON.parse(fs.readFileSync(keypairPath, 'utf8')),
  );
  const signer = await createKeyPairSignerFromBytes(keypairBytes);
  const publicKey = new PublicKey(signer.address);
  if (!publicKey.equals(AUTHORIZED_COLLECTOR)) {
    throw new Error(
      `Only ${AUTHORIZED_COLLECTOR.toBase58()} can collect fees; loaded ${publicKey.toBase58()}`,
    );
  }
  return { publicKey, signer };
}

function formatSOL(lamports: bigint): string {
  return `${lamports / 1_000_000_000n}.${(lamports % 1_000_000_000n)
    .toString()
    .padStart(9, '0')}`;
}

function sleep(ms: number, shouldStop: () => boolean): Promise<void> {
  return new Promise((resolve) => {
    const deadline = Date.now() + ms;
    const check = () => {
      if (shouldStop() || Date.now() >= deadline) {
        resolve();
      } else {
        setTimeout(check, Math.min(250, deadline - Date.now()));
      }
    };
    check();
  });
}

async function buildSignedTransaction(
  batch: CollectableWrapper[],
  collector: { publicKey: PublicKey; signer: KeyPairSigner },
  recentBlockhash: string,
  lastValidBlockHeight: number,
  withDefrag: boolean = false,
) {
  const unsignedMessage = createCollectMessage(
    batch,
    collector.publicKey,
    recentBlockhash,
    BigInt(lastValidBlockHeight),
    withDefrag,
  );
  const message = setTransactionMessageFeePayerSigner(
    collector.signer,
    unsignedMessage,
  );
  return signTransactionMessageWithSigners(message);
}

/**
 * A v1 message carrying arbitrary instructions. The wrapper path has its own
 * builder shaped around Collect/Defrag pairs; the core phases need the generic
 * form, and both must set loadedAccountsDataSizeLimit or a multi-leg batch
 * fails MaxLoadedAccountsDataSizeExceeded before executing anything.
 */
function createGenericMessage(
  instructions: Instruction[],
  feePayer: PublicKey,
  recentBlockhash: string,
  lastValidBlockHeight: bigint,
  computeUnitLimit: number,
  loadedAccountsDataSizeLimit: number,
) {
  return pipe(
    createTransactionMessage({ version: 1 }),
    (message) =>
      setTransactionMessageFeePayer(address(feePayer.toBase58()), message),
    (message) =>
      setTransactionMessageLifetimeUsingBlockhash(
        { blockhash: blockhash(recentBlockhash), lastValidBlockHeight },
        message,
      ),
    (message) => appendTransactionMessageInstructions(instructions, message),
    (message) =>
      setTransactionMessageConfig(
        { computeUnitLimit, loadedAccountsDataSizeLimit },
        message,
      ),
  );
}

/** Wire size of an instruction list, or null when it cannot be built at all. */
function genericSerializedSize(
  instructions: Instruction[],
  feePayer: PublicKey,
): number | null {
  try {
    const message = createGenericMessage(
      instructions,
      feePayer,
      SIZE_CHECK_BLOCKHASH,
      0n,
      MAX_COMPUTE_UNITS,
      MAX_LOADED_ACCOUNTS_DATA_SIZE,
    );
    return Buffer.from(
      getBase64EncodedWireTransaction(compileTransaction(message)),
      'base64',
    ).length;
  } catch (error) {
    if (
      error instanceof Error &&
      (error.message.includes('too large') ||
        error.message.includes('account addresses'))
    ) {
      return null;
    }
    throw error;
  }
}

async function simulateTransaction(
  encodedTransaction: string,
  sigVerify: boolean,
): Promise<SimulationResult['value']> {
  const simulation = await rpc<SimulationResult>('simulateTransaction', [
    encodedTransaction,
    {
      commitment: 'confirmed',
      encoding: 'base64',
      replaceRecentBlockhash: false,
      sigVerify,
    },
  ]);
  return simulation.value;
}

async function run(): Promise<void> {
  if (process.argv.includes('--help')) {
    console.log(`Usage:
  yarn collect:wrapper-fees --wrapper=ADDRESS
      Dry-run exactly one wrapper. Use this before any public sweep.

  yarn collect:wrapper-fees --wrapper=ADDRESS --execute
      Simulate, submit, and confirm exactly one wrapper Collect.

  yarn collect:wrapper-fees --all-public
      Dry-run all initialized public wrappers with collectible fees.

  yarn collect:wrapper-fees --all-public --execute
      Collect all public wrappers in v1 batches, stopping on the first error.

  yarn collect:wrapper-fees --defrag --all-public
      Dry-run defrag + collect: predicts each wrapper's packed size from its
      free list and reports the cumulative SOL a full defrag + sweep recovers.

  yarn collect:wrapper-fees --defrag --wrapper=ADDRESS --execute
      Defrag then collect one wrapper in a single transaction.

  yarn collect:wrapper-fees --defrag --all-public --execute
      Defrag + collect every wrapper with anything to recover.

Core-program phases (see scripts/README-fee-collection.md):
  yarn collect:wrapper-fees --markets
      Dry-run the core market sweep: accrued advanced-order fees plus any
      already-free tail. Relocates nothing.

  yarn collect:wrapper-fees --markets --global-clean --execute
      Sweep markets and clean expired orders in the same pass.

  yarn collect:wrapper-fees --markets --tail-tighten --min-shrink=3 --execute
      Also truncate free tails of at least 3 blocks. Safe hourly, but note
      traders re-pay most of that rent on their next placement.

Options:
  --wrapper=ADDRESS           Select exactly one test wrapper.
  --all-public               Explicitly select all fee-bearing public wrappers.
  --defrag                   Prepend a Defrag to each Collect (same transaction),
                             so freed rent is swept with the fees. Requires the
                             wrapper program to be at v3.0.25 or later.
  --max-wrappers-per-tx=N    Safety cap below the v1 maximum of ${V1_MAX_WRAPPERS}.
  --delay-ms=N               Delay between transactions (default ${DEFAULT_DELAY_MS}ms).
  --execute                  Submit; without this flag the command is read-only.
  --markets                  Sweep core markets for accrued fees. Needs no
                             wrapper target; relocates nothing.
  --global-clean             Clean expired orders for their gas deposits.
                             Requires --markets: one clean exactly cancels one
                             signature fee, so it must ride along.
  --clean-unbacked           Also clean unbacked global orders (extends
                             --global-clean). Only orders that have rested
                             UNCHANGED since the previous pass are cleaned, so
                             fast-churning slots are left alone; use --loop (a
                             single run just sets the baseline). Each is
                             confirmed by simulation first; a dry run previews
                             which traders have unbacked orders.
                             CLEAN_SKIP_TRADERS whitelists makers to leave alone.
  --max-unbacked-sims=N      Cap unbacked confirmation simulations per pass
                             (default ${DEFAULT_MAX_UNBACKED_SIMS}); longest-resting
                             orders first, the rest wait for a later pass.
  --tail-tighten             Also take markets whose free tail can truncate.
  --min-shrink=N             With --tail-tighten, require N blocks of shrink.
  --max-markets=N            Cap markets per run, highest recovery first.
  --loop=SECONDS             Run continuously until Ctrl-C, starting a pass every
                             SECONDS measured from the previous pass's START (so
                             run time does not stack on top of the interval). A
                             pass that overruns the interval is followed
                             immediately by the next - never concurrently - and
                             a pass that errors is logged and the loop continues.

Execution safety:
  Every transaction is simulated before submission. Ctrl-C stops before the
  next transaction; a second Ctrl-C exits immediately. Any error stops the run
  (in --loop mode, only the current pass; the loop continues to the next).

Environment:
  RPC_URL                    Required Solana RPC URL.
  KEYPAIR_PATH               Authorized collector keypair; required with --execute.
  MAX_WRAPPERS_PER_TX        Alternative batch-size safety cap.
  COLLECT_DELAY_MS           Alternative delay between transactions (default 1000ms).
  CLEAN_SKIP_TRADERS         Maker pubkeys (comma/space separated) never cleaned.
  MAX_UNBACKED_SIMS          Alternative unbacked-sim cap (default ${DEFAULT_MAX_UNBACKED_SIMS}).
  UNBACKED_STATE_PATH        If set, persist the unbacked-order baseline here so
                             a restart keeps "unchanged since last run" state.`);
    return;
  }

  if (!RPC_URL) throw new Error('RPC_URL missing from env');

  const execute = process.argv.includes('--execute');
  const allPublic = process.argv.includes('--all-public');
  const withDefrag = process.argv.includes('--defrag');
  // Core-program phases. Opt-in, so every existing invocation is unchanged.
  const sweepMarkets = process.argv.includes('--markets');
  const globalClean = process.argv.includes('--global-clean');
  const cleanUnbacked = process.argv.includes('--clean-unbacked');
  const maxUnbackedSims =
    parsePositiveInteger('max-unbacked-sims', 'MAX_UNBACKED_SIMS') ??
    DEFAULT_MAX_UNBACKED_SIMS;
  // Maker pubkeys whose orders must never be cleaned, from CLEAN_SKIP_TRADERS
  // (comma- or whitespace-separated). A safety whitelist for traders we do not
  // want to disturb even when their global orders are technically cleanable.
  const skipTraders = new Set(
    (process.env.CLEAN_SKIP_TRADERS ?? '')
      .split(/[\s,]+/)
      .map((s) => s.trim())
      .filter((s) => s.length > 0),
  );
  const tailTighten = process.argv.includes('--tail-tighten');
  const minShrinkBlocks = tailTighten
    ? (parsePositiveInteger('min-shrink', 'MIN_SHRINK_BLOCKS') ?? 1)
    : 0;
  const maxMarkets = parsePositiveInteger('max-markets', 'MAX_MARKETS');
  const wrapperOption = optionValue('wrapper');
  const coreOnly =
    (sweepMarkets || globalClean) && !allPublic && wrapperOption === undefined;
  if (tailTighten && !sweepMarkets) {
    throw new Error(
      '--tail-tighten applies to the market sweep; pass --markets as well',
    );
  }
  // GlobalClean pays one gas deposit per order and a transaction costs one
  // signature, so cleaning alone barely covers itself. Requiring it to ride
  // with another phase is the operator's instruction and the economics agree.
  if (globalClean && !sweepMarkets) {
    throw new Error(
      '--global-clean must ride along with --markets: a clean-only run barely pays for its own signature',
    );
  }
  if (cleanUnbacked && !globalClean) {
    throw new Error(
      '--clean-unbacked extends --global-clean; pass --global-clean as well',
    );
  }
  if (!coreOnly && allPublic === (wrapperOption !== undefined)) {
    throw new Error(
      'Choose exactly one target: --wrapper=ADDRESS or --all-public (or use --markets for the core-only phases)',
    );
  }
  const selectedWrapper = wrapperOption
    ? new PublicKey(wrapperOption)
    : undefined;
  const maxWrappersPerTransaction = parsePositiveInteger(
    'max-wrappers-per-tx',
    'MAX_WRAPPERS_PER_TX',
  );
  const delayMs =
    parsePositiveInteger('delay-ms', 'COLLECT_DELAY_MS') ?? DEFAULT_DELAY_MS;
  const collector = execute ? await loadCollector() : undefined;
  const collectorPublicKey = collector?.publicKey ?? AUTHORIZED_COLLECTOR;
  const connection = new Connection(RPC_URL, 'confirmed');

  // Declared before the core phase because that phase's shouldStop/sleep
  // closures capture it; a `let` referenced before this point throws a
  // temporal-dead-zone error the moment the core send loop calls shouldStop().
  let stopRequested = false;
  let interruptCount = 0;
  const onInterrupt = () => {
    interruptCount += 1;
    if (interruptCount > 1) {
      console.error('\nSecond interrupt received; exiting immediately.');
      process.exit(130);
    }
    stopRequested = true;
    console.error(
      '\nStop requested. The current transaction may finish; no next transaction will be submitted.',
    );
  };
  process.on('SIGINT', onInterrupt);

  if (sweepMarkets || globalClean) {
    console.log(`Mode: ${execute ? 'EXECUTE' : 'DRY RUN'} (core phases)`);
    const coreResult = await runCoreCollections(
      {
        rpc,
        bs58encode: (buffer: Buffer) => bs58.encode(buffer),
        collector: collectorPublicKey,
        measure: (instructions) =>
          genericSerializedSize(instructions, collectorPublicKey),
        sleep: (ms: number) => sleep(ms, () => stopRequested),
        simulate: async (instructions, units, dataSize) => {
          const { value } = await rpc<{
            value: { blockhash: string; lastValidBlockHeight: number };
          }>('getLatestBlockhash', [{ commitment: 'confirmed' }]);
          const message = createGenericMessage(
            instructions,
            collectorPublicKey,
            value.blockhash,
            BigInt(value.lastValidBlockHeight),
            units,
            dataSize,
          );
          const encoded = getBase64EncodedWireTransaction(
            compileTransaction(message),
          );
          const simulation = await simulateTransaction(encoded, false);
          return {
            err: simulation.err,
            unitsConsumed: simulation.unitsConsumed,
          };
        },
        buildAndSend: async (instructions, units, dataSize) => {
          if (collector === undefined)
            throw new Error('collector keypair required to execute');
          const { value } = await rpc<{
            value: { blockhash: string; lastValidBlockHeight: number };
          }>('getLatestBlockhash', [{ commitment: 'confirmed' }]);
          const unsigned = createGenericMessage(
            instructions,
            collectorPublicKey,
            value.blockhash,
            BigInt(value.lastValidBlockHeight),
            units,
            dataSize,
          );
          const signed = await signTransactionMessageWithSigners(
            setTransactionMessageFeePayerSigner(collector.signer, unsigned),
          );
          const encoded = getBase64EncodedWireTransaction(signed);
          const signature = await rpc<string>('sendTransaction', [
            encoded,
            {
              encoding: 'base64',
              preflightCommitment: 'confirmed',
              maxRetries: 3,
            },
          ]);
          await connection.confirmTransaction(
            {
              signature,
              blockhash: value.blockhash,
              lastValidBlockHeight: value.lastValidBlockHeight,
            },
            'confirmed',
          );
          return signature;
        },
      },
      {
        execute,
        sweepMarkets,
        globalClean,
        cleanUnbacked,
        maxUnbackedSims,
        unbackedTracker,
        skipTraders,
        minShrinkBlocks,
        minRecoveryLamports: 100_000n,
        maxMarkets,
        delayMs,
        shouldStop: () => stopRequested,
      },
    );
    console.log(
      `Core phases: ${coreResult.transactionsSent} transactions, ` +
        `${formatSOL(coreResult.recoveredLamports + coreResult.cleanRewardLamports)} recovered ` +
        `(${coreResult.marketsSelected} markets, ${coreResult.cleanableOrders} orders cleaned` +
        (coreResult.cleansSkippedRaced > 0
          ? `, ${coreResult.cleansSkippedRaced} skipped as raced`
          : '') +
        `)`,
    );
    if (coreOnly) return;
  }

  console.log(`Mode: ${execute ? 'EXECUTE' : 'DRY RUN'}`);
  console.log(`Operation: ${withDefrag ? 'defrag + collect' : 'collect only'}`);
  console.log('Transaction version: v1');
  console.log(
    `Target: ${selectedWrapper ? `single wrapper ${selectedWrapper.toBase58()}` : 'all public wrappers'}`,
  );
  console.log(`Collector: ${collectorPublicKey.toBase58()}`);
  console.log('Fetching initialized wrapper state accounts...');

  const wrapperResult = await rpc<{
    context: { slot: number };
    value: {
      pubkey: string;
      account: {
        data: [string, 'base64'];
        lamports: number;
        space: number;
      };
    }[];
  }>('getProgramAccounts', [
    WRAPPER_PROGRAM_ID.toBase58(),
    {
      commitment: 'confirmed',
      encoding: 'base64',
      // Defrag needs the whole account to walk the free list; plain collect
      // only needs the header.
      ...(withDefrag ? {} : { dataSlice: { offset: 0, length: 40 } }),
      filters: [
        {
          memcmp: { offset: 0, bytes: WRAPPER_STATE_DISCRIMINATOR_BASE58 },
        },
      ],
      withContext: true,
    },
  ]);
  const wrapperAccounts = selectedWrapper
    ? wrapperResult.value.filter(({ pubkey }) =>
        new PublicKey(pubkey).equals(selectedWrapper),
      )
    : wrapperResult.value;
  if (selectedWrapper && wrapperAccounts.length === 0) {
    throw new Error(
      `${selectedWrapper.toBase58()} is not an initialized wrapper state account`,
    );
  }
  console.log(
    `Found ${wrapperResult.value.length} initialized wrappers; selected ${wrapperAccounts.length}`,
  );

  // In defrag mode the account keeps only the rent its packed size needs, so
  // the collectible amount is measured against the predicted post-defrag
  // size. Wrappers whose free list cannot be walked are excluded: the
  // program's own validation would reject their defrag and fail the whole
  // batch. Sweep those with plain collect mode instead.
  const unpredictable: string[] = [];
  const annotated = wrapperAccounts.flatMap(({ pubkey, account }) => {
    const data = Buffer.from(account.data[0], 'base64');
    if (data.length < 40) {
      throw new Error(`RPC returned a short wrapper header for ${pubkey}`);
    }
    if (!withDefrag) {
      return [{ pubkey, account, data, predictedSpace: account.space }];
    }
    const predictedSpace = predictDefraggedSpace(data);
    if (predictedSpace === null) {
      unpredictable.push(pubkey);
      return [];
    }
    return [{ pubkey, account, data, predictedSpace }];
  });
  if (unpredictable.length > 0) {
    console.warn(
      `Excluding ${unpredictable.length} wrappers whose free list cannot be walked (defrag would reject them): ${unpredictable.join(', ')}`,
    );
  }
  const accountLengths = [
    ...new Set(annotated.map(({ predictedSpace }) => predictedSpace)),
  ];
  const rentByLength = await getRentExemptBalances(accountLengths);
  const collectableWrappers: CollectableWrapper[] = annotated
    .map(({ pubkey, account, data, predictedSpace }) => {
      return {
        pubkey: new PublicKey(pubkey),
        trader: new PublicKey(data.subarray(8, 40)),
        amount: BigInt(account.lamports) - rentByLength.get(predictedSpace)!,
        space: account.space,
        predictedSpace,
        defragCu: withDefrag ? estimateDefragCu(account.space) : undefined,
      };
    })
    .filter(({ amount }) => amount > 0n)
    .sort((a, b) => (a.amount === b.amount ? 0 : a.amount > b.amount ? -1 : 1));

  if (collectableWrappers.length === 0) {
    console.log('Nothing to collect from the selected target.');
    return;
  }

  const totalLamports = collectableWrappers.reduce(
    (sum, { amount }) => sum + amount,
    0n,
  );
  console.log(
    `${collectableWrappers.length} wrappers have ${formatSOL(totalLamports)} SOL ${withDefrag ? 'recoverable by defrag + collect' : 'collectible'}`,
  );
  if (withDefrag) {
    const bytesFreed = collectableWrappers.reduce(
      (sum, { space, predictedSpace }) =>
        sum + (space - (predictedSpace ?? space)),
      0,
    );
    console.log(
      `  defrag shrinks the selected wrappers by ${bytesFreed} bytes in total`,
    );
  }
  const tableLimit = 25;
  console.table(
    collectableWrappers
      .slice(0, tableLimit)
      .map(({ pubkey, trader, amount, space, predictedSpace }) => ({
        wrapper: pubkey.toBase58(),
        trader: trader.toBase58(),
        ...(withDefrag ? { bytes: space, defraggedBytes: predictedSpace } : {}),
        recoverableSOL: formatSOL(amount),
      })),
  );
  if (collectableWrappers.length > tableLimit) {
    console.log(
      `  (table shows the ${tableLimit} largest; ${collectableWrappers.length - tableLimit} more omitted)`,
    );
  }

  const batches = packCollectTransactions(
    collectableWrappers,
    collectorPublicKey,
    maxWrappersPerTransaction,
    withDefrag,
  );
  const batchSizes = batches.map((batch) => batch.length);
  const maximumWireSize = Math.max(
    ...batches.map(
      (batch) =>
        serializedSize(batch, collectorPublicKey, withDefrag) ?? Infinity,
    ),
  );
  console.log('\nCollection plan:');
  console.log(`  Transactions: ${batches.length}`);
  console.log(`  Wrappers per full transaction: ${Math.max(...batchSizes)}`);
  console.log(
    `  Largest serialized transaction: ${maximumWireSize}/${V1_MAX_TRANSACTION_SIZE} bytes`,
  );
  console.log(`  Final transaction wrappers: ${batchSizes.at(-1)}`);
  if (batches.length > 1) {
    console.log(`  Delay between transactions: ${delayMs}ms`);
  }

  if (!execute) {
    if (selectedWrapper) {
      const latestBlockhash = await connection.getLatestBlockhash('confirmed');
      const message = createCollectMessage(
        batches[0],
        collectorPublicKey,
        latestBlockhash.blockhash,
        BigInt(latestBlockhash.lastValidBlockHeight),
        withDefrag,
      );
      const encodedTransaction = getBase64EncodedWireTransaction(
        compileTransaction(message),
      );
      console.log('\nSimulating the single-wrapper v1 transaction...');
      const simulation = await simulateTransaction(encodedTransaction, false);
      if (simulation.err !== null) {
        console.error(`Simulation failed: ${JSON.stringify(simulation.err)}`);
        for (const log of simulation.logs ?? []) console.error(`  ${log}`);
        process.exitCode = 1;
        return;
      }
      console.log(
        `Simulation passed: ${simulation.unitsConsumed ?? 'unknown'} CU, ${simulation.loadedAccountsDataSize ?? 'unknown'} loaded-account bytes${
          withDefrag
            ? ` (budgeted ${estimateBatchDefragCu(batches[0], withDefrag)} CU)`
            : ''
        }`,
      );
    }
    console.log(
      `\nDry run only. Re-run with ${withDefrag ? '--defrag ' : ''}${selectedWrapper ? `--wrapper=${selectedWrapper.toBase58()}` : '--all-public'} --execute to submit this exact target class.`,
    );
    return;
  }

  let collected = 0;
  let collectedLamports = 0n;
  let successfulTransactions = 0;
  let failed = false;
  try {
    for (const [index, batch] of batches.entries()) {
      if (stopRequested) break;

      let landed = false;
      let landedSignature: string | undefined;
      for (let attempt = 1; attempt <= MAX_SEND_ATTEMPTS; attempt++) {
        const latestBlockhash =
          await connection.getLatestBlockhash('confirmed');
        const transaction = await buildSignedTransaction(
          batch,
          collector!,
          latestBlockhash.blockhash,
          latestBlockhash.lastValidBlockHeight,
          withDefrag,
        );
        const encodedTransaction = getBase64EncodedWireTransaction(transaction);
        const wireSize = Buffer.from(encodedTransaction, 'base64').length;

        console.log(
          `\nTx ${index + 1}/${batches.length}${attempt > 1 ? ` (attempt ${attempt}/${MAX_SEND_ATTEMPTS})` : ''}: simulating ${batch.length} wrappers (${wireSize} bytes)...`,
        );
        const simulation = await simulateTransaction(encodedTransaction, true);
        if (simulation.err !== null) {
          failed = true;
          console.error(`Simulation failed: ${JSON.stringify(simulation.err)}`);
          for (const log of simulation.logs ?? []) console.error(`  ${log}`);
          break;
        }
        const predictedCu: number = estimateBatchDefragCu(batch, withDefrag);
        console.log(
          `Simulation passed: ${simulation.unitsConsumed ?? 'unknown'} CU, ${simulation.loadedAccountsDataSize ?? 'unknown'} loaded-account bytes${
            withDefrag ? ` (budgeted ${predictedCu} CU)` : ''
          }`,
        );
        if (stopRequested) break;

        let signature: string | undefined;
        try {
          signature = await rpc<string>('sendTransaction', [
            encodedTransaction,
            {
              encoding: 'base64',
              maxRetries: 3,
              preflightCommitment: 'confirmed',
              skipPreflight: false,
            },
          ]);
          const confirmation = await connection.confirmTransaction(
            { signature, ...latestBlockhash },
            'confirmed',
          );
          if (confirmation.value.err !== null) {
            throw new Error(
              `confirmation failed: ${JSON.stringify(confirmation.value.err)}`,
            );
          }
          landed = true;
          landedSignature = signature;
          break;
        } catch (error) {
          // An expired blockhash means the transaction can no longer land, so
          // rebuilding the same batch on a fresh blockhash is safe. Check it
          // really did not land first (last-valid-block race), and abort on
          // every other failure.
          const expired =
            error instanceof Error &&
            (error.name === 'TransactionExpiredBlockheightExceededError' ||
              error.message.includes('block height exceeded'));
          if (expired && signature !== undefined) {
            const statuses = await rpc<{
              value: ({ err: unknown } | null)[];
            }>('getSignatureStatuses', [
              [signature],
              { searchTransactionHistory: true },
            ]);
            const status = statuses.value[0];
            if (status !== null && status.err === null) {
              console.log(
                `Tx ${index + 1}/${batches.length} landed despite the expiry report (${signature}).`,
              );
              landed = true;
              landedSignature = signature;
              break;
            }
            if (status !== null) {
              failed = true;
              console.error(
                `Tx ${index + 1}/${batches.length} landed WITH an error: ${JSON.stringify(status.err)} (${signature})`,
              );
              break;
            }
            if (attempt < MAX_SEND_ATTEMPTS) {
              console.warn(
                `Tx ${index + 1}/${batches.length} expired without landing; retrying with a fresh blockhash...`,
              );
              continue;
            }
          }
          failed = true;
          console.error(`Tx ${index + 1}/${batches.length} failed:`, error);
          break;
        }
      }
      if (!landed || failed || stopRequested) {
        if (!landed && !failed && !stopRequested) failed = true;
        break;
      }

      const batchLamports = batch.reduce((sum, { amount }) => sum + amount, 0n);
      collected += batch.length;
      collectedLamports += batchLamports;
      successfulTransactions += 1;
      console.log(
        `Confirmed tx ${index + 1}/${batches.length}: ${batch.length} wrappers, ${formatSOL(batchLamports)} SOL, ${landedSignature}`,
      );

      if (index + 1 < batches.length && !stopRequested) {
        console.log(
          `Waiting ${delayMs}ms before the next transaction (Ctrl-C to stop)...`,
        );
        await sleep(delayMs, () => stopRequested);
      }
    }
  } finally {
    process.off('SIGINT', onInterrupt);
  }

  console.log(
    `\nCollected ${formatSOL(collectedLamports)} SOL from ${collected}/${collectableWrappers.length} wrappers in ${successfulTransactions}/${batches.length} transactions.`,
  );
  if (stopRequested) console.log('Stopped by operator request.');
  if (failed) process.exitCode = 1;
}

// Optionally run forever on an interval instead of once, so the collector can
// be left running without an external cron. Each pass is an independent full
// collection (the script keeps no state between passes), so a pass that throws
// is logged and the loop simply continues to the next one rather than dying.
// run() installs and removes its own SIGINT handler per pass; the loop adds its
// own flag so Ctrl-C stops it between passes and during the interval sleep too.
async function main(): Promise<void> {
  loadUnbackedState();
  const loopArg = process.argv.find((a) => a.startsWith('--loop='))?.slice(7);
  if (loopArg === undefined) {
    await run();
    saveUnbackedState();
    return;
  }
  const loopSeconds = Number(loopArg);
  if (!Number.isFinite(loopSeconds) || loopSeconds <= 0) {
    throw new Error(
      `invalid --loop: ${loopArg} (interval in seconds, must be > 0)`,
    );
  }
  let stopLoop = false;
  process.on('SIGINT', () => {
    stopLoop = true;
    console.error(
      '\nStop requested; finishing the current pass, then exiting the loop.',
    );
  });
  // Fixed-rate schedule: a pass starts every `loopSeconds` measured from the
  // PREVIOUS pass's start, not from its end, so run time does not stack on top
  // of the interval. If a pass runs longer than the interval, the next starts
  // immediately after it (never concurrently - passes are strictly
  // sequential, so there is no overlap), and the schedule simply stops drifting
  // further behind rather than trying to catch up with back-to-back passes.
  const intervalMs = loopSeconds * 1000;
  for (let pass = 1; !stopLoop; pass += 1) {
    const startedAt = Date.now();
    console.log(
      `\n===== collection pass ${pass} @ ${new Date().toISOString()} =====`,
    );
    try {
      await run();
    } catch (error) {
      console.error(
        `pass ${pass} failed:`,
        error instanceof Error ? error.message : error,
      );
    }
    // Persist after every pass (even a failed one), so a restart resumes from
    // the latest baseline rather than re-establishing it from scratch.
    saveUnbackedState();
    if (stopLoop) break;
    const elapsedMs = Date.now() - startedAt;
    const waitMs = intervalMs - elapsedMs;
    if (waitMs > 0) {
      console.log(
        `pass took ${(elapsedMs / 1000).toFixed(1)}s; sleeping ${(waitMs / 1000).toFixed(1)}s ` +
          `to keep the ${loopSeconds}s interval (Ctrl-C to stop)`,
      );
      await sleep(waitMs, () => stopLoop);
    } else {
      console.log(
        `pass took ${(elapsedMs / 1000).toFixed(1)}s, over the ${loopSeconds}s interval; ` +
          `starting the next pass immediately`,
      );
    }
  }
  console.log('loop stopped.');
}

if (require.main === module) {
  main().catch((error) => {
    console.error('Error:', error);
    process.exit(1);
  });
}
