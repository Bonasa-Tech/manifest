# What is verified #

## No loss of funds ##

The market keeps two ghost aggregates on `MarketFixed`, only compiled under the
`certora` feature: `withdrawable_*_atoms`, the sum of every seat balance, and
`orderbook_*_atoms`, the sum of what every resting order has reserved. The
property is that the vault covers both, for base and for quote:

```
market_vault == withdrawable + orderbook
```

## No loss of funds for global orders ##

A global order is not backed by the market vault. The maker's tokens sit in the
global vault, a token account shared by every market that trades the mint, and
they are only pulled into the market vault at the moment the order is matched.
The global vault is therefore a second, independent source of funds, and it gets
its own aggregate, `global_deposited_atoms` on `GlobalFixed`, and its own
property:

```
global_vault == sum of all global deposits
```

A resting global order reserves nothing on the market, so it contributes zero to
`orderbook`. That is what keeps the two properties independent, and together they
say that the tokens a global trade takes out of the global vault land in the
market vault, and are credited to somebody, in the same instruction. The rules in
`certora/spec/global_checks.rs` assert both, plus the exact amounts moved, for
matching a global maker (full, partial, and unbacked), resting a global order,
cancelling one, and global deposit and withdraw.

The global account keeps two red-black trees. They are replaced with
`state/cvt_global_mock.rs`, which models a global account with two seats owned by
the same two traders the market mock knows about. Only the lookups and the tree
rebalancing are mocked; everything that touches a balance is the real code.

## Seat pubkey integrity ##

The matching code identifies the maker by the pubkey stored in the claimed
seat node, and for a global order that key decides whose global deposit pays
for the fill. The rules assume the seat node carries the key of the trader
sitting in it (`cvt_assume_seat_pubkeys`), and
`certora/spec/seat_pubkey_checks.rs` discharges that assumption by induction:
`claim_seat` writes the pubkey into the node
(`rule_claim_seat_writes_trader_pubkey`), and every other verified operation --
deposit, withdraw, matching, resting, cancel, releasing the other seat --
leaves the stored pubkeys unchanged (`rule_seat_pubkey_preserved_by_*`). Once a
seat is released its block returns to the free list and may be reused, so the
invariant is stated only while the seat is held. Swap is covered by
composition rather than a dedicated rule: in the verified model its only seat
writes go through `claim_seat` and `update_balance`, both covered above; a
dedicated rule hits a prover pointer-analysis limitation in the swap account
loader.

## Gas prepayments ##

Every global order deposits `GAS_DEPOSIT_LAMPORTS` on the global account when it
is placed, and whoever removes the order claims it. These are lamports, not
tokens, so they get their own conservation rules: the prepayment moves exactly
`GAS_DEPOSIT_LAMPORTS` per order from the gas payer to the global account
(`rule_global_gas_prepayment`), and cancelling a global order with the system
program present pays exactly one refund to the gas receiver
(`rule_cancel_global_order_gas_refund_*`). The system-program transfer is
summarized as a direct lamport move under the `certora` feature.

## Order types ##

`certora/spec/order_type_checks.rs` covers the types that are not plain limit
orders. Post only and global takers never trade: in production a crossing one
fails with `PostOnlyCrosses`; under the `certora` feature `require!` compiles to
an assume, so the property is stated as "no reachable execution records a
trade". Immediate-or-cancel and reverse takers keep the funds invariant. Reverse
and reverse-tight makers, which come back onto the other side of the book when
they are filled, keep the funds invariant on the way round — including the
coalesce path (`rule_reverse_coalesce_*`, `rule_reverse_tight_coalesce_*`),
where the come-back order is folded into an existing resting order — including
one sitting up to a single price increment away, the window `RestingOrder::eq`
tolerates — and the maker is debited the exact growth of that order's backing,
computed at the coalesce target's own price.

## Token-2022 transfer fees and hooks ##

Whether a mint carries a transfer fee or a transfer hook is mint state the
prover does not model, so in `try_to_reduce_global_tokens` the decision "treat
this global order as unbacked" is a nondeterministic choice under the `certora`
feature, placed exactly where the production checks are: after the balance
check, before the balance is reduced. This over-approximates the production
branches and covers the property those checks exist for — a transfer that will
be rejected must not eat the maker's global deposit.

## Writing rules against the mocked state ##

Two traps, both found the hard way by chasing counter-examples that turned out
to be artifacts rather than bugs:

- An assumption that **relates** two pieces of mocked state must be written
  against the **stored fields**, not by equating a whole value with a
  constructed one. The prover does not carry the latter relation back into the
  mock's memory, so the code re-reads a value unrelated to the one assumed.
  (Assumptions that merely havoc a field, by equating it with a *fresh* nondet,
  are fine — that is why the older rules never hit this.) See the coalesce price
  window in `cvt_assume_reverse_coalesce_preconditions`.
- Narrow integer fields read out of mocked memory do not keep their width: the
  prover will happily pick a `u16` spread larger than `u16::MAX`, which makes
  `base - spread` in `reverse_price` underflow. Bound them explicitly.

## Known gaps ##

