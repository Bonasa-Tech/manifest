'use client';

import { getSolscanSigUrl, setupClient } from '@/lib/util';
import {
  ManifestClient,
  Market,
  RestingOrder,
} from '@bonasa-tech/manifest-sdk';
import { WrapperCancelOrderParams } from '@bonasa-tech/manifest-sdk/wrapper';
import { WrapperOpenOrder } from '@bonasa-tech/manifest-sdk/wrapperObj';
import {
  getAssociatedTokenAddressSync,
  TOKEN_2022_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from '@solana/spl-token';
import { useConnection, useWallet } from '@solana/wallet-adapter-react';
import {
  PublicKey,
  Transaction,
  TransactionInstruction,
} from '@solana/web3.js';
import { useEffect, useState, ReactElement, ChangeEvent } from 'react';
import { toast } from 'react-toastify';
import { useAppState } from './AppWalletProvider';
import { ensureError } from '@/lib/error';

const MyStatus = ({
  marketAddress,
}: {
  marketAddress: string;
}): ReactElement => {
  const {
    connected,
    sendTransaction,
    signTransaction,
    publicKey: signerPub,
  } = useWallet();
  const { connection: conn } = useConnection();
  const { network } = useAppState();

  const [baseWalletBalance, setBaseWalletBalance] = useState<number>(0);
  const [quoteWalletBalance, setQuoteWalletBalance] = useState<number>(0);
  const [baseMint, setBaseMint] = useState<string>('');
  const [quoteMint, setQuoteMint] = useState<string>('');
  const [baseExchangeBalance, setBaseExchangeBalance] = useState<number>(0);
  const [quoteExchangeBalance, setQuoteExchangeBalance] = useState<number>(0);
  const [quoteGlobalBalance, setQuoteGlobalBalance] = useState<number>(0);
  const [myBids, setMyBids] = useState<RestingOrder[]>([]);
  const [myAsks, setMyAsks] = useState<RestingOrder[]>([]);
  const [myWrapperOpenOrders, setMyWrapperOpenOrders] = useState<
    WrapperOpenOrder[]
  >([]);
  const [clientOrderId, setClientOrderId] = useState('0');
  const [actOnQuote, setActOnQuote] = useState(true);
  const [amountTokens, setAmountTokens] = useState('0');

  const handleSetClientOrderId = (
    event: ChangeEvent<HTMLInputElement>,
  ): void => {
    setClientOrderId(event.target.value);
  };

  const handleSetActOnQuote = (_event: ChangeEvent<HTMLInputElement>): void => {
    setActOnQuote(true);
  };

  const handleSetActOnBase = (_event: ChangeEvent<HTMLInputElement>): void => {
    setActOnQuote(false);
  };

  const handleSetAmountTokens = (
    event: ChangeEvent<HTMLInputElement>,
  ): void => {
    setAmountTokens(event.target.value);
  };

  const deposit = async (): Promise<void> => {
    const marketPub = new PublicKey(marketAddress);
    const mClient = await setupClient(
      conn,
      marketPub,
      signerPub,
      connected,
      sendTransaction,
      network,
    );

    const mintPub = actOnQuote
      ? mClient.market.quoteMint()
      : mClient.market.baseMint();

    const depositIx = mClient.depositIx(
      signerPub!,
      mintPub,
      Number(amountTokens),
    );
    try {
      const tx = new Transaction().add(depositIx);
      const { blockhash } = await conn.getLatestBlockhash();
      tx.recentBlockhash = blockhash;
      tx.feePayer = signerPub!;
      const signedTx = await signTransaction!(tx);
      const sig = await conn.sendRawTransaction(signedTx.serialize());
      console.log(`deposit: ${getSolscanSigUrl(sig, network)}`);
      toast.success(`deposit: ${getSolscanSigUrl(sig, network)}`);
    } catch (err) {
      console.log(err);
      toast.error(`deposit: ${ensureError(err).message}`);
    }
  };

  const withdraw = async (): Promise<void> => {
    const marketPub = new PublicKey(marketAddress);
    if (!connected) {
      toast.error('withdraw: must be connected before setting up client');
      throw new Error('must be connected before setting up client');
    }

    const mClient = await setupClient(
      conn,
      marketPub,
      signerPub,
      connected,
      sendTransaction,
      network,
    );

    const mintPub = actOnQuote
      ? mClient.market.quoteMint()
      : mClient.market.baseMint();

    const withdrawIx = mClient.withdrawIx(
      signerPub!,
      mintPub,
      Number(amountTokens),
    );
    try {
      const sig = await sendTransaction(
        new Transaction().add(withdrawIx),
        conn,
      );
      console.log(`withdraw: ${getSolscanSigUrl(sig, network)}`);
      toast.success(`withdraw: ${getSolscanSigUrl(sig, network)}`);
    } catch (err) {
      console.log(err);
      toast.error(`withdraw: ${ensureError(err).message}`);
    }
  };

  const cancelOrder = async (): Promise<void> => {
    const marketPub = new PublicKey(marketAddress);
    const mClient = await setupClient(
      conn,
      marketPub,
      signerPub as PublicKey,
      connected,
      sendTransaction,
      network,
    );

    const cancelParams: WrapperCancelOrderParams = {
      clientOrderId: Number(clientOrderId),
    };
    const cancelOrderIx: TransactionInstruction =
      mClient.cancelOrderIx(cancelParams);
    try {
      const sig = await sendTransaction(
        new Transaction().add(cancelOrderIx),
        conn,
      );
      console.log(`cancelOrder: ${getSolscanSigUrl(sig, network)}`);
      toast.success(`cancelOrder: ${getSolscanSigUrl(sig, network)}`);
    } catch (err) {
      console.log(err);
      toast.error(`cancelOrder: ${ensureError(err).message}`);
    }
  };

  const cancelAllOrders = async (): Promise<void> => {
    const marketPub = new PublicKey(marketAddress);
    const mClient = await setupClient(
      conn,
      marketPub,
      signerPub as PublicKey,
      connected,
      sendTransaction,
      network,
    );

    const cancelAllIx = mClient.cancelAllIx();

    try {
      const sig = await sendTransaction(
        new Transaction().add(cancelAllIx),
        conn,
      );
      console.log(`cancelAllOrders: ${getSolscanSigUrl(sig, network)}`);
      toast.success(`cancelAllOrders: ${getSolscanSigUrl(sig, network)}`);
    } catch (err) {
      console.log(err);
      toast.error(`cancelAllOrders: ${ensureError(err).message}`);
    }
  };

  useEffect(() => {
    if (signerPub) {
      const updateState = async (): Promise<void> => {
        const marketPub = new PublicKey(marketAddress);
        const mClient: ManifestClient = await ManifestClient.getClientReadOnly(
          conn,
          marketPub,
          signerPub,
        );
        await mClient.reload();
        const market: Market = mClient.market;
        setBaseMint(market.baseMint().toBase58());
        setQuoteMint(market.quoteMint().toBase58());

        try {
          const baseBalance = await conn.getTokenAccountBalance(
            getAssociatedTokenAddressSync(
              market.baseMint(),
              signerPub,
              true,
              mClient.isBase22 ? TOKEN_2022_PROGRAM_ID : TOKEN_PROGRAM_ID,
            ),
          );
          setBaseWalletBalance(baseBalance.value.uiAmount!);
        } catch (err) {
          console.log(err);
          // don't notify since not having an ata would trigger spammy notifications...
        }
        try {
          const quoteBalance = await conn.getTokenAccountBalance(
            getAssociatedTokenAddressSync(
              market.quoteMint(),
              signerPub,
              true,
              mClient.isQuote22 ? TOKEN_2022_PROGRAM_ID : TOKEN_PROGRAM_ID,
            ),
          );
          setQuoteWalletBalance(quoteBalance.value.uiAmount!);
        } catch (err) {
          console.log(err);
          // don't notify since not having an ata would trigger spammy notifications...
        }

        // Get the client to get a wrapper which is needed for client order ids.
        if (mClient.wrapper) {
          const wrapperOpenOrders: WrapperOpenOrder[] | null =
            mClient.wrapper.openOrdersForMarket(marketPub);
          if (wrapperOpenOrders) {
            setMyWrapperOpenOrders(wrapperOpenOrders);
          }
        }

        const signerAddr = signerPub.toBase58();
        setMyAsks(
          market
            .asks()
            .filter(
              (restingOrder: RestingOrder) =>
                restingOrder.trader.toBase58() == signerAddr,
            ),
        );
        setMyBids(
          market
            .bids()
            .filter(
              (restingOrder: RestingOrder) =>
                restingOrder.trader.toBase58() == signerAddr,
            ),
        );

        setBaseExchangeBalance(
          market.getWithdrawableBalanceTokens(signerPub, true),
        );
        setQuoteExchangeBalance(
          market.getWithdrawableBalanceTokens(signerPub, false),
        );
        if (mClient.quoteGlobal) {
          setQuoteGlobalBalance(
            await mClient.quoteGlobal.getGlobalBalanceTokens(conn, signerPub),
          );
        }
      };

      updateState().catch((e) => {
        toast.error(`updateState: ${ensureError(e).message}`);
      });

      const id = setInterval(updateState, 10_000);

      return (): void => clearInterval(id);
    }
  }, [signerPub, conn, marketAddress]);

  const NoWallet = (): ReactElement => (
    <h1 className="text-gray-200">Wallet is not connected</h1>
  );

  const WithWallet = (): ReactElement => (
    <div className="flex min-h-screen flex-col items-center justify-evenly p-10">
      <div className="flex flex-col gap-6 text-gray-200">
        <pre className="bg-gray-800 p-4 rounded-lg text-sm">
          <strong>Wallet PublicKey:</strong> {signerPub?.toBase58()}
          <br />
          <strong>Base Mint PublicKey:</strong> {baseMint}
          <br />
          <strong>Quote Mint PublicKey:</strong> {quoteMint}
        </pre>
        <pre className="bg-gray-800 p-4 rounded-lg text-sm">
          <strong>Wallet Balances:</strong>
          <ul>
            <li>Base: {baseWalletBalance}</li>
            <li>Quote: {quoteWalletBalance}</li>
          </ul>
        </pre>
        <pre className="bg-gray-800 p-4 rounded-lg text-sm">
          <strong>Exchange Balances:</strong>
          <ul>
            <li>Base: {baseExchangeBalance}</li>
            <li>Quote: {quoteExchangeBalance}</li>
            {quoteGlobalBalance != 0 && (
              <li>Global Quote: {quoteGlobalBalance}</li>
            )}
          </ul>
        </pre>

        <div className="flex gap-6">
          <label className="flex items-center cursor-pointer">
            <input
              name="action"
              type="radio"
              checked={actOnQuote}
              onChange={handleSetActOnQuote}
              className="mr-2"
            />
            Quote
          </label>
          <label className="flex items-center cursor-pointer">
            <input
              name="action"
              type="radio"
              checked={!actOnQuote}
              onChange={handleSetActOnBase}
              className="mr-2"
            />
            Base
          </label>
        </div>

        <div className="flex flex-col mb-4">
          <label className="font-bold mb-2">Amount Tokens</label>
          <input
            className="bg-gray-800 border border-gray-700 rounded p-2 text-gray-200 focus:outline-none focus:border-gray-500"
            type="text"
            value={amountTokens}
            onChange={handleSetAmountTokens}
          />
        </div>

        <button
          className="py-2 px-4 rounded bg-blue-500 text-white disabled:opacity-50"
          onClick={deposit}
          disabled={!connected}
        >
          Deposit
        </button>
        <button
          className="py-2 px-4 rounded bg-blue-500 text-white disabled:opacity-50"
          onClick={withdraw}
          disabled={!connected}
        >
          Withdraw
        </button>

        <div className="flex flex-col mb-4">
          <label className="font-bold mb-2">Client Order ID</label>
          <input
            className="bg-gray-800 border border-gray-700 rounded p-2 text-gray-200 focus:outline-none focus:border-gray-500"
            type="text"
            value={clientOrderId}
            onChange={handleSetClientOrderId}
          />
        </div>

        <button
          className="py-2 px-4 rounded bg-red-500 text-white disabled:opacity-50"
          onClick={cancelOrder}
          disabled={!connected}
        >
          {myAsks.length || myBids.length
            ? 'Cancel By Order ID'
            : 'No Orders to Cancel'}
        </button>
        <button
          className="py-2 px-4 rounded bg-red-500 text-white disabled:opacity-50"
          onClick={cancelAllOrders}
          disabled={!connected}
        >
          {myAsks.length || myBids.length
            ? 'Cancel All Orders'
            : 'No Orders to Cancel'}
        </button>

        <pre className="bg-gray-800 p-4 rounded-lg text-sm mt-4">
          <strong>Asks</strong>
          <table className="table-auto w-full text-left text-sm border-collapse">
            <thead>
              <tr className="border-b border-gray-700">
                <th className="py-2">Price</th>
                <th className="py-2">Amount</th>
                <th className="py-2">ClientOrderId</th>
              </tr>
            </thead>
            <tbody>
              {myAsks.map((restingOrder: RestingOrder, i: number) => (
                <tr key={i} className="border-b border-gray-700">
                  <td className="py-2">{restingOrder.tokenPrice}</td>
                  <td className="py-2">{Number(restingOrder.numBaseTokens)}</td>
                  <td className="py-2">
                    {myWrapperOpenOrders
                      .filter((openOrder: WrapperOpenOrder) => {
                        return (
                          Number(openOrder.orderSequenceNumber) ==
                          Number(restingOrder.sequenceNumber)
                        );
                      })
                      .reduce((_acc, current) => {
                        return current.clientOrderId.toString();
                      }, '')}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </pre>

        <pre className="bg-gray-800 p-4 rounded-lg text-sm mt-4">
          <strong>Bids</strong>
          <table className="table-auto w-full text-left text-sm border-collapse">
            <thead>
              <tr className="border-b border-gray-700">
                <th className="py-2">Price</th>
                <th className="py-2">Amount</th>
                <th className="py-2">ClientOrderId</th>
              </tr>
            </thead>
            <tbody>
              {myBids.map((restingOrder: RestingOrder, i: number) => (
                <tr key={i} className="border-b border-gray-700">
                  <td className="py-2">{restingOrder.tokenPrice}</td>
                  <td className="py-2">{Number(restingOrder.numBaseTokens)}</td>
                  <td className="py-2">
                    {myWrapperOpenOrders
                      .filter((openOrder: WrapperOpenOrder) => {
                        return (
                          Number(openOrder.orderSequenceNumber) ==
                          Number(restingOrder.sequenceNumber)
                        );
                      })
                      .reduce((_acc, current) => {
                        return current.clientOrderId.toString();
                      }, '')}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </pre>
      </div>
    </div>
  );

  return connected ? <WithWallet /> : <NoWallet />;
};

export default MyStatus;
