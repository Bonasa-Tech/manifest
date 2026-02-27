import 'dotenv/config';

import { Connection, PublicKey } from '@solana/web3.js';
import { Market, RestingOrder, ClaimedSeat } from '@bonasa-tech/manifest-sdk';
import { OrderType } from '@bonasa-tech/manifest-sdk/manifest/types';
import {
  FIXED_MANIFEST_HEADER_SIZE,
  NIL,
} from '@bonasa-tech/manifest-sdk/constants';

const { RPC_URL } = process.env;

if (!RPC_URL) {
  throw new Error('RPC_URL missing from env');
}

const orderTypeToString = (orderType: OrderType): string => {
  switch (orderType) {
    case OrderType.Limit:
      return 'Limit';
    case OrderType.ImmediateOrCancel:
      return 'IOC';
    case OrderType.PostOnly:
      return 'PostOnly';
    case OrderType.Global:
      return 'Global';
    case OrderType.Reverse:
      return 'Reverse';
    case OrderType.ReverseTight:
      return 'ReverseTight';
    default:
      return `Unknown(${orderType})`;
  }
};

const formatExpiration = (
  lastValidSlot: bigint | number,
  orderType: OrderType,
): string => {
  // Reverse orders never expire
  if (orderType === OrderType.Reverse || orderType === OrderType.ReverseTight) {
    return 'Never';
  }
  const slot = Number(lastValidSlot);
  // 0 or max u32 means no expiration
  if (slot === 0 || slot === 4294967295) {
    return 'Never';
  }
  return slot.toString();
};

const formatPrice = (price: number): string => {
  if (price >= 1000) {
    return price.toLocaleString(undefined, { maximumFractionDigits: 2 });
  } else if (price >= 1) {
    return price.toFixed(4);
  } else if (price >= 0.0001) {
    return price.toFixed(6);
  } else {
    return price.toExponential(4);
  }
};

const formatSize = (size: number): string => {
  if (size >= 1000000) {
    return `${(size / 1000000).toFixed(2)}M`;
  } else if (size >= 1000) {
    return `${(size / 1000).toFixed(2)}K`;
  } else if (size >= 1) {
    return size.toFixed(4);
  } else {
    return size.toFixed(6);
  }
};

const formatBalance = (atoms: number, decimals: number): string => {
  const tokens = atoms / 10 ** decimals;
  if (tokens === 0) {
    return '0';
  } else if (tokens >= 1) {
    return tokens.toLocaleString(undefined, {
      minimumFractionDigits: 2,
      maximumFractionDigits: 6,
    });
  } else {
    return tokens.toFixed(6);
  }
};

const formatVolume = (atoms: number, decimals: number): string => {
  const tokens = atoms / 10 ** decimals;
  if (tokens === 0) {
    return '0';
  } else if (tokens >= 1) {
    return tokens.toLocaleString(undefined, {
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    });
  } else {
    return tokens.toFixed(6);
  }
};

const printFreeList = (buffer: Buffer): void => {
  // Read freeListHeadIndex from market header (offset 176)
  const freeListHeadIndex = buffer.readUInt32LE(176);

  console.log('\n\x1b[35m=== FREE LIST ===\x1b[0m');

  if (freeListHeadIndex === NIL) {
    console.log('  (empty)');
    return;
  }

  const indices: number[] = [];
  let currentIndex = freeListHeadIndex;
  const maxIterations = 10000; // Safety limit
  let iterations = 0;

  while (currentIndex !== NIL && iterations < maxIterations) {
    indices.push(currentIndex);
    // Read next_index from the first 4 bytes of the node in the dynamic section
    const nodeOffset = FIXED_MANIFEST_HEADER_SIZE + currentIndex;
    if (nodeOffset + 4 > buffer.length) {
      console.log(`  Error: Node offset ${nodeOffset} exceeds buffer length`);
      break;
    }
    currentIndex = buffer.readUInt32LE(nodeOffset);
    iterations++;
  }

  if (iterations >= maxIterations) {
    console.log(`  Warning: Reached max iterations (${maxIterations})`);
  }

  console.log(`Free list length: ${indices.length}`);
  console.log(`Head index: ${freeListHeadIndex}`);
  console.log('');
  console.log('Linked list traversal:');

  // Print as a linked list
  const parts: string[] = [];
  for (let i = 0; i < indices.length; i++) {
    parts.push(indices[i].toString());
  }
  parts.push('NIL');

  // Print in chunks to avoid very long lines
  const chunkSize = 10;
  for (let i = 0; i < parts.length; i += chunkSize) {
    const chunk = parts.slice(i, Math.min(i + chunkSize, parts.length));
    const prefix = i === 0 ? '  ' : '    -> ';
    console.log(prefix + chunk.join(' -> '));
  }
};

