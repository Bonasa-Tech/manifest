// Bound how many /completeFills requests (including body reads) verification
// keeps in flight, and back off when the server asks us to. The stats server no
// longer applies a per-minute cap - it queues on concurrency instead - so there
// is no fixed pace to hold below; an edge proxy in front of it can still return
// 429, which is what the shared cooldown handles.
//
// This used to be a single process-wide queue, one request at a time. That is
// far below what the server serves (it admits STATS_MAX_CONCURRENT_QUERIES = 32
// expensive queries and queues past that) and it made a busy market's paging
// hostage to every other market's: with ~3.5k markets sharing one queue, a
// window that takes ~4s to fetch on its own took ~29 minutes in CI, because its
// six pages were interleaved behind thousands of single-page requests.
//
// The cap stays below the server's admission ceiling and matches the number of
// markets the verifier processes at once, so the pool - not this queue - is
// what bounds the load.
export const VERIFICATION_FETCH_CONCURRENCY = 8;

export function createVerificationFetch({
  fetchImpl = fetch,
  now = Date.now,
  sleep = (ms: number) =>
    new Promise<void>((resolve) => setTimeout(resolve, ms)),
  warn = console.warn,
  concurrency = VERIFICATION_FETCH_CONCURRENCY,
} = {}) {
  let inFlight = 0;
  // FIFO, so requests are issued in the order they were made rather than
  // whichever slot happens to free up first.
  const waiting: Array<() => void> = [];
  const acquire = (): Promise<void> =>
    new Promise<void>((resolve) => {
      if (inFlight < concurrency) {
        inFlight++;
        resolve();
        return;
      }
      waiting.push(() => {
        inFlight++;
        resolve();
      });
    });
  const release = (): void => {
    inFlight--;
    waiting.shift()?.();
  };
  let nextRequestAt = 0;

  return <T>(url: string, logPrefix: string): Promise<T> => {
    const request = (async (): Promise<T> => {
      await acquire();
      try {
        const maxAttempts = 5;
        for (let attempt = 1; attempt <= maxAttempts; attempt++) {
          while (now() < nextRequestAt) {
            await sleep(nextRequestAt - now());
          }
          let retryable = true;
          try {
            const response = await fetchImpl(url);
            if (!response.ok) {
              retryable = response.status === 429 || response.status >= 500;
              if (response.status === 429 || response.status === 503) {
                const header = response.headers.get('retry-after');
                const seconds = header === null ? NaN : Number(header);
                const retryAfterMs = Number.isFinite(seconds)
                  ? seconds * 1000
                  : Date.parse(header ?? '') - now();
                // Servers that send no Retry-After get a conservative cooldown,
                // shared even when this request exhausts its retries: a 429 waits
                // out a full window, a 503 is transient congestion.
                const fallbackMs = response.status === 429 ? 60_000 : 1_000;
                nextRequestAt = Math.max(
                  nextRequestAt,
                  now() +
                    (Number.isFinite(retryAfterMs) && retryAfterMs > 0
                      ? retryAfterMs
                      : fallbackMs),
                );
              }
              await response.body?.cancel().catch(() => {});
              throw new Error(
                `Failed to fetch fills: ${response.status} ${response.statusText}`,
              );
            }
            // Body streaming failures must also retry the same page.
            return (await response.json()) as T;
          } catch (error) {
            if (!retryable || attempt === maxAttempts) throw error;
            nextRequestAt = Math.max(
              nextRequestAt,
              now() + 1000 * 2 ** (attempt - 1),
            );
            warn(
              logPrefix,
              `completeFills fetch failed (${String(error)}), retrying in ${Math.ceil((nextRequestAt - now()) / 1000)}s (attempt ${attempt}/${maxAttempts})...`,
            );
          }
        }
        throw new Error('completeFills retries exhausted');
      } finally {
        // A failed market must not hold its slot: release on every exit path,
        // including the retry-exhausted and permanent-error throws above.
        release();
      }
    })();
    return request;
  };
}
