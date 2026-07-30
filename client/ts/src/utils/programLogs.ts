export interface ProgramDataLog {
  data: string;
  invocationIndex: number;
}

interface InvocationFrame {
  programId: string;
  targetInvocationIndex?: number;
}

const INVOKE_PATTERN = /^Program ([1-9A-HJ-NP-Za-km-z]+) invoke \[\d+\]$/;
const EXIT_PATTERN =
  /^Program ([1-9A-HJ-NP-Za-km-z]+) (?:success|failed(?::.*)?)$/;
const DATA_PREFIX = 'Program data: ';

/**
 * Return only data emitted while the requested program is the active Solana
 * invocation frame. The invocation index is stable across top-level and CPI
 * calls and can be used to associate decoded events with inferred effects.
 */
export function extractProgramDataLogs(
  messages: string[],
  targetProgramId: string,
): ProgramDataLog[] {
  const frames: InvocationFrame[] = [];
  const results: ProgramDataLog[] = [];
  let targetInvocationIndex = 0;

  for (const message of messages) {
    const invoke = message.match(INVOKE_PATTERN);
    if (invoke) {
      const frame: InvocationFrame = { programId: invoke[1] };
      if (frame.programId === targetProgramId) {
        frame.targetInvocationIndex = targetInvocationIndex++;
      }
      frames.push(frame);
      continue;
    }

    const exit = message.match(EXIT_PATTERN);
    if (exit) {
      const frame = frames.at(-1);
      if (frame?.programId === exit[1]) {
        frames.pop();
      } else {
        frames.length = 0;
      }
      continue;
    }

    const active = frames.at(-1);
    if (
      active?.programId === targetProgramId &&
      active.targetInvocationIndex !== undefined &&
      message.startsWith(DATA_PREFIX)
    ) {
      results.push({
        data: message.slice(DATA_PREFIX.length),
        invocationIndex: active.targetInvocationIndex,
      });
    }
  }

  return results;
}