const printSeatsTable = (market: Market): void => {
  const seats = market.claimedSeats();
  const baseDecimals = market.baseDecimals();
  const quoteDecimals = market.quoteDecimals();

  console.log('\n\x1b[36m=== SEATS ===\x1b[0m');
  console.log(
    `${'Pubkey'.padEnd(46)} | ${'Base Withdrawable'.padStart(18)} | ${'Quote Withdrawable'.padStart(18)} | ${'Total Base'.padStart(14)} | ${'Total Quote'.padStart(14)} | ${'Quote Volume'.padStart(14)}`,
  );
  console.log('-'.repeat(148));

  if (seats.length === 0) {
    console.log('  (no seats)');
    return;
  }

  let totalBaseWithdraw = 0;
  let totalQuoteWithdraw = 0;
  let totalBaseAll = 0;
  let totalQuoteAll = 0;
  let totalQuoteVol = 0;

  for (const seat of seats) {
    const pubkey = seat.publicKey.toBase58();
    const balances = market.getBalances(seat.publicKey);

    const baseWithdrawTokens = Number(seat.baseBalance) / 10 ** baseDecimals;
    const quoteWithdrawTokens = Number(seat.quoteBalance) / 10 ** quoteDecimals;
    const totalBaseTokens =
      balances.baseWithdrawableBalanceTokens +
      balances.baseOpenOrdersBalanceTokens;
    const totalQuoteTokens =
      balances.quoteWithdrawableBalanceTokens +
      balances.quoteOpenOrdersBalanceTokens;
    const quoteVolTokens = Number(seat.quoteVolume) / 10 ** quoteDecimals;

    totalBaseWithdraw += baseWithdrawTokens;
    totalQuoteWithdraw += quoteWithdrawTokens;
    totalBaseAll += totalBaseTokens;
    totalQuoteAll += totalQuoteTokens;
    totalQuoteVol += quoteVolTokens;

    const baseWithdraw = formatBalance(Number(seat.baseBalance), baseDecimals);
    const quoteWithdraw = formatBalance(
      Number(seat.quoteBalance),
      quoteDecimals,
    );
    const totalBase = formatBalance(
      totalBaseTokens * 10 ** baseDecimals,
      baseDecimals,
    );
    const totalQuote = formatBalance(
      totalQuoteTokens * 10 ** quoteDecimals,
      quoteDecimals,
    );
    const quoteVol = formatVolume(Number(seat.quoteVolume), quoteDecimals);

    console.log(
      `${pubkey.padEnd(46)} | ${baseWithdraw.padStart(18)} | ${quoteWithdraw.padStart(18)} | ${totalBase.padStart(14)} | ${totalQuote.padStart(14)} | ${quoteVol.padStart(14)}`,
    );
  }

  // Print summary
  console.log('-'.repeat(148));
  const sumBaseWithdraw = formatVolume(
    totalBaseWithdraw * 10 ** baseDecimals,
    baseDecimals,
  );
  const sumQuoteWithdraw = formatVolume(
    totalQuoteWithdraw * 10 ** quoteDecimals,
    quoteDecimals,
  );
  const sumTotalBase = formatVolume(
    totalBaseAll * 10 ** baseDecimals,
    baseDecimals,
  );
  const sumTotalQuote = formatVolume(
    totalQuoteAll * 10 ** quoteDecimals,
    quoteDecimals,
  );
  const sumQuoteVol = formatVolume(
    totalQuoteVol * 10 ** quoteDecimals,
    quoteDecimals,
  );

  console.log(
    `${'TOTAL (' + seats.length + ' seats)'.padEnd(46)} | ${sumBaseWithdraw.padStart(18)} | ${sumQuoteWithdraw.padStart(18)} | ${sumTotalBase.padStart(14)} | ${sumTotalQuote.padStart(14)} | ${sumQuoteVol.padStart(14)}`,
  );
};

const printOrderTable = (orders: RestingOrder[], side: 'BID' | 'ASK'): void => {
  const color = side === 'BID' ? '\x1b[32m' : '\x1b[31m';
  const reset = '\x1b[0m';

  console.log(`\n${color}=== ${side}S ===${reset}`);
  console.log(
    `${'Price'.padStart(14)} | ${'Size'.padStart(14)} | ${'Type'.padStart(12)} | ${'Expiration'.padStart(20)} | ${'SeqNum'.padStart(12)} | Trader`,
  );
  console.log('-'.repeat(115));

  if (orders.length === 0) {
    console.log('  (no orders)');
    return;
  }

  for (const order of orders) {
    const price = formatPrice(order.tokenPrice);
    const size = formatSize(Number(order.numBaseTokens));
    const orderType = orderTypeToString(order.orderType);
    const expiration = formatExpiration(
      Number(order.lastValidSlot),
      order.orderType,
    );
    const trader = order.trader.toBase58().slice(0, 8) + '...';

    let typeDisplay = orderType;
    if (
      (order.orderType === OrderType.Reverse ||
        order.orderType === OrderType.ReverseTight) &&
      order.spreadBps !== undefined
    ) {
      typeDisplay = `${orderType}(${order.spreadBps.toFixed(2)}bps)`;
    }

    const seqNum = order.sequenceNumber.toString();

    console.log(
      `${color}${price.padStart(14)}${reset} | ${size.padStart(14)} | ${typeDisplay.padStart(12)} | ${expiration.padStart(20)} | ${seqNum.padStart(12)} | ${trader}`,
    );
  }
};

