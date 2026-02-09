import { Connection, PublicKey } from "@solana/web3.js";

const MFX_STATS_URL = "https://mfx-stats-mainnet.fly.dev/tickers";
const TOKEN_2022_PROGRAM_ID = new PublicKey("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

// Extension type discriminators (first byte of each extension in the TLV data)
enum ExtensionType {
  TransferFeeConfig = 1,
  TransferFeeAmount = 2,
  MintCloseAuthority = 3,
  ConfidentialTransferMint = 4,
  ConfidentialTransferAccount = 5,
  DefaultAccountState = 6,
  ImmutableOwner = 7,
  MemoTransfer = 8,
  NonTransferable = 9,
  InterestBearingConfig = 10,
  CpiGuard = 11,
  PermanentDelegate = 12,
  NonTransferableAccount = 13,
  TransferHook = 14,
  TransferHookAccount = 15,
  MetadataPointer = 18,
  TokenMetadata = 19,
  GroupPointer = 20,
  GroupMemberPointer = 22,
  ConfidentialTransferFee = 24,
  ScaledUiAmountConfig = 29,
}

interface Ticker {
  ticker_id: string;
  base_currency: string;
  target_currency: string;
  pool_id: string;
}

interface ExtensionInfo {
  mint: string;
  isToken2022: boolean;
  hasTransferFee: boolean;
  hasNonZeroTransferFee: boolean;
  hasMutableTransferFee: boolean;
  hasPermanentDelegate: boolean;
  hasTransferHook: boolean;
  hasCpiGuard: boolean;
  hasMintCloseAuthority: boolean;
  hasNonTransferable: boolean;
  hasInterestBearing: boolean;
  hasConfidentialTransfer: boolean;
  hasDefaultAccountState: boolean;
  hasMetadataPointer: boolean;
  hasTokenMetadata: boolean;
  hasGroupPointer: boolean;
  hasGroupMemberPointer: boolean;
  hasScaledUiAmount: boolean;
  transferFeeBps: number | null;
  maxTransferFeeBps: number | null;
  transferFeeAuthority: string | null;
  permanentDelegate: string | null;
  transferHookProgramId: string | null;
}

function parseExtensionsFromAccountData(data: Buffer): {
  extensions: Set<number>;
  transferFeeBps: number | null;
  maxFee: bigint | null;
  transferFeeAuthority: string | null;
  permanentDelegate: string | null;
  transferHookProgramId: string | null;
} {
  const extensions = new Set<number>();
  let transferFeeBps: number | null = null;
  let maxFee: bigint | null = null;
  let transferFeeAuthority: string | null = null;
  let permanentDelegate: string | null = null;
  let transferHookProgramId: string | null = null;

  // Token-2022 mint layout:
  //   Bytes 0-81:   Base Mint data (82 bytes)
  //   Bytes 82-164: Padding (83 zero bytes to match Account struct size)
  //   Byte 165:     AccountType (1 = Mint)
  //   Bytes 166+:   TLV extension entries
  //
  // Each TLV entry: 2 bytes type (u16 LE) + 2 bytes length (u16 LE) + N bytes data

  if (data.length <= 166) {
    return { extensions, transferFeeBps, maxFee, transferFeeAuthority, permanentDelegate, transferHookProgramId };
  }

  let offset = 166; // Skip base mint (82) + padding (83) + account type (1)

  while (offset + 4 <= data.length) {
    const extType = data.readUInt16LE(offset);
    const extLen = data.readUInt16LE(offset + 2);
    offset += 4;

    if (extType === 0 && extLen === 0) break; // End sentinel
    if (offset + extLen > data.length) break; // Malformed

    extensions.add(extType);

    // Parse TransferFeeConfig details
    if (extType === ExtensionType.TransferFeeConfig && extLen >= 108) {
      // TransferFeeConfig layout (108 bytes):
      //   transfer_fee_config_authority: OptionalNonZeroPubkey (32 bytes)
      //   withdraw_withheld_authority:   OptionalNonZeroPubkey (32 bytes)
      //   withheld_amount:               u64 (8 bytes)
      //   older_transfer_fee:            TransferFee (18 bytes)
      //     epoch: u64 (8), maximum_fee: u64 (8), transfer_fee_basis_points: u16 (2)
      //   newer_transfer_fee:            TransferFee (18 bytes)

      const extData = data.subarray(offset, offset + extLen);

      // transfer_fee_config_authority at offset 0 (32 bytes, zeros = None)
      const authorityPk = new PublicKey(extData.subarray(0, 32));
      if (!authorityPk.equals(PublicKey.default)) {
        transferFeeAuthority = authorityPk.toBase58();
      }

      // newer_transfer_fee starts at offset 90 (32+32+8+18 = 90)
      const newerOffset = 90;
      if (extData.length >= newerOffset + 18) {
        // epoch at +0, maximum_fee at +8, basis_points at +16
        maxFee = extData.readBigUInt64LE(newerOffset + 8);
        transferFeeBps = extData.readUInt16LE(newerOffset + 16);
      } else {
        // Fall back to older at offset 72
        const olderOffset = 72;
        if (extData.length >= olderOffset + 18) {
          maxFee = extData.readBigUInt64LE(olderOffset + 8);
          transferFeeBps = extData.readUInt16LE(olderOffset + 16);
        }
      }
    }

    // Parse PermanentDelegate
    if (extType === ExtensionType.PermanentDelegate && extLen >= 32) {
      const delegate = new PublicKey(data.subarray(offset, offset + 32));
      if (!delegate.equals(PublicKey.default)) {
        permanentDelegate = delegate.toBase58();
      }
    }

    // Parse TransferHook
    if (extType === ExtensionType.TransferHook && extLen >= 64) {
      // TransferHook layout (64 bytes):
      //   authority:   OptionalNonZeroPubkey (32 bytes)
      //   program_id:  Pubkey (32 bytes)
      const hookProgramId = new PublicKey(data.subarray(offset + 32, offset + 64));
      if (!hookProgramId.equals(PublicKey.default)) {
        transferHookProgramId = hookProgramId.toBase58();
      }
    }

    offset += extLen;
  }

  return { extensions, transferFeeBps, maxFee, transferFeeAuthority, permanentDelegate, transferHookProgramId };
}

async function checkMint(connection: Connection, mintStr: string): Promise<ExtensionInfo> {
  const result: ExtensionInfo = {
    mint: mintStr,
    isToken2022: false,
    hasTransferFee: false,
    hasNonZeroTransferFee: false,
    hasMutableTransferFee: false,
    hasPermanentDelegate: false,
    hasTransferHook: false,
    hasCpiGuard: false,
    hasMintCloseAuthority: false,
    hasNonTransferable: false,
    hasInterestBearing: false,
    hasConfidentialTransfer: false,
    hasDefaultAccountState: false,
    hasMetadataPointer: false,
    hasTokenMetadata: false,
    hasGroupPointer: false,
    hasGroupMemberPointer: false,
    hasScaledUiAmount: false,
    transferFeeBps: null,
    maxTransferFeeBps: null,
    transferFeeAuthority: null,
    permanentDelegate: null,
    transferHookProgramId: null,
  };

  try {
    const pubkey = new PublicKey(mintStr);
    const accountInfo = await connection.getAccountInfo(pubkey);
    if (!accountInfo) return result;

    if (!accountInfo.owner.equals(TOKEN_2022_PROGRAM_ID)) {
      return result; // Not Token-2022
    }

    result.isToken2022 = true;
    const data = accountInfo.data;
    const { extensions, transferFeeBps, maxFee, transferFeeAuthority, permanentDelegate, transferHookProgramId } =
      parseExtensionsFromAccountData(Buffer.from(data));

    result.hasTransferFee = extensions.has(ExtensionType.TransferFeeConfig);
    result.hasMintCloseAuthority = extensions.has(ExtensionType.MintCloseAuthority);
    result.hasNonTransferable = extensions.has(ExtensionType.NonTransferable);
    result.hasCpiGuard = extensions.has(ExtensionType.CpiGuard);
    result.hasPermanentDelegate = extensions.has(ExtensionType.PermanentDelegate);
    result.hasTransferHook = extensions.has(ExtensionType.TransferHook);
    result.hasInterestBearing = extensions.has(ExtensionType.InterestBearingConfig);
    result.hasConfidentialTransfer = extensions.has(ExtensionType.ConfidentialTransferMint);
    result.hasDefaultAccountState = extensions.has(ExtensionType.DefaultAccountState);
    result.hasMetadataPointer = extensions.has(ExtensionType.MetadataPointer);
    result.hasTokenMetadata = extensions.has(ExtensionType.TokenMetadata);
    result.hasGroupPointer = extensions.has(ExtensionType.GroupPointer);
    result.hasGroupMemberPointer = extensions.has(ExtensionType.GroupMemberPointer);
    result.hasScaledUiAmount = extensions.has(ExtensionType.ScaledUiAmountConfig);

    if (result.hasTransferFee) {
      result.transferFeeBps = transferFeeBps;
      result.hasNonZeroTransferFee = (transferFeeBps ?? 0) > 0;
      result.transferFeeAuthority = transferFeeAuthority;
      result.hasMutableTransferFee = transferFeeAuthority !== null;
      if (maxFee !== null) {
        result.maxTransferFeeBps = Number(maxFee); // This is actually max fee amount, not bps
      }
    }

    if (result.hasPermanentDelegate) {
      result.permanentDelegate = permanentDelegate;
    }

    if (result.hasTransferHook) {
      result.transferHookProgramId = transferHookProgramId;
    }
  } catch (e: any) {
    console.error(`Error checking mint ${mintStr}: ${e.message}`);
  }

  return result;
}

async function main() {
  const rpcUrl = process.argv[2];
  if (!rpcUrl) {
    console.error("Usage: npx ts-node check-extensions.ts <RPC_URL>");
    console.error("  e.g. npx ts-node check-extensions.ts https://api.mainnet-beta.solana.com");
    process.exit(1);
  }

  console.log("Fetching tickers from mfx-stats...");
  const resp = await fetch(MFX_STATS_URL);
  const tickers: Ticker[] = await resp.json();

  // Extract unique mints
  const mints = new Set<string>();
  for (const t of tickers) {
    mints.add(t.base_currency);
    mints.add(t.target_currency);
  }
  console.log(`Found ${mints.size} unique mints across ${tickers.length} tickers`);

  const connection = new Connection(rpcUrl, { commitment: "confirmed" });

  // Batch using getMultipleAccountsInfo for efficiency
  const mintList = [...mints];
  const BATCH_SIZE = 100;
  const accountMap = new Map<string, any>();

  console.log(`Fetching account data in batches of ${BATCH_SIZE}...`);
  for (let i = 0; i < mintList.length; i += BATCH_SIZE) {
    const batch = mintList.slice(i, i + BATCH_SIZE);
    const pubkeys = batch.map((m) => new PublicKey(m));
    const accounts = await connection.getMultipleAccountsInfo(pubkeys);
    for (let j = 0; j < batch.length; j++) {
      accountMap.set(batch[j], accounts[j]);
    }
    const progress = Math.min(i + BATCH_SIZE, mintList.length);
    process.stderr.write(`\r  ${progress}/${mintList.length} mints fetched`);
  }
  process.stderr.write("\n");

  // Analyze each mint
  const results: ExtensionInfo[] = [];
  for (const mint of mintList) {
    const accountInfo = accountMap.get(mint);
    const info: ExtensionInfo = {
      mint,
      isToken2022: false,
      hasTransferFee: false,
      hasNonZeroTransferFee: false,
      hasMutableTransferFee: false,
      hasPermanentDelegate: false,
      hasTransferHook: false,
      hasCpiGuard: false,
      hasMintCloseAuthority: false,
      hasNonTransferable: false,
      hasInterestBearing: false,
      hasConfidentialTransfer: false,
      hasDefaultAccountState: false,
      hasMetadataPointer: false,
      hasTokenMetadata: false,
      hasGroupPointer: false,
      hasGroupMemberPointer: false,
      hasScaledUiAmount: false,
      transferFeeBps: null,
      maxTransferFeeBps: null,
      transferFeeAuthority: null,
      permanentDelegate: null,
      transferHookProgramId: null,
    };

    if (!accountInfo) {
      results.push(info);
      continue;
    }

    if (accountInfo.owner.equals(TOKEN_2022_PROGRAM_ID)) {
      info.isToken2022 = true;
      const data = Buffer.from(accountInfo.data);
      const parsed = parseExtensionsFromAccountData(data);

      info.hasTransferFee = parsed.extensions.has(ExtensionType.TransferFeeConfig);
      info.hasMintCloseAuthority = parsed.extensions.has(ExtensionType.MintCloseAuthority);
      info.hasNonTransferable = parsed.extensions.has(ExtensionType.NonTransferable);
      info.hasCpiGuard = parsed.extensions.has(ExtensionType.CpiGuard);
      info.hasPermanentDelegate = parsed.extensions.has(ExtensionType.PermanentDelegate);
      info.hasTransferHook = parsed.extensions.has(ExtensionType.TransferHook);
      info.hasInterestBearing = parsed.extensions.has(ExtensionType.InterestBearingConfig);
      info.hasConfidentialTransfer = parsed.extensions.has(ExtensionType.ConfidentialTransferMint);
      info.hasDefaultAccountState = parsed.extensions.has(ExtensionType.DefaultAccountState);
      info.hasMetadataPointer = parsed.extensions.has(ExtensionType.MetadataPointer);
      info.hasTokenMetadata = parsed.extensions.has(ExtensionType.TokenMetadata);
      info.hasGroupPointer = parsed.extensions.has(ExtensionType.GroupPointer);
      info.hasGroupMemberPointer = parsed.extensions.has(ExtensionType.GroupMemberPointer);
      info.hasScaledUiAmount = parsed.extensions.has(ExtensionType.ScaledUiAmountConfig);

      if (info.hasTransferFee) {
        info.transferFeeBps = parsed.transferFeeBps;
        info.hasNonZeroTransferFee = (parsed.transferFeeBps ?? 0) > 0;
        info.transferFeeAuthority = parsed.transferFeeAuthority;
        info.hasMutableTransferFee = parsed.transferFeeAuthority !== null;
        if (parsed.maxFee !== null) {
          info.maxTransferFeeBps = Number(parsed.maxFee);
        }
      }
      if (info.hasPermanentDelegate) {
        info.permanentDelegate = parsed.permanentDelegate;
      }
      if (info.hasTransferHook) {
        info.transferHookProgramId = parsed.transferHookProgramId;
      }
    }

    results.push(info);
  }

  // Output results
  const token2022Mints = results.filter((r) => r.isToken2022);
  const splTokenMints = results.filter((r) => !r.isToken2022);

  console.log(`\n${"=".repeat(120)}`);
  console.log(`RESULTS: ${token2022Mints.length} Token-2022 mints, ${splTokenMints.length} SPL Token mints`);
  console.log(`${"=".repeat(120)}\n`);

  // Print Token-2022 mints table
  if (token2022Mints.length > 0) {
    console.log("TOKEN-2022 MINTS:");
    console.log("-".repeat(160));
    const header = [
      "Mint".padEnd(46),
      "XferFee",
      "NonZero",
      "Mutable",
      "PermDlg",
      "XferHook",
      "CpiGrd",
      "CloseAuth",
      "NonXfer",
      "Fee(bps)",
      "Details",
    ].join(" | ");
    console.log(header);
    console.log("-".repeat(160));

    for (const r of token2022Mints) {
      const yn = (b: boolean) => (b ? "  YES " : "  no  ");
      const details: string[] = [];
      if (r.transferFeeBps !== null) details.push(`fee=${r.transferFeeBps}bps`);
      if (r.transferFeeAuthority) details.push(`feeAuth=${r.transferFeeAuthority.slice(0, 8)}..`);
      if (r.permanentDelegate) details.push(`delegate=${r.permanentDelegate.slice(0, 8)}..`);
      if (r.transferHookProgramId) details.push(`hook=${r.transferHookProgramId.slice(0, 8)}..`);

      const row = [
        r.mint.padEnd(46),
        yn(r.hasTransferFee),
        yn(r.hasNonZeroTransferFee),
        yn(r.hasMutableTransferFee),
        yn(r.hasPermanentDelegate),
        yn(r.hasTransferHook),
        yn(r.hasCpiGuard),
        yn(r.hasMintCloseAuthority),
        yn(r.hasNonTransferable),
        r.transferFeeBps !== null ? String(r.transferFeeBps).padStart(8) : "     n/a",
        details.join(", "),
      ].join(" | ");
      console.log(row);
    }
    console.log("-".repeat(160));
  }

  // Summary of concerning extensions
  console.log("\n--- SUMMARY OF CONCERNING EXTENSIONS ---");
  const withFees = token2022Mints.filter((r) => r.hasNonZeroTransferFee);
  const withHooks = token2022Mints.filter((r) => r.hasTransferHook && r.transferHookProgramId);
  const withDelegate = token2022Mints.filter((r) => r.hasPermanentDelegate && r.permanentDelegate);
  const withNonTransferable = token2022Mints.filter((r) => r.hasNonTransferable);

  console.log(`\nMints with non-zero transfer fees (${withFees.length}):`);
  for (const r of withFees) {
    console.log(`  ${r.mint} - ${r.transferFeeBps} bps, mutable=${r.hasMutableTransferFee}`);
  }

  console.log(`\nMints with active transfer hooks (${withHooks.length}):`);
  for (const r of withHooks) {
    console.log(`  ${r.mint} - hook program: ${r.transferHookProgramId}`);
  }

  console.log(`\nMints with permanent delegate (${withDelegate.length}):`);
  for (const r of withDelegate) {
    console.log(`  ${r.mint} - delegate: ${r.permanentDelegate}`);
  }

  console.log(`\nNon-transferable mints (${withNonTransferable.length}):`);
  for (const r of withNonTransferable) console.log(`  ${r.mint}`);

  // Also output JSON for further processing
  const jsonOut = JSON.stringify(results, null, 2);
  const fs = await import("fs");
  fs.writeFileSync("extension-results.json", jsonOut);
  console.log(`\nFull results written to extension-results.json`);
}

main().catch(console.error);