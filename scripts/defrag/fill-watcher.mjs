// Records which markets are trading, from the Manifest fill feed.
//
// The feed (wss://mfx-feed-mainnet.fly.dev) is a live broadcaster: it has no
// history, no replay and no per-market query, so a one-shot defrag run cannot
// ask it what traded since the last sweep. This process stays connected between
// sweeps and keeps a market -> newest fill slot map on disk, which
// `defrag.mjs --filled-since-defrag` reads.
//
// Run it wherever the sweep is scheduled, pointed at the same --state-dir:
//
//   node scripts/defrag/fill-watcher.mjs --state-dir=./defrag-state
//
// Flags:
//   --state-dir=DIR     where fill-state.json lives (default ./defrag-state)
//   --url=WSS           feed endpoint (default wss://mfx-feed-mainnet.fly.dev)
//   --flush=MS          how often to write the state file (default 5000)
//   --retain-slots=N    drop markets whose last fill is older than this
//                       (default 2_160_000, ~10 days)
//   --quiet             do not log periodic counts
//
// It is safe to restart: the state file is reloaded on start, so a restart
// loses only the fills that occurred while it was down. The sweep refuses to
// filter when the recorded slot falls too far behind the chain, so a watcher
// that dies fails the sweep loudly instead of silently shrinking it.

import fs from 'node:fs';
import path from 'node:path';
import WebSocket from 'ws';

const flag = (name) => process.argv.find((a) => a.startsWith(`--${name}=`))?.slice(name.length + 3);
const stateDir = flag('state-dir') ?? path.join(process.cwd(), 'defrag-state');
fs.mkdirSync(stateDir, { recursive: true });
const statePath = path.join(stateDir, flag('fill-state') ?? 'fill-state.json');
const FEED_URL = flag('url') ?? process.env.FILL_FEED_URL ?? 'wss://mfx-feed-mainnet.fly.dev';
const FLUSH_MS = Number(flag('flush') ?? 5_000);
// Bound the file. A market that has not traded in ten days is not one this
// filter needs an entry for: the sweep treats a missing entry as "no fill",
// which is the correct answer for a book that stopped trading.
const RETAIN_SLOTS = Number(flag('retain-slots') ?? 2_160_000);
const quiet = process.argv.includes('--quiet');
if (!Number.isFinite(FLUSH_MS) || FLUSH_MS < 250) throw new Error(`invalid --flush: ${FLUSH_MS}`);
if (!Number.isInteger(RETAIN_SLOTS) || RETAIN_SLOTS < 1) throw new Error(`invalid --retain-slots: ${RETAIN_SLOTS}`);

/// market -> newest fill slot seen. Seeded from disk so a restart keeps
/// whatever the previous run had learned.
const markets = new Map();
let lastSeenSlot = 0;
// The slot from which coverage is CONTINUOUS. A market that last filled before
// this is simply unknown to us, not quiet, and the sweep must keep it rather
// than assume silence. Starting fresh, or restarting after a gap, moves this
// forward; that is the whole point of recording it.
let coverageFrom = 0;
let priorLastSeenSlot = 0;
let resumed = false;
if (fs.existsSync(statePath)) {
  try {
    const prior = JSON.parse(fs.readFileSync(statePath, 'utf8'));
    for (const [m, slot] of Object.entries(prior.markets ?? {})) markets.set(m, slot);
    lastSeenSlot = prior.lastSeenSlot ?? 0;
    priorLastSeenSlot = lastSeenSlot;
    coverageFrom = prior.coverageFrom ?? 0;
    resumed = true;
    console.log(`resumed ${markets.size} markets from ${statePath} (slot ${lastSeenSlot}, coverage from ${coverageFrom})`);
  } catch (e) {
    console.warn(`ignoring unreadable ${statePath}: ${e.message}`);
  }
}
// Slots missed while down are a hole in coverage. Allow a short restart
// window, beyond which coverage has to begin again at the first fill we see.
const RESTART_TOLERANCE_SLOTS = Number(flag('restart-tolerance') ?? 150); // ~1 min
let coverageSettled = !resumed;

