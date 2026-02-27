'use client';

import React, {
  useMemo,
  ReactElement,
  ReactNode,
  useState,
  useEffect,
  createContext,
  useContext,
  Dispatch,
  SetStateAction,
  useRef,
} from 'react';
import {
  ConnectionProvider,
  WalletProvider,
} from '@solana/wallet-adapter-react';
import { WalletAdapterNetwork } from '@solana/wallet-adapter-base';
import { WalletModalProvider } from '@solana/wallet-adapter-react-ui';
import WalletConnection from './WalletConnection';
import { ManifestClient, Market } from '@bonasa-tech/manifest-sdk';
import {
  AccountInfo,
  Connection,
  GetProgramAccountsResponse,
  PublicKey,
} from '@solana/web3.js';
import { ToastContainer, toast } from 'react-toastify';
import { ensureError } from '@/lib/error';
import {
  Cluster,
  getClusterFromConnection,
} from '@bonasa-tech/manifest-sdk/utils/solana';
import NavBar from './NavBar';
import {
  ActiveByAddr,
  LabelsByAddr,
  VolumeByAddr,
  HasToken22ByAddr,
} from '@/lib/types';
import { fetchAndSetMfxAddrLabels } from '@/lib/address-labels';
import { checkForToken22 } from '@/lib/util';

require('react-toastify/dist/ReactToastify.css');
require('@solana/wallet-adapter-react-ui/styles.css');

interface AppStateContextValue {
  loading: boolean;
  network: WalletAdapterNetwork | null;
  marketAddrs: string[];
  labelsByAddr: LabelsByAddr;
  infoByAddr: LabelsByAddr;
  activeByAddr: ActiveByAddr;
  marketVolumes: VolumeByAddr;
  dailyVolumes: VolumeByAddr;
  hasToken22ByAddr: HasToken22ByAddr;
  setMarketAddrs: Dispatch<SetStateAction<string[]>>;
  setLabelsByAddr: Dispatch<SetStateAction<LabelsByAddr>>;
  setInfoByAddr: Dispatch<SetStateAction<LabelsByAddr>>;
  setActiveByAddr: Dispatch<SetStateAction<ActiveByAddr>>;
  setMarketVolumes: Dispatch<SetStateAction<VolumeByAddr>>;
}

const AppStateContext = createContext<AppStateContextValue | undefined>(
  undefined,
);

export const useAppState = (): AppStateContextValue => {
  const context = useContext(AppStateContext);
  if (!context) {
    throw new Error('useAppState must be used within AppWalletProvider');
  }
  return context;
};

