// Read-only mainnet inventory. No wallet, instructions, or transaction submission.
import 'dotenv/config';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { gzipSync, gunzipSync } from 'node:zlib';
import { createHash } from 'node:crypto';
import bs58 from 'bs58';

export const PROGRAM = 'MNFSTqtC93rEfYHB6hF82sKdZpUDFWkViLByLd1k1Ms';
export const DISCRIMINANT = 4859840929024028656n;
export const NIL = 0xffffffff;
const HEADER = 256;
const BLOCK = 80;
const MAINNET_GENESIS = '5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d';

export function analyzeMarket(data) {
  assert(data.length >= HEADER, 'short header');
  assert.equal(data.readBigUInt64LE(0), DISCRIMINANT, 'discriminant');
  assert.equal(data[8], 0, 'unsupported market version');
  const allocated = data.readUInt32LE(152);
  assert.equal(allocated % BLOCK, 0, 'allocation alignment');
  assert(HEADER + allocated <= data.length, 'truncated allocation');
  const seen = new Set();
  const u32 = (index, offset = 0) => data.readUInt32LE(HEADER + index + offset);
  function claim(index) {
    assert(
      index % BLOCK === 0 && index + BLOCK <= allocated,
      'invalid node index',
    );
    assert(!seen.has(index), 'cycle or overlapping trees/free list');
    seen.add(index);
  }
  function tree(root) {
    const nodes = [];
    const pending = [[root, NIL]];
    while (pending.length) {
      const [index, parent] = pending.pop();
      if (index === NIL) continue;
      claim(index);
      assert.equal(u32(index, 8), parent, 'parent link');
      nodes.push(index);
      pending.push([u32(index), index], [u32(index, 4), index]);
    }
    return nodes;
  }
  const bids = tree(data.readUInt32LE(156));
  const asks = tree(data.readUInt32LE(164));
  const seats = tree(data.readUInt32LE(172));
  const seatSet = new Set(seats);
  const ownersWithOrders = new Set();
  for (const index of [...bids, ...asks]) {
    const owner = u32(index, 16 + 32);
    assert(seatSet.has(owner), 'order references missing seat');
    ownersWithOrders.add(owner);
  }
  const free = new Set();
  for (let index = data.readUInt32LE(176); index !== NIL; index = u32(index)) {
    claim(index);
    free.add(index);
  }
  assert.equal(seen.size * BLOCK, allocated, 'unclassified allocated nodes');
  let zeroBalanceSeatsWithOrders = 0;
  const reclaimableSeatIndices = [];
  for (const index of seats) {
    const payload = HEADER + index + 16;
    if (
      data.readBigUInt64LE(payload + 32) === 0n &&
      data.readBigUInt64LE(payload + 40) === 0n
    ) {
      if (ownersWithOrders.has(index)) zeroBalanceSeatsWithOrders++;
      else reclaimableSeatIndices.push(index);
    }
  }
  let tailFreeNodes = 0;
  for (
    let index = allocated - BLOCK;
    index >= 0 && free.has(index);
    index -= BLOCK
  )
    tailFreeNodes++;
  const liveNodes = bids.length + asks.length + seats.length;
  const retainedNodes = liveNodes - reclaimableSeatIndices.length;
  return {
    version: data[8],
    baseMint: bs58.encode(data.subarray(16, 48)),
    quoteMint: bs58.encode(data.subarray(48, 80)),
    bytes: data.length,
    allocatedNodes: allocated / BLOCK,
    unallocatedBytes: data.length - HEADER - allocated,
    bids: bids.length,
    asks: asks.length,
    seats: seats.length,
    freeNodes: free.size,
    tailFreeNodes,
    reclaimableSeats: reclaimableSeatIndices.length,
    zeroBalanceSeatsWithOrders,
    reclaimableSeatIndices,
    targetBytes: {
      tailOnly: HEADER + allocated - tailFreeNodes * BLOCK,
      compactFree: HEADER + liveNodes * BLOCK,
      compactFreeAndSeats: HEADER + retainedNodes * BLOCK,
      // Keep up to N existing spare nodes; never enlarge a market.
      keep1: Math.min(data.length, HEADER + (retainedNodes + 1) * BLOCK),
      keep2: Math.min(data.length, HEADER + (retainedNodes + 2) * BLOCK),
      keep8: Math.min(data.length, HEADER + (retainedNodes + 8) * BLOCK),
      keep32: Math.min(data.length, HEADER + (retainedNodes + 32) * BLOCK),
    },
  };
}

