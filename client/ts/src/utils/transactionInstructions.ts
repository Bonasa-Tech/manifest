import bs58 from 'bs58';

/**
 * A transaction instruction (top-level or CPI) with its program id and account
 * keys resolved to base58 strings and its data decoded to bytes.
 */
export interface NormalizedInstruction {
  programId: string;
  accountKeys: string[];
  data: Uint8Array;
  stackHeight: number;
}

function toBase58(key: unknown): string {
  return typeof key === 'string'
    ? key
    : (key as { toBase58(): string }).toBase58();
}

/**
 * Resolve the full ordered account key list (static keys followed by keys
 * loaded from address lookup tables) as base58 strings.
 */
export function resolveAccountKeys(tx: any): string[] {
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
export function topLevelInstructions(
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

/**
 * Inner instructions (CPIs) in execution order, grouped by the index of the
 * top-level instruction that made them.
 */
export function innerInstructionGroups(
  tx: any,
  accountKeys: string[],
): Map<number, NormalizedInstruction[]> {
  const groups: Map<number, NormalizedInstruction[]> = new Map();
  for (const group of tx.meta?.innerInstructions ?? []) {
    groups.set(
      group.index,
      group.instructions.map((ix: any) => normalizeInner(ix, accountKeys)),
    );
  }
  return groups;
}

/**
 * Every instruction the runtime executed, in execution order: each top-level
 * instruction followed by the CPIs it made. This is the same order as the
 * `Program <id> invoke` lines in the transaction logs, so the n-th instruction
 * of a program here is the n-th invocation of that program in the logs.
 */
export function executedInstructions(tx: any): NormalizedInstruction[] {
  const accountKeys: string[] = resolveAccountKeys(tx);
  const groups: Map<number, NormalizedInstruction[]> = innerInstructionGroups(
    tx,
    accountKeys,
  );
  const result: NormalizedInstruction[] = [];
  topLevelInstructions(tx, accountKeys).forEach((topIx, topIndex) => {
    result.push(topIx, ...(groups.get(topIndex) ?? []));
  });
  return result;
}
