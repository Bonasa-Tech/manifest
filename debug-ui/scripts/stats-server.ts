import 'dotenv/config';
import { sleep } from '@/lib/util';
import * as promClient from 'prom-client';
import cors from 'cors';
import express, { RequestHandler } from 'express';
import promBundle from 'express-prom-bundle';
import {
  VOLUME_CHECKPOINT_DURATION_SEC,
  DATABASE_CHECKPOINT_DURATION_SEC,
  PORT,
} from './stats_utils/constants';
import { CompleteFillsQueryOptions } from './stats_utils/types';
import { ManifestStatsServer } from './stats_utils/manifestStatsServer';

// Global error handlers to catch unhandled errors and log them before exit
process.on('unhandledRejection', (reason, promise) => {
  console.error('Unhandled Rejection at:', promise, 'reason:', reason);
  console.error('Stack:', reason instanceof Error ? reason.stack : 'No stack');
});

process.on('uncaughtException', (error) => {
  console.error('Uncaught Exception:', error);
  console.error('Stack:', error.stack);
  // Give time for logs to flush before exit
  setTimeout(() => process.exit(1), 1000);
});

const { READ_ONLY } = process.env;

const IS_READ_ONLY = READ_ONLY === 'true';
if (IS_READ_ONLY) {
  console.log('⚠️  Running in READ-ONLY mode - database writes are disabled');
}

const fills: promClient.Counter<'market'> = new promClient.Counter({
  name: 'fills',
  help: 'Number of fills',
  labelNames: ['market'] as const,
});

const reconnects: promClient.Counter<string> = new promClient.Counter({
  name: 'reconnects',
  help: 'Number of reconnects to websocket',
});

const volume: promClient.Gauge<'market' | 'mint' | 'side'> =
  new promClient.Gauge({
    name: 'volume',
    help: 'Volume in last 24 hours in tokens',
    labelNames: ['market', 'mint', 'side'] as const,
  });

const lastPrice: promClient.Gauge<'market'> = new promClient.Gauge({
  name: 'last_price',
  help: 'Last traded price',
  labelNames: ['market'] as const,
});

const depth: promClient.Gauge<'depth_bps' | 'market' | 'trader'> =
  new promClient.Gauge({
    name: 'depth',
    help: 'Notional in orders at a given depth by trader',
    labelNames: ['depth_bps', 'market', 'trader'] as const,
  });

const dbQueryCount: promClient.Counter<'query_type' | 'status'> =
  new promClient.Counter({
    name: 'db_query_count',
    help: 'Number of database queries executed',
    labelNames: ['query_type', 'status'] as const,
  });

const dbQueryDuration: promClient.Histogram<'query_type'> =
  new promClient.Histogram({
    name: 'db_query_duration_seconds',
    help: 'Duration of database queries in seconds',
    labelNames: ['query_type'] as const,
    buckets: [0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1, 2, 5],
  });

/**
 * Server for serving stats according to this spec:
 * https://docs.google.com/document/d/1v27QFoQq1SKT3Priq3aqPgB70Xd_PnDzbOCiuoCyixw/edit?tab=t.0
 */