const run = async () => {
  const args = process.argv.slice(2);
  const showSeats = args.includes('--seats');
  const showFreeList = args.includes('--freelist');
  const marketPkArg = args.find((arg) => !arg.startsWith('--'));

  if (!marketPkArg) {
    console.error(
      'Usage: tsx scripts/view-market.ts <market_pubkey> [--seats] [--freelist]',
    );
    console.error(
      'Example: tsx scripts/view-market.ts HyLPjWF8HQxF9iHCBpYPkVpR43SUNusoX4eZ5u2HYPt1',
    );
    console.error(
      '         tsx scripts/view-market.ts HyLPjWF8HQxF9iHCBpYPkVpR43SUNusoX4eZ5u2HYPt1 --seats',
    );
    console.error(
      '         tsx scripts/view-market.ts HyLPjWF8HQxF9iHCBpYPkVpR43SUNusoX4eZ5u2HYPt1 --freelist',
    );
    process.exit(1);
  }

  let marketPk: PublicKey;
  try {
    marketPk = new PublicKey(marketPkArg);
  } catch {
    console.error(`Invalid public key: ${marketPkArg}`);
    process.exit(1);
  }

  const connection = new Connection(RPC_URL!);

  console.log(`\nLoading market ${marketPk.toBase58()}...`);

  // Fetch raw account info to access the buffer for free list
  const accountInfo = await connection.getAccountInfo(marketPk);
  if (!accountInfo) {
    console.error(`Failed to load market account: ${marketPkArg}`);
    process.exit(1);
  }
  const buffer = accountInfo.data;

  const market = Market.loadFromBuffer({
    address: marketPk,
    buffer,
  });

  const currentSlot = await connection.getSlot();

  // Get bids and asks sorted by price (most competitive first)
  const bids = market.bidsL2(); // Already sorted best (highest) to worst
  const asks = market.asksL2(); // Already sorted best (lowest) to worst

  // Print market info
  console.log('\n' + '='.repeat(90));
  console.log(`MARKET: ${marketPk.toBase58()}`);
  console.log('='.repeat(90));
  console.log(`Base Mint:  ${market.baseMint().toBase58()}`);
  console.log(`Quote Mint: ${market.quoteMint().toBase58()}`);
  console.log(
    `Decimals:   Base=${market.baseDecimals()}, Quote=${market.quoteDecimals()}`,
  );
  console.log(
    `Volume:     ${Number(market.quoteVolume()).toLocaleString()} quote atoms`,
  );
  console.log(`Current Slot: ${currentSlot}`);

  const bestBid = market.bestBidPrice();
  const bestAsk = market.bestAskPrice();
  if (bestBid && bestAsk) {
    const spread = bestAsk - bestBid;
    const spreadPct = ((spread / bestAsk) * 100).toFixed(4);
    console.log(
      `\nBest Bid: ${formatPrice(bestBid)} | Best Ask: ${formatPrice(bestAsk)} | Spread: ${formatPrice(spread)} (${spreadPct}%)`,
    );
  } else if (bestBid) {
    console.log(`\nBest Bid: ${formatPrice(bestBid)} | Best Ask: (none)`);
  } else if (bestAsk) {
    console.log(`\nBest Bid: (none) | Best Ask: ${formatPrice(bestAsk)}`);
  } else {
    console.log(`\nOrderbook is empty`);
  }

  console.log(`\nOrders: ${bids.length} bids, ${asks.length} asks`);

  // Print asks first (in reverse order so lowest ask is at bottom, closest to bids)
  const asksReversed = [...asks].reverse();
  printOrderTable(asksReversed, 'ASK');

  // Print bids
  printOrderTable(bids, 'BID');

  // Print seats if flag is set
  if (showSeats) {
    printSeatsTable(market);
  }

  // Print free list if flag is set
  if (showFreeList) {
    printFreeList(buffer);
  }

  console.log('\n');
};

run().catch((e) => {
  console.error('Error:', e);
  process.exit(1);
});
