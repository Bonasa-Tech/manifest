/**
 * Known aggregator program IDs and their names.
 * Used to identify which aggregator routed a transaction.
 */
export const AGGREGATOR_PROGRAM_IDS = {
  MEXkeo4BPUCZuEJ4idUUwMPu4qvc9nkqtLn3yAyZLxg: 'Swissborg',
  T1TANpTeScyeqVzzgNViGDNrkQ6qHz9KrSBS4aNXvGT: 'Titan',
  '6m2CDdhRgxpH4WjvdzxAYbGxwdGUz5MziiL5jek2kBma': 'OKX',
  proVF4pMXVaYqmy4NjniPh4pqKNfMmsihgd4wdkCX3u: 'OKX',
  va1t8sdGkReA6XFgAeZGXmdQoiEtMirwy4ifLv7yGdH: 'OKX',
  DF1ow4tspfHX9JwWJsAb9epbkA8hmpSEAtxXy1V27QBH: 'DFlow',
  JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4: 'Jupiter',
  SPURp82qAR9nvzy8j1gP31zmzGytrgDBKcpGzeGkka8: 'Spur',
  s7SunwrPG5SbViEKiViaDThPRJxkkTrNx2iRPN3exNC: 'Bitget',
  '2UUgGySTVXmKFatH7pGQo84ZrzdSYF5zw9iqrGwBMuuj': 'Bitget',
  HuTkmnrv4zPnArMqpbMbFhfwzTR7xfWQZHH1aQKzDKFZ: 'Liquid Mesh',
  FqGg2Y1FNxMiGd51Q6UETixQWkF5fB92MysbYogRJb3P: 'HawkFi',
  Sett1erwx2eqT5A8uvu8GBxDFT2W5TNnhirL7hLmb8m: '0x',
  bung5E8oYryGA7d2ivgoe1spoZRS88cvcwoK23TtExg: 'Bungee',
  '2DAtv2URAcb9ZHVMHEB8E3TFBTk9PoNSYknjq8DVr69c': 'Magpie',
} as const;

/**
 * Known originating protocol program IDs and their names.
 * Used to identify which protocol initiated a transaction (for dual attribution
 * scenarios like Kamino using Spur aggregator).
 */
export const ORIGINATING_PROTOCOL_IDS = {
  LiMoM9rMhrdYrfzUCxQppvxCSG1FcrUK9G8uLq4A1GF: 'kamino',
  UMnFStVeG1ecZFc2gc5K3vFy3sMpotq8C91mXBQDGwh: 'cabana',
  BQ72nSv9f3PRyRKCBnHLVrerrv37CYTHm5h3s9VSGQDV: 'jupiter', // JUP 1
  '2MFoS3MPtvyQ4Wh4M9pdfPjz6UhVoNbFbGJAskCPCj3h': 'jupiter', // JUP 2
  HU23r7UoZbqTUuh3vA7emAGztFtqwTeVips789vqxxBw: 'jupiter', // JUP 3
  '6LXutJvKUw8Q5ue2gCgKHQdAN4suWW8awzFVC6XCguFx': 'jupiter', // JUP 5
  CapuXNQoDviLvU1PxFiizLgPNQCxrsag1uMeyk6zLVps: 'jupiter', // JUP 6
  GGztQqQ6pCPaJQnNpXBgELr5cs3WwDakRbh1iEMzjgSJ: 'jupiter', // JUP 7
  '9nnLbotNTcUhvbrsA6Mdkx45Sm82G35zo28AqUvjExn8': 'jupiter', // JUP 8
  '6U91aKa8pmMxkJwBCfPTmUEfZi6dHe7DcFq2ALvB2tbB': 'jupiter', // JUP 12
  '4xDsmeTWPNjgSVSS1VTfzFq3iHZhp77ffPkAmkZkdu71': 'jupiter', // JUP 14
  GP8StUXNYSZjPikyRsvkTbvRV1GBxMErb59cpeCJnDf1: 'jupiter', // JUP 15
  HFqp6ErWHY6Uzhj8rFyjYuDya2mXUpYEk8VW75K9PSiY: 'jupiter', // JUP 16
  '9yj3zvLS3fDMqi1F8zhkaWfq8TZpZWHe6cz1Sgt7djXf': 'phantom',
  '8psNvWTrdNTiVRNzAgsou9kETXNJm2SXZyaKuJraVRtf': 'phantom',
  B3111yJCeHBcA1bizdJjUFPALfhAfSRnAbJzGUtnt56A: 'binance',
  BN111JnbLtbmQqqiCh7h2pDKhAhMx4wi77Mj7jJFbyp8: 'binance',
  BN111AnCthcdPVNJ6jkir9TDaS7xqXT8EhetAmYpNqFt: 'binance',
  '7JCe3GHwkEr3feHgtLXnmuJ1yB3A7coSeyynxTBgdG8k': 'coinbase',
  F7p3dFrjRTbtRp8FRF6qHLomXbKRBzpvBLjtQcfcgmNe: 'relay',
  AgmLJBMDCqWynYnQiPCuj9ewsNNsBJXyzoUhD9LJzN51: 'fomo',
  JTXJTXfr1wVRMEzqiPhXUr69zJtfGuLh5qEiXG772Zj: 'jtx',
  sighWH8KaiT7QhtV4w29ReVF8kG6D5yG3EQP1KYyGVF: 'jupui',
} as const;

