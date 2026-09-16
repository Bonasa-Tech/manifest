import { strict as assert } from 'node:assert';
import { EventEmitter } from 'node:events';
import type { Request, RequestHandler, Response } from 'express';
import { createExpensiveQueryAdmission } from '../../../scripts/stats_utils/httpAdmission';

interface FakeResponse extends EventEmitter {
  statusCode: number;
  body: unknown;
  headers: Record<string, string>;
  headersSent: boolean;
  setHeader: (name: string, value: string) => FakeResponse;
  status: (code: number) => FakeResponse;
  json: (body: unknown) => FakeResponse;
  finish: () => void;
}

function makeResponse(): FakeResponse {
  const res = new EventEmitter() as FakeResponse;
  res.statusCode = 0;
  res.body = undefined;
  res.headers = {};
  res.headersSent = false;
  res.setHeader = (name: string, value: string): FakeResponse => {
    res.headers[name] = value;
    return res;
  };
  res.status = (code: number): FakeResponse => {
    res.statusCode = code;
    return res;
  };
  res.json = (body: unknown): FakeResponse => {
    res.body = body;
    res.headersSent = true;
    res.emit('finish');
    return res;
  };
  res.finish = (): void => {
    res.headersSent = true;
    res.emit('finish');
  };
  return res;
}

function send(
  admission: RequestHandler,
  ip: string = '1.2.3.4',
): { res: FakeResponse; admitted: () => boolean } {
  const req = { ip, socket: {} } as unknown as Request;
  const res = makeResponse();
  let admitted = false;
  admission(req, res as unknown as Response, () => {
    admitted = true;
  });
  return { res, admitted: () => admitted };
}

describe('expensive query admission', () => {
  it('queues past the concurrency limit instead of rejecting', async () => {
    const admission = createExpensiveQueryAdmission({ maxConcurrent: 1 });
    const first = send(admission);
    const second = send(admission);
    assert.equal(first.admitted(), true);
    assert.equal(second.admitted(), false);
    assert.equal(second.res.statusCode, 0);

    first.res.finish();
    assert.equal(second.admitted(), true);
    assert.equal(second.res.statusCode, 0);
  });

  it('frees the slot when a client disconnects mid-flight', () => {
    const admission = createExpensiveQueryAdmission({ maxConcurrent: 1 });
    const first = send(admission);
    const second = send(admission);
    assert.equal(second.admitted(), false);

    first.res.emit('close');
    assert.equal(second.admitted(), true);
  });

  it('drops a queued request whose client disconnects', () => {
    const admission = createExpensiveQueryAdmission({ maxConcurrent: 1 });
    const first = send(admission);
    const queued = send(admission);
    const behind = send(admission);

    queued.res.emit('close');
    first.res.finish();
    assert.equal(queued.admitted(), false);
    assert.equal(behind.admitted(), true);
  });

  it('sheds load with Retry-After once the queue is full', () => {
    const admission = createExpensiveQueryAdmission({
      maxConcurrent: 1,
      maxQueued: 1,
    });
    send(admission);
    send(admission);
    const shed = send(admission);
    assert.equal(shed.admitted(), false);
    assert.equal(shed.res.statusCode, 503);
    assert.equal(shed.res.headers['Retry-After'], '1');
  });

  it('times a queued request out', async () => {
    const admission = createExpensiveQueryAdmission({
      maxConcurrent: 1,
      maxQueueWaitMs: 5,
    });
    send(admission);
    const queued = send(admission);
    await new Promise((resolve) => setTimeout(resolve, 20));
    assert.equal(queued.admitted(), false);
    assert.equal(queued.res.statusCode, 503);
  });

  it('applies no per-client request cap by default', () => {
    const admission = createExpensiveQueryAdmission({ maxConcurrent: 1 });
    for (let i = 0; i < 100; i++) {
      const { res, admitted } = send(admission);
      assert.equal(admitted(), true);
      res.finish();
    }
  });

  it('enforces an opt-in per-client cap per client', () => {
    const admission = createExpensiveQueryAdmission({
      maxConcurrent: 4,
      maxRequestsPerMinute: 2,
    });
    for (let i = 0; i < 2; i++) {
      send(admission, 'a').res.finish();
    }
    const blocked = send(admission, 'a');
    assert.equal(blocked.admitted(), false);
    assert.equal(blocked.res.statusCode, 429);
    assert.ok(Number(blocked.res.headers['Retry-After']) > 0);

    const other = send(admission, 'b');
    assert.equal(other.admitted(), true);
  });

  it('does not spend the capped budget on requests it sheds', () => {
    const admission = createExpensiveQueryAdmission({
      maxConcurrent: 1,
      maxQueued: 0,
      maxRequestsPerMinute: 2,
    });
    const first = send(admission, 'a');
    const shed = send(admission, 'a');
    assert.equal(shed.res.statusCode, 503);

    first.res.finish();
    // The shed request was refunded, so the second real request still fits.
    const second = send(admission, 'a');
    assert.equal(second.admitted(), true);
    second.res.finish();

    const third = send(admission, 'a');
    assert.equal(third.res.statusCode, 429);
  });
});
