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
import * as fs from 'fs';
import { PROGRAM_ID as WRAPPER_PROGRAM_ID } from '../client/ts/src/wrapper';

const { RPC_URL, KEYPAIR_PATH } = process.env;

const AUTHORIZED_COLLECTOR = new PublicKey(
  'B6dmr2UAn2wgjdm3T4N1Vjd8oPYRRTguByW7AEngkeL6',
);
const SYSTEM_PROGRAM = '11111111111111111111111111111111';
const COLLECT_DISCRIMINANT = 7;
// Wrapper state discriminator: little-endian u64 value 1.
const WRAPPER_STATE_DISCRIMINATOR_BASE58 = 'Ahg1opVcGX';
const SIZE_CHECK_BLOCKHASH = '11111111111111111111111111111111';
const V1_MAX_TRANSACTION_SIZE = 4096;
const V1_MAX_ACCOUNT_ADDRESSES = 64;
// collector + System Program + wrapper program are shared by every instruction.
const V1_MAX_WRAPPERS = V1_MAX_ACCOUNT_ADDRESSES - 3;
const MAX_COMPUTE_UNITS = 1_400_000;
const MAX_LOADED_ACCOUNTS_DATA_SIZE = 64 * 1024 * 1024;
const DEFAULT_DELAY_MS = 10_000;

