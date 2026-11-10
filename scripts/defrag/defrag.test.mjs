import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { WebSocketServer } from 'ws';

const BLOCKHASH = '11111111111111111111111111111111';

async function startRpcServer() {
  const server = http.createServer((req, res) => {
    let body = '';
    req.on('data', (chunk) => {
      body += chunk;
    });
    req.on('end', () => {
      const call = JSON.parse(body);
      const result =
        call.method === 'getProgramAccounts'
          ? []
          : call.method === 'getSlot'
            ? 1_000
            : call.method === 'getLatestBlockhash'
              ? {
                  context: { slot: 1_000 },
                  value: { blockhash: BLOCKHASH, lastValidBlockHeight: 2_000 },
                }
              : null;
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ jsonrpc: '2.0', id: call.id, result }));
    });
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  return {
    server,
    url: `http://127.0.0.1:${server.address().port}`,
  };
}

function runDefrag(args, env) {
  const child = spawn(
    process.execPath,
    ['scripts/defrag/defrag.mjs', ...args],
    {
      cwd: process.cwd(),
      env: { ...process.env, ...env },
    },
  );
  let output = '';
  child.stdout.on('data', (chunk) => {
    output += chunk;
  });
  child.stderr.on('data', (chunk) => {
    output += chunk;
  });
  return { child, getOutput: () => output };
}

function waitForExit(child, timeoutMs = 8_000) {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      child.kill('SIGKILL');
      reject(new Error(`child did not exit within ${timeoutMs}ms`));
    }, timeoutMs);
    child.once('exit', (code, signal) => {
      clearTimeout(timeout);
      resolve({ code, signal });
    });
  });
}

test('fill mode requires sweep-only and an explicit loop frequency', async () => {
  const missingSweep = runDefrag(
    ['--filled-since-defrag', '--all', '--loop=1'],
    { RPC_URL: 'http://127.0.0.1:1' },
  );
  const missingSweepResult = await waitForExit(missingSweep.child);
  assert.equal(missingSweepResult.code, 1);
  assert.match(missingSweep.getOutput(), /requires --sweep-only/);

  const missingLoop = runDefrag(
    ['--filled-since-defrag', '--sweep-only', '--all'],
    { RPC_URL: 'http://127.0.0.1:1' },
  );
  const missingLoopResult = await waitForExit(missingLoop.child);
  assert.equal(missingLoopResult.code, 1);
  assert.match(missingLoop.getOutput(), /requires --loop=SECONDS/);
});

test('ordinary dry runs remain one-shot and write the ordinary plan cache', async (t) => {
  const { server, url } = await startRpcServer();
  const stateDir = fs.mkdtempSync(
    path.join(os.tmpdir(), 'manifest-defrag-once-'),
  );
  t.after(() => fs.rmSync(stateDir, { recursive: true, force: true }));
  t.after(() => new Promise((resolve) => server.close(resolve)));

  const { child, getOutput } = runDefrag(
    ['--sweep-only', '--all', '--v1', `--state-dir=${stateDir}`],
    { RPC_URL: url },
  );
  const result = await waitForExit(child);
  const output = getOutput();

  assert.equal(result.code, 0, output);
  assert.match(output, /Dry run only/);
  assert.doesNotMatch(output, /continuous sweep/);
  assert.deepEqual(fs.readdirSync(stateDir).sort(), [
    'inspect-payloads.json',
    'plan-cache.json',
  ]);
});

test('fill mode owns one reconnecting socket and repeats without fill-state files', async (t) => {
  const { server: rpcServer, url: rpcUrl } = await startRpcServer();
  const stateDir = fs.mkdtempSync(
    path.join(os.tmpdir(), 'manifest-defrag-loop-'),
  );
  const wsServer = new WebSocketServer({ port: 0, host: '127.0.0.1' });
  await new Promise((resolve) => wsServer.once('listening', resolve));
  t.after(() => fs.rmSync(stateDir, { recursive: true, force: true }));
  t.after(() => new Promise((resolve) => rpcServer.close(resolve)));
  t.after(() => new Promise((resolve) => wsServer.close(resolve)));

  let connections = 0;
  wsServer.on('connection', (socket) => {
    connections++;
    socket.send(
      JSON.stringify({ market: 'mock-market', slot: 1_000 + connections }),
    );
    if (connections === 1) setTimeout(() => socket.close(), 50);
  });

  const { child, getOutput } = runDefrag(
    [
      '--sweep-only',
      '--all',
      '--v1',
      '--filled-since-defrag',
      '--loop=0.25',
      `--state-dir=${stateDir}`,
    ],
    {
      RPC_URL: rpcUrl,
      FILL_FEED_URL: `ws://127.0.0.1:${wsServer.address().port}`,
    },
  );
  t.after(() => {
    if (child.exitCode === null) child.kill('SIGKILL');
  });

  await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      clearInterval(poll);
      reject(new Error(getOutput()));
    }, 8_000);
    const poll = setInterval(() => {
      if (
        connections === 2 &&
        getOutput().includes('continuous sweep 2 finished')
      ) {
        clearTimeout(timeout);
        clearInterval(poll);
        child.kill('SIGTERM');
        resolve();
      }
    }, 25);
  });
  await waitForExit(child);

  const output = getOutput();
  assert.equal(connections, 2, output);
  assert.match(output, /fill feed closed; reconnecting in 1000ms/);
  assert.match(output, /continuous sweep 2 finished/);
  assert.deepEqual(fs.readdirSync(stateDir).sort(), ['inspect-payloads.json']);
});
