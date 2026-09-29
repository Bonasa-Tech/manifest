// Read-only expansion of the market assessment to globals and token vaults.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { gzipSync, gunzipSync } from 'node:zlib';
import { createHash } from 'node:crypto';
import bs58 from 'bs58';
import {
  PROGRAM,
  DISCRIMINANT,
  NIL,
  analyzeMarket,
  rpc,
} from './assess-market-rent.mjs';

const GLOBAL = 10787423733276977665n;
const TOKEN = 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA';
const TOKEN22 = 'TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb';
const pk = (data, offset) => bs58.encode(data.subarray(offset, offset + 32));

export function analyzeGlobal(data, referencedOwners = new Set()) {
  assert(data.length >= 96);
  assert.equal(data.readBigUInt64LE(), GLOBAL);
  const allocated = data.readUInt32LE(88);
  assert.equal(allocated + 96, data.length);
  assert.equal(allocated % 64, 0);
  const seen = new Set();
  function claim(index) {
    assert(index % 64 === 0 && index + 64 <= allocated, 'global index bounds');
    assert(!seen.has(index), 'global duplicate/cycle');
    seen.add(index);
  }
  function tree(root) {
    const nodes = [];
    const stack = [[root, NIL]];
    while (stack.length) {
      const [i, parent] = stack.pop();
      if (i === NIL) continue;
      claim(i);
      assert.equal(data.readUInt32LE(96 + i + 8), parent);
      nodes.push(i);
      stack.push(
        [data.readUInt32LE(96 + i), i],
        [data.readUInt32LE(96 + i + 4), i],
      );
    }
    return nodes;
  }
  const traders = tree(data.readUInt32LE(72));
  const deposits = new Set(tree(data.readUInt32LE(76)));
  assert.equal(traders.length, deposits.size);
  assert.equal(traders.length, data.readUInt16LE(94));
  let zeroSeats = 0,
    reclaimableSeats = 0,
    depositedAtoms = 0n;
  const depositBalances = {};
  for (const index of traders) {
    const owner = pk(data, 96 + index + 16);
    const deposit = data.readUInt32LE(96 + index + 48);
    assert(deposits.delete(deposit), 'missing or reused global deposit');
    assert.equal(pk(data, 96 + deposit + 16), owner);
    const balance = data.readBigUInt64LE(96 + deposit + 48);
    depositBalances[owner] = balance.toString();
    depositedAtoms += balance;
    if (balance === 0n) {
      zeroSeats++;
      if (!referencedOwners.has(owner)) reclaimableSeats++;
    }
  }
  let freeNodes = 0;
  for (
    let index = data.readUInt32LE(84);
    index !== NIL;
    index = data.readUInt32LE(96 + index)
  ) {
    claim(index);
    freeNodes++;
  }
  assert.equal(seen.size * 64, allocated, 'unclassified global nodes');
  return {
    mint: pk(data, 8),
    vault: pk(data, 40),
    bytes: data.length,
    seats: traders.length,
    freeNodes,
    zeroSeats,
    reclaimableSeats,
    depositedAtoms: depositedAtoms.toString(),
    depositBalances,
    targetBytes: 96 + 128 * (traders.length - reclaimableSeats),
  };
}

