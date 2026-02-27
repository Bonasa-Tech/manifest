import { ManifestClient, Market } from '@bonasa-tech/manifest-sdk';
import {
  AddressLookupTableAccount,
  AddressLookupTableProgram,
  Connection,
  Keypair,
  MessageV0,
  PublicKey,
  Transaction,
  VersionedTransaction,
} from '@solana/web3.js';
import { Pool } from 'pg';

const { RPC_URL, DATABASE_URL, PRIVATE_KEY } = process.env;

if (!RPC_URL) {
  throw new Error('RPC_URL missing from env');
}
if (!DATABASE_URL) {
  throw new Error('DATABASE_URL missing from env');
}
if (!PRIVATE_KEY) {
  throw new Error('PRIVATE_KEY missing from env');
}

async function initDatabase(pool: Pool): Promise<void> {
  try {
    // Create tables if they don't exist
    await pool.query(`
            CREATE TABLE IF NOT EXISTS alts (
                address TEXT NOT NULL PRIMARY KEY,
                created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
                remaining_space INTEGER NOT NULL )
        `);

    await pool.query(`
            CREATE TABLE IF NOT EXISTS alt_markets (
                alt TEXT NOT NULL,
                market TEXT NOT NULL PRIMARY KEY )
        `);

    console.log('Database schema initialized');
  } catch (error) {
    console.error('Error initializing database:', error);
    throw error;
  }
}

async function findMarketsWithoutAlt(
  connection: Connection,
  pool: Pool,
): Promise<string[]> {
  const allMarketPks = new Set(
    (await ManifestClient.listMarketPublicKeys(connection)).map((pk) =>
      pk.toString(),
    ),
  );
  const altQueryResult = await pool.query('SELECT market from alt_markets');
  const altMarketPks = new Set(
    altQueryResult.rows.map((r: any) => r.market as string),
  );
  return allMarketPks.difference(altMarketPks).values().toArray();
}

async function addMarketToAlts(
  connection: Connection,
  pool: Pool,
  keypair: Keypair,
  market: string,
) {
  const marketPk = new PublicKey(market);
  const client = await ManifestClient.getClientReadOnly(connection, marketPk);
  const pksForSwap = await client.getSwapAltPks();
  // Assume programs are already on ALT as a rough estimate.
  const spaceNeeded = pksForSwap.size - 3;

  const altsWithSufficientSpace = await pool.query(
    'SELECT * from alts WHERE remaining_space > $1 ORDER BY remaining_space ASC',
    [spaceNeeded],
  );

  for (const row of altsWithSufficientSpace.rows) {
    const altAddress = row.address as string;
    const altPk = new PublicKey(altAddress);
    const altAi = await connection.getAccountInfo(altPk);
    const alt = AddressLookupTableAccount.deserialize(altAi!.data);
    const currentAltPks = new Set(alt.addresses.map((a) => a.toString()));
    const remaingSpace = 256 - currentAltPks.union(pksForSwap).size;
    if (remaingSpace < 0) continue;

    // We successfully identified an ALT that has enough space to host our new market.
    const pksToAdd = pksForSwap.difference(currentAltPks);
    const ix = AddressLookupTableProgram.extendLookupTable({
      payer: keypair.publicKey,
      authority: keypair.publicKey,
      lookupTable: altPk,
      addresses: pksToAdd
        .values()
        .map((pk) => new PublicKey(pk))
        .toArray(),
    });
    const { blockhash, lastValidBlockHeight } =
      await connection.getLatestBlockhash('finalized');
    const tx = new VersionedTransaction(
      MessageV0.compile({
        payerKey: keypair.publicKey,
        instructions: [ix],
        recentBlockhash: blockhash,
      }),
    );
    tx.sign([keypair]);

    console.log(
      'add market',
      market,
      'to existing alt',
      altAddress,
      'remaining_space',
      row.remaining_space,
      '->',
      remaingSpace,
    );

    const signature = await connection.sendTransaction(tx);
    await connection.confirmTransaction({
      signature,
      blockhash,
      lastValidBlockHeight,
    });

    await pool.query(
      'UPDATE alts SET remaining_space = $1 WHERE address = $2',
      [remaingSpace, altAddress],
    );
    await pool.query('INSERT INTO alt_markets(alt, market) VALUES ($1, $2)', [
      altAddress,
      market,
    ]);

    return;
  }

  // We could not find an existing ALT, need to create a new one.
  const recentSlot = await connection.getSlot();
  let [createIx, altPk] = AddressLookupTableProgram.createLookupTable({
    authority: keypair.publicKey,
    payer: keypair.publicKey,
    recentSlot: recentSlot,
  });
  let extendIx = AddressLookupTableProgram.extendLookupTable({
    payer: keypair.publicKey,
    authority: keypair.publicKey,
    lookupTable: altPk,
    addresses: pksForSwap
      .values()
      .map((pk) => new PublicKey(pk))
      .toArray(),
  });
  const { blockhash, lastValidBlockHeight } =
    await connection.getLatestBlockhash('finalized');
  const tx = new VersionedTransaction(
    MessageV0.compile({
      payerKey: keypair.publicKey,
      instructions: [createIx, extendIx],
      recentBlockhash: blockhash,
    }),
  );
  tx.sign([keypair]);

  const altAddress = altPk.toString();
  console.log('add market', market, 'to new alt', altAddress);

  const signature = await connection.sendTransaction(tx, {
    preflightCommitment: 'processed',
  });
  await connection.confirmTransaction({
    signature,
    blockhash,
    lastValidBlockHeight,
  });

  const remaingSpace = 256 - pksForSwap.size;

  await pool.query(
    'INSERT INTO alts(address, remaining_space) VALUES ($1, $2)',
    [altAddress, remaingSpace],
  );
  await pool.query('INSERT INTO alt_markets(alt, market) VALUES ($1, $2)', [
    altAddress,
    market,
  ]);
}

async function main() {
  const connection = new Connection(RPC_URL!, 'confirmed');
  const pool = new Pool({
    connectionString: DATABASE_URL!,
    ssl: { rejectUnauthorized: false }, // May be needed depending on Fly Postgres configuration
  });
  const keypair = Keypair.fromSecretKey(
    Uint8Array.from(PRIVATE_KEY!.split(',').map(Number)),
  );
  await initDatabase(pool);

  const markets: string[] = await findMarketsWithoutAlt(connection, pool);
  console.log('found', markets.length, 'without ALTs', markets);
  for (const market of markets) {
    await addMarketToAlts(connection, pool, keypair, market);
  }
}
main();
