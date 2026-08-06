import type { RequestHandler } from 'express';

type ClientWindow = {
  startedAt: number;
  requests: number;
};

/**
 * Bound unauthenticated work before a route can acquire a shared database or
 * RPC resource. This intentionally lives in-process: deployment edge controls
 * are useful but must not be the only admission control protecting services.
 */
export function createExpensiveQueryAdmission({
  maxConcurrent,
  maxRequestsPerMinute,
}: {
  maxConcurrent: number;
  maxRequestsPerMinute: number;
}): RequestHandler {
  const clients = new Map<string, ClientWindow>();
  const maxTrackedClients = 10_000;
  let active = 0;

  return (req, res, next) => {
    const now = Date.now();
    const client = req.ip || req.socket.remoteAddress || 'unknown';
    const previous = clients.get(client);
    if (!previous && clients.size >= maxTrackedClients) {
      for (const [key, value] of clients) {
        if (now - value.startedAt >= 60_000) {
          clients.delete(key);
        }
      }
      if (clients.size >= maxTrackedClients) {
        res.status(429).json({ error: 'Rate limit capacity exceeded' });
        return;
      }
    }
    const window =
      previous && now - previous.startedAt < 60_000
        ? previous
        : { startedAt: now, requests: 0 };

    if (window.requests >= maxRequestsPerMinute) {
      res.status(429).json({ error: 'Rate limit exceeded' });
      return;
    }
    window.requests += 1;
    clients.set(client, window);

    if (active >= maxConcurrent) {
      res.status(503).json({ error: 'Service is busy; retry shortly' });
      return;
    }
    active += 1;

    let released = false;
    const release = () => {
      if (!released) {
        released = true;
        active -= 1;
      }
    };
    res.once('finish', release);
    res.once('close', release);
    next();
  };
}
