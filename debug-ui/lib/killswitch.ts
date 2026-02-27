import { ManifestClient, Wrapper } from '@bonasa-tech/manifest-sdk';
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
import { getSolscanSigUrl, getSolscanUrl } from './util';
import { shortenAddr } from './address-labels';
import { ensureError } from './error';

type SendTransaction = (
  transaction: Transaction | VersionedTransaction,
  connection: Connection,
  options?: SendTransactionOptions,
) => Promise<TransactionSignature>;

export const killswitch = async (
  conn: Connection,
  marketAddrs: string[],
  userPub: PublicKey | null,
  sendTransaction: SendTransaction,
  network: WalletAdapterNetwork | null,
): Promise<void> => {
  if (!userPub) {
    throw new Error('must be connected before setting up client');
  }

  const userWrapper = await ManifestClient['fetchFirstUserWrapper'](
    conn,
    userPub,
  );
  if (!userWrapper) {
    throw new Error('no userWrapper');
  }

  const wrapper = await Wrapper.loadFromAddress({
    connection: conn,
    address: userWrapper.pubkey,
  });

  for (const marketAddr of marketAddrs) {
    const marketPub = new PublicKey(marketAddr);
    const oos = wrapper.openOrdersForMarket(marketPub);
    if (!oos || oos.length === 0) {
      console.log(`no oos exist for ${marketAddr} skipping...`);
      continue;
    }

    console.log(
      `oos exist for ${marketAddr} trying to cancel all and withdraw all...`,
    );

    try {
      const mfx = await ManifestClient.getClientForMarketNoPrivateKey(
        conn,
        marketPub,
        userPub,
      );

      const cancelIx = mfx.cancelAllIx();
      const withdrawIx = mfx.withdrawAllIx();

      const ixs = [cancelIx, ...withdrawIx];

      console.log(
        `sending ${ixs.length} killswitch ixs for ${shortenAddr(marketAddr)}...`,
      );
      const tx = new Transaction().add(...ixs);
      const { blockhash } = await conn.getLatestBlockhash();
      tx.recentBlockhash = blockhash;
      tx.feePayer = userPub;

      const sig = await sendTransaction(tx, conn);

      console.log(
        `killswitch:${shortenAddr(marketAddr)} tx: ${getSolscanUrl(sig, network)}`,
      );
      toast.success(
        `killswitch:${shortenAddr(marketAddr)}: ${getSolscanSigUrl(sig, network)}`,
      );
    } catch (e) {
      console.error(ensureError(e));
      toast.error(
        `killswitch:${shortenAddr(marketAddr)}: ${ensureError(e).message}`,
      );
    }
  }
};
