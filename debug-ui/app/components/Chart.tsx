'use client';

import { useEffect, useRef, ReactElement, useState } from 'react';
import {
  createChart,
  IChartApi,
  ISeriesApi,
  ColorType,
  CandlestickData,
  Time,
} from 'lightweight-charts';
import { FillLogResult, Market } from '@bonasa-tech/manifest-sdk';
import { useConnection } from '@solana/wallet-adapter-react';
import { PublicKey } from '@solana/web3.js';
import { toast } from 'react-toastify';
import { useAppState } from './AppWalletProvider';

const Chart = ({ marketAddress }: { marketAddress: string }): ReactElement => {
  const chartContainerRef = useRef<HTMLDivElement | null>(null);
  const chartRef = useRef<IChartApi | null>(null);
  const candlestickSeriesRef = useRef<ISeriesApi<'Candlestick'> | null>(null);
  const marketRef = useRef<Market | null>(null); // To track the latest market value

  const { labelsByAddr, hasToken22ByAddr } = useAppState();

  const [chartEntries, setChartEntries] = useState<CandlestickData[]>([]);

  const { connection: conn } = useConnection();

  useEffect(() => {
    const marketPub = new PublicKey(marketAddress);
    Market.loadFromAddress({
      connection: conn,
      address: marketPub,
    }).then((m) => {
      console.log('got market', m);
      marketRef.current = m;
    });
  }, [conn, marketAddress, labelsByAddr]);

  useEffect(() => {
    const feedUrl = process.env.NEXT_PUBLIC_FEED_URL;
    if (!feedUrl) {
      toast.error('NEXT_PUBLIC_FEED_URL not set');
      throw new Error('NEXT_PUBLIC_FEED_URL not set');
    }
    const ws = new WebSocket(feedUrl);
    let fillsInCurrentInterval: CandlestickData | null = null;

    ws.onmessage = async (message): Promise<void> => {
      const fill: FillLogResult = JSON.parse(message.data);
      if (fill.market !== marketAddress) {
        return;
      }

      const aggregateFillData = async (
        fill: FillLogResult,
        fillsInInterval: CandlestickData | null,
      ): Promise<CandlestickData> => {
        const time = await conn.getBlockTime(fill.slot);
        if (!time) return fillsInInterval!;

        const timestamp = Math.floor(time / 60) * 60; // Group by minute

        const quoteTokens =
          Number(fill.quoteAtoms) /
          10 ** Number(marketRef.current?.quoteDecimals() || 0);
        const baseTokens =
          Number(fill.baseAtoms) /
          10 ** Number(marketRef.current?.baseDecimals() || 0);

        const price = Number((quoteTokens / baseTokens).toFixed(4));

        if (!fillsInInterval) {
          return {
            time: timestamp as Time,
            open: price,
            high: price,
            low: price,
            close: price,
          };
        } else {
          return {
            time: timestamp as Time,
            open: fillsInInterval.open,
            high: Math.max(fillsInInterval.high, price),
            low: Math.min(fillsInInterval.low, price),
            close: price,
          };
        }
      };

      const updatedCandlestick = await aggregateFillData(
        fill,
        fillsInCurrentInterval,
      );

      if (
        !fillsInCurrentInterval ||
        updatedCandlestick.time !== fillsInCurrentInterval.time
      ) {
        if (fillsInCurrentInterval) {
          setChartEntries(
            (prevEntries) =>
              [
                ...prevEntries.filter(Boolean),
                fillsInCurrentInterval,
              ] as CandlestickData[],
          );
        }
        fillsInCurrentInterval = updatedCandlestick;
      } else {
        fillsInCurrentInterval = updatedCandlestick;
      }

      candlestickSeriesRef.current?.setData([
        ...chartEntries.filter(Boolean),
        fillsInCurrentInterval,
      ] as CandlestickData[]);
    };

    return (): void => {
      ws.close();
    };
  }, [chartEntries, conn, marketAddress]);

  useEffect(() => {
    if (chartContainerRef.current) {
      const chart = createChart(chartContainerRef.current, {
        width: chartContainerRef.current.clientWidth,
        height: 400,
        layout: {
          background: {
            type: ColorType.Solid,
            color: '#1A202C',
          },
          textColor: '#D3D3D3',
        },
        grid: {
          vertLines: {
            color: '#2B2B43',
          },
          horzLines: {
            color: '#2B2B43',
          },
        },
        timeScale: {
          borderColor: '#485c7b',
        },
        watermark: {
          visible: false,
        },
      });

      chartRef.current = chart;

      const candlestickSeries = chart.addCandlestickSeries({
        upColor: '#4caf50',
        downColor: '#ff5252',
        borderDownColor: '#ff5252',
        borderUpColor: '#4caf50',
        wickDownColor: '#ff5252',
        wickUpColor: '#4caf50',
      });

      candlestickSeriesRef.current = candlestickSeries;

      const handleResize = (): void => {
        if (chartContainerRef.current) {
          chart.applyOptions({ width: chartContainerRef.current.clientWidth });
        }
      };
      window.addEventListener('resize', handleResize);

      return (): void => {
        chart.remove();
        window.removeEventListener('resize', handleResize);
      };
    }
  }, []);

  useEffect(() => {
    candlestickSeriesRef.current?.setData(chartEntries as CandlestickData[]);
  }, [chartEntries]);

  return (
    <div className="bg-gray-800 p-4 rounded-lg w-full">
      <h2 className="text-xl font-semibold text-gray-200 mb-4 text-center">
        {labelsByAddr[marketAddress]}
        {hasToken22ByAddr[marketAddress] && (
          <span className="ml-3 inline-block bg-blue-500 text-white text-xs font-semibold px-2 py-1 rounded">
            TOKEN_2022
          </span>
        )}
      </h2>

      <div ref={chartContainerRef} className="w-full h-96" />
    </div>
  );
};

export default Chart;
