import { assert } from 'chai';
import { extractProgramDataLogs } from '../src/utils/programLogs';

const MANIFEST = 'MNFSTqtC93rEfYHB6hF82sKdZpUDFWkViLByLd1k1Ms';
const OTHER = '11111111111111111111111111111111';

describe('program log attribution', () => {
  it('accepts data only from the active Manifest invocation frame', () => {
    const logs = [
      `Program ${OTHER} invoke [1]`,
      'Program data: forged-top-level',
      `Program ${MANIFEST} invoke [2]`,
      'Program data: real-cpi',
      `Program ${MANIFEST} success`,
      'Program data: forged-after-cpi',
      `Program ${OTHER} success`,
    ];

    assert.deepEqual(extractProgramDataLogs(logs, MANIFEST), [
      { data: 'real-cpi', invocationIndex: 0 },
    ]);
  });

  it('assigns stable indexes to separate Manifest invocations', () => {
    const logs = [
      `Program ${MANIFEST} invoke [1]`,
      'Program data: first',
      `Program ${MANIFEST} success`,
      `Program ${MANIFEST} invoke [1]`,
      'Program data: second',
      `Program ${MANIFEST} success`,
    ];

    assert.deepEqual(extractProgramDataLogs(logs, MANIFEST), [
      { data: 'first', invocationIndex: 0 },
      { data: 'second', invocationIndex: 1 },
    ]);
  });
});