- No unexpected reverts (no overflow, no panic, only deliberate `require!`
  failures) is verified for the new paths too, in `no_revert_checks.rs`:
  matching a global maker, cancelling and resting a global order, global
  deposit and withdraw, and the reverse coalesce on both sides. Like the
  pre-existing withdraw and cancel no-revert rules, these assume the trade's
  arithmetic fits in a u64 -- an order too large to price is legitimately
  rejected, which is not an unexpected revert. The reverse coalesce rules need a
  little more: the bid come-back size is a division by the reverse price and the
  grown order is a further multiply, so the rule bounds those through the
  maker order's full value (an upper bound on what the trade computes
  internally). The one path still not covered is a dedicated no-revert rule for
  swap, where it hits the same prover pointer-analysis limitation as the swap
  seat rule; swap's component operations are each covered by the deposit,
  matching and withdraw no-revert rules.

- `place_single_order` in `state/market_helpers.rs` is the model of one iteration
  of the matching loop in `Market::place_order`. It has to be kept behaviourally
  identical to the body of that loop by hand.
- The mock book holds at most one resting order per side, so multi-level
  matching is only covered one step at a time, and the mocked global account has
  two seats.
- The SPL transfer summaries move the requested amount exactly, so the
  received-amount-differs-from-requested handling for fee-bearing token-2022
  deposits (`process_deposit_core`, `process_global_deposit_core`) is verified
  in the fee-less case only.
- Global seat eviction is verified at the state level (`evict_and_take_seat`,
  `rule_global_evict`); the `global_evict` processor around it (seat fee,
  eviction fee, token transfers) is not.

# Requirements for compilation from Rust to SBF ##

1. Instal Certora CLI

```
pip install certora-cli
```

2. Solana CLI: 2.2.12

```
sh -c "$(curl -sSfL https://release.anza.xyz/v2.2.12/install)"
```

3. Install Certora version of platform-tools 1.41

   Go to https://github.com/Certora/certora-solana-platform-tools?tab=readme-ov-file#installation-of-executables and follow the instructions. 

4. Install `just` https://github.com/casey/just


# Build Solana prover from sources (only available for Certora employees) #

1. Install rustfilt to demangle Rust symbol names

```shell
cargo install rustfilt
```

2. Download https://github.com/Certora/EVMVerifier
3. Switch to branch `jorge/solana-jsm`
4. Follow installation instructions from here https://github.com/Certora/EVMVerifier?tab=readme-ov-file#installation

# Generate SBF file #

1. `cd programs/manifest`
2. `just build-sbf`

# How to run the prover #

## Configuration Parameters for Just ##

Just is controlled by environment variables. These are used to provide location for `certoraRun`, the key for the prover, etc. The easiest way to maintain them is to place them in a file called `.env` somewhere in the ancestor of the `justfile`. This can be at the root of the project, or even in the parent directory shared accross multiple projects. 

A typical `.env` file looks like this:
```
$ cat .env
CERTORA=[LOCATION OF emv.jar]
CERTORA_CLI=certoraRun
CERTORAKEY=[MYKEY]
```

Environment variables can also be used to pass extra options to various build scripts. This is usually only necessary in advanced scenarios.

## Run locally (only available for Certora employees) ##

You need to follow the steps from "Build Solana prover from sources".
Then, type:

1. `cd programs/manifest`
2. `just verify RULE_NAME EXTRA_PROVER_OPTS`

where `RULE_NAME` must be a public Rust function using `#[rule]`, and
`EXTRA_PROVER_OPTS` follows syntax of options passed to the jar
file. For instance, options such as `-bmc 3 -assumeUnwindCond ` that
tells the prover to unroll all loops up to 3 without adding the
"unwinding" assertion.

To verify all the rules locally and check that they return the expected result,
run the `verify-manifest` script located in `programs/manifest`: 

```
cd programs/manifest
./verify-manifest -r rules.json
./verify-manifest -r rules-rb-tree.json
```
Running `verify-manifest` requires `python3` `>= 3.13` 

## Run remotely ##

1. `cd programs/manifest`
2. `just verify-remote RULE_NAME EXTRA_PROVER_OPTS`

where `EXTRA_PROVER_OPTS` follows syntax of options passed to
`CertoraRun`.

After typing the above command, you should see something like this:

```
Connecting to server...
Job submitted to server
Follow your job at https://prover.certora.com
Once the job is completed, the results will be available at https://prover.certora.com/output/26873/37ce3f42dbd9419b942c693c7921652d?anonymousKey=b02ea230da2cf7b5d2681d86361744227668170d
```

If you open that above link then you will see the result of running
the Certora prover.

**VERY IMPORTANT**: both commands `just verify` and `just
verify-remote` will compile the Rust code each time before calling the
Solana prover (i.e., it calls the command `build-sbf`)


## Running locally vs remotely ##

Be aware that `just verify` calls directly the jar file while `just
verify-remote` calls the script `certoraRun`.  Therefore, the option
names can vary.  For instance,

```shell
just verify RULE_NAME -bmc 3 -assumeUnwindCond
```

and

```shell
just verify-remote RULE_NAME --loop_iter 3 --optimistic_loop
```