type CollectableWrapper = {
  pubkey: PublicKey;
  trader: PublicKey;
  amount: bigint;
  space: number;
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

export function createCollectMessage(
  batch: CollectableWrapper[],
  collector: PublicKey,
  recentBlockhash: string,
  lastValidBlockHeight: bigint,
) {
  const instructions = batch.map(({ pubkey }) =>
    createCollectInstruction(pubkey, collector),
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
): number | null {
  if (batch.length > V1_MAX_WRAPPERS) return null;
  try {
    const message = createCollectMessage(
      batch,
      collector,
      SIZE_CHECK_BLOCKHASH,
      0n,
    );
    const transaction = compileTransaction(message);
    return Buffer.from(getBase64EncodedWireTransaction(transaction), 'base64')
      .length;
  } catch (error) {
    if (
      error instanceof Error &&
      (error.message.includes('too large') ||
        error.message.includes('64 account'))
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

  for (const wrapper of wrappers) {
    const candidate = [...current, wrapper];
    if (
      candidate.length <= configuredCap &&
      (serializedSize(candidate, collector) ?? Infinity) <=
        V1_MAX_TRANSACTION_SIZE
    ) {
      current = candidate;
      continue;
    }

    if (current.length === 0) {
      throw new Error(
        `A single Collect instruction exceeds ${V1_MAX_TRANSACTION_SIZE} bytes`,
      );
    }
    batches.push(current);
    current = [wrapper];
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
) {
  const unsignedMessage = createCollectMessage(
    batch,
    collector.publicKey,
    recentBlockhash,
    BigInt(lastValidBlockHeight),
  );
  const message = setTransactionMessageFeePayerSigner(
    collector.signer,
    unsignedMessage,
  );
  return signTransactionMessageWithSigners(message);
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

Options:
  --wrapper=ADDRESS           Select exactly one test wrapper.
  --all-public               Explicitly select all fee-bearing public wrappers.
  --max-wrappers-per-tx=N    Safety cap below the v1 maximum of ${V1_MAX_WRAPPERS}.
  --delay-ms=N               Delay between transactions (default ${DEFAULT_DELAY_MS}ms).
  --execute                  Submit; without this flag the command is read-only.

Execution safety:
  Every transaction is simulated before submission. Ctrl-C stops before the
  next transaction; a second Ctrl-C exits immediately. Any error stops the run.

Environment:
  RPC_URL                    Required Solana RPC URL.
  KEYPAIR_PATH               Authorized collector keypair; required with --execute.
  MAX_WRAPPERS_PER_TX        Alternative batch-size safety cap.
  COLLECT_DELAY_MS           Alternative delay between transactions.`);
    return;
  }

  if (!RPC_URL) throw new Error('RPC_URL missing from env');

  const execute = process.argv.includes('--execute');
  const allPublic = process.argv.includes('--all-public');
  const wrapperOption = optionValue('wrapper');
  if (allPublic === (wrapperOption !== undefined)) {
    throw new Error(
      'Choose exactly one target: --wrapper=ADDRESS or --all-public',
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

  console.log(`Mode: ${execute ? 'EXECUTE' : 'DRY RUN'}`);
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
      dataSlice: { offset: 0, length: 40 },
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

  const accountLengths = [
    ...new Set(wrapperAccounts.map(({ account }) => account.space)),
  ];
  const rentByLength = await getRentExemptBalances(accountLengths);
  const collectableWrappers: CollectableWrapper[] = wrapperAccounts
    .map(({ pubkey, account }) => {
      const header = Buffer.from(account.data[0], 'base64');
      if (header.length !== 40) {
        throw new Error(`RPC returned a short wrapper header for ${pubkey}`);
      }
      return {
        pubkey: new PublicKey(pubkey),
        trader: new PublicKey(header.subarray(8, 40)),
        amount: BigInt(account.lamports) - rentByLength.get(account.space)!,
        space: account.space,
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
    `${collectableWrappers.length} wrappers have ${formatSOL(totalLamports)} SOL collectible`,
  );
  console.table(
    collectableWrappers.map(({ pubkey, trader, amount }) => ({
      wrapper: pubkey.toBase58(),
      trader: trader.toBase58(),
      collectibleSOL: formatSOL(amount),
    })),
  );

  const batches = packCollectTransactions(
    collectableWrappers,
    collectorPublicKey,
    maxWrappersPerTransaction,
  );
  const batchSizes = batches.map((batch) => batch.length);
  const maximumWireSize = Math.max(
    ...batches.map(
      (batch) => serializedSize(batch, collectorPublicKey) ?? Infinity,
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
        `Simulation passed: ${simulation.unitsConsumed ?? 'unknown'} CU, ${simulation.loadedAccountsDataSize ?? 'unknown'} loaded-account bytes`,
      );
    }
    console.log(
      `\nDry run only. Re-run with ${selectedWrapper ? `--wrapper=${selectedWrapper.toBase58()}` : '--all-public'} --execute to submit this exact target class.`,
    );
    return;
  }

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

  let collected = 0;
  let collectedLamports = 0n;
  let successfulTransactions = 0;
  let failed = false;
  try {
    for (const [index, batch] of batches.entries()) {
      if (stopRequested) break;

      const latestBlockhash = await connection.getLatestBlockhash('confirmed');
      const transaction = await buildSignedTransaction(
        batch,
        collector!,
        latestBlockhash.blockhash,
        latestBlockhash.lastValidBlockHeight,
      );
      const encodedTransaction = getBase64EncodedWireTransaction(transaction);
      const wireSize = Buffer.from(encodedTransaction, 'base64').length;

      console.log(
        `\nTx ${index + 1}/${batches.length}: simulating ${batch.length} wrappers (${wireSize} bytes)...`,
      );
      const simulation = await simulateTransaction(encodedTransaction, true);
      if (simulation.err !== null) {
        failed = true;
        console.error(`Simulation failed: ${JSON.stringify(simulation.err)}`);
        for (const log of simulation.logs ?? []) console.error(`  ${log}`);
        break;
      }
      console.log(
        `Simulation passed: ${simulation.unitsConsumed ?? 'unknown'} CU, ${simulation.loadedAccountsDataSize ?? 'unknown'} loaded-account bytes`,
      );
      if (stopRequested) break;

      let signature: string;
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
      } catch (error) {
        failed = true;
        console.error(`Tx ${index + 1}/${batches.length} failed:`, error);
        break;
      }

      const batchLamports = batch.reduce((sum, { amount }) => sum + amount, 0n);
      collected += batch.length;
      collectedLamports += batchLamports;
      successfulTransactions += 1;
      console.log(
        `Confirmed tx ${index + 1}/${batches.length}: ${batch.length} wrappers, ${formatSOL(batchLamports)} SOL, ${signature}`,
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

if (require.main === module) {
  run().catch((error) => {
    console.error('Error:', error);
    process.exit(1);
  });
}
