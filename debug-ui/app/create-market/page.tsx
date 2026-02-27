'use client';

import { ManifestClient, Market } from '@bonasa-tech/manifest-sdk';
import { FIXED_MANIFEST_HEADER_SIZE } from '@bonasa-tech/manifest-sdk/constants';
import { PROGRAM_ID } from '@bonasa-tech/manifest-sdk/manifest';
import {
  MINT_SIZE,
  getMinimumBalanceForRentExemptMint,
  TOKEN_PROGRAM_ID,
  createInitializeMint2Instruction,
} from '@solana/spl-token';
import { useConnection, useWallet } from '@solana/wallet-adapter-react';
import {
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
  TransactionInstruction,
} from '@solana/web3.js';
import { ChangeEvent, ReactElement, useEffect, useState } from 'react';
import { useAppState } from '../components/AppWalletProvider';
import { withAccessControl } from '@/lib/withAccessControl';
import { getSolscanSigUrl, sleep } from '@/lib/util';
import { toast } from 'react-toastify';
import { ensureError } from '@/lib/error';
import MintTo from '../components/MintTo';

const CreateMarket = (): ReactElement => {
  const { connection: conn } = useConnection();
  const {
    connected,
    sendTransaction,
    publicKey: signerPub,
    signTransaction,
  } = useWallet();
  const { marketAddrs, setMarketAddrs, network } = useAppState();

  const [occupiedPairs, setOccupiedPairs] = useState<[string, string][]>([]);

  const [keypair, setKeypair] = useState<string>('');
  const [pubkey, setPubkey] = useState<string>('');

  const [mintKeypair, setMintKeypair] = useState<string>('');
  const [mintAddr, setMintAddr] = useState<string>('');
  const [decimals, setDecimals] = useState<number>(9);

  const [marketKeypair, setMarketKeypair] = useState<string>('');
  const [marketAddr, setMarketAddr] = useState<string>('');
  const [baseAddr, setBaseAddr] = useState<string>('');
  const [quoteAddr, setQuoteAddr] = useState<string>('');

  const genKeypair = (): void => {
    const keypair = Keypair.generate();
    const keypairArray = Array.from(keypair.secretKey);
    setKeypair(JSON.stringify(keypairArray));
    setPubkey(keypair.publicKey.toBase58());
  };

  const copyKeypair = (): void => {
    navigator.clipboard.writeText(keypair);
    alert('Keypair copied to clipboard!');
  };

  const shortenKeypair = (keypair: string): string => {
    const parsedKeypair = keypair ? keypair : '';
    return `${parsedKeypair.slice(0, 6)}...${parsedKeypair.slice(-6)}`;
  };

  const handleTokenMintKeypairChange = (
    e: ChangeEvent<HTMLInputElement>,
  ): void => {
    const inputKeypair = e.target.value;
    setMintKeypair(inputKeypair);

    try {
      const parsedArray = JSON.parse(inputKeypair);
      const mintKeypair = Keypair.fromSecretKey(Uint8Array.from(parsedArray));
      setMintAddr(mintKeypair.publicKey.toBase58());
    } catch (e) {
      setMintAddr('');
      toast.error(`handleTokenMintKeypairChange: ${ensureError(e).message}`);
    }
  };

  const handleDecimalsChange = (e: ChangeEvent<HTMLInputElement>): void => {
    setDecimals(Number(e.target.value));
  };

  const handleBaseAddrChange = (e: ChangeEvent<HTMLInputElement>): void => {
    setBaseAddr(e.target.value);
  };

  const handleQuoteAddrChange = (e: ChangeEvent<HTMLInputElement>): void => {
    setQuoteAddr(e.target.value);
  };

  const handleMarketKeypairChange = (
    e: ChangeEvent<HTMLInputElement>,
  ): void => {
    const inputKeypair = e.target.value;
    setMarketKeypair(inputKeypair);

    try {
      const parsedArray = JSON.parse(inputKeypair);
      const marketKeypair = Keypair.fromSecretKey(Uint8Array.from(parsedArray));
      setMarketAddr(marketKeypair.publicKey.toBase58());
    } catch (e) {
      setMarketAddr('');
      toast.error(`setMarketAddr: ${ensureError(e).message}`);
    }
  };

  const createMint = async (): Promise<void> => {
    if (!connected || !signerPub) {
      toast.error('createMint: not connected or no signerPUb');
      throw new Error('not connected or no signerPUb');
    }

    const parsedKeypair = Keypair.fromSecretKey(
      new Uint8Array(JSON.parse(mintKeypair)),
    );

    const lamports = await getMinimumBalanceForRentExemptMint(conn);

    const tx = new Transaction().add(
      SystemProgram.createAccount({
        fromPubkey: signerPub,
        newAccountPubkey: parsedKeypair.publicKey,
        space: MINT_SIZE,
        lamports,
        programId: TOKEN_PROGRAM_ID,
      }),
      createInitializeMint2Instruction(
        parsedKeypair.publicKey,
        decimals,
        signerPub,
        signerPub,
        TOKEN_PROGRAM_ID,
      ),
    );

    const { blockhash } = await conn.getLatestBlockhash();
    tx.recentBlockhash = blockhash;
    tx.feePayer = signerPub!;
    tx.sign(parsedKeypair);

    const sig = await sendTransaction(tx, conn);
    console.log(`createMint: ${getSolscanSigUrl(sig, network)}`);
    toast.success(`createMint: ${getSolscanSigUrl(sig, network)}`);
  };

  const createMarket = async (): Promise<void> => {
    if (!connected || !signerPub) {
      toast.error('createMarket: not connected or no signerPUb');
      throw new Error('not connected or no signerPUb');
    }

    const parsedKeypair = Keypair.fromSecretKey(
      new Uint8Array(JSON.parse(marketKeypair)),
    );

    const basePub = new PublicKey(baseAddr);
    const quotePub = new PublicKey(quoteAddr);
    const marketPub = new PublicKey(marketAddr);

    const lamports = await conn.getMinimumBalanceForRentExemption(
      FIXED_MANIFEST_HEADER_SIZE,
    );
    const createAccountIx: TransactionInstruction = SystemProgram.createAccount(
      {
        fromPubkey: signerPub,
        newAccountPubkey: marketPub,
        space: FIXED_MANIFEST_HEADER_SIZE,
        lamports,
        programId: PROGRAM_ID,
      },
    );

    const createMarketIx = ManifestClient['createMarketIx'](
      signerPub,
      basePub,
      quotePub,
      marketPub,
    );

    const tx: Transaction = new Transaction();
    tx.add(createAccountIx);
    tx.add(createMarketIx);
    const { blockhash } = await conn.getLatestBlockhash();
    tx.recentBlockhash = blockhash;
    tx.feePayer = signerPub!;
    tx.sign(parsedKeypair);
    const signedTx = await signTransaction!(tx);
    const sig = await conn.sendRawTransaction(signedTx.serialize());

    console.log(`createMarket: ${getSolscanSigUrl(sig, network)}`);
    toast.success(`createMarket: ${getSolscanSigUrl(sig, network)}`);

    // give on-chain state time to catch up
    await sleep(5_000);

    setMarketAddrs([...marketAddrs, marketAddr]);
  };

  const marketPairExists = (): boolean => {
    return occupiedPairs.some(
      ([quote, base]) =>
        (quote === quoteAddr && base === baseAddr) ||
        (quote === baseAddr && base === quoteAddr),
    );
  };

  useEffect(() => {
    const fetchMarketPairs = async (): Promise<void> => {
      try {
        const fetchPromises = marketAddrs.map(async (m: string) => {
          const marketPub = new PublicKey(m);
          const market = await Market.loadFromAddress({
            connection: conn,
            address: marketPub,
          });

          const baseAddr = market.baseMint().toBase58();
          const quoteAddr = market.quoteMint().toBase58();

          return [quoteAddr, baseAddr] as [string, string];
        });

        const pairs = await Promise.all(fetchPromises);
        setOccupiedPairs(pairs);
      } catch (e) {
        console.error('error fetching market pairs', e);
        toast.error(`fetchmarketpairs: ${ensureError(e).message}`);
      }
    };

    fetchMarketPairs();
  }, [marketAddrs, conn]);

  return (
    <main className="flex flex-col items-center justify-center min-h-screen bg-gray-900 text-gray-200 p-8">
      <div className="w-full max-w-4xl bg-gray-800 p-8 rounded-lg shadow-lg text-center">
        <h2 className="text-3xl font-bold mb-6">Generate Keypair</h2>

        <button
          onClick={genKeypair}
          className="bg-blue-500 hover:bg-blue-600 text-white font-bold py-2 px-6 rounded mb-6"
        >
          Generate Keypair
        </button>

        {keypair && (
          <div className="mb-6">
            <p className="text-gray-300 mb-2">
              <strong>Public Key:</strong>{' '}
              <span className="text-yellow-400">{pubkey}</span>
            </p>
            <p className="text-gray-300">
              <strong>Keypair (shortened):</strong>{' '}
              <span className="text-yellow-400">{shortenKeypair(keypair)}</span>
            </p>
          </div>
        )}

        {keypair && (
          <button
            onClick={copyKeypair}
            className="bg-green-500 hover:bg-green-600 text-white font-bold py-2 px-6 rounded"
          >
            Copy Keypair
          </button>
        )}
      </div>

      <div className="w-full max-w-4xl bg-gray-800 p-8 rounded-lg shadow-lg text-center mt-10">
        <h2 className="text-3xl font-bold mb-6">Create Token Mint</h2>

        <label className="block text-gray-300 mb-2 text-left">
          Supply a Keypair (in format [0, 1, 2, ...]):
        </label>
        <input
          type="text"
          value={mintKeypair}
          onChange={handleTokenMintKeypairChange}
          placeholder="[0, 1, 2, ...]"
          className="w-full p-2 mb-4 bg-gray-700 text-gray-200 rounded"
        />

        {mintKeypair && (
          <div className="mb-4">
            <p className="text-gray-300">
              <strong>Mint Public Key:</strong>{' '}
              <span className="text-yellow-400">{mintAddr}</span>
            </p>
          </div>
        )}

        <label className="block text-gray-300 mb-2 text-left">
          Specify Decimals:
        </label>
        <input
          type="number"
          value={decimals}
          onChange={handleDecimalsChange}
          className="w-full p-2 mb-4 bg-gray-700 text-gray-200 rounded"
        />

        <button
          className="bg-green-500 hover:bg-green-600 text-white font-bold py-2 px-6 rounded disabled:opacity-50"
          disabled={!connected}
          onClick={createMint}
        >
          Create Token Mint
        </button>
      </div>

      <div className="w-full max-w-4xl bg-gray-800 p-8 rounded-lg shadow-lg text-center mt-10">
        <h2 className="text-3xl font-bold mb-6">Create Market</h2>

        <label className="block text-gray-300 mb-2 text-left">
          Supply a Market Keypair (in format [0, 1, 2, ...]):
        </label>
        <input
          type="text"
          value={marketKeypair}
          onChange={handleMarketKeypairChange}
          placeholder="[0, 1, 2, ...]"
          className="w-full p-2 mb-4 bg-gray-700 text-gray-200 rounded"
        />

        {marketKeypair && (
          <div className="mb-4">
            <p className="text-gray-300">
              <strong>Market Public Key:</strong>{' '}
              <span className="text-yellow-400">{marketAddr}</span>
            </p>
          </div>
        )}

        <label className="block text-gray-300 mb-2 text-left">Base Mint:</label>
        <input
          type="text"
          value={baseAddr}
          onChange={handleBaseAddrChange}
          placeholder="Base Mint Public Key"
          className="w-full p-2 mb-4 bg-gray-700 text-gray-200 rounded"
        />

        <label className="block text-gray-300 mb-2 text-left">
          Quote Mint:
        </label>
        <input
          type="text"
          value={quoteAddr}
          onChange={handleQuoteAddrChange}
          placeholder="Quote Mint Public Key"
          className="w-full p-2 mb-4 bg-gray-700 text-gray-200 rounded"
        />

        {marketPairExists() ? (
          <p className="text-red-500 font-bold">
            You cannot create another market with the same pair of mints. To
            create multiple markets for a given pair, please use the SDK
            directly.
          </p>
        ) : (
          <button
            className="bg-blue-500 hover:bg-blue-600 text-white font-bold py-2 px-6 rounded disabled:opacity-50"
            disabled={!connected || !marketKeypair || !baseAddr || !quoteAddr}
            onClick={createMarket}
          >
            Create Market
          </button>
        )}
      </div>

      <MintTo />
    </main>
  );
};

export default withAccessControl(CreateMarket);
