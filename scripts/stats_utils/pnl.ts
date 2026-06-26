import { Market } from '../../client/ts/src';
import { SOL_MINT, SOL_USDC_MARKET } from './constants';

export interface TraderPnLDetails {
  totalPnL: number;
  positions: {
    [mint: string]: {
      tokenMint: string;
      marketKey: string | null;
      position: number;
      acquisitionValue: number;
      currentPrice: number;
      marketValue: number;
      pnl: number;
    };
  };
}

/**
 * Calculate PnL for a trader based on their positions
 * TODO: PnL on all quote asset markets
 */
export function calculateTraderPnL(
  trader: string,
  traderPositions: Map<string, Map<string, number>>,
  traderAcquisitionValue: Map<string, Map<string, number>>,
  markets: Map<string, Market>,
  lastPriceByMarket: Map<string, number>,
  baseMintToStablecoinMarkets: Map<string, string[]>,
  includeDetails: boolean = false,
): number | TraderPnLDetails {
  let totalPnL = 0;

  if (!traderPositions.has(trader)) {
    return includeDetails ? { totalPnL: 0, positions: {} } : 0;
  }

  // Setup for detailed return if needed
  const positionDetails: {
    [mint: string]: {
      tokenMint: string;
      marketKey: string | null;
      position: number;
      acquisitionValue: number;
      currentPrice: number;
      marketValue: number;
      pnl: number;
    };
  } = {};

  const positions = traderPositions.get(trader)!;
  const acquisitionValues = traderAcquisitionValue.get(trader)!;

  // Calculate PnL for each base token position
  for (const [baseMint, baseAtomPosition] of positions.entries()) {
    // Skip zero positions
    if (baseAtomPosition === 0) continue;

    // Find stablecoin market for this base token (USDC, USDT, PYUSD, USDS, USD1)
    let usdcMarket: Market | null = null;
    let marketKey: string | null = null;
    let lastPriceAtoms = 0;

    // Special handling for wSOL - directly use the preferred market (SOL/USDC)
    if (baseMint === SOL_MINT) {
      if (markets.has(SOL_USDC_MARKET)) {
        usdcMarket = markets.get(SOL_USDC_MARKET)!;
        marketKey = SOL_USDC_MARKET;
        lastPriceAtoms = lastPriceByMarket.get(marketKey) || 0;
      }
    }

    if (!usdcMarket || !marketKey || lastPriceAtoms === 0) {
      // Look up only the stablecoin markets for this base mint instead of
      // scanning (and base58-encoding) every market. Candidates are pre-indexed
      // in insertion order, so picking the first with a positive price matches
      // the previous full-scan behavior.
      const candidateMarkets =
        baseMintToStablecoinMarkets.get(baseMint) || [];
      for (const marketPk of candidateMarkets) {
        const market = markets.get(marketPk);
        if (market === undefined) {
          continue;
        }
        // Skip markets with zero price
        const price = lastPriceByMarket.get(marketPk) || 0;
        if (price > 0) {
          usdcMarket = market;
          marketKey = marketPk;
          lastPriceAtoms = price;
          break;
        }
      }
    }

    // Skip if no stablecoin market found for this token or if price is zero
    if (!usdcMarket || !marketKey || lastPriceAtoms === 0) continue;

    // Calculate current value in USD (using stablecoin market)
    const baseDecimals = usdcMarket.baseDecimals();
    const quoteDecimals = usdcMarket.quoteDecimals();
    const basePosition = baseAtomPosition / 10 ** baseDecimals;

    // Convert price from atoms to actual price
    const priceInQuote = lastPriceAtoms * 10 ** (baseDecimals - quoteDecimals);

    // Calculate current market value
    const currentPositionValue = basePosition * priceInQuote;

    // Get acquisition value
    const acquisitionValue = acquisitionValues.get(baseMint) || 0;

    // PnL = current value - cost basis (in USD equivalent)
    const positionPnL = currentPositionValue - acquisitionValue;

    // Add to total PnL
    totalPnL += positionPnL;

    // Store detailed position info if requested
    if (includeDetails) {
      positionDetails[baseMint] = {
        tokenMint: baseMint,
        marketKey,
        position: basePosition,
        acquisitionValue,
        currentPrice: priceInQuote,
        marketValue: currentPositionValue,
        pnl: positionPnL,
      };
    }
  }

  // Return either detailed object or just the total PnL number
  return includeDetails ? { totalPnL, positions: positionDetails } : totalPnL;
}
