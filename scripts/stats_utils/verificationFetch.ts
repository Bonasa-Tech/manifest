// Serialize /completeFills requests (including body reads) so verification
// never has more than one page in flight, and back off when the server asks us
// to. The stats server no longer applies a per-minute cap - it queues on
// concurrency instead - so there is no fixed pace to hold below; an edge proxy
// in front of it can still return 429, which is what the cooldown handles.
export function createVerificationFetch({
  fetchImpl = fetch,
  now = Date.now,
  sleep = (ms: number) =>
    new Promise<void>((resolve) => setTimeout(resolve, ms)),
  warn = console.warn,
} = {}) {
  let queue: Promise<void> = Promise.resolve();
  let nextRequestAt = 0;

  return <T>(url: string, logPrefix: string): Promise<T> => {
    const request = queue.then(async () => {
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
    });
    // A failed market must not poison the queue for subsequent markets.
    queue = request.then(
      () => {},
      () => {},
    );
    return request;
  };
}
