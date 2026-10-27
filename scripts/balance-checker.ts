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

  const response = await fetch(DISCORD_WEBHOOK_URL, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ content }),
  });
  if (!response.ok) {
    throw new Error(
      `Failed to send Discord message: ${response.status} ${response.statusText}`,
    );
  }
}

async function sendDiscordMessageBestEffort(
  content: string,
  description: string,
): Promise<void> {
  try {
    await sendDiscordMessage(content);
  } catch (error) {
    console.error(`Failed to send ${description}:`, error);
  }
}

// Markets with a known, already-investigated vault shortfall caused by a
// token issuer acting outside the program (e.g. a Token-2022 permanent
// delegate burning from the vault). These are expected to stay mismatched
// forever, so they are excluded from failing the checker. Any new shortfall
// on a market not in this list still fails and must be investigated before
// being added here.
const KNOWN_RUGGED_MARKETS: Set<string> = new Set([
  // SILV/USDC. The SILV permanent delegate (the issuer's Squads multisig)
  // burned the base vault balance on 2026-10-07 in tx
  // BuEFcSSqhQU8ZemQK9jVzLKKZMusH1VVZqEN4qJbvJ7unwdHmuQjRaAH95aZnL1rVHxVPqArpUFGQB96ZZHsFbE.
  '72bGMGVVDbmEPRc16K5wT22sN4Wwg6YJN2GThAPCFFfT',
]);

// Returns the token balance of a vault account. A vault whose mint was closed
// by its issuer (Token-2022 mint-close-authority) comes back unparsed because
// the RPC cannot resolve decimals without the mint, so fall back to reading
// the raw SPL token amount (u64 LE at offset 64; same layout for Token and
// Token-2022). A missing vault counts as zero so the market is still checked.
function vaultBalanceAtoms(
  info: AccountInfo<Buffer | ParsedAccountData> | null,
): bigint {
  if (!info) {
    return 0n;
  }
  const data: Buffer | ParsedAccountData = info.data;
  if ('parsed' in data) {
    return BigInt(data.parsed['info']['tokenAmount']['amount']);
  }
  return data.readBigUInt64LE(64);
}

const run = async (): Promise<void> => {
  const connection: Connection = new Connection(RPC_URL);
  const marketPks: PublicKey[] =
    await ManifestClient.listMarketPublicKeys(connection);

  const mismatchedMarkets: string[] = [];
  for (const marketPk of marketPks) {
    // Load the market directly instead of through ManifestClient: the client
    // unpacks both mints and crashes on markets whose mint account was closed
    // by its issuer, and the mints are not needed to check balances.
    const marketForMints: Market = await Market.loadFromAddress({
      connection,
      address: marketPk,
    });
    const baseMint: PublicKey = marketForMints.baseMint();
    const quoteMint: PublicKey = marketForMints.quoteMint();

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
    const baseVaultBalanceAtoms: bigint = vaultBalanceAtoms(
      parsedAccounts.value[1],
    );
    const quoteVaultBalanceAtoms: bigint = vaultBalanceAtoms(
      parsedAccounts.value[2],
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
        if (KNOWN_RUGGED_MARKETS.has(marketPk.toBase58())) {
          console.log(
            'Ignoring shortfall on known rugged market',
            marketPk.toBase58(),
          );
        } else {
          mismatchedMarkets.push(marketPk.toBase58());
        }
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
  const uncheckedGlobals: string[] = [];
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
      console.error(
        `Error checking global ${globalAccount.pubkey.toBase58()}: ${error}`,
      );
      uncheckedGlobals.push(globalAccount.pubkey.toBase58());
    }
  }

  const hasMismatch: boolean =
    mismatchedMarkets.length > 0 || mismatchedGlobals.length > 0;
  const mismatchDetails: string[] = [];
  if (mismatchedMarkets.length > 0) {
    mismatchDetails.push(`Markets: ${mismatchedMarkets.join(', ')}`);
  }
  if (mismatchedGlobals.length > 0) {
    mismatchDetails.push(`Globals: ${mismatchedGlobals.join(', ')}`);
  }
  const detailsStr: string = mismatchDetails.join('; ');
  if (hasMismatch) {
    console.error(`Balance mismatch detected! ${detailsStr}`);
    await sendDiscordMessageBestEffort(
      `**Balance Checker Alert**\nBalance mismatch detected! ${detailsStr}`,
      'balance-mismatch alert',
    );
  }

  if (uncheckedGlobals.length > 0) {
    const uncheckedDetails: string = uncheckedGlobals.join(', ');
    console.error(
      `Balance verification incomplete for globals: ${uncheckedDetails}`,
    );
    await sendDiscordMessageBestEffort(
      `**Balance Checker Incomplete**\nCould not verify ${uncheckedGlobals.length} global account(s): ${uncheckedGlobals.join(', ')}`,
      'incomplete-scan alert',
    );
  }

  if (hasMismatch || uncheckedGlobals.length > 0) {
    const failures: string[] = [];
    if (hasMismatch) {
      failures.push(`balance mismatch detected (${detailsStr})`);
    }
    if (uncheckedGlobals.length > 0) {
      failures.push(
        `${uncheckedGlobals.length} global account(s) could not be checked`,
      );
    }
    throw new Error(`Balance verification failed: ${failures.join('; ')}`);
  }

  await sendDiscordMessage(
    `**Balance Checker Report**\nAll ${marketPks.length} markets and ${globalPublicKeys.length} global accounts passed balance verification.`,
  );
};

run().catch(async (e) => {
  console.error('fatal error', e);
  try {
    await sendDiscordMessage(
      `**Balance Checker Error**\nFatal error occurred: ${e.message || e}`,
    );
  } catch (notificationError) {
    console.error('Failed to send fatal-error notification', notificationError);
  }
  throw e;
});
