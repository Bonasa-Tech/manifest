import { Connection, PublicKey } from '@solana/web3.js';
import { Metaplex, Pda } from '@metaplex-foundation/js';
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
  // Create Metaplex instance
  const metaplex: Metaplex = Metaplex.make(connection);

  // First try Metaplex metadata for SPL tokens
  const metadataAccount: Pda = metaplex.nfts().pdas().metadata({ mint });
  const metadataAccountInfo = await connection.getAccountInfo(metadataAccount);
  if (metadataAccountInfo) {
    const token = await metaplex.nfts().findByMint({ mintAddress: mint });
    return token.symbol;
  }

  // Then try SPL token registry
  const provider: TokenListContainer = await new TokenListProvider().resolve();
  const tokenList: TokenInfo[] = provider
    .filterByChainId(ENV.MainnetBeta)
    .getList();
  const tokenMap: Map<string, TokenInfo> = tokenList.reduce((map, item) => {
    map.set(item.address, item);
    return map;
  }, new Map<string, TokenInfo>());

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
