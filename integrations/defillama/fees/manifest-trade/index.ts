// Copy into DefiLlama/dimension-adapters at fees/manifest-trade/index.ts.
import { FetchOptions, SimpleAdapter } from '../../adapters/types';
import ADDRESSES from '../../helpers/coreAssets.json';
import { CHAIN } from '../../helpers/chains';
import { httpGet } from '../../utils/fetchURL';

// Collector: B6dmr2UAn2wgjdm3T4N1Vjd8oPYRRTguByW7AEngkeL6
// Core: MNFSTqtC93rEfYHB6hF82sKdZpUDFWkViLByLd1k1Ms
// Wrapper: wMNFSTkir3HgyZTsB7uqu3i7FA73grFCptPXgrZjksL
// Source/decoder: https://github.com/Bonasa-Tech/manifest/blob/main/scripts/stats_utils/feeCollections.ts
const endpoint = 'https://mfx-stats-mainnet.fly.dev/fees';
const label = 'Fee collections (including account resizing)';
const revenueLabel = 'Fee collections to treasury (including account resizing)';
const methodology =
  'SOL received by the Manifest fee collector from wrapper collection, market/vault defrag and expired global-order cleaning, including rent reclaimed through account resizing. Recognized at collection time. Transaction gas and collector-funded account expansion are added back to wallet balance changes to measure gross receipts. Unrelated wallet transfers are excluded.';

async function fetch(options: FetchOptions) {
  const { startTimestamp, endTimestamp } = options;
  const data = await httpGet(
    `${endpoint}?startTimestamp=${startTimestamp}&endTimestamp=${endTimestamp}`,
  );
  if (
    data.complete !== true ||
    data.methodology !== 'collection-basis-v1' ||
    data.startTimestamp !== startTimestamp ||
    data.endTimestamp !== endTimestamp ||
    data.indexedThrough < endTimestamp ||
    typeof data.feesLamports !== 'string' ||
    !/^\d+$/.test(data.feesLamports) ||
    data.revenueLamports !== data.feesLamports
  ) {
    throw new Error('Incomplete or invalid Manifest fee data');
  }
  const dailyFees = options.createBalances();
  dailyFees.add(ADDRESSES.solana.SOL, data.feesLamports, label);
  const dailyRevenue = options.createBalances();
  dailyRevenue.add(ADDRESSES.solana.SOL, data.revenueLamports, revenueLabel);
  return {
    dailyFees,
    dailyRevenue,
    dailyProtocolRevenue: dailyRevenue.clone(),
  };
}

const adapter: SimpleAdapter = {
  version: 2,
  pullHourly: true,
  chains: [CHAIN.SOLANA],
  // Conservative reporting start, not a claim about the first fee charged.
  // The stats indexer must backfill this boundary before submission.
  start: '2026-10-26',
  fetch,
  methodology: {
    Fees: 'SOL collected from wrapper fees, advanced-order fees, expired-order cleaning and account resizing, recognized when collected; excludes fees paid to separate UI/referral destinations.',
    Revenue:
      'Manifest retains the collected wrapper, advanced-order, cleaning and account-resizing proceeds, measured before collection gas and account-expansion funding.',
    ProtocolRevenue:
      'Collected wrapper, advanced-order, cleaning and account-resizing proceeds go to the authorized Manifest treasury collector.',
  },
  breakdownMethodology: {
    Fees: { [label]: methodology },
    Revenue: { [revenueLabel]: methodology },
    ProtocolRevenue: { [revenueLabel]: methodology },
  },
};

export default adapter;