/**
 * Signer addresses that act on behalf of the real taker. When one of these
 * appears as a transaction signer, the other signer is the actual taker and
 * should be substituted in place of it.
 */
export const DELEGATING_SIGNERS: Set<string> = new Set<string>([
  'sighWH8KaiT7QhtV4w29ReVF8kG6D5yG3EQP1KYyGVF',
]);

/**
 * If a known delegating signer signed the transaction, return the real taker:
 * the fee payer (original signer) when it isn't itself the delegating signer,
 * otherwise the other signer.
 * @param signers - Array of base58-encoded signer public key strings
 * @param originalSigner - The fee payer / first signer, if known
 * @returns The address to use as the taker, or undefined if there is no
 *   delegating signer or no distinct other signer to substitute.
 */
export function resolveTakerFromSigners(
  signers: string[] | undefined,
  originalSigner?: string,
): string | undefined {
  const hasDelegatingSigner: boolean =
    (signers?.some((signer) => DELEGATING_SIGNERS.has(signer)) ?? false) ||
    (originalSigner !== undefined && DELEGATING_SIGNERS.has(originalSigner));
  if (!hasDelegatingSigner) {
    return undefined;
  }
  // Prefer the fee payer (original signer) when it is the real taker, then fall
  // back to any other non-delegating signer.
  if (originalSigner && !DELEGATING_SIGNERS.has(originalSigner)) {
    return originalSigner;
  }
  return signers?.find((signer) => !DELEGATING_SIGNERS.has(signer));
}

/**
 * Detect aggregator from a list of account key strings.
 * @param accountKeys - Array of base58-encoded public key strings
 * @returns The name of the detected aggregator, or undefined if none found
 */
export function detectAggregatorFromKeys(
  accountKeys: string[],
): string | undefined {
  for (const account of accountKeys) {
    const aggregator =
      AGGREGATOR_PROGRAM_IDS[account as keyof typeof AGGREGATOR_PROGRAM_IDS];
    if (aggregator) {
      return aggregator;
    }
  }
  return undefined;
}

/**
 * Detect originating protocol from a list of account key strings.
 * @param accountKeys - Array of base58-encoded public key strings
 * @returns The name of the detected originating protocol, or undefined if none found
 */
export function detectOriginatingProtocolFromKeys(
  accountKeys: string[],
): string | undefined {
  for (const accountKey of accountKeys) {
    const protocol =
      ORIGINATING_PROTOCOL_IDS[
        accountKey as keyof typeof ORIGINATING_PROTOCOL_IDS
      ];
    if (protocol) {
      return protocol;
    }
  }
  return undefined;
}
