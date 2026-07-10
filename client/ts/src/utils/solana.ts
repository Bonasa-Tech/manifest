import { Connection, PublicKey } from '@solana/web3.js';

export type Cluster = 'mainnet-beta' | 'devnet' | 'localnet';

export async function getClusterFromConnection(
  connection: Connection,
): Promise<Cluster> {
  const hash = await connection.getGenesisHash();
  if (hash === '5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d') {
    return 'mainnet-beta';
  } else if (hash === 'EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG') {
    return 'devnet';
  } else {
    return 'localnet';
  }
}

// The Solana runtime log collector appends this exact line and stops
// recording when a transaction exceeds the log byte limit.
const LOG_TRUNCATED_MARKER: string = 'log truncated';

const PROGRAM_INVOKE_REGEX: RegExp =
  /^Program [1-9A-HJ-NP-Za-km-z]{32,44} invoke \[\d+\]$/;
const PROGRAM_RESULT_REGEX: RegExp =
  /^Program [1-9A-HJ-NP-Za-km-z]{32,44} (success$|failed: )/;
// Lines whose content is program-controlled and so could contain arbitrary
// text, e.g. the word "truncated".
const PROGRAM_CONTENT_REGEX: RegExp = /^Program (log|data|return): /;

/**
 * Detect whether a transaction's log messages were truncated by the runtime,
 * meaning Program data entries (and thus fills) may be missing.
 */
export function hasTruncatedLogs(logMessages: string[]): boolean {
  let invokeDepth: number = 0;

  for (const message of logMessages) {
    // Exact marker emitted by the runtime log collector.
    if (message.trim().toLowerCase() === LOG_TRUNCATED_MARKER) {
      return true;
    }

    // Catch marker variations, but only on runtime-emitted lines so that
    // program-controlled log content cannot false-positive.
    if (
      !PROGRAM_CONTENT_REGEX.test(message) &&
      message.toLowerCase().includes('truncated')
    ) {
      return true;
    }

    if (PROGRAM_INVOKE_REGEX.test(message)) {
      invokeDepth++;
    } else if (PROGRAM_RESULT_REGEX.test(message)) {
      invokeDepth--;
    }
  }

  // Every invoke should have a matching success/failed line. If not, the
  // logs ended mid-invocation even though no marker line survived.
  return invokeDepth !== 0;
}

export async function airdropSol(connection: Connection, recipient: PublicKey) {
  console.log(`Requesting airdrop for ${recipient}`);
  const signature = await connection.requestAirdrop(recipient, 2_000_000_000);
  const { blockhash, lastValidBlockHeight } =
    await connection.getLatestBlockhash();
  await connection.confirmTransaction({
    blockhash,
    lastValidBlockHeight,
    signature,
  });
}
