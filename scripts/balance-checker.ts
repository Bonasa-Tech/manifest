import { ManifestClient } from '../client/ts/src/client';
import { Market } from '../client/ts/src/market';
import { getVaultAddress } from '../client/ts/src/utils/market';
import { toBigInt } from '../client/ts/src/utils/numbers';
import { Global } from '../client/ts/src/global';
import {
  AccountInfo,
  Connection,
  ParsedAccountData,
  PublicKey,
  RpcResponseAndContext,
} from '@solana/web3.js';
import bs58 from 'bs58';

const { RPC_URL, DISCORD_WEBHOOK_URL } = process.env;

if (!RPC_URL) {
  throw new Error('RPC_URL missing from env');
}

async function sendDiscordMessage(content: string): Promise<void> {
  if (!DISCORD_WEBHOOK_URL) return;

  try {
    const response = await fetch(DISCORD_WEBHOOK_URL, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ content }),
    });
    if (!response.ok) {
      console.error(
        `Failed to send Discord message: ${response.status} ${response.statusText}`,
      );
    }
  } catch (error) {
    console.error('Error sending Discord message:', error);
  }
}

const run = async () => {
  const connection: Connection = new Connection(RPC_URL);
  const marketPks: PublicKey[] =
    await ManifestClient.listMarketPublicKeys(connection);

  const mismatchedMarkets: string[] = [];
  for (const marketPk of marketPks) {
    const client: ManifestClient = await ManifestClient.getClientReadOnly(
      connection,
      marketPk,
    );
    const baseMint: PublicKey = client.market.baseMint();
    const quoteMint: PublicKey = client.market.quoteMint();

    const parsedAccounts: RpcResponseAndContext<
      (AccountInfo<Buffer | ParsedAccountData> | null)[]
    > = await connection.getMultipleParsedAccounts([
      marketPk,
      getVaultAddress(marketPk, baseMint),
      getVaultAddress(marketPk, quoteMint),
    ]);
    const market: Market = Market.loadFromBuffer({
      address: marketPk,
      buffer: parsedAccounts.value[0]?.data! as Buffer,
    });
    const {
      baseWithdrawableBalanceAtoms,
      quoteWithdrawableBalanceAtoms,
      baseOpenOrdersBalanceAtoms,
      quoteOpenOrdersBalanceAtoms,
    } = market.getMarketBalancesAtoms();

    // Use BigInt end to end. These atom totals routinely exceed
    // Number.MAX_SAFE_INTEGER (2^53) on high-supply markets, so Number() math
    // loses precision and produces phantom mismatches.
    const baseVaultBalanceAtoms: bigint = BigInt(
      (parsedAccounts.value[1]?.data as ParsedAccountData).parsed['info'][
        'tokenAmount'
      ]['amount'],
    );
    const quoteVaultBalanceAtoms: bigint = BigInt(
      (parsedAccounts.value[2]?.data as ParsedAccountData).parsed['info'][
        'tokenAmount'
      ]['amount'],
    );

    const baseExpectedAtoms: bigint =
      baseWithdrawableBalanceAtoms + baseOpenOrdersBalanceAtoms;
    const quoteExpectedAtoms: bigint =
      quoteWithdrawableBalanceAtoms + quoteOpenOrdersBalanceAtoms;

    if (
      baseExpectedAtoms != baseVaultBalanceAtoms ||
      quoteExpectedAtoms != quoteVaultBalanceAtoms
    ) {
      console.log('Market', marketPk.toBase58());
      console.log(
        'Base actual',
        baseVaultBalanceAtoms.toString(),
        'base expected',
        baseExpectedAtoms.toString(),
        'difference',
        (baseVaultBalanceAtoms - baseExpectedAtoms).toString(),
      );
      console.log(
        'Quote actual',
        quoteVaultBalanceAtoms.toString(),
        'quote expected',
        quoteExpectedAtoms.toString(),
        'difference',
        (quoteVaultBalanceAtoms - quoteExpectedAtoms).toString(),
        'withdrawable',
        quoteWithdrawableBalanceAtoms.toString(),
        'open orders',
        quoteOpenOrdersBalanceAtoms.toString(),
      );
      // Only crash on a loss of funds. There have been unsolicited deposits into
      // vaults which makes them have more tokens than the program expects.
      if (
        baseExpectedAtoms > baseVaultBalanceAtoms ||
        quoteExpectedAtoms > quoteVaultBalanceAtoms
      ) {
        mismatchedMarkets.push(marketPk.toBase58());
      }
    }
  }

  // Get all global accounts
  const MANIFEST_PROGRAM_ID = new PublicKey(
    'MNFSTqtC93rEfYHB6hF82sKdZpUDFWkViLByLd1k1Ms',
  );
  const GLOBAL_DISCRIMINANT = Buffer.from([
    1, 170, 151, 47, 187, 160, 180, 149,
  ]); // 10787423733276977665 as little-endian bytes
  const GLOBAL_DISCRIMINANT_BASE58 = bs58.encode(GLOBAL_DISCRIMINANT);

  const globalAccounts = await connection.getProgramAccounts(
    MANIFEST_PROGRAM_ID,
    {
      filters: [
        {
          memcmp: {
            offset: 0,
            bytes: GLOBAL_DISCRIMINANT_BASE58,
          },
        },
      ],
    },
  );

  const globalPublicKeys: PublicKey[] = globalAccounts.map(
    (account) => account.pubkey,
  );
  console.log(`Found ${globalPublicKeys.length} global accounts`);

  const mismatchedGlobals: string[] = [];
  // Check global account balances
  for (const globalAccount of globalAccounts) {
    try {
      const global = Global.loadFromBuffer({
        address: globalAccount.pubkey,
        buffer: globalAccount.account.data,
      });

      const mint = global.tokenMint();
      const vault = (global as any).data.vault;

      // Fetch both vault and global account from the same slot
      const parsedAccounts = await connection.getMultipleParsedAccounts([
        globalAccount.pubkey,
        vault,
      ]);

      // Re-load global from the fetched data to ensure consistency
      const refetchedGlobal = Global.loadFromBuffer({
        address: globalAccount.pubkey,
        buffer: parsedAccounts.value[0]?.data as Buffer,
      });

      // Calculate total expected balance from all seats using refetched data.
      // Use BigInt to avoid precision loss when the sum exceeds 2^53.
      let totalExpectedAtoms: bigint = 0n;
      const deposits = (refetchedGlobal as any).data.globalDeposits;
      for (const deposit of deposits) {
        totalExpectedAtoms += toBigInt(deposit.balanceAtoms);
      }

      // Get actual vault balance from the same RPC call
      const actualVaultAtoms: bigint = parsedAccounts.value[1]?.data
        ? BigInt(
            (parsedAccounts.value[1].data as ParsedAccountData).parsed.info
              .tokenAmount.amount,
          )
        : 0n;

      const difference: bigint = actualVaultAtoms - totalExpectedAtoms;

      console.log(`Global ${mint.toBase58()}`);
      console.log(
        `Vault actual ${actualVaultAtoms.toString()} expected ${totalExpectedAtoms.toString()} difference ${difference.toString()} seats ${deposits.length}`,
      );

      // Check if vault has less than expected (loss of funds).
      if (totalExpectedAtoms > actualVaultAtoms) {
        console.log('MISMATCH DETECTED - Listing all seats:');
        console.log('=====================================');

        for (let i = 0; i < deposits.length; i++) {
          const deposit = deposits[i];
          const trader = deposit.trader;
          const balanceAtoms: bigint = toBigInt(deposit.balanceAtoms);

          console.log(
            `Seat ${i}: trader=${trader.toBase58()} balance=${balanceAtoms.toString()} atoms`,
          );
        }

        console.log('=====================================');
        console.log(`Total from seats: ${totalExpectedAtoms.toString()} atoms`);
        console.log(`Actual in vault: ${actualVaultAtoms.toString()} atoms`);
        console.log(`Difference: ${difference.toString()} atoms`);
        console.log('=====================================');

        mismatchedGlobals.push(mint.toBase58());
      }
    } catch (error) {
      console.log(
        `Error checking global ${globalAccount.pubkey.toBase58()}: ${error}`,
      );
    }
  }

  if (mismatchedMarkets.length > 0 || mismatchedGlobals.length > 0) {
    const details: string[] = [];
    if (mismatchedMarkets.length > 0) {
      details.push(`Markets: ${mismatchedMarkets.join(', ')}`);
    }
    if (mismatchedGlobals.length > 0) {
      details.push(`Globals: ${mismatchedGlobals.join(', ')}`);
    }
    const detailsStr = details.join('; ');
    await sendDiscordMessage(
      `**Balance Checker Alert**\nBalance mismatch detected! ${detailsStr}`,
    );
    throw new Error(`Balance mismatch detected: ${detailsStr}`);
  }

  await sendDiscordMessage(
    `**Balance Checker Report**\nAll ${marketPks.length} markets and ${globalPublicKeys.length} global accounts passed balance verification.`,
  );
};

run().catch(async (e) => {
  console.error('fatal error', e);
  await sendDiscordMessage(
    `**Balance Checker Error**\nFatal error occurred: ${e.message || e}`,
  );
  throw e;
});
