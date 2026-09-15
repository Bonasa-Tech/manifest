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
 * Metaplex Token Metadata program. The metadata account is a PDA of
 * ["metadata", programId, mint].
 */
const METAPLEX_METADATA_PROGRAM_ID: PublicKey = new PublicKey(
  'metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s',
);

// Symbols are permissionless, attacker-controlled strings. Cap the length and
// drop control characters before letting one into a response.
const MAX_SYMBOL_LENGTH = 16;

function sanitizeSymbol(raw: string): string {
  // eslint-disable-next-line no-control-regex
  return raw
    .replace(/\u0000+$/, '')
    .replace(/[\u0000-\u001f\u007f]/g, '')
    .trim()
    .slice(0, MAX_SYMBOL_LENGTH);
}

/**
 * Read the symbol from the mint's on-chain Metaplex metadata account.
 *
 * Deliberately decodes the account itself rather than using the Metaplex SDK's
 * findByMint(), which dereferences the metadata's off-chain JSON `uri`. That
 * uri is attacker-controlled - anyone can mint a token pointing at any URL - so
 * fetching it would let a public stats service be aimed at arbitrary hosts.
 * The symbol is stored on-chain next to the uri, so it can be read without
 * making any request beyond the getAccountInfo we already do.
 *
 * Returns '' when there is no metadata account or the layout does not parse.
 */
async function lookupMetaplexSymbol(
  connection: Connection,
  mint: PublicKey,
): Promise<string> {
  const [metadataAddress] = PublicKey.findProgramAddressSync(
    [
      Buffer.from('metadata'),
      METAPLEX_METADATA_PROGRAM_ID.toBuffer(),
      mint.toBuffer(),
    ],
    METAPLEX_METADATA_PROGRAM_ID,
  );

  const accountInfo = await connection.getAccountInfo(metadataAddress);
  if (!accountInfo || !accountInfo.owner.equals(METAPLEX_METADATA_PROGRAM_ID)) {
    return '';
  }

  // Layout: key (1) | update_authority (32) | mint (32) | name | symbol | uri,
  // each string borsh-encoded as u32 length followed by that many bytes.
  const data: Buffer = accountInfo.data;
  let offset = 1 + 32 + 32;

  const readString = (): string | undefined => {
    if (offset + 4 > data.length) {
      return undefined;
    }
    const length: number = data.readUInt32LE(offset);
    offset += 4;
    if (length > data.length - offset) {
      return undefined;
    }
    const value: string = data
      .subarray(offset, offset + length)
      .toString('utf8');
    offset += length;
    return value;
  };

  readString(); // name, unused
  const symbol: string | undefined = readString();
  return symbol === undefined ? '' : sanitizeSymbol(symbol);
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
  // Metaplex metadata first: it is where most SPL tokens actually publish
  // their symbol, and the curated registry below has not kept up with newer
  // mints. Only the on-chain account is read - the off-chain JSON uri is never
  // fetched, so a public stats service still cannot be pointed at arbitrary
  // hosts by whoever minted the token.
  try {
    const metaplexSymbol: string = await lookupMetaplexSymbol(connection, mint);
    if (metaplexSymbol) {
      return metaplexSymbol;
    }
  } catch (error) {
    console.log('Metaplex metadata lookup failed for', mint.toBase58(), error);
  }

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