export function analyzeVault(data, lamports, rent) {
  assert(data.length >= 165, 'short token account');
  assert([1, 2].includes(data[108]), 'uninitialized token account');
  assert.equal(
    data.readUInt32LE(129),
    0,
    'separate vault close authority requires review',
  );
  const amount = data.readBigUInt64LE(64);
  const native = data.readUInt32LE(109) === 1;
  const nativeReserve = native ? data.readBigUInt64LE(113) : 0n;
  const extensions = [];
  let withheld = 0n;
  let unknownCloseExtension = false;
  if (data.length > 165) {
    assert.equal(data[165], 2, 'not token account extension layout');
    for (let offset = 166; offset + 4 <= data.length; ) {
      const type = data.readUInt16LE(offset),
        size = data.readUInt16LE(offset + 2);
      if (type === 0) break;
      assert(offset + 4 + size <= data.length, 'truncated token extension');
      extensions.push({ type, size });
      if (type === 2) {
        assert.equal(size, 8);
        withheld += data.readBigUInt64LE(offset + 4);
      }
      // Known plain account extensions: TransferFeeAmount, ImmutableOwner,
      // MemoTransfer, CpiGuard, NonTransferableAccount, TransferHookAccount.
      if (![2, 7, 8, 13, 15, 27].includes(type)) unknownCloseExtension = true;
      offset += 4 + size;
    }
  }
  const minimum = rent(data.length);
  const nativeBacking = native ? amount + nativeReserve : 0n;
  assert(BigInt(lamports) >= nativeBacking, 'native backing deficit');
  const excess = native ? 0 : Math.max(0, lamports - minimum);
  return {
    mint: pk(data, 0),
    authority: pk(data, 32),
    bytes: data.length,
    lamports,
    amount: amount.toString(),
    native,
    nativeReserve: nativeReserve.toString(),
    withheldAtoms: withheld.toString(),
    extensions,
    rentLamports: minimum,
    excessLamports: excess,
    nativeReserveReductionLamports: native
      ? Math.max(0, Number(nativeReserve) - minimum)
      : 0,
    nativeUnaccountedLamports: native
      ? (BigInt(lamports) - nativeBacking).toString()
      : '0',
    closeCandidate: amount === 0n && withheld === 0n && !unknownCloseExtension,
  };
}

