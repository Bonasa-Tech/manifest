import { PublicKey, Connection } from '@solana/web3.js';
import { bignum } from '@metaplex-foundation/beet';
import { publicKey as beetPublicKey } from '@metaplex-foundation/beet-solana';
import { deserializeRedBlackTree } from './utils/redBlackTree';
import { toNum } from './utils/numbers';
import { FIXED_GLOBAL_HEADER_SIZE, NIL } from './constants';
import { getMint } from '@solana/spl-token';
import { globalDepositBeet } from './manifest/types';

export type GlobalDeposit = {
  trader: PublicKey;
  balanceAtoms: bignum;
};

export interface GlobalData {
  mint: PublicKey;
  vault: PublicKey;
  globalDeposits: GlobalDeposit[];
  numBytesAllocated: number;
  numSeatsClaimed: number;
}

export class Global {
  address: PublicKey;
  private data: GlobalData;
  private mintDecimals: number | null = null;

  private constructor({
    address,
    data,
  }: {
    address: PublicKey;
    data: GlobalData;
  }) {
    this.address = address;
    this.data = data;
  }

  /**
   * Returns a `Global` for a given address, a data buffer
   *
   * @param connection The Solana `Connection` object
   * @param address The `PublicKey` of the global account
   */
  static async loadFromAddress({
    connection,
    address,
  }: {
    connection: Connection;
    address: PublicKey;
  }): Promise<Global | null> {
    const accountInfo = await connection.getAccountInfo(address, 'confirmed');
    if (!accountInfo?.data) {
      // This is possible to fail because the global account was not initialized.
      return null;
    }
    return Global.loadFromBuffer({ address, buffer: accountInfo.data });
  }

  /**
   * Returns a `Global` for a given address, a data buffer
   *
   * @param globalAddress The `PublicKey` of the global account
   * @param buffer The buffer holding the market account data
   */
  static loadFromBuffer({
    address,
    buffer,
  }: {
    address: PublicKey;
    buffer: Buffer;
  }): Global {
    const globalData = Global.deserializeGlobalBuffer(buffer);
    return new Global({ address, data: globalData });
  }

  async reload(connection: Connection): Promise<void> {
    const accountInfo = await connection.getAccountInfo(
      this.address,
      'confirmed',
    );
    if (!accountInfo?.data) {
      throw new Error(`Failed to load ${this.address}`);
    }
    this.data = Global.deserializeGlobalBuffer(accountInfo.data);
  }

  async getMintDecimals(connection: Connection): Promise<number> {
    if (this.mintDecimals === null) {
      const mintInfo = await getMint(connection, this.data.mint);
      this.mintDecimals = mintInfo.decimals;
    }
    return this.mintDecimals;
  }

  async getGlobalBalanceTokens(
    connection: Connection,
    trader: PublicKey,
  ): Promise<number> {
    const deposit: GlobalDeposit | undefined = this.data.globalDeposits.find(
      (seat) => seat.trader.equals(trader),
    );
    if (!deposit) {
      return 0;
    }
    const decimals = await this.getMintDecimals(connection);
    return toNum(deposit.balanceAtoms) / 10 ** decimals;
  }

  getGlobalBalanceTokensWithDecimals(
    trader: PublicKey,
    decimals: number,
  ): number {
    const deposit: GlobalDeposit | undefined = this.data.globalDeposits.find(
      (seat) => seat.trader.equals(trader),
    );
    if (!deposit) {
      return 0;
    } else {
      return toNum(deposit.balanceAtoms) / 10 ** decimals;
    }
  }

  tokenMint(): PublicKey {
    return this.data.mint;
  }

  hasSeat(trader: PublicKey): boolean {
    return this.data.globalDeposits.some((seat) => seat.trader.equals(trader));
  }

  vaultAddress(): PublicKey {
    return this.data.vault;
  }

  getAllDeposits(): GlobalDeposit[] {
    return this.data.globalDeposits;
  }

  getNumSeatsClaimed(): number {
    return this.data.numSeatsClaimed;
  }

  prettyPrint(): void {
    console.log('');
    console.log(`Global: ${this.address}`);
    console.log(`========================`);
    console.log(`Mint: ${this.data.mint.toBase58()}`);
    console.log(`Vault: ${this.data.vault.toBase58()}`);
    console.log(`NumBytesAllocated: ${this.data.numBytesAllocated}`);
    console.log(`NumSeatsClaimed: ${this.data.numSeatsClaimed}`);
    console.log(`ClaimedSeats: ${this.data.globalDeposits.length}`);
    this.data.globalDeposits.forEach((seat) => {
      console.log(
        `publicKey: ${seat.trader.toBase58()} 
        balanceAtoms: ${seat.balanceAtoms.toString()} `,
      );
    });
    console.log(`========================`);
  }

  /**
   * Deserializes global data from a given mint and returns a `Global` object
   *
   * This includes both the fixed and dynamic parts of the market.
   * https://github.com/Bonasa-Tech/manifest/blob/main/programs/manifest/src/state/global.rs
   *
   * @param data The data buffer to deserialize
   */
  private static deserializeGlobalBuffer(data: Buffer): GlobalData {
    let offset = 0;
    offset += 8; // Skip discriminant

    const mint = beetPublicKey.read(data, offset);
    offset += beetPublicKey.byteSize;
    const vault = beetPublicKey.read(data, offset);
    offset += beetPublicKey.byteSize;

    const _globalSeatsRootIndex = data.readUInt32LE(offset);
    offset += 4;
    const globalAmountsRootIndex = data.readUInt32LE(offset);
    offset += 4;
    const _globalAmountsMaxIndex = data.readUInt32LE(offset);
    offset += 4;
    const _freeListHeadIndex = data.readUInt32LE(offset);
    offset += 4;

    const numBytesAllocated = data.readUInt32LE(offset);
    offset += 4;

    offset += 1; // Skip vault_bump
    offset += 1; // Skip global_bump

    const numSeatsClaimed = data.readUInt16LE(offset);
    offset += 2;

    const globalDeposits: GlobalDeposit[] =
      globalAmountsRootIndex != NIL
        ? deserializeRedBlackTree(
            data.subarray(FIXED_GLOBAL_HEADER_SIZE),
            globalAmountsRootIndex,
            globalDepositBeet,
          )
        : [];

    return {
      mint,
      vault,
      globalDeposits,
      numBytesAllocated,
      numSeatsClaimed,
    };
  }
}
