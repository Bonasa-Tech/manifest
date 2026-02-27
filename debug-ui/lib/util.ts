import { ManifestClient } from '@bonasa-tech/manifest-sdk';
import { TOKEN_PROGRAM_ID } from '@solana/spl-token';
import {
  SendTransactionOptions,
  WalletAdapterNetwork,
} from '@solana/wallet-adapter-base';
import {
  Connection,
  PublicKey,
  Transaction,
  TransactionSignature,
  VersionedTransaction,
} from '@solana/web3.js';
import { toast } from 'react-toastify';

type SendTransaction = (
  transaction: Transaction | VersionedTransaction,
  connection: Connection,
  options?: SendTransactionOptions,
) => Promise<TransactionSignature>;

export const sleep = (ms: number): Promise<void> =>
  new Promise((resolve) => setTimeout(resolve, ms));

export const setupClient = async (
  conn: Connection,
  marketPub: PublicKey,
  signerPub: PublicKey | null,
  connected: boolean,
  sendTransaction: SendTransaction,
  network: WalletAdapterNetwork | null,
): Promise<ManifestClient> => {
  if (!connected || !signerPub) {
    throw new Error('must be connected before setting up client');
  }

  const { setupNeeded, instructions, wrapperKeypair } =
    await ManifestClient.getSetupIxs(conn, marketPub, signerPub as PublicKey);

  if (setupNeeded) {
    console.log(`sending ${instructions.length} setup ixs...`);
    const tx = new Transaction().add(...instructions);
    const { blockhash } = await conn.getLatestBlockhash();
    tx.recentBlockhash = blockhash;
    tx.feePayer = signerPub!;
    if (wrapperKeypair) {
      tx.sign(wrapperKeypair);
    }

    const sig = await sendTransaction(tx, conn);

    console.log(`setupTx: ${getSolscanUrl(sig, network)}`);
    toast.success(`setup: ${getSolscanSigUrl(sig, network)}`);

    console.log('sleeping 5 seconds to ensure state catches up');

    await sleep(5_000);
  }

  const mClient = await ManifestClient.getClientForMarketNoPrivateKey(
    conn,
    marketPub,
    signerPub as PublicKey,
  );

  return mClient;
};

export const getSolscanUrl = (
  address: string,
  cluster: WalletAdapterNetwork | null = WalletAdapterNetwork.Devnet,
): string => {
  const baseUrl = 'https://solscan.io';
  const clusterPath =
    cluster !== WalletAdapterNetwork.Mainnet ? `?cluster=${cluster}` : '';
  return `${baseUrl}/account/${address}${clusterPath}`;
};

export const getSolscanSigUrl = (
  sig: string,
  cluster: WalletAdapterNetwork | null = WalletAdapterNetwork.Devnet,
): string => {
  const baseUrl = 'https://solscan.io';
  const clusterPath =
    cluster !== WalletAdapterNetwork.Mainnet ? `?cluster=${cluster}` : '';
  return `${baseUrl}/tx/${sig}${clusterPath}`;
};

export const shortenAddress = (address: string): string => {
  return `${address.slice(0, 3)}...${address.slice(-3)}`;
};

export const shortenSig = (address: string): string => {
  return `${address.slice(0, 6)}...${address.slice(-6)}`;
};

export const checkForToken22 = async (
  conn: Connection,
  mint: PublicKey,
): Promise<boolean> => {
  const localStorageKey: string = mint.toBase58() + '-Is22';

  if (localStorage.getItem(localStorageKey)) {
    return localStorage.getItem(localStorageKey)! === 'true';
  }

  const acc = await conn.getAccountInfo(mint);
  if (!acc) {
    throw new Error('checkForToken22: account does not exist');
  }

  const is22: boolean = acc.owner.toBase58() !== TOKEN_PROGRAM_ID.toBase58();
  localStorage.setItem(localStorageKey, is22.toString());
  return is22;
};
