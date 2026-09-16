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
const CONSUMED_PATTERN =
  /^Program ([1-9A-HJ-NP-Za-km-z]+) consumed (\d+) of \d+ compute units$/;
const DATA_PREFIX = 'Program data: ';

/**
 * Walk the invocation stack described by the runtime's invoke/success/failed
 * lines and call `visit` with every other line emitted while the requested
 * program is the active frame, along with that invocation's index. The
 * invocation index is stable across top-level and CPI calls.
 */
function walkTargetFrames(
  messages: string[],
  targetProgramId: string,
  visit: (message: string, invocationIndex: number) => void,
): void {
  const frames: InvocationFrame[] = [];
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
      active.targetInvocationIndex !== undefined
    ) {
      visit(message, active.targetInvocationIndex);
    }
  }
}

/**
 * Return only data emitted while the requested program is the active Solana
 * invocation frame. The invocation index is stable across top-level and CPI
 * calls and can be used to associate decoded events with inferred effects.
 */
export function extractProgramDataLogs(
  messages: string[],
  targetProgramId: string,
): ProgramDataLog[] {
  const results: ProgramDataLog[] = [];
  walkTargetFrames(messages, targetProgramId, (message, invocationIndex) => {
    if (message.startsWith(DATA_PREFIX)) {
      results.push({
        data: message.slice(DATA_PREFIX.length),
        invocationIndex,
      });
    }
  });
  return results;
}

/**
 * Compute units consumed by each invocation of the requested program, keyed
 * by invocation index. The runtime emits "Program <id> consumed N of M compute
 * units" inside the program's own frame just before it exits, and N includes
 * every CPI that invocation made. Invocations whose consumed line was
 * truncated away are absent from the result.
 */
export function extractProgramComputeUnits(
  messages: string[],
  targetProgramId: string,
): Map<number, number> {
  const results: Map<number, number> = new Map();
  walkTargetFrames(messages, targetProgramId, (message, invocationIndex) => {
    const consumed = message.match(CONSUMED_PATTERN);
    if (consumed && consumed[1] === targetProgramId) {
      results.set(invocationIndex, Number(consumed[2]));
    }
  });
  return results;
}
