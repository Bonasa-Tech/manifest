// /completeFills shares a 30 requests/minute admission limit. Serialize requests
// (including body reads) and leave a little headroom below that limit.
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
        nextRequestAt = now() + 2100;
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
              // Older servers send no Retry-After. A 429 needs a full window
              // cooldown, shared even when this request exhausts its retries.
              const fallbackMs = response.status === 429 ? 60_000 : 0;
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
