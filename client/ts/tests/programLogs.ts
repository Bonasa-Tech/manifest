import { assert } from 'chai';
import {
  extractProgramComputeUnits,
  extractProgramDataLogs,
} from '../src/utils/programLogs';
import {
  detectAggregatorFromKeys,
  getInvokedProgramIds,
  resolveOriginatingProtocol,
} from '../src/aggregators';

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

  it('reports compute units per Manifest invocation, including CPIs', () => {
    const logs = [
      `Program ${OTHER} invoke [1]`,
      `Program ${MANIFEST} invoke [2]`,
      `Program ${OTHER} invoke [3]`,
      `Program ${OTHER} consumed 1000 of 100000 compute units`,
      `Program ${OTHER} success`,
      `Program log: Program ${MANIFEST} consumed 9 of 9 compute units`,
      `Program ${MANIFEST} consumed 25000 of 100000 compute units`,
      `Program ${MANIFEST} success`,
      `Program ${OTHER} consumed 40000 of 200000 compute units`,
      `Program ${OTHER} success`,
      `Program ${MANIFEST} invoke [1]`,
      `Program ${MANIFEST} consumed 7000 of 50000 compute units`,
      `Program ${MANIFEST} success`,
      `Program ${MANIFEST} invoke [1]`,
      'Log truncated',
    ];

    assert.deepEqual(
      [...extractProgramComputeUnits(logs, MANIFEST)],
      [
        [0, 25000],
        [1, 7000],
      ],
    );
  });

  it('attributes aggregators only when the runtime invoked them', () => {
    const jupiter = 'JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4';
    const tx = {
      meta: {
        logMessages: [
          `Program ${OTHER} invoke [1]`,
          `Program log: Program ${jupiter} invoke [2]`,
          `Program ${OTHER} success`,
        ],
      },
    };
    const invoked = getInvokedProgramIds(tx);
    assert.deepEqual(invoked, [OTHER]);
    assert.isUndefined(detectAggregatorFromKeys(invoked));

    tx.meta.logMessages.splice(1, 0, `Program ${jupiter} invoke [2]`);
    assert.equal(detectAggregatorFromKeys(getInvokedProgramIds(tx)), 'Jupiter');
  });

  it('attributes protocols that sign rather than invoke', () => {
    // Relay's solver signs the transaction directly; it is never an invoked
    // program, so invoked program ids alone can never tag it.
    const relaySigner = 'F7p3dFrjRTbtRp8FRF6qHLomXbKRBzpvBLjtQcfcgmNe';
    assert.isUndefined(resolveOriginatingProtocol([OTHER], undefined));
    assert.equal(resolveOriginatingProtocol([OTHER], [relaySigner]), 'relay');
    // A protocol program still wins over the signers.
    const kamino = 'LiMoM9rMhrdYrfzUCxQppvxCSG1FcrUK9G8uLq4A1GF';
    assert.equal(resolveOriginatingProtocol([kamino], [relaySigner]), 'kamino');
  });
});
