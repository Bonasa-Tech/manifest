# ![Logo](assets/manifest-wide.png)

*The Unlimited Orderbook*


[![codecov](https://codecov.io/gh/CKS-Systems/manifest/graph/badge.svg?token=PJ3Y2BVMM8)](https://codecov.io/gh/CKS-Systems/manifest)
[![Code Review - Rust](https://github.com/Bonasa-Tech/manifest/actions/workflows/ci-code-review-rust.yml/badge.svg)](https://github.com/Bonasa-Tech/manifest/actions/workflows/ci-code-review-rust.yml)
[![Code Review - Typescript](https://github.com/Bonasa-Tech/manifest/actions/workflows/ci-code-review-ts.yml/badge.svg)](https://github.com/Bonasa-Tech/manifest/actions/workflows/ci-code-review-ts.yml)

Manifest is the next generation liquidity primitive on Solana.
No more permissioned markets.
No more trading fees.
No more expensive rent to start a market.
Capital efficiency built-in.
Maximal freedom to exchange risk.
Trade everything for nothing.

## Whitepaper

Read [The Orderbook Manifesto](https://manifest.trade/whitepaper.pdf)

## Comparison

|  |    Openbook    | Phoenix  |Manifest              |
|--|----------------|-------------------|----------------------|
| Crankless |No |Yes |Yes |
| Feeless |No |No |Yes|
| Atomic lot sizes |No |No |Yes|
| Anchor |Yes |No|No|
| Creation Rent|2 SOL |3+ SOL |.007 SOL|
| License|GPL |Business |GPL|
| Read optimized| Yes | No | Yes |
| Swap accounts| 16 | 8 | 8 |
| [CU](https://bonasa-tech.github.io/manifest/dev/bench/) | :white_check_mark: | :white_check_mark: | :white_check_mark: :white_check_mark: |
| Token 22                                                | No                 | No                 | Yes                                   |
| Composable wrapper                                      | No                 | No                 | Yes                                   |
| Capital Efficient                                       | No                 | No                 | Yes                                   |

### Details:

- Cranks were originally used in serum to address the need for solana programs to identify all accounts before landing on chain. This has become obsolete now that orderbooks pack all data into a predictable account.
- No trading fees forever on Manifest.
- Lot sizes restrict expressable prices. This meaningfully matters to orderflow through routers that have non-standard sizes. Manifest reduces the min trade size to atomic and increases the the range of expressable prices to cover all that are needed.
- Anchor is great for starting on Solana, but more advanced programs should not take the compute tradeoff for the convenience.
- Rent is a critical cost savings for Manifest. This enables smaller value tokens with less volume to still have orderbooks.
- Manifest aims to be freedom maximizing, so is its open source GPL-3.0 License.
- Open orders separation was a necessary feature for margin trading. Read locks to get the open orders for a trader are frequent on a margin exchange. The default wrapper implementation of Manifest allows a margin exchange to read lock an account without significant contention and land its transactions more often.
- Number of accounts for a swap is a limiter for some routers. Manifest swaps that do not use global orders achieve the theoretical minimum number of accounts.
- CU is a major cost for market makers. Benchmarking demonstrates higher percentile CU improvements, significantly lessening the cost to actively trade.
- Token 22 is the new version of token program. While it is not useful for defi and will make orderbooks less efficient, there are some notable tokens that will use it. Manifest only takes the performance hit to support token22 precisely when needed and moving token22 tokens, and only then.
- A new core vs. wrapper program architecture enables greater composability for traders and exchange interfaces. Customize feature sets and distribution for any market requirement.
- Capital efficient order type that allows market making on multiple markets while reusing capital across them.

## Design Overview

### Data Structure

The innovation that allows this next leap in onchain trading is the [`hypertree`](https://github.com/Bonasa-Tech/manifest/tree/main/lib). All data in the market account fits into graph nodes of the same size (80 bytes), which lets independent data structures grow without being fully initialized from the start by interleaving

The market account holds all relevant information. It begins with a header that stores all of the fixed information for the market like BaseMint, QuoteMint. All variable data (RestingOrders and ClaimedSeats) are in the dynamic
byte array after the header. There are 3 RedBlack trees for Bids, Asks,
ClaimedSeats and 1 LinkedList for FreeListNodes, overlapping across each other. All are graphs where each vertex along with adjacency list fits in 80 bytes, allowing them to use the same blocks.

<pre>
--------------------------------------------------------------------------------------------------------
|                   Header                    |                               Dynamic                   |
--------------------------------------------------------------------------------------------------------
| BaseMint, QuoteMint, BidsRootIndex, ...     | Bid | Ask | FreeListNode | Seat | Seat | Bid | Bid | Ask|
--------------------------------------------------------------------------------------------------------
</pre>

### Core vs Wrapper

Manifest implements the orderbook as an infrastructure layer primitive and creates the purest form of risk exchange possible. Other orderbooks get bogged down by special feature requests from trading teams that ultimately make the program bloated and confusing. Manifest strives to only include features that are absolutely necessary to be in the base layer. Anything that can be handled at layers above on the stack will not be done in manifest. This simplification makes formal verification of the program feasible.

Manifest should be interacted with though a wrapper program. Features like ClientOrderId, FillOrKill, PostOnlySlide, adjusting orders for insufficient funds, can and should be in a separate program that does a CPI into Manifest. A reference implementation and deployment of a wrapper are provided, showing what can be done outside the core of an orderbook without needing to be in the orderbook itself.

### Global Orders

Global orders are a new type of order for trading on Solana. When resting orders across many markets, cost of capital can be expensive. This is the problem that global orders look to address. A global order is an order that does not lock the tokens to support the order on the market. The same tokens that would have supported an order on one market, can now support orders across many markets, with the tokens moved just in time as there is a fill.

### Reverse Orders

Reverse orders are a special type of order available on Manifest designed to replicate the replacement mechanism inherent in Automated Market Makers (AMMs). These are limit orders that automatically switch sides when filled - buys convert to sells, and sells convert to buys using all the proceeds from the prior fill on the new order. These orders also allow configurable spreads, making them a more customizable version of AMMs. This enables users to have various fixed fees across a series of orders. By utilizing a series of reverse orders, users can fully replicate a concentrated liquidity AMM position. Orderbook liquidity providers benefit from reduced gas costs, as there’s no need to replace orders after they are filled. Therefore, reverse orders are permanent discretized liquidity directly on the orderbook. Similar to AMMs, reverse orders on Manifest make on-chain orderbook market making more accessible for everyone.

### Building

```
cargo build-sbf
```

### Open Questions
- Is tickless a good idea? This inverts time priority since it makes the most recent order able to provide negligible price improvement. This could disrupt behavior near mid and lead to unforeseen patterns.
- Is global lock contention going to be a problem? Global provides capital efficiency that will be attractive to traders, but the extra lock contention for landing transactions, not only for placing a global, but also added to anyone who might match with it, may be problematic. There is a possibility that some markets may have restrictions on global usage to protect the land rates of normal traders.

## Testing

### Program Test

```
cargo test-sbf
```

### Typescript client test

```
sh local-validator-test.sh
```
## Resources
### Client SDK
  [NPM Package](https://www.npmjs.com/package/@bonasa-tech/manifest-sdk)

### Audit
[View Report](https://www.manifest.trade/audit.pdf)

### Formal Verification
Instructions for how to run formal verification are in [Certora_README](https://github.com/Bonasa-Tech/manifest/blob/main/Certora_README.md)

From a high level, 4 sets of properties were formally verified

- Red black tree properties. Red black tree verification on the core data structure, including the additional hypertree properties.
- Loss of funds. Properties that show that all funds are accounted for and that there is no loss from any sequence of interactions with the program.
- Availability. Properties that show that valid operations, like cancelling orders or withdrawing funds will always succeed.
- Matching. Properties that go beyond loss of funds to show that not only does the exchange itself not lose funds overall, but the matching mechanism properly accounts for who the funds belong to.

Verification is rerun against head every day using the certora prover. The rules
mostly lie within the certora directory. There are some parts of the code that
are only enabled with the certora feature, but that is so the verification can
go beyond proving invariants at the end and do it on intermediate steps.

### Tip Jar
  B6dmr2UAn2wgjdm3T4N1Vjd8oPYRRTguByW7AEngkeL6

### Debugging
A fork of solana explorer with insruction decoding and fill log parsing has been made in
[this repo](https://github.com/Bonasa-Tech/explorer/tree/master) which is hosted at
[explorer.manifest.trade](https://explorer.manifest.trade/). Due to the lack of updates from anchor and hacky
changes to get the explorer to handle parsing a non-anchor program, we are not
currently going to try to backport our changes.
