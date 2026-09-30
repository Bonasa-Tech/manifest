import { expect } from 'chai';
import {
  AccountInfo,
  Connection,
  Keypair,
  PublicKey,
  Transaction,
} from '@solana/web3.js';
import { TOKEN_PROGRAM_ID } from '@solana/spl-token';
import BN from 'bn.js';
import { ManifestClient } from '../src/client';
import {
  FIXED_MANIFEST_HEADER_SIZE,
  FIXED_WRAPPER_HEADER_SIZE,
  NIL,
} from '../src/constants';
import {
  claimedSeatBeet,
  PROGRAM_ID as MANIFEST_PROGRAM_ID,
} from '../src/manifest';
import {
  createClaimSeatInstruction,
  marketInfoBeet,
  PROGRAM_ID as WRAPPER_PROGRAM_ID,
} from '../src/wrapper';

function nodeHeader(buffer: Buffer, offset: number): void {
  for (let i = 0; i < 3; i++) buffer.writeUInt32LE(NIL, offset + 4 * i);
}

describe('client setup after defrag', () => {
  const trader = Keypair.generate();
  const market = Keypair.generate().publicKey;
  const wrapper = Keypair.generate().publicKey;
  const baseMint = Keypair.generate().publicKey;
  const quoteMint = Keypair.generate().publicKey;
  const originalFetch = globalThis.fetch;

  beforeEach(() => {
    globalThis.fetch = async () =>
      new Response(JSON.stringify({ wrapper: wrapper.toBase58() }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
  });
  afterEach(() => {
    globalThis.fetch = originalFetch;
  });

  function marketBuffer(
    index: number | null,
    seatOwner = trader.publicKey,
  ): Buffer {
    const data = Buffer.alloc(FIXED_MANIFEST_HEADER_SIZE + 240);
    data.writeBigUInt64LE(4859840929024028656n, 0);
    data[9] = data[10] = 6;
    baseMint.toBuffer().copy(data, 16);
    quoteMint.toBuffer().copy(data, 48);
    data.writeUInt32LE(240, 152);
    for (const offset of [156, 160, 164, 168, 176])
      data.writeUInt32LE(NIL, offset);
    data.writeUInt32LE(index ?? NIL, 172);
    if (index !== null) {
      const offset = FIXED_MANIFEST_HEADER_SIZE + index;
      nodeHeader(data, offset);
      data[offset + 13] = 1;
      const [seat] = claimedSeatBeet.serialize({
        trader: seatOwner,
        baseWithdrawableBalance: new BN(0),
        quoteWithdrawableBalance: new BN(0),
        quoteVolume: new BN(0),
        padding: new Array(8).fill(0),
      });
      seat.copy(data, offset + 16);
    }
    return data;
  }

  function wrapperBuffer(index: number): Buffer {
    const data = Buffer.alloc(FIXED_WRAPPER_HEADER_SIZE + 96);
    data.writeBigUInt64LE(1n, 0);
    trader.publicKey.toBuffer().copy(data, 8);
    data.writeUInt32LE(96, 40);
    data.writeUInt32LE(NIL, 44);
    data.writeUInt32LE(0, 48);
    nodeHeader(data, FIXED_WRAPPER_HEADER_SIZE);
    data[FIXED_WRAPPER_HEADER_SIZE + 13] = 1;
    const [info] = marketInfoBeet.serialize({
      market,
      ordersRootIndex: NIL,
      traderIndex: index,
      baseBalance: new BN(0),
      quoteBalance: new BN(0),
      quoteVolume: new BN(0),
      cancelAllScanCursor: 0,
      numOpenGlobalOrders: 0,
      lastSyncedOrderSequenceNumber: new BN(0),
    });
    info.copy(data, FIXED_WRAPPER_HEADER_SIZE + 16);
    return data;
  }

  function fixture(index: number | null, seatOwner = trader.publicKey) {
    let coreData = marketBuffer(index, seatOwner);
    let wrapperData = wrapperBuffer(80);
    const sent: Transaction[] = [];
    const mintData = Buffer.alloc(82);
    mintData[44] = 6;
    mintData[45] = 1;
    const account = (data: Buffer, owner: PublicKey): AccountInfo<Buffer> => ({
      data,
      owner,
      lamports: 1_000_000,
      executable: false,
      rentEpoch: 0,
    });
    const getAccountInfo = async (address: PublicKey) => {
      if (address.equals(market)) return account(coreData, MANIFEST_PROGRAM_ID);
      if (address.equals(wrapper))
        return account(wrapperData, WRAPPER_PROGRAM_ID);
      if (address.equals(baseMint) || address.equals(quoteMint))
        return account(mintData, TOKEN_PROGRAM_ID);
      return null; // No global accounts.
    };
    const connection = {
      getAccountInfo,
      getAccountInfoAndContext: async (address: PublicKey) => ({
        context: { slot: 1 },
        value: await getAccountInfo(address),
      }),
      sendTransaction: async (transaction: Transaction) => {
        sent.push(transaction);
        // Model ClaimSeat's post-state: it repairs the existing MarketInfo,
        // retaining a live seat or allocating a new one after harvesting.
        const nextIndex =
          index !== null && seatOwner.equals(trader.publicKey) ? index : 160;
        coreData = marketBuffer(nextIndex);
        wrapperData = wrapperBuffer(nextIndex);
        return 'claim-seat-signature';
      },
      confirmTransaction: async () => ({
        context: { slot: 1 },
        value: { err: null },
      }),
    } as unknown as Connection;
    return { connection, sent };
  }

  const expectedClaim = createClaimSeatInstruction({
    manifestProgram: MANIFEST_PROGRAM_ID,
    owner: trader.publicKey,
    market,
    wrapperState: wrapper,
  });

  for (const scenario of [
    { name: 'harvested seat', index: null, needsSetup: true },
    { name: 'moved seat', index: 0, needsSetup: true },
    {
      name: 'old index reused by another trader',
      index: 80,
      seatOwner: Keypair.generate().publicKey,
      needsSetup: true,
    },
    { name: 'current seat', index: 80, needsSetup: false },
  ]) {
    it(`returns the required setup for a ${scenario.name}`, async () => {
      const { connection } = fixture(scenario.index, scenario.seatOwner);
      const setup = await ManifestClient.getSetupIxs(
        connection,
        market,
        trader.publicKey,
      );
      expect(setup.setupNeeded).to.equal(scenario.needsSetup);
      expect(setup.wrapperKeypair).to.equal(null);
      expect(setup.instructions).to.deep.equal(
        scenario.needsSetup ? [expectedClaim] : [],
      );
    });

    it(`creates a client with a current seat after a ${scenario.name}`, async () => {
      const { connection, sent } = fixture(scenario.index, scenario.seatOwner);
      const client = await ManifestClient.getClientForMarket(
        connection,
        market,
        trader,
      );
      expect(sent.length).to.equal(scenario.needsSetup ? 1 : 0);
      if (scenario.needsSetup)
        expect(sent[0].instructions).to.deep.equal([expectedClaim]);
      const seat = client.market
        .claimedSeats()
        .find((seat) => seat.publicKey.equals(trader.publicKey));
      expect(seat).not.to.equal(undefined);
      expect(seat!.dataIndex).to.equal(
        scenario.index === 0 ? 0 : scenario.needsSetup ? 160 : 80,
      );
      const setup = await ManifestClient.getSetupIxs(
        connection,
        market,
        trader.publicKey,
      );
      expect(setup.setupNeeded).to.equal(false);
    });
  }

  it('requires setup before creating a client without a private key for a harvested seat', async () => {
    const { connection, sent } = fixture(null);
    try {
      await ManifestClient.getClientForMarketNoPrivateKey(
        connection,
        market,
        trader.publicKey,
      );
      expect.fail('expected setup to be required');
    } catch (error) {
      expect((error as Error).message).to.equal(
        'setup ixs need to be executed first',
      );
    }
    expect(sent).to.have.length(0);
  });
});