async function main() {
  const input = process.argv[2];
  let snapshot;
  if (input) snapshot = JSON.parse(gunzipSync(fs.readFileSync(input)));
  else {
    const genesis = await rpc('getGenesisHash', []);
    assert.equal(genesis, '5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d');
    console.log('Fetching markets and globals together...');
    const parts = [];
    for (const kind of [DISCRIMINANT, GLOBAL]) {
      const bytes = Buffer.alloc(8);
      bytes.writeBigUInt64LE(kind);
      parts.push(
        await rpc('getProgramAccounts', [
          PROGRAM,
          {
            commitment: 'finalized',
            withContext: true,
            encoding: 'base64',
            filters: [{ memcmp: { offset: 0, bytes: bs58.encode(bytes) } }],
          },
        ]),
      );
    }
    const core = {
      context: { slot: Math.max(...parts.map((x) => x.context.slot)) },
      componentSlots: parts.map((x) => x.context.slot),
      value: parts.flatMap((x) => x.value),
    };
    const vaultKeys = new Set();
    for (const { account } of core.value) {
      const data = Buffer.from(account.data[0], 'base64');
      const kind = data.readBigUInt64LE();
      if (kind === DISCRIMINANT) {
        vaultKeys.add(pk(data, 80));
        vaultKeys.add(pk(data, 112));
      } else if (kind === GLOBAL) vaultKeys.add(pk(data, 40));
      else throw new Error('Unknown Manifest account discriminator');
    }
    console.log(`Fetching ${vaultKeys.size} distinct token vaults...`);
    const keys = [...vaultKeys],
      vaults = [],
      slots = [];
    for (let start = 0; start < keys.length; start += 100) {
      const batch = keys.slice(start, start + 100);
      const response = await rpc('getMultipleAccounts', [
        batch,
        {
          commitment: 'finalized',
          encoding: 'base64',
          minContextSlot: core.context.slot,
        },
      ]);
      slots.push(response.context.slot);
      response.value.forEach((account, i) => {
        assert(account, `missing vault ${batch[i]}`);
        vaults.push({ pubkey: batch[i], account });
      });
      if (start % 1000 === 0)
        console.log(`Read ${vaults.length}/${keys.length} vaults`);
    }
    const rentAccount = await rpc('getAccountInfo', [
      'SysvarRent111111111111111111111111111111111',
      {
        commitment: 'finalized',
        encoding: 'base64',
        minContextSlot: core.context.slot,
      },
    ]);
    const data = Buffer.from(rentAccount.value.data[0], 'base64');
    const rent = {
      rate: Number(data.readBigUInt64LE()),
      threshold: data.readDoubleLE(8),
      slot: rentAccount.context.slot,
      checks: [],
    };
    for (const size of [96, 165, 182, 256, 128000])
      rent.checks.push({
        size,
        lamports: await rpc('getMinimumBalanceForRentExemption', [size]),
      });
    snapshot = {
      genesis,
      core,
      vaults,
      vaultSlots: { min: Math.min(...slots), max: Math.max(...slots) },
      rent,
      fetchedAt: new Date().toISOString(),
      blockTime: await rpc('getBlockTime', [core.context.slot]),
    };
    const file = `.git/assessment-snapshots/all-rent-${core.context.slot}.json.gz`;
    fs.writeFileSync(file, gzipSync(JSON.stringify(snapshot)));
    console.log(`Snapshot: ${file}`);
  }
  const rent = (size) =>
    Math.floor((size + 128) * snapshot.rent.rate * snapshot.rent.threshold);
  for (const check of snapshot.rent.checks)
    assert.equal(rent(check.size), check.lamports);
  const markets = [],
    globalsRaw = [],
    refsByMint = new Map(),
    ownersByMint = new Map(),
    globalOrders = [];
  for (const { pubkey, account } of snapshot.core.value) {
    assert.equal(account.owner, PROGRAM);
    const data = Buffer.from(account.data[0], 'base64');
    if (data.readBigUInt64LE() === GLOBAL) {
      globalsRaw.push({ pubkey, account, data });
      continue;
    }
    const row = analyzeMarket(data);
    const stack = [data.readUInt32LE(156), data.readUInt32LE(164)];
    while (stack.length) {
      const index = stack.pop();
      if (index === NIL) continue;
      const offset = 256 + index;
      stack.push(data.readUInt32LE(offset), data.readUInt32LE(offset + 4));
      if (data[offset + 16 + 41] === 3) {
        const mint = data[offset + 16 + 40] ? row.quoteMint : row.baseMint;
        const traderIndex = data.readUInt32LE(offset + 48);
        const owner = pk(data, 256 + traderIndex + 16);
        const baseAtoms = data.readBigUInt64LE(offset + 32);
        const price =
          data.readBigUInt64LE(offset + 16) +
          (data.readBigUInt64LE(offset + 24) << 64n);
        const required = data[offset + 56]
          ? (baseAtoms * price + 10n ** 18n - 1n) / 10n ** 18n
          : baseAtoms;
        const lastValidSlot = data.readUInt32LE(offset + 52);
        globalOrders.push({
          market: pubkey,
          index,
          mint,
          owner,
          lastValidSlot,
          expired:
            lastValidSlot !== 0 &&
            lastValidSlot < snapshot.core.componentSlots[0],
          requiredAtoms: required.toString(),
        });
        refsByMint.set(mint, (refsByMint.get(mint) ?? 0) + 1);
        if (!ownersByMint.has(mint)) ownersByMint.set(mint, new Set());
        ownersByMint.get(mint).add(owner);
      }
    }
    const excess = account.lamports - rent(row.bytes);
    assert(excess >= 0);
    markets.push({
      market: pubkey,
      ...row,
      baseVault: pk(data, 80),
      quoteVault: pk(data, 112),
      excessLamports: excess,
      recoverableLamports: Object.fromEntries(
        Object.entries(row.targetBytes).map(([name, size]) => [
          name,
          account.lamports - rent(size),
        ]),
      ),
    });
  }
  const globals = globalsRaw.map(({ pubkey, account, data }) => {
    const row = analyzeGlobal(data, ownersByMint.get(pk(data, 8)) ?? new Set());
    const orders = refsByMint.get(row.mint) ?? 0;
    const gasReserve = orders * 5000;
    const excess = account.lamports - rent(row.bytes) - gasReserve;
    assert(excess >= 0, `global reserve deficit: ${pubkey}`);
    return {
      global: pubkey,
      ...row,
      lamports: account.lamports,
      globalOrders: orders,
      gasReserveLamports: gasReserve,
      excessAfterGasReserveLamports: excess,
      resizeReleasedLamports: rent(row.bytes) - rent(row.targetBytes),
      // Extra conservative reserve for the retained seats' admission-fee purpose.
      retainedSeatFeeBufferLamports:
        (row.seats - row.reclaimableSeats) * 2 * rent(165),
    };
  });
  const globalsByMint = new Map(globals.map((row) => [row.mint, row]));
  for (const order of globalOrders) {
    const global = globalsByMint.get(order.mint);
    assert(global, 'global order mint missing global');
    order.globalBalanceAtoms = global.depositBalances[order.owner] ?? '0';
    order.underfunded =
      BigInt(order.globalBalanceAtoms) < BigInt(order.requiredAtoms) ||
      BigInt(order.requiredAtoms) > 0xffffffffffffffffn;
    order.cleanEligible = order.expired || order.underfunded;
  }
  const vaults = snapshot.vaults.map(({ pubkey, account }) => {
    assert(
      [TOKEN, TOKEN22].includes(account.owner),
      'unexpected vault program',
    );
    const row = analyzeVault(
      Buffer.from(account.data[0], 'base64'),
      account.lamports,
      rent,
    );
    assert.equal(row.authority, pubkey, 'unexpected vault authority');
    return { vault: pubkey, tokenProgram: account.owner, ...row };
  });
  const byVault = new Map(vaults.map((row) => [row.vault, row]));
  const retirementVaults = new Set(),
    retirementMarkets = [],
    retirementGlobals = [];
  for (const row of markets) {
    const base = byVault.get(row.baseVault),
      quote = byVault.get(row.quoteVault);
    assert.equal(base.mint, row.baseMint);
    assert.equal(quote.mint, row.quoteMint);
    if (
      row.targetBytes.compactFreeAndSeats === 256 &&
      base.closeCandidate &&
      quote.closeCandidate
    ) {
      retirementMarkets.push(row.market);
      retirementVaults.add(base.vault);
      retirementVaults.add(quote.vault);
    }
  }
  for (const row of globals) {
    const vault = byVault.get(row.vault);
    assert.equal(vault.mint, row.mint);
    if (
      row.targetBytes === 96 &&
      row.globalOrders === 0 &&
      vault.closeCandidate
    ) {
      retirementGlobals.push(row.global);
      retirementVaults.add(row.vault);
    }
  }
  const sum = (rows, key) => rows.reduce((sum, row) => sum + row[key], 0);
  const globalSummary = {
    accounts: globals.length,
    seats: sum(globals, 'seats'),
    freeNodes: sum(globals, 'freeNodes'),
    reclaimableSeats: sum(globals, 'reclaimableSeats'),
    globalOrders: sum(globals, 'globalOrders'),
    gasReserveLamports: sum(globals, 'gasReserveLamports'),
    resizeReleasedLamports: sum(globals, 'resizeReleasedLamports'),
    excessAfterGasReserveLamports: sum(
      globals,
      'excessAfterGasReserveLamports',
    ),
    retainedSeatFeeBufferLamports: sum(
      globals,
      'retainedSeatFeeBufferLamports',
    ),
    recoverableWithSeatFeeBufferLamports: globals.reduce(
      (s, row) =>
        s +
        row.resizeReleasedLamports +
        Math.max(
          0,
          row.excessAfterGasReserveLamports - row.retainedSeatFeeBufferLamports,
        ),
      0,
    ),
  };
  const vaultSummary = {
    accounts: vaults.length,
    classic: vaults.filter((x) => x.tokenProgram === TOKEN).length,
    token2022: vaults.filter((x) => x.tokenProgram === TOKEN22).length,
    native: vaults.filter((x) => x.native).length,
    excessNonNativeLamports: sum(vaults, 'excessLamports'),
    nativeReserveReductionLamports: sum(
      vaults,
      'nativeReserveReductionLamports',
    ),
    nativeUnaccountedLamports: vaults
      .reduce((s, row) => s + BigInt(row.nativeUnaccountedLamports), 0n)
      .toString(),
    dataSizes: [...new Set(vaults.map((x) => x.bytes))].sort((a, b) => a - b),
    retiringEmptyVaults: retirementVaults.size,
    additionalClosureLamports: vaults
      .filter((row) => retirementVaults.has(row.vault))
      .reduce((s, row) => s + row.lamports - row.excessLamports, 0),
  };
  const combined = Object.fromEntries(
    Object.keys(markets[0].recoverableLamports).map((name) => {
      const marketLamports = markets.reduce(
        (s, row) => s + row.recoverableLamports[name],
        0,
      );
      return [
        name,
        {
          marketLamports,
          globalLamports: globalSummary.recoverableWithSeatFeeBufferLamports,
          vaultLamports: vaultSummary.excessNonNativeLamports,
          totalLamports:
            marketLamports +
            globalSummary.recoverableWithSeatFeeBufferLamports +
            vaultSummary.excessNonNativeLamports,
        },
      ];
    }),
  );
  const retiringMarkets = new Set(retirementMarkets);
  const totalsWithRetirement = Object.fromEntries(
    Object.keys(combined).map((name) => [
      name,
      combined[name].totalLamports +
        markets
          .filter((row) => retiringMarkets.has(row.market))
          .reduce((s, row) => s + rent(row.targetBytes[name]), 0) +
        retirementGlobals.length * rent(96) +
        vaultSummary.additionalClosureLamports,
    ]),
  );
  const summary = {
    fetchedAt: snapshot.fetchedAt,
    blockTime: snapshot.blockTime,
    coreSlot: snapshot.core.context.slot,
    coreComponentSlots: snapshot.core.componentSlots,
    vaultSlots: snapshot.vaultSlots,
    rent: snapshot.rent,
    snapshotSha256: createHash('sha256')
      .update(JSON.stringify(snapshot))
      .digest('hex'),
    markets: markets.length,
    globals: globalSummary,
    vaults: vaultSummary,
    combined,
    gasPrepayments: {
      orders: globalOrders.length,
      reserveLamports: globalOrders.length * 5000,
      marketsWithGlobalOrders: new Set(globalOrders.map((x) => x.market)).size,
      globalsWithOrders: refsByMint.size,
      expired: globalOrders.filter((x) => x.expired).length,
      underfunded: globalOrders.filter((x) => x.underfunded).length,
      cleanEligible: globalOrders.filter((x) => x.cleanEligible).length,
      cleanupRefundLamports:
        globalOrders.filter((x) => x.cleanEligible).length * 5000,
      globalsWithoutOrders: globals.filter((x) => x.globalOrders === 0).length,
      surplusAfterRentAndGasLamports:
        globalSummary.excessAfterGasReserveLamports,
      exactAbandonedGasLamports: null,
    },
    optionalRetirement: {
      markets: retirementMarkets.length,
      globals: retirementGlobals.length,
      marketHeadersLamports: retirementMarkets.length * rent(256),
      globalHeadersLamports: retirementGlobals.length * rent(96),
      vaultClosureLamports: vaultSummary.additionalClosureLamports,
      totalsWithRetirement,
    },
  };
  const out = 'docs/assessments/mainnet-rent';
  fs.writeFileSync(
    `${out}/expanded-summary.json`,
    JSON.stringify(summary, null, 2) + '\n',
  );
  fs.writeFileSync(
    `${out}/globals.json`,
    JSON.stringify(globals, null, 2) + '\n',
  );
  fs.writeFileSync(`${out}/vaults.json`, JSON.stringify(vaults) + '\n');
  fs.writeFileSync(
    `${out}/expanded-markets.json`,
    JSON.stringify(markets) + '\n',
  );
  fs.writeFileSync(
    `${out}/gas-prepayments.json`,
    JSON.stringify(globalOrders) + '\n',
  );
  console.log(JSON.stringify(summary, null, 2));
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href
)
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
