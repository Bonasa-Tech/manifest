import 'dotenv/config';

import { FillFeed } from '../client/ts/src/fillFeed';
import { FillFeedBlockSub } from '../client/ts/src/fillFeedBlockSub';
import { Connection } from '@solana/web3.js';
import * as promClient from 'prom-client';
import { sendDiscordNotification } from './stats_utils/utils';

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
import express from 'express';
import promBundle from 'express-prom-bundle';

const { RPC_URL, TVL_DISCORD_WEBHOOK_URL } = process.env;

if (!RPC_URL) {
  throw new Error('RPC_URL missing from env');
}

// Alert manifest alerts when a transaction has truncated logs, since fills
// may be silently missing from the feed.
const onTruncatedLogs = (signature: string, slot: number): void => {
  if (!TVL_DISCORD_WEBHOOK_URL) {
    console.warn(
      'TVL_DISCORD_WEBHOOK_URL missing from env, skipping truncated logs alert',
    );
    return;
  }
  const message: string = [
    `**Truncated logs detected in fill feed**`,
    `Fills may be missing from the fills feed for this transaction.`,
    `Slot: ${slot}`,
    `[View on Solscan](https://solscan.io/tx/${signature})`,
  ].join('\n');
  void sendDiscordNotification(TVL_DISCORD_WEBHOOK_URL, message, {
    title: '⚠️ Truncated Logs in Fill Feed',
    timestamp: true,
  });
};

const rpcUrl = RPC_URL as string;
// Default to no block feed
// USE_BLOCK_FEED === 'true';
const useBlockFeed = false;

const monitorFeed = async (feed: FillFeed | FillFeedBlockSub) => {
  // 5 minutes
  const deadThreshold = 300_000;
  // eslint-disable-next-line no-constant-condition
  while (true) {
    await sleep(60_000);
    const msSinceUpdate = feed.msSinceLastUpdate();
    if (msSinceUpdate > deadThreshold) {
      throw new Error(
        `fillFeed has had no updates since ${deadThreshold / 1_000} seconds ago.`,
      );
    }
  }
};

const run = async () => {
  // Prometheus monitoring for this feed on the default prometheus port.
  promClient.collectDefaultMetrics({
    labels: {
      app: 'fillFeed',
    },
  });

  const register = new promClient.Registry();
  register.setDefaultLabels({
    app: 'fillFeed',
  });
  const metricsApp = express();
  metricsApp.listen(9090);

  const promMetrics = promBundle({
    includeMethod: true,
    metricsApp,
    autoregister: false,
  });
  metricsApp.use(promMetrics);

  const timeoutMs = 5_000;

  console.log(
    `starting feed... (using ${useBlockFeed ? 'block' : 'GSFA'} feed)`,
  );
  let feed: FillFeed | FillFeedBlockSub | null = null;
  while (true) {
    try {
      console.log('setting up connection...');
      const conn = new Connection(rpcUrl, 'confirmed');
      console.log('setting up feed...');
      feed = useBlockFeed
        ? new FillFeedBlockSub(conn, 1234, onTruncatedLogs)
        : new FillFeed(conn, onTruncatedLogs);

      if (useBlockFeed) {
        await Promise.all([
          monitorFeed(feed),
          (feed as FillFeedBlockSub).start(),
        ]);
      } else {
        await Promise.all([monitorFeed(feed), (feed as FillFeed).parseLogs()]);
      }
    } catch (e: unknown) {
      console.error('start:feed: error: ', e);
      if (feed) {
        console.log('shutting down feed before restarting...');
        try {
          if (useBlockFeed) {
            await (feed as FillFeedBlockSub).stop();
          } else {
            await (feed as FillFeed).stopParseLogs();
          }
          console.log('feed has shut down successfully');
        } catch (stopErr) {
          console.warn('Error stopping feed:', stopErr);
        }
      }
    } finally {
      console.warn(`sleeping ${timeoutMs / 1000} before restarting`);
      await sleep(timeoutMs);
    }
  }
};

run().catch((e) => {
  console.error('fatal error');
  // we do indeed want to throw here
  throw e;
});