const AppWalletProvider = ({
  children,
}: {
  children: ReactNode;
}): ReactElement => {
  const [network, setNetwork] = useState<WalletAdapterNetwork | null>(null);
  const [marketAddrs, setMarketAddrs] = useState<string[]>([]);
  const [marketVolumes, setMarketVolumes] = useState<VolumeByAddr>({});
  const [dailyVolumes, setDailyVolumes] = useState<VolumeByAddr>({});
  const [labelsByAddr, setLabelsByAddr] = useState<LabelsByAddr>({});
  const [infoByAddr, setInfoByAddr] = useState<LabelsByAddr>({});
  const [activeByAddr, setActiveByAddr] = useState<ActiveByAddr>({});
  const [loading, setLoading] = useState<boolean>(false);
  const [has22ByAddr, setHas22ByAddr] = useState<HasToken22ByAddr>({});
  const setupRun = useRef(false);

  const rpcUrl = process.env.NEXT_PUBLIC_RPC_URL;
  if (!rpcUrl) {
    toast.error('NEXT_PUBLIC_RPC_URL not set');
    throw new Error('NEXT_PUBLIC_RPC_URL not set');
  }

  const determineNetworkFromRpcUrl = async (
    url: string,
  ): Promise<WalletAdapterNetwork> => {
    if (url.includes('mainnet')) {
      return WalletAdapterNetwork.Mainnet;
    } else if (url.includes('devnet')) {
      return WalletAdapterNetwork.Devnet;
    } else if (url.includes('testnet')) {
      return WalletAdapterNetwork.Testnet;
    }

    // Try to determine the network from the genesis hash.
    const cluster: Cluster = await getClusterFromConnection(
      new Connection(url),
    );
    if (cluster == 'mainnet-beta') {
      return WalletAdapterNetwork.Mainnet;
    } else if (cluster == 'devnet') {
      return WalletAdapterNetwork.Devnet;
    }

    toast.error('determineNetworkFromRpcUrl: Unknown network');
    throw new Error('Unknown network');
  };

  useEffect(() => {
    if (setupRun.current) {
      return;
    }

    setupRun.current = true;

    const fetchState = async (): Promise<void> => {
      try {
        const detectedNetwork = await determineNetworkFromRpcUrl(rpcUrl);
        setNetwork(detectedNetwork);

        const conn: Connection = new Connection(rpcUrl, 'confirmed');
        const marketProgramAccounts: GetProgramAccountsResponse =
          await ManifestClient.getMarketProgramAccounts(conn);
        const marketPubs: PublicKey[] = marketProgramAccounts.map(
          (a) => a.pubkey,
        );
        const marketAddrs: string[] = marketPubs.map((p) => p.toBase58());
        const volumeByAddr: VolumeByAddr = {};
        marketProgramAccounts.forEach(
          (
            a: Readonly<{ account: AccountInfo<Buffer>; pubkey: PublicKey }>,
          ) => {
            const market: Market = Market.loadFromBuffer({
              address: a.pubkey,
              buffer: a.account.data,
            });
            if (
              market.quoteMint().toBase58() ==
              'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v'
            ) {
              volumeByAddr[market.address.toBase58()] =
                Number(market.quoteVolume()) / 10 ** 6;
            } else {
              volumeByAddr[market.address.toBase58()] = 0;
            }
          },
        );
        marketAddrs.sort((addr1: string, addr2: string) => {
          return volumeByAddr[addr2] - volumeByAddr[addr1];
        });
        setMarketAddrs(marketAddrs);
        setMarketVolumes(volumeByAddr);

        // Fine to do an N^2 search until the number of markets gets too big.
        const activeByAddr: ActiveByAddr = {};
        const marketsByAddr: { [key: string]: Market } = {};
        marketProgramAccounts.forEach(
          (
            acct1: Readonly<{
              account: AccountInfo<Buffer>;
              pubkey: PublicKey;
            }>,
          ) => {
            const market: Market = Market.loadFromBuffer({
              address: acct1.pubkey,
              buffer: acct1.account.data,
            });
            marketsByAddr[acct1.pubkey.toBase58()] = market;
            let foundBigger: boolean = false;

            marketProgramAccounts.forEach(
              (
                acct2: Readonly<{
                  account: AccountInfo<Buffer>;
                  pubkey: PublicKey;
                }>,
              ) => {
                const market2: Market = Market.loadFromBuffer({
                  address: acct2.pubkey,
                  buffer: acct2.account.data,
                });
                if (
                  market.baseMint().toString() ==
                    market2.baseMint().toString() &&
                  market.quoteMint().toString() ==
                    market2.quoteMint().toString() &&
                  volumeByAddr[market2.address.toBase58()] >
                    volumeByAddr[market.address.toBase58()]
                ) {
                  foundBigger = true;
                }
              },
            );
            activeByAddr[market.address.toBase58()] = !foundBigger;
          },
        );
        setActiveByAddr(activeByAddr);

        const activeAddrs = Object.entries(activeByAddr)
          .filter(([_, active]) => active)
          .map(([addr]) => addr);
        const res: [string, boolean][] = await Promise.all(
          activeAddrs.map(async (addr) => {
            const market = marketsByAddr[addr];
            if (!market) {
              throw new Error(
                'missing market in mapping. this should never happen',
              );
            }
            const [quoteIs22, baseIs22] = await Promise.all([
              checkForToken22(conn, market.quoteMint()),
              checkForToken22(conn, market.baseMint()),
            ]);
            const has22 = quoteIs22 || baseIs22;

            return [addr, has22];
          }),
        );
        const hasToken22ByAddr: HasToken22ByAddr = res.reduce((acc, curr) => {
          const [addr, has22] = curr;
          acc[addr] = has22;
          return acc;
        }, {} as HasToken22ByAddr);
        setHas22ByAddr(hasToken22ByAddr);

        fetchAndSetMfxAddrLabels(
          conn,
          marketProgramAccounts,
          setLabelsByAddr,
          setInfoByAddr,
        );

        const tickers = await fetch(
          'https://mfx-stats-mainnet.fly.dev/tickers',
        );
        const dailyVolumeByAddr: VolumeByAddr = {};
        (await tickers.json()).forEach((ticker: any) => {
          dailyVolumeByAddr[ticker['ticker_id']] = ticker['target_volume'];
        });
        setDailyVolumes(dailyVolumeByAddr);
      } catch (e) {
        console.error('fetching app state:', e);
        toast.error(`fetchTickers: ${ensureError(e).message}`);
      } finally {
        setLoading(false);
      }
    };

    fetchState();
  }, [rpcUrl]);

  const endpoint = useMemo(() => rpcUrl, [rpcUrl]);

  const wallets = useMemo(() => {
    if (!network) return [];
    return [];
  }, [network]);

  if (!network) {
    return <div>Loading...</div>;
  }

  return (
    <ConnectionProvider endpoint={endpoint}>
      <WalletProvider wallets={wallets}>
        <WalletModalProvider>
          <AppStateContext.Provider
            value={{
              network,
              marketAddrs,
              marketVolumes,
              activeByAddr,
              labelsByAddr,
              infoByAddr,
              setLabelsByAddr,
              setInfoByAddr,
              setMarketAddrs,
              setMarketVolumes,
              setActiveByAddr,
              dailyVolumes,
              hasToken22ByAddr: has22ByAddr,
              loading,
            }}
          >
            <WalletConnection />
            <NavBar />
            {children}
            <div className="fixed bottom-4 right-4 bg-gray-800 text-white px-4 py-2 rounded-lg shadow-lg text-sm z-50 pointer-events-none">
              {network ? `connected to ${network}` : 'loading network...'}
            </div>
            <ToastContainer
              position="bottom-right"
              autoClose={15_000}
              theme="dark"
              pauseOnHover={true}
            />
          </AppStateContext.Provider>
        </WalletModalProvider>
      </WalletProvider>
    </ConnectionProvider>
  );
};

export default AppWalletProvider;
