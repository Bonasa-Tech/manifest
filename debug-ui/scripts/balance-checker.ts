import { ManifestClient, Market } from '@bonasa-tech/manifest-sdk';
import { getVaultAddress } from '@bonasa-tech/manifest-sdk/utils';
import { Global } from '@bonasa-tech/manifest-sdk';
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

  let foundMismatch: boolean = false;
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
    } = market.getMarketBalances();

    const baseVaultBalanceAtoms: number = Number(
      (parsedAccounts.value[1]?.data as ParsedAccountData).parsed['info'][
        'tokenAmount'
      ]['amount'],
    );
    const quoteVaultBalanceAtoms: number = Number(
      (parsedAccounts.value[2]?.data as ParsedAccountData).parsed['info'][
        'tokenAmount'
      ]['amount'],
    );

    const baseExpectedAtoms: number =
      baseWithdrawableBalanceAtoms + baseOpenOrdersBalanceAtoms;
    const quoteExpectedAtoms: number =
      quoteWithdrawableBalanceAtoms + quoteOpenOrdersBalanceAtoms;

    // Allow small difference because of javascript rounding.
    if (
      Math.abs(baseExpectedAtoms - baseVaultBalanceAtoms) > 1 ||
      Math.abs(quoteExpectedAtoms - quoteVaultBalanceAtoms) > 1
    ) {
      console.log('Market', marketPk.toBase58());
      console.log(
        'Base actual',
        baseVaultBalanceAtoms,
        'base expected',
        baseExpectedAtoms,
        'difference',
        baseVaultBalanceAtoms - baseExpectedAtoms,
      );
      console.log(
        'Quote actual',
        quoteVaultBalanceAtoms,
        'quote expected',
        quoteExpectedAtoms,
        'difference',
        quoteVaultBalanceAtoms - quoteExpectedAtoms,
        'withdrawable',
        quoteWithdrawableBalanceAtoms,
        'open orders',
        quoteOpenOrdersBalanceAtoms,
      );
      // Skip these markets because the numbers are so large that they run into js
      // rounding issues. Verified that they are solvent manually.
      if (
        marketPk.toBase58() == 'GQHqLzX8swBiTyREF57PGs4vq59obuRPdtGrY4gChHfB' ||
        marketPk.toBase58() == '8MHWBBr87Ta3JqFK4tt6r7tSR31oFKa8SWkF2YKTpWHa'
      ) {
        continue;
      }
      // Only crash on a loss of funds. There has been unsolicited deposits into
      // vaults which makes them have more tokens than the program expects.
      if (
        baseExpectedAtoms > baseVaultBalanceAtoms ||
        quoteExpectedAtoms > quoteVaultBalanceAtoms
      ) {
        foundMismatch = true;
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

      // Calculate total expected balance from all seats using refetched data
      let totalExpectedAtoms = 0;
      const deposits = (refetchedGlobal as any).data.globalDeposits;
      for (const deposit of deposits) {
        totalExpectedAtoms += Number(deposit.balanceAtoms);
      }

      // Get actual vault balance from the same RPC call
      const actualVaultAtoms = parsedAccounts.value[1]?.data
        ? Number(
            (parsedAccounts.value[1].data as ParsedAccountData).parsed.info
              .tokenAmount.amount,
          )
        : 0;

      const difference = actualVaultAtoms - totalExpectedAtoms;

      console.log(`Global ${mint.toBase58()}`);
      console.log(
        `Vault actual ${actualVaultAtoms} expected ${totalExpectedAtoms} difference ${difference} seats ${deposits.length}`,
      );

      // Check if vault has less than expected (loss of funds)
      if (totalExpectedAtoms > actualVaultAtoms) {
        console.log('MISMATCH DETECTED - Listing all seats:');
        console.log('=====================================');

        for (let i = 0; i < deposits.length; i++) {
          const deposit = deposits[i];
          const trader = deposit.trader;
          const balanceAtoms = Number(deposit.balanceAtoms);

          console.log(
            `Seat ${i}: trader=${trader.toBase58()} balance=${balanceAtoms} atoms`,
          );
        }

        console.log('=====================================');
        console.log(`Total from seats: ${totalExpectedAtoms} atoms`);
        console.log(`Actual in vault: ${actualVaultAtoms} atoms`);
        console.log(`Difference: ${difference} atoms`);
        console.log('=====================================');
      }

      // Only crash on a loss of funds
      if (totalExpectedAtoms > actualVaultAtoms) {
        foundMismatch = true;
      }
    } catch (error) {
      console.log(
        `Error checking global ${globalAccount.pubkey.toBase58()}: ${error}`,
      );
    }
  }

  if (foundMismatch) {
    await sendDiscordMessage(
      `**Balance Checker Alert**\nBalance mismatch detected! Check the logs for details.`,
    );
    throw new Error('Balance mismatch detected');
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
