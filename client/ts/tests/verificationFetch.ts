import { strict as assert } from 'node:assert';
import { createVerificationFetch } from '../../../scripts/stats_utils/verificationFetch';

describe('verification fills request pacing', () => {
  function harness(responses: Array<Response | Error>) {
    let time = Date.parse('2026-01-01T00:00:00Z');
    const starts: number[] = [];
    const urls: string[] = [];
    const request = createVerificationFetch({
      now: () => time,
      sleep: async (ms: number) => {
        time += ms;
      },
      warn: () => {},
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

  it('paces all markets and pages below 30 requests per minute', async () => {
    const { request, starts } = harness(Array.from({ length: 40 }, ok));
    await Promise.all(
      Array.from({ length: 40 }, (_, i) => request(`page-${i}`, '')),
    );
    for (let i = 1; i < starts.length; i++) {
      assert.ok(starts[i] - starts[i - 1] >= 2100);
    }
    assert.ok(starts[30] - starts[0] > 60_000);
  });

  for (const retryAfter of [
    undefined,
    '90',
    'Thu, 01 Jan 2026 00:01:30 GMT',
    'invalid',
  ]) {
    it(`shares 429 cooldown and retries the same page (${retryAfter})`, async () => {
      const { request, starts, urls } = harness([
        new Response('', {
          status: 429,
          headers: retryAfter ? { 'Retry-After': retryAfter } : {},
        }),
        ok(),
        ok(),
      ]);
      await Promise.all([request('page-a', ''), request('page-b', '')]);
      assert.deepEqual(urls, ['page-a', 'page-a', 'page-b']);
      assert.ok(
        starts[1] - starts[0] >=
          (retryAfter && retryAfter !== 'invalid' ? 90_000 : 60_000),
      );
      assert.ok(starts[2] - starts[1] >= 2100);
    });
  }

  it('preserves cooldown after retry exhaustion and keeps the queue usable', async () => {
    const { request, starts } = harness([
      ...Array.from({ length: 5 }, () => new Response('', { status: 429 })),
      ok(),
    ]);
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
    const { request, urls } = harness([
      new Response('', { status: 400 }),
      ok(),
    ]);
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
