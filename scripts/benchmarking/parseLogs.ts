export type LogContext = {
  rawLogs: string[];
  errors: string[];
  logMessages: string[];
  dataLogs: string[];
  programId: string;
  depth: number;
  id: number;
  instructionIndex: number;
  invokeResult?: string;
  cuConsumed?: number;
};

/**
 * @private
 */
function newLogContext(
  programId: string,
  depth: number,
  id: number,
  instructionIndex: number,
): LogContext {
  return {
    logMessages: [],
    dataLogs: [],
    rawLogs: [],
    errors: [],
    programId,
    depth,
    id,
    instructionIndex,
  };
}

/**
 * Parses transaction logs and provides additional context such as
 * - programId that generated the message
 * - call id of instruction, that generated the message
 * - call depth of instruction
 * - data messages, log messages and error messages
 * @param logs logs from TransactionResponse.meta.logs
 * @returns parsed logs with call depth and additional context
 */
export function parseLogs(logs: string[]): LogContext[] {
  const parserRe =
    /(?<logTruncated>^Log truncated$)|(?<programInvoke>^Program (?<invokeProgramId>[1-9A-HJ-NP-Za-km-z]{32,}) invoke \[(?<level>\d+)\]$)|(?<programSuccessResult>^Program (?<successResultProgramId>[1-9A-HJ-NP-Za-km-z]{32,}) success$)|(?<programFailedResult>^Program (?<failedResultProgramId>[1-9A-HJ-NP-Za-km-z]{32,}) failed: (?<failedResultErr>.*)$)|(?<programCompleteFailedResult>^Program failed to complete: (?<failedCompleteError>.*)$)|(?<programLog>^^Program log: (?<logMessage>.*)$)|(?<programConsumption>^^Program consumption: (?<consumptionComputeUnits>\d*) units remaining$)|(?<programData>^Program data: (?<data>.*)$)|(?<programConsumed>^Program (?<consumedProgramId>[1-9A-HJ-NP-Za-km-z]{32,}) consumed (?<consumedComputeUnits>\d*) of (?<allComputedUnits>\d*) compute units$)|(?<programReturn>^Program return: (?<returnProgramId>[1-9A-HJ-NP-Za-km-z]{32,}) (?<returnMessage>.*)$)/s;
  const result: LogContext[] = [];
  let id = -1;
  let currentInstruction = 0;
  let currentDepth = 0;
  const callStack: string[] = [];
  const callIds: number[] = [];
  for (const log of logs) {
    const match = parserRe.exec(log);
    if (!match || !match.groups) {
      throw new Error(`Failed to parse log line: ${log}`);
    }

    if (match.groups.logTruncated) {
      result[callIds[callIds.length - 1]].invokeResult = 'Log truncated';
    } else if (match.groups.programInvoke) {
      callStack.push(match.groups.invokeProgramId);
      id += 1;
      currentDepth += 1;
      callIds.push(id);
      if (match.groups.level != currentDepth.toString())
        throw new Error(
          `invoke depth mismatch, log: ${match.groups.level}, expected: ${currentDepth}`,
        );
      result.push(
        newLogContext(
          callStack[callStack.length - 1],
          callStack.length,
          id,
          currentInstruction,
        ),
      );
      result[result.length - 1].rawLogs.push(log);
    } else if (match.groups.programSuccessResult) {
      const lastProgram = callStack.pop();
      const lastCallIndex = callIds.pop();
      if (lastCallIndex === undefined) throw new Error('callIds malformed');
      if (lastProgram != match.groups.successResultProgramId)
        throw new Error('[ProramSuccess] callstack mismatch');
      result[lastCallIndex].rawLogs.push(log);
      currentDepth -= 1;
      if (currentDepth === 0) {
        currentInstruction += 1;
      }
    } else if (match.groups.programFailedResult) {
      const lastProgram = callStack.pop();
      if (lastProgram != match.groups.failedResultProgramId)
        throw new Error('[ProgramFailed] callstack mismatch');
      result[callIds[callIds.length - 1]].rawLogs.push(log);
      result[callIds[callIds.length - 1]].errors.push(
        match.groups.failedResultErr,
      );
    } else if (match.groups.programCompleteFailedResult) {
      result[callIds[callIds.length - 1]].rawLogs.push(log);
      result[callIds[callIds.length - 1]].errors.push(
        match.groups.failedCompleteError,
      );
    } else if (match.groups.programLog) {
      result[callIds[callIds.length - 1]].rawLogs.push(log);
      result[callIds[callIds.length - 1]].logMessages.push(
        match.groups.logMessage,
      );
    } else if (match.groups.programData) {
      result[callIds[callIds.length - 1]].rawLogs.push(log);
      result[callIds[callIds.length - 1]].dataLogs.push(match.groups.data);
    } else if (match.groups.programConsumed) {
      result[callIds[callIds.length - 1]].rawLogs.push(log);
      result[callIds[callIds.length - 1]].cuConsumed = Number(
        match.groups.consumedComputeUnits,
      );
    } else if (match.groups.programReturn) {
      if (callStack[callStack.length - 1] != match.groups.returnProgramId)
        throw new Error('[InvokeReturn]: callstack mismatch');
      result[callIds[callIds.length - 1]].invokeResult =
        match.groups.returnMessage;
    }
  }

  return result;
}