let dirty = false;
let fills = 0;

function flush() {
  if (!dirty) return;
  // Prune before writing so the file cannot grow without bound.
  if (lastSeenSlot > RETAIN_SLOTS) {
    const floor = lastSeenSlot - RETAIN_SLOTS;
    for (const [m, slot] of markets) if (slot < floor) markets.delete(m);
  }
  const body = JSON.stringify({
    updatedAt: new Date().toISOString(),
    lastSeenSlot,
    coverageFrom,
    feed: FEED_URL,
    markets: Object.fromEntries(markets),
  });
  // Write-then-rename so a sweep never reads a half-written file.
  const tmp = `${statePath}.tmp`;
  fs.writeFileSync(tmp, body);
  fs.renameSync(tmp, statePath);
  dirty = false;
}

setInterval(() => {
  flush();
  if (!quiet) {
    console.log(`${new Date().toISOString()} slot ${lastSeenSlot} | ${markets.size} markets | ${fills} fills seen`);
  }
}, FLUSH_MS).unref?.();

let backoff = 1_000;
let activeSocket;
let reconnectTimer;

function connect() {
  // `error` is normally followed by `close`, and both may arrive for one
  // failed connection. Keep one owner for the socket and one reconnect timer
  // so an outage cannot fan out into multiple live feed subscriptions.
  if (activeSocket || reconnectTimer) return;
  const ws = new WebSocket(FEED_URL);
  activeSocket = ws;
  let alive = false;
  let closeReason = 'closed';

  ws.on('open', () => {
    alive = true;
    backoff = 1_000;
    console.log(`connected to ${FEED_URL}`);
  });

  ws.on('message', (data) => {
    let fill;
    try {
      fill = JSON.parse(String(data));
    } catch {
      return; // heartbeats and anything else that is not a fill
    }
    const { market, slot } = fill ?? {};
    if (typeof market !== 'string' || !Number.isFinite(slot)) return;
    // First fill after a start tells us where the chain is, which is the only
    // way this process learns the current slot without an RPC.
    if (!coverageSettled) {
      coverageSettled = true;
      const gap = slot - priorLastSeenSlot;
      if (gap > RESTART_TOLERANCE_SLOTS) {
        coverageFrom = slot;
        console.warn(
          `was down for ~${gap} slots; coverage restarts at ${slot}. ` +
            'Markets last swept before that are reported unknown, not quiet.',
        );
      }
      dirty = true;
    } else if (coverageFrom === 0) {
      coverageFrom = slot;
      dirty = true;
    }
    fills++;
    if (slot > lastSeenSlot) lastSeenSlot = slot;
    // Keep the newest: the feed can deliver slightly out of order.
    if (!(markets.get(market) >= slot)) markets.set(market, slot);
    dirty = true;
  });

  const reconnect = () => {
    // Ignore a duplicate terminal event from a socket already handled.
    if (activeSocket !== ws) return;
    activeSocket = undefined;
    if (!alive && backoff < 30_000) backoff = Math.min(backoff * 2, 30_000);
    alive = false;
    flush();
    // Brief WebSocket observation gaps are an accepted tradeoff for this
    // operational filter. We intentionally keep the existing coverage window
    // across a transport reconnect; a fill seen only during the gap can be
    // missed until that market fills again. The defrag runner's lag gate still
    // refuses state that remains materially behind when a sweep starts.
    console.warn(`feed ${closeReason}; reconnecting in ${backoff}ms`);
    reconnectTimer = setTimeout(() => {
      reconnectTimer = undefined;
      connect();
    }, backoff);
  };

  ws.on('error', (e) => {
    closeReason = `error: ${e.message}`;
    // `close` is the sole reconnect path, guaranteeing one replacement socket.
    ws.terminate();
  });
  ws.on('close', reconnect);
}

for (const sig of ['SIGINT', 'SIGTERM']) {
  process.on(sig, () => {
    flush();
    console.log(`\nflushed ${markets.size} markets to ${statePath}`);
    process.exit(0);
  });
}

connect();