export async function rpc(method, params, attempt = 0) {
  assert(process.env.RPC_URL, 'RPC_URL is required (or use --snapshot)');
  let response;
  try {
    response = await fetch(process.env.RPC_URL, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
      signal: AbortSignal.timeout(120000),
    });
  } catch {
    throw new Error(`${method}: network request failed`);
  }
  assert(response.ok, `${method}: HTTP ${response.status}`);
  const body = await response.json();
  if (body.error?.code === -32016 && attempt < 5) {
    await new Promise((resolve) => setTimeout(resolve, 1000 * (attempt + 1)));
    return rpc(method, params, attempt + 1);
  }
  assert(
    !body.error && body.result !== undefined,
    `${method}: RPC error code ${body.error?.code}`,
  );
  return body.result;
}

async function main() {
  const args = process.argv.slice(2);
  assert(
    args.every(
      (arg, i) => i % 2 === 1 || ['--snapshot', '--out'].includes(arg),
    ),
    'Usage: node scripts/assess-market-rent.mjs [--snapshot file.json.gz] [--out directory]',
  );
  assert.equal(args.length % 2, 0, 'missing argument value');
  const options = Object.fromEntries(
    Array.from({ length: args.length / 2 }, (_, i) => [
      args[i * 2],
      args[i * 2 + 1],
    ]),
  );
  const out = options['--out'] ?? 'docs/assessments/mainnet-rent';
  let snapshot;
  if (options['--snapshot']) {
    snapshot = JSON.parse(gunzipSync(fs.readFileSync(options['--snapshot'])));
  } else {
    const genesis = await rpc('getGenesisHash', []);
    assert.equal(genesis, MAINNET_GENESIS, 'RPC is not mainnet');
    const discriminator = Buffer.alloc(8);
    discriminator.writeBigUInt64LE(DISCRIMINANT);
    console.log('Fetching every Manifest market at finalized commitment...');
    const accounts = await rpc('getProgramAccounts', [
      PROGRAM,
      {
        commitment: 'finalized',
        encoding: 'base64',
        withContext: true,
        filters: [{ memcmp: { offset: 0, bytes: bs58.encode(discriminator) } }],
      },
    ]);
    assert(accounts.value.length > 0, 'empty market inventory');
    const slot = accounts.context.slot;
    const blockTime = await rpc('getBlockTime', [slot]);
    const rentAccount = await rpc('getAccountInfo', [
      'SysvarRent111111111111111111111111111111111',
      { encoding: 'base64', commitment: 'finalized', minContextSlot: slot },
    ]);
    const rentData = Buffer.from(rentAccount.value.data[0], 'base64');
    const rate = Number(rentData.readBigUInt64LE(0));
    const threshold = rentData.readDoubleLE(8);
    const maxSize = Math.max(
      ...accounts.value.map(
        ({ account }) => Buffer.from(account.data[0], 'base64').length,
      ),
    );
    const rentChecks = [];
    for (const size of [0, HEADER, HEADER + BLOCK, maxSize]) {
      const lamports = await rpc('getMinimumBalanceForRentExemption', [
        size,
        { commitment: 'finalized' },
      ]);
      rentChecks.push({ size, lamports });
    }
    snapshot = {
      genesis,
      program: PROGRAM,
      fetchedAt: new Date().toISOString(),
      blockTime,
      accounts,
      rent: {
        rate,
        threshold,
        context: rentAccount.context,
        checks: rentChecks,
      },
    };
    fs.mkdirSync('.git/assessment-snapshots', { recursive: true });
    const snapshotPath = `.git/assessment-snapshots/mainnet-markets-${slot}.json.gz`;
    fs.writeFileSync(snapshotPath, gzipSync(JSON.stringify(snapshot)));
    console.log(`Snapshot: ${snapshotPath}`);
  }
  assert.equal(snapshot.genesis, MAINNET_GENESIS);
  assert.equal(snapshot.program, PROGRAM);
  const rent = (size) => {
    const value = Math.floor(
      (size + 128) * snapshot.rent.rate * snapshot.rent.threshold,
    );
    assert(Number.isSafeInteger(value) && value >= 0, 'unsafe rent arithmetic');
    return value;
  };
  for (const check of snapshot.rent.checks)
    assert.equal(rent(check.size), check.lamports, 'rent RPC cross-check');
  const rows = [];
  const keys = new Set();
  for (const { pubkey, account } of snapshot.accounts.value) {
    assert(!keys.has(pubkey), 'duplicate market');
    keys.add(pubkey);
    assert.equal(account.owner, PROGRAM);
    assert.equal(account.executable, false);
    assert(Number.isSafeInteger(account.lamports), 'unsafe balance arithmetic');
    const data = Buffer.from(account.data[0], 'base64');
    assert.equal(account.data[1], 'base64');
    if (account.space !== undefined) assert.equal(data.length, account.space);
    let row;
    try {
      row = analyzeMarket(data);
    } catch (error) {
      throw new Error(`${pubkey}: ${error.message}`);
    }
    const currentRent = rent(data.length);
    assert(
      account.lamports >= currentRent,
      `${pubkey}: below current rent minimum`,
    );
    rows.push({
      market: pubkey,
      ...row,
      lamports: account.lamports,
      currentRentLamports: currentRent,
      excessLamports: account.lamports - currentRent,
      rentReleasedLamports: Object.fromEntries(
        Object.entries(row.targetBytes).map(([name, size]) => [
          name,
          currentRent - rent(size),
        ]),
      ),
    });
  }
  const sum = (get) => rows.reduce((total, row) => total + get(row), 0);
  const totals = Object.fromEntries(
    [
      'bytes',
      'allocatedNodes',
      'unallocatedBytes',
      'bids',
      'asks',
      'seats',
      'freeNodes',
      'tailFreeNodes',
      'reclaimableSeats',
      'zeroBalanceSeatsWithOrders',
      'lamports',
      'currentRentLamports',
      'excessLamports',
    ].map((key) => [key, sum((row) => row[key])]),
  );
  const scenarios = Object.fromEntries(
    Object.keys(rows[0].targetBytes).map((name) => [
      name,
      {
        bytesRemoved: sum((row) => row.bytes - row.targetBytes[name]),
        rentReleasedLamports: sum((row) => row.rentReleasedLamports[name]),
        marketsShrunk: rows.filter((row) => row.targetBytes[name] < row.bytes)
          .length,
      },
    ]),
  );
  rows.sort(
    (a, b) =>
      b.rentReleasedLamports.compactFreeAndSeats -
      a.rentReleasedLamports.compactFreeAndSeats,
  );
  const summary = {
    program: PROGRAM,
    genesis: snapshot.genesis,
    slot: snapshot.accounts.context.slot,
    commitment: 'finalized',
    fetchedAt: snapshot.fetchedAt,
    blockTime: snapshot.blockTime,
    snapshotSha256: createHash('sha256')
      .update(JSON.stringify(snapshot))
      .digest('hex'),
    rent: snapshot.rent,
    markets: rows.length,
    totals,
    scenarios,
    emptyAfterReclamation: rows.filter(
      (row) => row.targetBytes.compactFreeAndSeats === HEADER,
    ).length,
    top10Share:
      rows
        .slice(0, 10)
        .reduce(
          (sum, row) => sum + row.rentReleasedLamports.compactFreeAndSeats,
          0,
        ) / scenarios.compactFreeAndSeats.rentReleasedLamports,
  };
  fs.mkdirSync(out, { recursive: true });
  fs.writeFileSync(
    path.join(out, 'summary.json'),
    JSON.stringify(summary, null, 2) + '\n',
  );
  fs.writeFileSync(path.join(out, 'markets.json'), JSON.stringify(rows) + '\n');
  console.log(JSON.stringify(summary, null, 2));
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href
) {
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
