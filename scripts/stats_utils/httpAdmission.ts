import type { RequestHandler, Response } from 'express';

type ClientWindow = {
  startedAt: number;
  requests: number;
};

type Waiter = {
  admit: () => void;
  reject: () => void;
};

const WINDOW_MS = 60_000;

/**
 * Bound unauthenticated work before a route can acquire a shared database or
 * RPC resource. This intentionally lives in-process: deployment edge controls
 * are useful but must not be the only admission control protecting services.
 *
 * Concurrency is the load-bearing control: it is what keeps a burst of
 * expensive queries from exhausting the connection pool. Callers over the
 * limit wait for a slot rather than being rejected outright, so a client that
 * sends requests faster than the service drains them is slowed, not failed.
 *
 * maxRequestsPerMinute is optional and off by default. It rejects rather than
 * slows, which is the wrong shape for first-party callers that legitimately
 * page through large result sets; enable it only where a hard per-client cap
 * is worth failed requests.
 */
export function createExpensiveQueryAdmission({
  maxConcurrent,
  maxRequestsPerMinute,
  maxQueued = 128,
  maxQueueWaitMs = 10_000,
}: {
  maxConcurrent: number;
  maxRequestsPerMinute?: number;
  maxQueued?: number;
  maxQueueWaitMs?: number;
}): RequestHandler {
  const clients = new Map<string, ClientWindow>();
  const maxTrackedClients = 10_000;
  const queue: Waiter[] = [];
  let active = 0;

  const retryAfter = (res: Response, seconds: number): void => {
    res.setHeader('Retry-After', String(Math.max(1, Math.ceil(seconds))));
  };

  const releaseSlot = (): void => {
    const next = queue.shift();
    if (next) {
      // Hand the slot straight to the next waiter; active stays unchanged.
      next.admit();
      return;
    }
    active -= 1;
  };

  return (req, res, next) => {
    const now = Date.now();
    const client = req.ip || req.socket.remoteAddress || 'unknown';

    let window: ClientWindow | undefined;
    if (maxRequestsPerMinute !== undefined) {
      const previous = clients.get(client);
      if (!previous && clients.size >= maxTrackedClients) {
        for (const [key, value] of clients) {
          if (now - value.startedAt >= WINDOW_MS) {
            clients.delete(key);
          }
        }
        if (clients.size >= maxTrackedClients) {
          retryAfter(res, WINDOW_MS / 1000);
          res.status(429).json({ error: 'Rate limit capacity exceeded' });
          return;
        }
      }
      window =
        previous && now - previous.startedAt < WINDOW_MS
          ? previous
          : { startedAt: now, requests: 0 };

      if (window.requests >= maxRequestsPerMinute) {
        retryAfter(res, (window.startedAt + WINDOW_MS - now) / 1000);
        res.status(429).json({ error: 'Rate limit exceeded' });
        return;
      }
      window.requests += 1;
      clients.set(client, window);
    }

    // A request that never ran must not count against the caller's budget,
    // or a busy service spends a client's whole window on rejections.
    const refund = (): void => {
      if (window) {
        window.requests -= 1;
      }
    };

    let settled = false;
    const admit = (): void => {
      if (settled) {
        return;
      }
      settled = true;
      let released = false;
      const release = (): void => {
        if (!released) {
          released = true;
          releaseSlot();
        }
      };
      res.once('finish', release);
      res.once('close', release);
      next();
    };

    if (active < maxConcurrent) {
      active += 1;
      admit();
      return;
    }

    if (queue.length >= maxQueued) {
      refund();
      retryAfter(res, 1);
      res.status(503).json({ error: 'Service is busy; retry shortly' });
      return;
    }

    const waiter: Waiter = {
      admit: () => {
        clearTimeout(timer);
        res.off('close', onClose);
        admit();
      },
      reject: () => {
        if (settled) {
          return;
        }
        settled = true;
        clearTimeout(timer);
        res.off('close', onClose);
        refund();
        if (!res.headersSent) {
          retryAfter(res, 1);
          res.status(503).json({ error: 'Service is busy; retry shortly' });
        }
      },
    };
    const drop = (): void => {
      const index = queue.indexOf(waiter);
      if (index !== -1) {
        queue.splice(index, 1);
      }
      waiter.reject();
    };
    const onClose = (): void => drop();
    const timer = setTimeout(drop, maxQueueWaitMs);
    // Don't hold the event loop open for a queued request.
    timer.unref?.();
    res.once('close', onClose);
    queue.push(waiter);
  };
}
