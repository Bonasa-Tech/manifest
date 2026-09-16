import { strict as assert } from 'node:assert';
import { createVerificationFetch } from '../../../scripts/stats_utils/verificationFetch';

describe('verification fills request pacing', () => {
  // Requests are issued in the order they are made, so a shifted response queue
  // still lines up 1:1 with the urls even when several are in flight at once.
  function harness(responses: Array<Response | Error>, concurrency?: number) {
    let time = Date.parse('2026-01-01T00:00:00Z');
    const starts: number[] = [];
    const urls: string[] = [];
    const request = createVerificationFetch({
      now: () => time,
      sleep: async (ms: number) => {
        time += ms;
      },
      warn: () => {},
      concurrency,
      fetchImpl: (async (url) => {
        starts.push(time);
        urls.push(String(url));
        const response = responses.shift();
        if (response instanceof Error) throw response;
        assert.ok(response, 'unexpected extra request');
        return response;
      }) as typeof fetch,
    });
    return { request, starts, urls };
  }
  const ok = () => Response.json({ fills: [], hasMore: false });
  const tick = (): Promise<void> =>
    new Promise<void>((resolve) => setImmediate(resolve));

  it('issues markets and pages in order without an artificial pace', async () => {
    const { request, starts, urls } = harness(Array.from({ length: 40 }, ok));
    await Promise.all(
      Array.from({ length: 40 }, (_, i) => request(`page-${i}`, '')),
    );
    assert.deepEqual(
      urls,
      Array.from({ length: 40 }, (_, i) => `page-${i}`),
    );
    assert.equal(starts[starts.length - 1] - starts[0], 0);
  });

  it('caps requests in flight and hands each freed slot to the next waiter', async () => {
    const concurrency = 3;
    const gate: Array<() => void> = [];
    const urls: string[] = [];
    const request = createVerificationFetch({
      warn: () => {},
      concurrency,
      fetchImpl: (async (url) => {
        urls.push(String(url));
        await new Promise<void>((resolve) => gate.push(resolve));
        return ok();
      }) as typeof fetch,
    });

    const pageCount = 8;
    const all = Promise.all(
      Array.from({ length: pageCount }, (_, i) => request(`page-${i}`, '')),
    );
    await tick();
    assert.deepEqual(urls, ['page-0', 'page-1', 'page-2']);

    while (gate.length > 0) {
      gate.shift()!();
      await tick();
    }
    await all;
    assert.deepEqual(
      urls,
      Array.from({ length: pageCount }, (_, i) => `page-${i}`),
    );
  });

  it('frees the slot of a request that exhausts its retries', async () => {
    const { request, urls } = harness(
      [
        ...Array.from({ length: 5 }, () => new Response('', { status: 503 })),
        ok(),
      ],
      1,
    );
    await assert.rejects(request('failed', ''));
    assert.deepEqual(await request('next', ''), { fills: [], hasMore: false });
    assert.deepEqual(urls, [
      'failed',
      'failed',
      'failed',
      'failed',
      'failed',
      'next',
    ]);
  });

  for (const retryAfter of [
    undefined,
    '90',
    'Thu, 01 Jan 2026 00:01:30 GMT',
    'invalid',
  ]) {
    it(`shares 429 cooldown and retries the same page (${retryAfter})`, async () => {
      const { request, starts, urls } = harness(
        [
          new Response('', {
            status: 429,
            headers: retryAfter ? { 'Retry-After': retryAfter } : {},
          }),
          ok(),
          ok(),
        ],
        1,
      );
      await Promise.all([request('page-a', ''), request('page-b', '')]);
      assert.deepEqual(urls, ['page-a', 'page-a', 'page-b']);
      assert.ok(
        starts[1] - starts[0] >=
          (retryAfter && retryAfter !== 'invalid' ? 90_000 : 60_000),
      );
    });
  }

  it('preserves cooldown after retry exhaustion and keeps the queue usable', async () => {
    const { request, starts } = harness(
      [
        ...Array.from({ length: 5 }, () => new Response('', { status: 429 })),
        ok(),
      ],
      1,
    );
    const results = await Promise.allSettled([
      request('failed', ''),
      request('next', ''),
    ]);
    assert.equal(results[0].status, 'rejected');
    assert.equal(results[1].status, 'fulfilled');
    assert.equal(starts.length, 6);
    assert.ok(starts[5] - starts[4] >= 60_000);
  });

  it('fails permanent HTTP errors immediately and continues the queue', async () => {
    const { request, urls } = harness(
      [new Response('', { status: 400 }), ok()],
      1,
    );
    const results = await Promise.allSettled([
      request('bad', ''),
      request('good', ''),
    ]);
    assert.equal(results[0].status, 'rejected');
    assert.equal(results[1].status, 'fulfilled');
    assert.deepEqual(urls, ['bad', 'good']);
  });

  it('retries service, network, and body read failures', async () => {
    const { request, urls } = harness([
      new Response('', { status: 503 }),
      new TypeError('fetch failed'),
      new Response(
        new ReadableStream({
          start(controller) {
            controller.error(new TypeError('terminated'));
          },
        }),
      ),
      ok(),
    ]);
    assert.deepEqual(await request('page', ''), { fills: [], hasMore: false });
    assert.deepEqual(urls, ['page', 'page', 'page', 'page']);
  });
});
