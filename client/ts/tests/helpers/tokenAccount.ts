import { Connection, PublicKey } from '@solana/web3.js';
import { getAccount, TOKEN_PROGRAM_ID } from '@solana/spl-token';

const TOKEN_ACCOUNT_RETRIES = 10;
const TOKEN_ACCOUNT_RETRY_DELAY_MS = 200;

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export async function waitForTokenAccount(
  connection: Connection,
  tokenAccount: PublicKey,
  tokenProgramId: PublicKey = TOKEN_PROGRAM_ID,
): Promise<void> {
  let lastError: unknown;
  for (let attempt = 0; attempt < TOKEN_ACCOUNT_RETRIES; attempt++) {
    try {
      await getAccount(connection, tokenAccount, 'confirmed', tokenProgramId);
      return;
    } catch (error) {
      lastError = error;
      await sleep(TOKEN_ACCOUNT_RETRY_DELAY_MS);
    }
  }
  throw lastError;
}