const run = async () => {
  // Validate environment variables
  const { RPC_URL, DATABASE_URL } = process.env;

  if (!RPC_URL) {
    throw new Error('RPC_URL missing from env');
  }

  if (!DATABASE_URL && !IS_READ_ONLY) {
    console.warn(
      'WARNING: DATABASE_URL not found in environment. Data persistence will not work!',
    );
  }

  // Set up Prometheus metrics
  promClient.collectDefaultMetrics({
    labels: {
      app: 'stats',
    },
  });

  const register = new promClient.Registry();
  register.setDefaultLabels({
    app: 'stats',
  });
  const metricsApp = express();

  // Find available port starting from 9090
  const findAvailablePort = async (startPort: number): Promise<number> => {
    const maxAttempts = 10;
    for (let i = 0; i < maxAttempts; i++) {
      const port = startPort + i;
      try {
        await new Promise<void>((resolve, reject) => {
          const server = metricsApp
            .listen(port, () => {
              server.close();
              resolve();
            })
            .on('error', reject);
        });
        return port;
      } catch (error: any) {
        if (error.code !== 'EADDRINUSE') {
          throw error;
        }
        // Port is in use, try next one
      }
    }
    throw new Error(
      `Could not find available port after ${maxAttempts} attempts starting from ${startPort}`,
    );
  };

  const metricsPort = await findAvailablePort(9090);
  metricsApp.listen(metricsPort, () => {
    console.log(`Prometheus metrics server listening on port ${metricsPort}`);
  });

  const promMetrics = promBundle({
    includeMethod: true,
    metricsApp,
    autoregister: false,
  });
  metricsApp.use(promMetrics);

  // Initialize the stats server
  const statsServer: ManifestStatsServer = new ManifestStatsServer(
    RPC_URL,
    IS_READ_ONLY,
    DATABASE_URL,
    {
      fills,
      reconnects,
      volume,
      lastPrice,
      depth,
      dbQueryCount,
      dbQueryDuration,
    },
  );

  try {
    await statsServer.initialize();
  } catch (error) {
    console.error('Error initializing server:', error);
    throw error;
  }

  // Set up Express routes
  const tickersHandler: RequestHandler = (_req, res) => {
    res.send(statsServer.getTickers());
  };
  const metadataHandler: RequestHandler = (_req, res) => {
    res.send(JSON.stringify(Object.fromEntries(statsServer.getMetadata())));
  };
  const orderbookHandler: RequestHandler = async (req, res) => {
    res.send(
      await statsServer.getOrderbook(
        req.query.ticker_id as string,
        Number(req.query.depth),
      ),
    );
  };
  const volumeHandler: RequestHandler = async (_req, res) => {
    res.send(await statsServer.getVolume());
  };
  const tradersHandler: RequestHandler = (req, res) => {
    const includeDebug = req.query.debug === 'true';
    const limit = req.query.limit ? parseInt(req.query.limit as string) : 500;
    res.send(statsServer.getTraders(includeDebug, limit));
  };
  const recentFillsHandler: RequestHandler = (req, res) => {
    res.send(statsServer.getRecentFills(req.query.market as string));
  };
  const completeFillsHandler: RequestHandler = async (req, res) => {
    try {
      const options: CompleteFillsQueryOptions = {
        market: req.query.market as string,
        taker: req.query.taker as string,
        maker: req.query.maker as string,
        wallet: req.query.wallet as string,
        signature: req.query.signature as string,
        limit: parseInt(req.query.limit as string) || 100,
        offset: parseInt(req.query.offset as string) || 0,
        fromSlot: req.query.fromSlot
          ? parseInt(req.query.fromSlot as string)
          : undefined,
        toSlot: req.query.toSlot
          ? parseInt(req.query.toSlot as string)
          : undefined,
      };

      const result = await statsServer.getCompleteFillsFromDatabase(options);
      res.send(result);
    } catch (error) {
      console.error('Error in completeFills handler:', error);
      res.status(500).send({ error: 'Internal server error' });
    }
  };
  const altsHandler: RequestHandler = async (_req, res) => {
    res.send(await statsServer.getAlts());
  };
  const notionalHandler: RequestHandler = async (_req, res) => {
    res.send(await statsServer.getNotional());
  };
  const checkpointsHandler: RequestHandler = (_req, res) => {
    res.send(statsServer.getCheckpoints());
  };
  const checkpointStatusHandler: RequestHandler = (_req, res) => {
    res.send(statsServer.getCheckpointStatus());
  };

  const backfillHandler: RequestHandler = async (req, res) => {
    try {
      const signature = req.query.signature as string;
      if (!signature) {
        res.status(400).send({ error: 'signature parameter is required' });
        return;
      }

      const result = await statsServer.backfillTransaction(signature);
      res.send({
        success: true,
        signature,
        backfilled: result.backfilled,
        alreadyExisted: result.alreadyExisted,
      });
    } catch (error) {
      console.error('Error in backfill handler:', error);
      res.status(500).send({ error: 'Internal server error' });
    }
  };

  const wrapperHandler: RequestHandler = (req, res) => {
    const owner = req.query.owner as string;
    if (!owner) {
      res.status(400).send({ error: 'owner parameter is required' });
      return;
    }

    const wrapper = statsServer.getWrapper(owner);
    if (wrapper) {
      res.send({ owner, wrapper });
    } else {
      res.status(404).send({ error: 'No wrapper found for owner', owner });
    }
  };

  const wrappersHandler: RequestHandler = (_req, res) => {
    res.send(statsServer.getAllWrappers());
  };

  const app = express();
  app.use(cors());

  // HTTP request metrics middleware - tracks latency and status codes per endpoint
  const httpMetrics = promBundle({
    includeMethod: true,
    includePath: true,
    includeStatusCode: true,
    metricsPath: '/metrics', // Expose on main app too for convenience
    promClient: {
      collectDefaultMetrics: {},
    },
    normalizePath: [
      // Normalize paths with query params to avoid cardinality explosion
      ['^/orderbook.*', '/orderbook'],
      ['^/wrapper.*', '/wrapper'],
      ['^/backfill.*', '/backfill'],
      ['^/completeFills.*', '/completeFills'],
      ['^/recentFills.*', '/recentFills'],
      ['^/traders.*', '/traders'],
    ],
  });
  app.use(httpMetrics);

  // Global timeout middleware - 30 second timeout for all requests
  app.use((req, res, next) => {
    res.setTimeout(30_000, () => {
      console.error(`Request timeout: ${req.method} ${req.path} ${req.query}`);
      if (!res.headersSent) {
        res.status(503).send({ error: 'Request timeout' });
      }
    });
    next();
  });

  app.get('/tickers', tickersHandler);
  app.get('/metadata', metadataHandler);
  app.get('/orderbook', orderbookHandler);
  app.get('/volume', volumeHandler);
  app.get('/traders', tradersHandler);
  app.get('/traders/debug', (req, res) => {
    const limit = req.query.limit ? parseInt(req.query.limit as string) : 500;
    res.send(statsServer.getTraders(true, limit));
  });
  app.get('/recentFills', recentFillsHandler);
  app.get('/completeFills', completeFillsHandler);
  app.get('/alts', altsHandler);
  app.get('/notional', notionalHandler);
  app.get('/checkpoints', checkpointsHandler);
  app.get('/checkpointStatus', checkpointStatusHandler);
  app.get('/backfill', backfillHandler);
  app.get('/wrapper', wrapperHandler);
  app.get('/wrappers', wrappersHandler);

  // Add health check endpoint for Fly.io
  app.get('/health', (_req, res) => {
    res.status(200).send('OK');
  });

  app.listen(Number(PORT!), () => {
    console.log(`Server running on port ${PORT}`);
  });

  // Set up graceful shutdown
  const gracefulShutdown = async (signal: string) => {
    console.log(`Received ${signal}, saving state before exit...`);
    try {
      if (DATABASE_URL && !IS_READ_ONLY) {
        await statsServer.saveState();
      }
      await statsServer.shutdown();
      console.log('State saved, exiting');
      process.exit(0);
    } catch (error) {
      console.error('Error during shutdown:', error);
      process.exit(1);
    }
  };

  process.on('SIGINT', () => gracefulShutdown('SIGINT'));
  process.on('SIGTERM', () => gracefulShutdown('SIGTERM'));

  await Promise.all([
    // Isolate advancing checkpoints so that it is more reliable. The most
    // important feature of stats server is accurate volume reporting, so dont
    // let the database cause issues with live serving of volume.
    (async () => {
      // eslint-disable-next-line no-constant-condition
      while (true) {
        try {
          await Promise.all([
            statsServer.advanceCheckpoints(),
            sleep(VOLUME_CHECKPOINT_DURATION_SEC * 1_000),
          ]);
        } catch (error) {
          console.error('Error in advancing checkpoints:', error);
          // Continue the loop instead of crashing
          await sleep(5_000); // Add a short delay before retrying
        }
      }
    })(),
    (async () => {
      // eslint-disable-next-line no-constant-condition
      while (true) {
        try {
          await Promise.all([
            sleep(DATABASE_CHECKPOINT_DURATION_SEC * 1_000),
            statsServer.depthProbe(),
            DATABASE_URL && !IS_READ_ONLY
              ? statsServer.saveState()
              : Promise.resolve(),
          ]);
        } catch (error) {
          console.error('Error in saving loop:', error);
          // Continue the loop instead of crashing
          await sleep(5_000); // Add a short delay before retrying
        }
      }
    })(),
  ]);
};

run().catch((e) => {
  console.error('fatal error');
  throw e;
});
