/**
 * Known aggregator program IDs and their names.
 * Used to identify which aggregator routed a transaction.
 */
export const AGGREGATOR_PROGRAM_IDS = {
  MEXkeo4BPUCZuEJ4idUUwMPu4qvc9nkqtLn3yAyZLxg: 'Swissborg',
  T1TANpTeScyeqVzzgNViGDNrkQ6qHz9KrSBS4aNXvGT: 'Titan',
  '6m2CDdhRgxpH4WjvdzxAYbGxwdGUz5MziiL5jek2kBma': 'OKX',
  proVF4pMXVaYqmy4NjniPh4pqKNfMmsihgd4wdkCX3u: 'OKX',
  DF1ow4tspfHX9JwWJsAb9epbkA8hmpSEAtxXy1V27QBH: 'DFlow',
  JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4: 'Jupiter',
  SPURp82qAR9nvzy8j1gP31zmzGytrgDBKcpGzeGkka8: 'Spur',
  s7SunwrPG5SbViEKiViaDThPRJxkkTrNx2iRPN3exNC: 'Bitget',
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
  'GP8StUXNYSZjPikyRsvkTbvRV1GBxMErb59cpeCJnDf1': 'jupier', // JUP 15
  HFqp6ErWHY6Uzhj8rFyjYuDya2mXUpYEk8VW75K9PSiY: 'jupiter', // JUP 16
  '9yj3zvLS3fDMqi1F8zhkaWfq8TZpZWHe6cz1Sgt7djXf': 'phantom',
  '8psNvWTrdNTiVRNzAgsou9kETXNJm2SXZyaKuJraVRtf': 'phantom',
  B3111yJCeHBcA1bizdJjUFPALfhAfSRnAbJzGUtnt56A: 'binance',
  '7JCe3GHwkEr3feHgtLXnmuJ1yB3A7coSeyynxTBgdG8k': 'coinbase',
} as const;

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
