import { Connection, PublicKey } from '@solana/web3.js';
import {
  ENV,
  TokenInfo,
  TokenListContainer,
  TokenListProvider,
} from '@solana/spl-token-registry';
import {
  TOKEN_2022_PROGRAM_ID,
  getMetadataPointerState,
  getTokenMetadata,
  unpackMint,
} from '@solana/spl-token';

/**
 * The SPL token registry is a multi-megabyte list that is identical for every
 * mint, so resolve it once per process and reuse the derived map. This used to
 * be rebuilt on every lookupMintTicker() call, which meant one full copy of the
 * token list per in-flight lookup.
 *
 * Cached as the promise rather than the resolved value so concurrent callers
 * share a single fetch instead of racing. A failed resolve clears the cache so
 * the next caller retries rather than being stuck with a rejected promise.
 */
let tokenMapPromise: Promise<Map<string, TokenInfo>> | undefined;

function getTokenMap(): Promise<Map<string, TokenInfo>> {
  if (!tokenMapPromise) {
    tokenMapPromise = (async (): Promise<Map<string, TokenInfo>> => {
      const provider: TokenListContainer =
        await new TokenListProvider().resolve();
      const tokenList: TokenInfo[] = provider
        .filterByChainId(ENV.MainnetBeta)
        .getList();
      return tokenList.reduce((map, item) => {
        map.set(item.address, item);
        return map;
      }, new Map<string, TokenInfo>());
    })().catch((error) => {
      tokenMapPromise = undefined;
      throw error;
    });
  }
  return tokenMapPromise;
}

/**
 * Lookup the ticker symbol for a given mint address
 * Tries multiple sources in order:
 * 1. Metaplex metadata
 * 2. SPL token registry
 * 3. Token2022 metadata extension
 */
export async function lookupMintTicker(
  connection: Connection,
  mint: PublicKey,
): Promise<string> {
  // Do not follow arbitrary Metaplex JSON URIs from a public stats service.
  // The registry and Token-2022 extension below provide on-chain/curated
  // symbols without turning permissionless mint metadata into server fetches.
  const tokenMap: Map<string, TokenInfo> = await getTokenMap();

  const token: TokenInfo | undefined = tokenMap.get(mint.toBase58());
  if (token) {
    return token.symbol;
  }

  // Finally try Token2022 metadata extension as fallback
  try {
    const mintAccountInfo = await connection.getAccountInfo(mint);
    if (
      mintAccountInfo &&
      mintAccountInfo.owner.equals(TOKEN_2022_PROGRAM_ID)
    ) {
      const mintData = unpackMint(mint, mintAccountInfo, TOKEN_2022_PROGRAM_ID);
      const metadataPointer = getMetadataPointerState(mintData);

      if (metadataPointer && metadataPointer.metadataAddress) {
        const metadata = await getTokenMetadata(
          connection,
          mint,
          'confirmed',
          TOKEN_2022_PROGRAM_ID,
        );
        if (metadata && metadata.symbol) {
          return metadata.symbol;
        }
      }
    }
  } catch (error) {
    console.log('Token2022 metadata lookup failed for', mint.toBase58(), error);
  }

  return '';
}
