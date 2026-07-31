/**
 * FillLogResult is the message sent to subscribers of the FillFeed
 */
export type FillLogResult = {
  /** Public key for the market as base58. */
  market: string;
  /** Public key for the maker as base58. */
  maker: string;
  /** Public key for the taker as base58. */
  taker: string;
  /** Number of base atoms traded. */
  baseAtoms: string;
  /** Number of quote atoms traded. */
  quoteAtoms: string;
  /** Price as float. Quote atoms per base atom. Client is responsible for translating to tokens. */
  priceAtoms: number;
  /** Boolean to indicate which side the trade was. */
  takerIsBuy: boolean;
  /** Boolean to indicate whether the maker side is global. */
  isMakerGlobal: boolean;
  /** Sequential number for every order placed / matched wraps around at u64::MAX */
  makerSequenceNumber: string;
  /** Sequential number for every order placed / matched wraps around at u64::MAX */
  takerSequenceNumber: string;
  /** Slot number of the fill. */
  slot: number;
  /** Signature of the tx where the fill happened. */
  signature: string;
  /** Zero-based Manifest invocation index within the transaction. */
  invocationIndex?: number;
  /**
   * Public key of the original transaction signer as base58.
   * This represents the actual user when trades go through aggregators.
   * Optional for backwards compatibility.
   */
  originalSigner?: string;
  /**
   * Name of aggregator that sent the tx if detected.
   * Optional for backwards compatibility.
   */
  aggregator?: string;
  /**
   * Name of originating protocol that initiated the tx if detected.
   * For dual attribution (e.g., Kamino using Spur aggregator).
   * In the case of metagrregation (user on Jup gets Dflow for example), this
   * could be either.
   * Optional for backwards compatibility.
   */
  originatingProtocol?: string;
  /**
   * All transaction signers in order.
   * signers[0] is the fee payer, signers[1] is typically the owner for aggregator swaps.
   * Optional for backwards compatibility.
   */
  signers?: string[];
  /**
   * Block time of the transaction in seconds since Unix epoch.
   * Optional for backwards compatibility.
   */
  blockTime?: number;
};
