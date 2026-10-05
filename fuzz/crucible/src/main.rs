//! Stateful invariant fuzz harness for the manifest program.
//!
//! Replays randomized instruction sequences against the real program inside an in-process
//! LiteSVM and checks protocol invariants after every step. The program is loaded as a built
//! artifact (`programs/manifest.so`) rather than linked in, so the harness never participates in
//! the program's build and cannot perturb it.
//!
//! Four facts shape everything here, each established against the program source:
//!
//! * **Instruction data is borsh.** Every processor decodes its params with
//!   `BorshDeserialize::try_from_slice`. `idls/manifest.json` keeps the shank 1-byte
//!   `discriminant`, which holds crucible's codegen on its borsh/`AnchorSerialize` path -- a
//!   4-byte discriminator would switch it to bincode, whose `Vec` length prefix is a u64 where
//!   borsh writes a u32. So the generated `instruction::*` structs encode exactly what the
//!   program decodes, and this harness uses them for the data while passing account lists
//!   explicitly, which it must because of the next point.
//! * **Account lists are content-sniffed, not length-checked.** `SwapContext::load` chooses
//!   between the Swap and SwapV2 shapes by testing whether account 1 is owned by the manifest
//!   program, and the trailing `base_mint` / `token_program_quote` / `quote_mint` / `global` /
//!   `global_vault` slots are recognised by their owner or pubkey rather than by position
//!   (`programs/manifest/src/validation/loaders.rs`). A valid call can pass 3, 8 or 13 accounts
//!   to `BatchUpdate` and 13 or 14 to Swap, so actions build `Vec<AccountMeta>` by hand.
//! * **The orderbook and the seat list are a red-black tree inside the account bytes.**
//!   `hypertree` (the `lib/` crate) is compiled into the program, so its structural properties
//!   are protocol invariants, and the harness walks them from raw bytes. It deliberately does
//!   NOT reuse the program's own `validate_red_black_tree`: that function compares the right
//!   spine strictly while equal keys are inserted left and then rotated onto right links, so it
//!   reports a violation on any book holding three orders at one price. `verify_rb_tree` is no
//!   use either -- it seeds from `get_max_index()` with no fallback, and the seat and
//!   global-trader trees are always opened with `max = NIL`, which makes it vacuous on both.
//! * **The staged program is the default-feature build.** Confirmed from the build fingerprint:
//!   `features: ["default"]`. That matters three times over -- `MAX_GLOBAL_SEATS` is 999 rather
//!   than the 4 the `test` feature sets, market expansion uses `resize()` rather than the
//!   `allocate` CPI the `fuzz` feature substitutes, and the Clock sysvar is live so order expiry
//!   is real. Building the program with any of those features would fuzz a different binary.

#![allow(clippy::too_many_arguments)]

use crucible_fuzzer::anchor_lang::InstructionData;
use crucible_fuzzer::*;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_program_option::COption;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use spl_token_2022_interface::extension::transfer_fee::{TransferFeeAmount, TransferFeeConfig};
use spl_token_2022_interface::extension::{BaseStateWithExtensionsMut, ExtensionType, StateWithExtensionsMut};
use spl_token_2022_interface::state::{Account as T22Account, AccountState as T22AccountState, Mint as T22Mint};
use std::rc::Rc;

// Generated from the IDL at build time. idls/manifest.json is derived from the program's own
// source by ./gen-idl.py, and CI regenerates it and fails on a mismatch, so the bindings the
// harness compiles against cannot drift from the program it fuzzes.
crucible_idl_gen::declare_fuzz_program!("idls/manifest.json");

// SCOUT:CHECK-CONTRACT:BEGIN
// Semantic invariant checks have two modes:
//   default / SCOUT_CHECK_MODE=enforce: record a real Crucible fuzz violation;
//   SCOUT_CHECK_MODE=observe: emit nonce-bound reachability markers, never a violation.
// This exact alias is part of the trusted contract.  Generated setup and the
// macros below use `crate::`/`$crate` paths so a mutable prelude cannot replace
// Crucible's TestContext or violation/session functions with local lookalikes.
#[doc(hidden)]
extern crate crucible_test_context as __scout_crucible_test_context;

fn __scout_check_observe_mode() -> bool {
    std::env::var("SCOUT_CHECK_MODE").as_deref() == Ok("observe")
}

// Mute a property whose finding is already investigated and written up. Such a property keeps
// firing on the SAME known defect and floods the objective, hiding every other property's first
// finding behind thousands of duplicates -- observed at ~160 crashes per 25s on one target.
//
// Muting is ALWAYS announced on stderr, once per process. A silently disabled check is the exact
// false-negative trap this pipeline exists to avoid: a muted property is indistinguishable from a
// passing one unless the run says so out loud. `SCOUT_CHECK_MUTE` is also stripped from ordinary
// fuzz subprocesses alongside the other audit switches, so a stray shell variable can never
// quietly disable a check -- a caller must pass it explicitly.
fn __scout_check_announce_mutes(list: &str) {
    static MUTE_ONCE: std::sync::Once = std::sync::Once::new();
    MUTE_ONCE.call_once(|| {
        eprintln!("[SCOUT_CHECK_MUTED] {}", list);
    });
}

fn __scout_check_muted(property: &str) -> bool {
    match std::env::var("SCOUT_CHECK_MUTE") {
        Ok(list) => {
            let muted = list.split(',').any(|entry| entry.trim() == property);
            if muted {
                __scout_check_announce_mutes(&list);
            }
            muted
        }
        Err(_) => false,
    }
}

fn __scout_check_selected(property: &str) -> bool {
    if __scout_check_muted(property) {
        return false;
    }
    match std::env::var("SCOUT_CHECK_ONLY") {
        Ok(selected) => selected == property,
        Err(_) => true,
    }
}

fn __scout_check_nonce() -> Result<String, &'static str> {
    let nonce = std::env::var("SCOUT_CHECK_RUN").map_err(|_| "missing or non-Unicode SCOUT_CHECK_RUN")?;
    if nonce.is_empty() {
        return Err("empty SCOUT_CHECK_RUN");
    }
    if !nonce.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-')) {
        return Err("SCOUT_CHECK_RUN contains unsafe characters");
    }
    Ok(nonce)
}

fn __scout_check_emit_error(reason: &str) {
    static ERROR_ONCE: std::sync::Once = std::sync::Once::new();
    ERROR_ONCE.call_once(|| {
        // Never echo an invalid value: whitespace/newlines would forge protocol fields.
        eprintln!("[SCOUT_CHECK_ERROR] INVALID {}", reason);
    });
}

macro_rules! scout_check_session {
    () => {{
        if $crate::__scout_check_observe_mode() {
            // Coverage-only replay runs before Crucible's stateful initializer.  Set
            // this per-thread flag here so failed actions terminate accumulated chains
            // exactly as they did in the stateful campaign that produced the corpus.
            $crate::__scout_crucible_test_context::set_stateful_chain_mode(true);
            static SESSION_ONCE: std::sync::Once = std::sync::Once::new();
            SESSION_ONCE.call_once(|| match $crate::__scout_check_nonce() {
                Ok(nonce) => eprintln!("[SCOUT_CHECK_SESSION] {}", nonce),
                Err(reason) => $crate::__scout_check_emit_error(reason),
            });
        }
    }};
}

// Gate the *entire* property computation, not only its final predicate.  This
// prevents another property's fallible reads, eligibility logic, or shadow-hook
// arithmetic from panicking/starving an isolated SCOUT_CHECK_ONLY replay.
macro_rules! scout_run_property {
    ($property:literal, $expression:expr $(,)?) => {{
        if $crate::__scout_check_selected($property) {
            let _ = $expression;
        }
    }};
}

#[doc(hidden)]
#[macro_export]
macro_rules! __scout_check_impl {
    ($property:literal, $site:literal, $predicate:expr, $message:expr) => {{
        let __scout_observe = $crate::__scout_check_observe_mode();
        if !$crate::__scout_check_selected($property) {
            true
        } else {
            let __scout_nonce = if __scout_observe { Some($crate::__scout_check_nonce()) } else { None };
            if let Some(Err(ref __scout_error)) = __scout_nonce {
                // An invalid session can never produce an EVALUATED marker.  The
                // mechanical verifier therefore cannot mistake it for sound evidence.
                $crate::__scout_check_emit_error(__scout_error);
                false
            } else {
                // Keep the predicate in one lexical/runtime position.  Expressions
                // with reads or counters are evaluated exactly once per selected check.
                let __scout_check_result: bool = $predicate;
                if let Some(Ok(ref __scout_run)) = __scout_nonce {
                    eprintln!(
                        "[SCOUT_CHECK_EVALUATED] {} {} {} {}:{}",
                        __scout_run,
                        $property,
                        $site,
                        file!(),
                        line!()
                    );
                    if !__scout_check_result {
                        eprintln!(
                            "[SCOUT_CHECK_WOULD_VIOLATE] {} {} {} {}:{}",
                            __scout_run,
                            $property,
                            $site,
                            file!(),
                            line!()
                        );
                    }
                } else if !__scout_check_result {
                    $crate::__scout_crucible_test_context::record_violation($message);
                }
                __scout_check_result
            }
        }
    }};
}

macro_rules! scout_check {
    ($property:literal, $site:literal, $predicate:expr $(,)?) => {{
        $crate::__scout_check_impl!(
            $property,
            $site,
            $predicate,
            format!(
                "Invariant {} check {} failed at {}:{}",
                $property, $site, file!(), line!()
            )
        )
    }};
    ($property:literal, $site:literal, $predicate:expr, $($arg:tt)+) => {{
        $crate::__scout_check_impl!($property, $site, $predicate, format!($($arg)+))
    }};
}
// SCOUT:CHECK-CONTRACT:END

// ---------------------------------------------------------------------------------------------
// Program constants, mirrored from the source. Each is re-asserted against the live program in
// the test module below, so a change on the program side fails a test here instead of silently
// decoding garbage.
// ---------------------------------------------------------------------------------------------

/// `state/constants.rs` MARKET_FIXED_SIZE. `process_create_market` asserts the market account is
/// exactly this size, so it is mandatory rather than advisory.
const MARKET_FIXED_SIZE: usize = 256;
/// `state/constants.rs` MARKET_BLOCK_SIZE: one node, 16-byte header plus 64-byte payload.
const MARKET_BLOCK_SIZE: usize = 80;
/// `state/constants.rs` GLOBAL_FIXED_SIZE.
const GLOBAL_FIXED_SIZE: usize = 96;
/// `state/constants.rs` GLOBAL_BLOCK_SIZE: 16-byte header plus 48-byte payload.
const GLOBAL_BLOCK_SIZE: usize = 64;
/// `state/constants.rs` MARKET_FIXED_DISCRIMINANT.
const MARKET_FIXED_DISCRIMINANT: u64 = 4859840929024028656;
/// `state/constants.rs` GLOBAL_FIXED_DISCRIMINANT.
const GLOBAL_FIXED_DISCRIMINANT: u64 = 10787423733276977665;
/// `lib/src/hypertree.rs` NIL, for a non-certora build -- which the staged program is.
const NIL: u32 = u32::MAX;
/// `lib/src/red_black_tree.rs` RBTREE_OVERHEAD_BYTES.
const RB_HEADER: usize = 16;

/// The program under test, resolved relative to the harness working directory. Never absolute,
/// and never via `env!("CARGO_MANIFEST_DIR")`: the FuzzCorp worker launches the binary from the
/// bundle directory that holds `programs/` (`harness_run_dir_in_bundle`), and `cargo run` /
/// `cargo test` set the CWD to the crate root, so one relative literal serves both.
const TARGET_PROGRAM_ARTIFACT: &str = "programs/manifest.so";

// TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA and TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb.
// Spelled as bytes rather than taken from a dependency so the harness never has to agree with
// spl-token about its crate version. Both programs are already loaded by `TestContext::new()`;
// nothing here may `add_program` them, because `FUZZ_PROGRAM_SO` replaces the artifact of EVERY
// `add_program` call with no program-id filter, and under coverage mode that would silently swap
// the token programs for the manifest binary.
const SPL_TOKEN_ID: Pubkey = Pubkey::new_from_array([
    6, 221, 246, 225, 215, 101, 161, 147, 217, 203, 225, 70, 206, 235, 121, 172, 28, 180, 133, 237, 95, 91, 55, 145,
    58, 140, 245, 133, 126, 255, 0, 169,
]);
const SPL_TOKEN_2022_ID: Pubkey = Pubkey::new_from_array([
    6, 221, 246, 225, 238, 117, 143, 222, 24, 66, 93, 188, 228, 108, 205, 218, 182, 26, 252, 77, 131, 185, 13, 39, 254,
    189, 249, 40, 216, 161, 139, 252,
]);
const SYSTEM_PROGRAM_ID: Pubkey = Pubkey::new_from_array([0u8; 32]);

/// MNFSTqtC93rEfYHB6hF82sKdZpUDFWkViLByLd1k1Ms, asserted equal to the generated `manifest::ID`.
const MANIFEST_ID: Pubkey = Pubkey::new_from_array([
    5, 55, 164, 119, 56, 218, 37, 169, 136, 249, 100, 217, 155, 164, 58, 136, 207, 17, 135, 57, 91, 106, 207, 20, 131,
    245, 112, 34, 141, 128, 2, 178,
]);

/// `Rent::default().minimum_balance(256)`. The market account is created by the client with the
/// system program before `CreateMarket` initializes it.
const MARKET_RENT: u64 = 2_672_640;
/// Starting lamports per actor. Generous: `GlobalAddTrader` charges a seat fee of roughly 4.08M
/// lamports and every market expansion transfers the rent delta from the payer.
const ACTOR_LAMPORTS: u64 = 500_000_000_000;
const BASE_DECIMALS: u8 = 9;
const QUOTE_DECIMALS: u8 = 6;
/// Wallet balance minted to every actor on every leg, so a deposit or swap is bounded by
/// protocol logic rather than by a thin wallet.
const WALLET_ATOMS: u64 = 1_000_000_000_000_000;

/// Transfer fee on the Token-2022 quote leg, in basis points. 10% is deliberately large: it is
/// the fixture's only source of a transfer where the amount that ARRIVES differs from the amount
/// requested, and that divergence is the whole reason `processor/deposit.rs` credits the observed
/// vault delta rather than the requested amount -- and therefore the reason P-0001 is stated as
/// `>=` rather than the equality the program's own comment states, and the reason P-0002 excludes
/// resting bids. Without a fee-bearing mint every one of those paths is entered and none of them
/// diverges, so the design rationale for those properties would be untestable.
///
/// In force from the end of setup, not from the mint's creation: GlobalCreate refuses a mint whose
/// fee is live, so setup creates the leg's global first and raises the fee afterwards.
const T22_FEE_BPS: u16 = 1_000;

/// Actors. Index 2 is the adversary: the value-conservation property is stated about it, so it
/// must be a distinct signer with its own token accounts. Sharing a keypair or a wallet between
/// "different" actors is how a fixture manufactures findings.
const N_ACTORS: usize = 3;
const ADVERSARY: usize = 2;
/// Legs, indexing `wallets[actor]`.
const BASE: usize = 0;
const QUOTE: usize = 1;

// ---------------------------------------------------------------------------------------------
// Byte offsets. Taken from the `#[repr(C)]` declarations; each struct's fields sum exactly to its
// asserted size, which is what pins them:
//   MarketFixed 256 (state/market.rs)      GlobalFixed  96 (state/global.rs)
//   ClaimedSeat  64 (state/claimed_seat.rs) RestingOrder 64 (state/resting_order.rs)
//   GlobalTrader 48, GlobalDeposit 48 (state/global.rs)
// ---------------------------------------------------------------------------------------------

mod market {
    pub const DISCRIMINANT: usize = 0; // u64
    pub const BASE_MINT: usize = 16; // Pubkey
    pub const QUOTE_MINT: usize = 48; // Pubkey
    pub const ORDER_SEQUENCE_NUMBER: usize = 144; // u64
    pub const NUM_BYTES_ALLOCATED: usize = 152; // u32
    pub const BIDS_ROOT_INDEX: usize = 156; // u32
    pub const BIDS_BEST_INDEX: usize = 160; // u32, the cached top of the bid book
    pub const ASKS_ROOT_INDEX: usize = 164; // u32
    pub const ASKS_BEST_INDEX: usize = 168; // u32, the cached top of the ask book
    pub const CLAIMED_SEATS_ROOT_INDEX: usize = 172; // u32
    pub const FREE_LIST_HEAD_INDEX: usize = 176; // u32
}

mod seat {
    pub const TRADER: usize = 0; // Pubkey
    pub const BASE_WITHDRAWABLE: usize = 32; // u64
    pub const QUOTE_WITHDRAWABLE: usize = 40; // u64
}

mod order {
    pub const PRICE: usize = 0; // u128 as a little-endian pair of u64
    pub const NUM_BASE_ATOMS: usize = 16; // u64
    pub const SEQUENCE_NUMBER: usize = 24; // u64
    pub const TRADER_INDEX: usize = 32; // u32
    pub const IS_BID: usize = 40; // u8
    pub const ORDER_TYPE: usize = 41; // u8, OrderType is #[repr(transparent)] over u8
}

mod global {
    pub const DISCRIMINANT: usize = 0; // u64
    pub const TRADERS_ROOT_INDEX: usize = 72; // u32
    pub const DEPOSITS_ROOT_INDEX: usize = 76; // u32
    pub const DEPOSITS_MAX_INDEX: usize = 80; // u32, the cached max (lowest balance) for eviction
    pub const FREE_LIST_HEAD_INDEX: usize = 84; // u32
    pub const NUM_BYTES_ALLOCATED: usize = 88; // u32
    pub const NUM_SEATS_CLAIMED: usize = 94; // u16
}

mod gdeposit {
    pub const TRADER: usize = 0; // Pubkey
    pub const BALANCE_ATOMS: usize = 32; // u64
}

/// `payload_type`, byte 13 of a node header. 0 is a free block, which is what makes the
/// processors' index-hint checks sound: freeing zeroes the payload, byte 13 included.
mod node_type {
    pub const FREE: u8 = 0;
    pub const CLAIMED_SEAT: u8 = 1;
    pub const RESTING_ORDER: u8 = 2;
}

/// `OrderType::Global`, from `state/resting_order.rs` (Limit 0, ImmediateOrCancel 1, PostOnly 2,
/// Global 3, Reverse 4, ReverseTight 5).
///
/// This one value has to be singled out in the solvency properties. A global order RESTS IN THE
/// MARKET'S book but is backed by the GLOBAL account, not the market vault: placement takes the
/// global branch at `state/market.rs:1731-1747`, which calls only `try_to_add_to_global` and
/// never `update_balance`, so no atoms enter the market vault and no seat balance is debited --
/// and cancellation "gives nothing back on the market" (`state/market.rs:1882-1890`). The
/// program's own accounting agrees: `RestingOrder::get_orderbook_atoms` returns `(0, 0)` for a
/// global order (`state/resting_order.rs:241-250`). Counting one as a market liability therefore
/// reports insolvency on a vault that was never meant to cover it.
///
/// Reverse(4) and ReverseTight(5) are NOT in this class: they take the ordinary else-branch and
/// are debited through `update_balance`, so they are real market liabilities.
const ORDER_TYPE_GLOBAL: u8 = 3;

// ---------------------------------------------------------------------------------------------
// Little-endian readers. Every decode goes through these: a short slice yields None rather than
// panicking, so a property over a truncated or absent account reports nothing instead of
// aborting the campaign on a harness fault.
// ---------------------------------------------------------------------------------------------

fn u8at(d: &[u8], o: usize) -> Option<u8> {
    d.get(o).copied()
}
fn u16at(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(o..o + 2)?.try_into().ok()?))
}
fn u32at(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?))
}
fn u64at(d: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(d.get(o..o + 8)?.try_into().ok()?))
}
fn u128at(d: &[u8], o: usize) -> Option<u128> {
    Some(u128::from_le_bytes(d.get(o..o + 16)?.try_into().ok()?))
}
fn keyat(d: &[u8], o: usize) -> Option<Pubkey> {
    Some(Pubkey::new_from_array(d.get(o..o + 32)?.try_into().ok()?))
}
/// SPL token account `amount`, at offset 64 of both the legacy and the Token-2022 base layout.
fn token_amount(d: &[u8]) -> Option<u64> {
    u64at(d, 64)
}

// ---------------------------------------------------------------------------------------------
// PDAs, seeds taken from the program's own macros:
//   market vault  ["vault", market, mint]   validation/token_checkers.rs market_vault_seeds!
//   global        ["global", mint]          validation/manifest_checker.rs global_seeds!
//   global vault  ["global-vault", mint]    validation/token_checkers.rs global_vault_seeds!
// ---------------------------------------------------------------------------------------------

fn market_vault(market: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"vault", market.as_ref(), mint.as_ref()], &MANIFEST_ID).0
}
fn global_account(mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"global", mint.as_ref()], &MANIFEST_ID).0
}
fn global_vault(mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"global-vault", mint.as_ref()], &MANIFEST_ID).0
}

// ---------------------------------------------------------------------------------------------
// Red-black tree walking over the raw dynamic region of an account.
//
// A DataIndex is a BYTE OFFSET into the dynamic region, not a node ordinal, so a node's absolute
// offset is `fixed_size + index` and valid indices are multiples of the block stride
// (`lib/src/utils.rs` get_helper, `program/processor/shared.rs`).
// ---------------------------------------------------------------------------------------------

struct Tree<'a> {
    data: &'a [u8],
    fixed: usize,
    stride: usize,
}

/// What a walk found, or the first structural fault it hit.
#[derive(Default)]
struct Walk {
    /// Node indices, in in-order sequence.
    order: Vec<u32>,
    /// The first structural fault. The walk stops there.
    fault: Option<String>,
    /// Black height agreed at the NIL leaves, when they all agreed.
    black_height: Option<usize>,
}

impl<'a> Tree<'a> {
    fn new(data: &'a [u8], fixed: usize, stride: usize) -> Self {
        Self { data, fixed, stride }
    }

    /// One node's bytes, header included, or None when the index is out of range or misaligned.
    /// Misalignment is a fault in its own right: the program's own alignment checks are
    /// `debug_assert`s, which are compiled out of the release `.so`.
    fn node(&self, index: u32) -> Option<&'a [u8]> {
        if index == NIL {
            return None;
        }
        let index = index as usize;
        if index % self.stride != 0 {
            return None;
        }
        let start = self.fixed.checked_add(index)?;
        self.data.get(start..start.checked_add(self.stride)?)
    }

    fn left(&self, i: u32) -> Option<u32> {
        u32at(self.node(i)?, 0)
    }
    fn right(&self, i: u32) -> Option<u32> {
        u32at(self.node(i)?, 4)
    }
    fn parent(&self, i: u32) -> Option<u32> {
        u32at(self.node(i)?, 8)
    }
    fn color(&self, i: u32) -> Option<u8> {
        u8at(self.node(i)?, 12)
    }
    fn payload_type(&self, i: u32) -> Option<u8> {
        u8at(self.node(i)?, 13)
    }
    /// The payload: the node bytes past the 16-byte header.
    fn value(&self, i: u32) -> Option<&'a [u8]> {
        self.node(i)?.get(RB_HEADER..)
    }

    /// How many blocks the account currently has room for.
    fn block_count(&self, num_bytes_allocated: u32) -> usize {
        let available = self.data.len().saturating_sub(self.fixed).min(num_bytes_allocated as usize);
        available / self.stride
    }

    /// Walk from `root`, checking the structural properties a program-written account must
    /// satisfy and collecting the in-order node sequence.
    ///
    /// Bounded by the account's block count, so a cycle introduced by a bad link terminates with
    /// a fault instead of hanging the fuzzer.
    fn walk(&self, root: u32, budget: usize) -> Walk {
        let mut w = Walk::default();
        if root == NIL {
            w.black_height = Some(0);
            return w;
        }
        if self.node(root).is_none() {
            w.fault = Some(format!("root index {root} is out of range or misaligned"));
            return w;
        }
        match self.parent(root) {
            Some(p) if p != NIL => {
                w.fault = Some(format!("root {root} has parent link {p}, expected NIL"));
                return w;
            }
            None => {
                w.fault = Some(format!("root {root} unreadable"));
                return w;
            }
            _ => {}
        }
        // Iterative in-order traversal carrying the expected parent and the black depth above.
        let mut stack: Vec<(u32, u32, usize, bool)> = vec![(root, NIL, 0, false)];
        let mut seen = std::collections::HashSet::new();
        let limit = budget.saturating_mul(4).saturating_add(16);
        let mut steps = 0usize;
        while let Some((i, expected_parent, black_above, left_done)) = stack.pop() {
            steps += 1;
            if steps > limit {
                w.fault = Some("traversal exceeded the account's block count (cycle?)".into());
                return w;
            }
            if i == NIL {
                let height = black_above + 1; // every NIL leaf counts as one black
                match w.black_height {
                    None => w.black_height = Some(height),
                    Some(h) if h != height => {
                        w.fault = Some(format!("black height {height} != {h} at a NIL leaf"));
                        return w;
                    }
                    _ => {}
                }
                continue;
            }
            let Some(color) = self.color(i) else {
                w.fault = Some(format!("node {i} is out of range or misaligned"));
                return w;
            };
            if color > 1 {
                // `Color` is a repr(transparent) u8 so any byte is a valid bit pattern from
                // untrusted data, but the program only ever writes Black(0) or Red(1).
                w.fault = Some(format!("node {i} has colour byte {color}, expected 0 or 1"));
                return w;
            }
            let black_below = black_above + usize::from(color == 0);
            if !left_done {
                if !seen.insert(i) {
                    w.fault = Some(format!("node {i} reached twice (cycle or shared subtree)"));
                    return w;
                }
                match self.parent(i) {
                    Some(p) if p != expected_parent => {
                        w.fault = Some(format!("node {i} parent link {p} != its actual parent {expected_parent}"));
                        return w;
                    }
                    None => {
                        w.fault = Some(format!("node {i} parent unreadable"));
                        return w;
                    }
                    _ => {}
                }
                let (Some(l), Some(r)) = (self.left(i), self.right(i)) else {
                    w.fault = Some(format!("node {i} child links unreadable"));
                    return w;
                };
                if color == 1 {
                    for c in [l, r] {
                        if c != NIL && self.color(c) == Some(1) {
                            w.fault = Some(format!("red node {i} has red child {c}"));
                            return w;
                        }
                    }
                }
                stack.push((i, expected_parent, black_above, true));
                stack.push((l, i, black_below, false));
            } else {
                w.order.push(i);
                let Some(r) = self.right(i) else {
                    w.fault = Some(format!("node {i} right link unreadable"));
                    return w;
                };
                stack.push((r, i, black_below, false));
            }
        }
        w
    }

    /// Follow the free list from `head`. `FreeListNode { next_index, node_inner }` puts
    /// `next_index` in bytes 0..4, the same bytes `RBNode` uses for `left`
    /// (`lib/src/free_list.rs`). Bounded like `walk`.
    fn free_list(&self, head: u32, budget: usize) -> Result<Vec<u32>, String> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut cur = head;
        while cur != NIL {
            let Some(node) = self.node(cur) else {
                return Err(format!("free-list index {cur} is out of range or misaligned"));
            };
            if !seen.insert(cur) {
                return Err(format!("free list cycles at index {cur}"));
            }
            if out.len() > budget + 1 {
                return Err("free list is longer than the account's block count".into());
            }
            let Some(next) = u32at(node, 0) else {
                return Err(format!("free-list node {cur} unreadable"));
            };
            out.push(cur);
            cur = next;
        }
        Ok(out)
    }
}

/// `state/utils.rs` compare_trader_keys: the first 8 bytes as a BIG-endian u64, tie-broken by the
/// whole key. Used as the tree key for both ClaimedSeat and GlobalTrader
/// (`state/claimed_seat.rs:61-65`, `state/global.rs:217-221`). Note the big-endian read -- a
/// little-endian one would order the same keys differently and report false violations.
fn compare_trader_keys(left: &Pubkey, right: &Pubkey) -> std::cmp::Ordering {
    let (lb, rb) = (left.to_bytes(), right.to_bytes());
    let lhs = u64::from_be_bytes(lb[..8].try_into().unwrap_or_default());
    let rhs = u64::from_be_bytes(rb[..8].try_into().unwrap_or_default());
    match lhs.cmp(&rhs) {
        std::cmp::Ordering::Equal => lb.cmp(&rb),
        other => other,
    }
}

/// The first position where a price sequence runs against its side's ordering, if any.
///
/// The direction is per side, and getting it wrong is the easiest way to write a property that
/// fires on a correct book. `impl Ord for RestingOrder`
/// (`programs/manifest/src/state/resting_order.rs`) is:
///
/// ```text
/// if self.get_is_bid() { self.price.cmp(&other.price) }   // bids: key ascends with price
/// else                 { other.price.cmp(&self.price) }   // asks: key ascends as price DESCENDS
/// ```
///
/// so an in-order walk yields ascending prices on the bid side and DESCENDING prices on the ask
/// side. Both put the tree's maximum at the top of book, which is why `bids_best_index` and
/// `asks_best_index` can both be the cached max.
///
/// The comparison is NON-STRICT in both directions: inserts place equal keys to the left but
/// rotations can lift an equal key onto a right link, so a strict check reports a violation on
/// any book holding three orders at one price. That is exactly the defect in the program's own
/// `validate_red_black_tree` (`lib/src/red_black_tree.rs`), and the reason this harness walks the
/// tree itself.
/// `scout_check!` with the property id and site prepended to the message.
///
/// `scout_check!`'s custom-message form records ONLY the formatted text, so a violation arrives
/// without saying which property produced it -- the id is in the harness source and nowhere in
/// the crash. Every argument here is a literal, so `concat!` composes the prefix at compile time.
macro_rules! prop_check {
    ($property:literal, $site:literal, $predicate:expr, $fmt:literal $(, $arg:expr)* $(,)?) => {
        scout_check!($property, $site, $predicate, concat!($property, " ", $site, ": ", $fmt) $(, $arg)*)
    };
}

fn first_out_of_order(prices: &[u128], ascending: bool) -> Option<usize> {
    prices.windows(2).position(|w| if ascending { w[1] < w[0] } else { w[1] > w[0] })
}

// ---------------------------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------------------------

/// One market leg, so actions can talk about base and quote uniformly.
#[derive(Clone, Copy)]
struct Leg {
    mint: Pubkey,
    vault: Pubkey,
    global: Pubkey,
    global_vault: Pubkey,
    token_program: Pubkey,
}

#[derive(Clone)]
struct ManifestFixture {
    ctx: TestContext,
    program_id: Pubkey,

    /// Distinct signers. `actors[ADVERSARY]` is the designated attacker: it is the actor the
    /// liveness tests credit when checking that conservation can fail, and the one to extend a
    /// future per-actor property around. Distinct keypairs and distinct token accounts throughout,
    /// because sharing either between "different" actors is how a fixture manufactures findings.
    actors: Vec<Rc<Keypair>>,
    /// Wallet token accounts, `wallets[actor][BASE|QUOTE]`, on the legacy-SPL market.
    wallets: Vec<[Pubkey; 2]>,
    /// Wallet token accounts on the Token-2022 market, same indexing.
    t22_wallets: Vec<[Pubkey; 2]>,

    market: Pubkey,
    base: Leg,
    quote: Leg,

    /// A second market on a Token-2022 pair, so `transfer_checked`, the optional-mint accounts
    /// and the 2022 vault sizing are reachable rather than only the legacy path.
    t22_market: Pubkey,
    t22_base: Leg,
    t22_quote: Leg,

    /// A mint with a market and a global but no seats, reserved so `CreateMarket` and
    /// `GlobalCreate` stay reachable as actions instead of being consumed by setup.
    spare_base_mint: Pubkey,
    spare_quote_mint: Pubkey,

    /// Total atoms of each traded mint held across EVERY account the fixture knows about, as of
    /// the end of setup: `[spl base, spl quote, t22 base, t22 quote]`. P-0004 compares against
    /// this.
    ///
    /// System-wide rather than per-actor, which is a correction. A per-actor statement -- "the
    /// adversary cannot end better off" -- cannot distinguish being robbed from taking the other
    /// side of an order the counterparty signed, and a fuzzer will cheerfully sign economically
    /// degenerate ones. Measured: actor 0 rested an ask of 255 base atoms at 6.5536e-7 quote
    /// atoms per base atom, a total notional of 0.000167 quote atoms, i.e. below one atom; the
    /// adversary then filled it for 0 quote and gained 255 base. That fires a per-actor property
    /// (it did, 324 times in three minutes) while creating no value at all -- the maker lost
    /// exactly what the taker gained.
    ///
    /// Conservation is the statement that survives: atoms cannot be CREATED. It needs no exchange
    /// rate and is blind to transfers between actors, which is what a trade is.
    supply_baseline: [u128; 4],

    /// Highest `order_sequence_number` observed on the market, for P-0007.
    max_seen_sequence: u64,
}

impl ManifestFixture {
    // --------------------------------------------------------------------------- construction --

    fn mint(ctx: &mut TestContext, decimals: u8, authority: &Pubkey, program: &Pubkey) -> Pubkey {
        let key = Pubkey::new_unique();
        let mut b = ctx.create_mint().pubkey(key).decimals(decimals).mint_authority(*authority).is_initialized(true);
        if *program != SPL_TOKEN_ID {
            // The program routes by `mint.owner_pubkey()` (`processor/create_market.rs`), so the
            // owner is what makes a leg exercise the Token-2022 paths. The base 82-byte mint
            // body is identical between the two programs.
            b = b.owner(*program).owner_unverified();
        }
        b.create().expect("setup: create mint");
        key
    }

    /// A Token-2022 mint carrying a `TransferFeeConfig` at `bps`, with `authority` as the
    /// transfer-fee-config authority so the fee can be raised later (`set_t22_transfer_fee`).
    ///
    /// Built by packing real account bytes rather than by CPI, for the same reason the plain mints
    /// are: setup runs once per fuzz input. The TLV layout comes from the extension crate rather
    /// than being hand-rolled, because an off-by-one there would make the program's own
    /// `StateWithExtensions::unpack` fail and turn every Token-2022 action into a silent no-op.
    ///
    /// `epoch: 0` on both fee records so the fee is active immediately -- the program selects
    /// `newer_transfer_fee` only once the current epoch has reached its `epoch`.
    fn mint_t22_with_fee(ctx: &mut TestContext, decimals: u8, authority: &Pubkey, bps: u16) -> Pubkey {
        let len = ExtensionType::try_calculate_account_len::<T22Mint>(&[ExtensionType::TransferFeeConfig])
            .expect("setup: size a transfer-fee mint");
        let mut data = vec![0u8; len];
        {
            let mut state = StateWithExtensionsMut::<T22Mint>::unpack_uninitialized(&mut data)
                .expect("setup: unpack an uninitialized transfer-fee mint");
            let cfg = state.init_extension::<TransferFeeConfig>(true).expect("setup: init TransferFeeConfig");
            // A mutable fee. The authority is what makes "the fee went up after the global was
            // created" a history the chain can actually have, rather than a fixture fabrication.
            cfg.transfer_fee_config_authority.0 = authority.to_bytes().into();
            for fee in [&mut cfg.older_transfer_fee, &mut cfg.newer_transfer_fee] {
                fee.epoch = 0u64.into();
                fee.maximum_fee = u64::MAX.into();
                fee.transfer_fee_basis_points = bps.into();
            }
            state.base = T22Mint {
                mint_authority: COption::Some(*authority),
                supply: 0,
                decimals,
                is_initialized: true,
                freeze_authority: COption::None,
            };
            state.pack_base();
            state.init_account_type().expect("setup: tag the mint account type");
        }
        let key = Pubkey::new_unique();
        ctx.create_account()
            .pubkey(key)
            .owner(SPL_TOKEN_2022_ID)
            .owner_unverified()
            .data(&data)
            .create()
            .expect("setup: create the transfer-fee mint");
        key
    }

    /// Put a new transfer fee in force on a mint built by `mint_t22_with_fee`, in place.
    ///
    /// The account-level equivalent of the fee authority sending `SetTransferFee`: the fee in
    /// force moves to `older_transfer_fee` and the new one becomes `newer_transfer_fee`. The real
    /// instruction schedules the new fee two epochs out; this writes it as already reached
    /// (`epoch: 0`), which is the state the chain is in once those epochs pass. The program
    /// (`is_global_mint_matchable`) and Token-2022's `transfer_checked` both read only the fee
    /// for the current epoch, so neither can tell the two apart.
    fn set_t22_transfer_fee(ctx: &mut TestContext, mint: &Pubkey, bps: u16) {
        ctx.update_account(mint, |data| {
            let mut state =
                StateWithExtensionsMut::<T22Mint>::unpack(data).expect("setup: unpack the transfer-fee mint");
            let cfg = state.get_extension_mut::<TransferFeeConfig>().expect("setup: find TransferFeeConfig");
            cfg.older_transfer_fee = cfg.newer_transfer_fee;
            cfg.newer_transfer_fee.epoch = 0u64.into();
            cfg.newer_transfer_fee.transfer_fee_basis_points = bps.into();
        })
        .expect("setup: raise the transfer fee");
    }

    /// A Token-2022 token account carrying `TransferFeeAmount`, which `transfer_checked` requires
    /// on any account of a fee-bearing mint. A plain 165-byte account would make every transfer of
    /// that mint fail, turning this blind spot into an always-failing action instead.
    fn wallet_t22_with_fee(ctx: &mut TestContext, mint: &Pubkey, owner: &Pubkey) -> Pubkey {
        let len = ExtensionType::try_calculate_account_len::<T22Account>(&[ExtensionType::TransferFeeAmount])
            .expect("setup: size a fee-bearing token account");
        let mut data = vec![0u8; len];
        {
            let mut state = StateWithExtensionsMut::<T22Account>::unpack_uninitialized(&mut data)
                .expect("setup: unpack an uninitialized fee-bearing token account");
            let withheld = state.init_extension::<TransferFeeAmount>(true).expect("setup: init TransferFeeAmount");
            withheld.withheld_amount = 0u64.into();
            state.base = T22Account {
                mint: *mint,
                owner: *owner,
                amount: WALLET_ATOMS,
                delegate: COption::None,
                state: T22AccountState::Initialized,
                is_native: COption::None,
                delegated_amount: 0,
                close_authority: COption::None,
            };
            state.pack_base();
            state.init_account_type().expect("setup: tag the token account type");
        }
        let key = Pubkey::new_unique();
        ctx.create_account()
            .pubkey(key)
            .owner(SPL_TOKEN_2022_ID)
            .owner_unverified()
            .data(&data)
            .create()
            .expect("setup: create the fee-bearing token account");
        key
    }

    fn wallet(ctx: &mut TestContext, mint: &Pubkey, owner: &Pubkey, program: &Pubkey) -> Pubkey {
        let key = Pubkey::new_unique();
        let mut b = ctx.create_token_account().pubkey(key).mint(*mint).token_owner(*owner).amount(WALLET_ATOMS);
        if *program != SPL_TOKEN_ID {
            b = b.owner(*program).owner_unverified();
        }
        b.create().expect("setup: create wallet token account");
        key
    }

    /// `CreateMarket` cannot stand alone. The market account must already be owned by the
    /// program and be uninitialized (`ManifestAccountInfo::new_init` ->
    /// `verify_owned_by_manifest` + uninitialized discriminant), and the supported way to get
    /// there is a system `create_account` in the SAME transaction, which requires the new
    /// account's key to sign. So market creation is a compound action by construction: two
    /// instructions, one `send_batch`.
    fn create_market(
        ctx: &mut TestContext,
        payer: &Rc<Keypair>,
        base_mint: &Pubkey,
        quote_mint: &Pubkey,
    ) -> anyhow::Result<Pubkey> {
        let market_kp = Keypair::new();
        let market = market_kp.pubkey();

        // system_instruction::create_account, hand-encoded: the system program's bincode layout
        // is a 4-byte LE instruction index (0 = CreateAccount), then lamports u64, space u64,
        // owner Pubkey. Spelled out rather than taken from a dependency so the harness need not
        // agree with any solana-program version about this struct.
        let mut data = Vec::with_capacity(4 + 8 + 8 + 32);
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&MARKET_RENT.to_le_bytes());
        data.extend_from_slice(&(MARKET_FIXED_SIZE as u64).to_le_bytes());
        data.extend_from_slice(MANIFEST_ID.as_ref());
        let create = Instruction {
            program_id: SYSTEM_PROGRAM_ID,
            accounts: vec![AccountMeta::new(payer.pubkey(), true), AccountMeta::new(market, true)],
            data,
        };

        let init = Instruction {
            program_id: MANIFEST_ID,
            accounts: vec![
                AccountMeta::new(payer.pubkey(), true),
                AccountMeta::new(market, false),
                AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
                AccountMeta::new_readonly(*base_mint, false),
                AccountMeta::new_readonly(*quote_mint, false),
                AccountMeta::new(market_vault(&market, base_mint), false),
                AccountMeta::new(market_vault(&market, quote_mint), false),
                AccountMeta::new_readonly(SPL_TOKEN_ID, false),
                AccountMeta::new_readonly(SPL_TOKEN_2022_ID, false),
            ],
            data: manifest::instruction::CreateMarket.data(),
        };

        ctx.raw_call(create).signers(&[payer, &market_kp]).add_transaction()?;
        ctx.raw_call(init).signers(&[payer, &market_kp]).add_transaction()?;
        match ctx.send_batch()? {
            Some(o) if o.is_success() => Ok(market),
            Some(o) => anyhow::bail!("CreateMarket failed: {:?}", o.logs()),
            None => anyhow::bail!("CreateMarket produced no transaction"),
        }
    }

    /// `GlobalCreate` is payer-signed only; the global account and its vault are PDAs the program
    /// creates by CPI. Keyed on the MINT, so it can only ever succeed once per mint.
    fn create_global(ctx: &mut TestContext, payer: &Rc<Keypair>, leg: &Leg) -> anyhow::Result<()> {
        let ix = Instruction {
            program_id: MANIFEST_ID,
            accounts: vec![
                AccountMeta::new(payer.pubkey(), true),
                AccountMeta::new(leg.global, false),
                AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
                AccountMeta::new_readonly(leg.mint, false),
                AccountMeta::new(leg.global_vault, false),
                AccountMeta::new_readonly(leg.token_program, false),
            ],
            data: manifest::instruction::GlobalCreate.data(),
        };
        let o = ctx.raw_call(ix).signers(&[payer]).send()?;
        if !o.is_success() {
            anyhow::bail!("GlobalCreate failed: {:?}", o.logs());
        }
        Ok(())
    }

    fn leg(market: &Pubkey, mint: Pubkey, token_program: Pubkey) -> Leg {
        Leg {
            mint,
            vault: market_vault(market, &mint),
            global: global_account(&mint),
            global_vault: global_vault(&mint),
            token_program,
        }
    }

    // ------------------------------------------------------------------------------- decoding --

    fn data(&self, key: &Pubkey) -> Option<&[u8]> {
        self.ctx.account_data(key).ok()
    }

    fn token_balance(&self, key: &Pubkey) -> u64 {
        self.data(key).and_then(token_amount).unwrap_or(0)
    }

    /// A market's seat tree, walked, with the account bytes and block budget alongside.
    fn seats(&self, market_key: &Pubkey) -> Option<(&[u8], usize, Walk)> {
        let d = self.data(market_key)?;
        if u64at(d, market::DISCRIMINANT)? != MARKET_FIXED_DISCRIMINANT {
            return None;
        }
        let allocated = u32at(d, market::NUM_BYTES_ALLOCATED)?;
        let tree = Tree::new(d, MARKET_FIXED_SIZE, MARKET_BLOCK_SIZE);
        let budget = tree.block_count(allocated);
        let root = u32at(d, market::CLAIMED_SEATS_ROOT_INDEX)?;
        Some((d, budget, tree.walk(root, budget)))
    }

    /// The byte index of `trader`'s ClaimedSeat node on a market, if it has one.
    fn seat_index_on(&self, market_key: &Pubkey, trader: &Pubkey) -> Option<u32> {
        let (d, _, walk) = self.seats(market_key)?;
        let tree = Tree::new(d, MARKET_FIXED_SIZE, MARKET_BLOCK_SIZE);
        walk.order.into_iter().find(|&i| tree.value(i).and_then(|v| keyat(v, seat::TRADER)).as_ref() == Some(trader))
    }

    fn seat_index(&self, trader: &Pubkey) -> Option<u32> {
        self.seat_index_on(&self.market, trader)
    }

    /// Up to `n` sequence numbers of live orders belonging to `trader`, so a cancel can name a
    /// real order rather than almost always missing.
    fn my_order_sequences(&self, trader: &Pubkey, n: usize) -> Vec<u64> {
        let mut out = Vec::new();
        if n == 0 {
            return out;
        }
        let Some((d, budget, _)) = self.seats(&self.market) else { return out };
        let Some(seat_idx) = self.seat_index(trader) else { return out };
        let tree = Tree::new(d, MARKET_FIXED_SIZE, MARKET_BLOCK_SIZE);
        for off in [market::BIDS_ROOT_INDEX, market::ASKS_ROOT_INDEX] {
            let Some(root) = u32at(d, off) else { continue };
            for i in tree.walk(root, budget).order {
                let Some(v) = tree.value(i) else { continue };
                if u32at(v, order::TRADER_INDEX) == Some(seat_idx) {
                    if let Some(seq) = u64at(v, order::SEQUENCE_NUMBER) {
                        out.push(seq);
                        if out.len() >= n {
                            return out;
                        }
                    }
                }
            }
        }
        out
    }

    /// `trader`'s balance in a global account, by walking the deposits tree.
    fn global_deposit_of(&self, leg: &Leg, trader: &Pubkey) -> u64 {
        let Some(d) = self.data(&leg.global) else { return 0 };
        if u64at(d, global::DISCRIMINANT) != Some(GLOBAL_FIXED_DISCRIMINANT) {
            return 0;
        }
        let Some(allocated) = u32at(d, global::NUM_BYTES_ALLOCATED) else { return 0 };
        let tree = Tree::new(d, GLOBAL_FIXED_SIZE, GLOBAL_BLOCK_SIZE);
        let budget = tree.block_count(allocated);
        let Some(root) = u32at(d, global::DEPOSITS_ROOT_INDEX) else { return 0 };
        for i in tree.walk(root, budget).order {
            let Some(v) = tree.value(i) else { continue };
            if keyat(v, gdeposit::TRADER).as_ref() == Some(trader) {
                return u64at(v, gdeposit::BALANCE_ATOMS).unwrap_or(0);
            }
        }
        0
    }

    /// Every account in the fixture that can hold atoms of a given leg's mint: the three actors'
    /// wallets, the market vault, and the global vault. Tokens enter the system only in setup and
    /// are never minted afterwards, so this total can fall (a Token-2022 transfer fee moves atoms
    /// into the mint's own withheld pool, which is not one of these accounts) but must never rise.
    fn mint_total(&self, leg: &Leg, wallets: &[[Pubkey; 2]], slot: usize) -> u128 {
        let mut total: u128 = 0;
        for w in wallets {
            total = total.saturating_add(self.token_balance(&w[slot]) as u128);
        }
        total = total.saturating_add(self.token_balance(&leg.vault) as u128);
        total = total.saturating_add(self.token_balance(&leg.global_vault) as u128);
        total
    }

    /// The four per-mint totals, in the order `supply_baseline` records them.
    fn mint_totals(&self) -> [u128; 4] {
        [
            self.mint_total(&self.base, &self.wallets, BASE),
            self.mint_total(&self.quote, &self.wallets, QUOTE),
            self.mint_total(&self.t22_base, &self.t22_wallets, BASE),
            self.mint_total(&self.t22_quote, &self.t22_wallets, QUOTE),
        ]
    }

    fn note_sequence(&mut self) {
        if let Some(d) = self.data(&self.market) {
            if let Some(seq) = u64at(d, market::ORDER_SEQUENCE_NUMBER) {
                self.max_seen_sequence = self.max_seen_sequence.max(seq);
            }
        }
    }

    fn actor(&self, sel: u8) -> Rc<Keypair> {
        self.actors[sel as usize % N_ACTORS].clone()
    }

    fn ix(&self, data: Vec<u8>, accounts: Vec<AccountMeta>) -> Instruction {
        Instruction { program_id: self.program_id, accounts, data }
    }

    /// Send one instruction. A failed transaction is a normal fuzzing outcome -- the program
    /// rejecting an invalid request is the program working -- so this reports false rather than
    /// propagating.
    fn send(&mut self, ix: Instruction, signers: &[&Rc<Keypair>]) -> bool {
        let kps: Vec<&Keypair> = signers.iter().map(|k| &***k).collect();
        self.ctx.raw_call(ix).signers(&kps).send().map(|o| o.is_success()).unwrap_or(false)
    }
}

#[fuzz_fixture]
impl ManifestFixture {
    pub fn setup() -> Self {
        let mut ctx = TestContext::new();
        let program_id = Pubkey::new_from_array(manifest::ID.to_bytes());
        // The ONLY add_program call. Everything else the program CPIs into (SPL Token, p-token,
        // Token-2022, ATA) is already loaded by TestContext::new(); adding one here would be
        // replaced wholesale by the target binary under coverage mode, because FUZZ_PROGRAM_SO
        // applies to every add_program with no program-id filter.
        ctx.add_program(&program_id, TARGET_PROGRAM_ARTIFACT).expect("setup: load programs/manifest.so");

        let actors: Vec<Rc<Keypair>> = (0..N_ACTORS).map(|_| Rc::new(Keypair::new())).collect();
        for a in &actors {
            ctx.create_account()
                .pubkey(a.pubkey())
                .lamports(ACTOR_LAMPORTS)
                .owner(SYSTEM_PROGRAM_ID)
                .create()
                .expect("setup: fund actor");
        }
        let payer = actors[0].clone();

        // Legacy SPL legs at 9/6 decimals, matching the program's own fixture. Distinct mints
        // per leg and per market: a shared mint across accounts of one type is what makes the
        // account-mutation engine's cross-authority probe fire on every instruction carrying two
        // of them, which is a fixture artefact rather than a protocol defect.
        let base_mint = Self::mint(&mut ctx, BASE_DECIMALS, &payer.pubkey(), &SPL_TOKEN_ID);
        let quote_mint = Self::mint(&mut ctx, QUOTE_DECIMALS, &payer.pubkey(), &SPL_TOKEN_ID);
        let market =
            Self::create_market(&mut ctx, &payer, &base_mint, &quote_mint).expect("setup: create the SPL market");
        let base = Self::leg(&market, base_mint, SPL_TOKEN_ID);
        let quote = Self::leg(&market, quote_mint, SPL_TOKEN_ID);

        // A Token-2022 market, left cold (no seats, no deposits) so its own init paths stay
        // reachable while the legacy market carries the stateful load.
        let t22_base_mint = Self::mint(&mut ctx, BASE_DECIMALS, &payer.pubkey(), &SPL_TOKEN_2022_ID);
        // The quote leg carries a transfer fee; the base leg stays a plain 2022 mint, so one
        // market exercises both the fee-bearing and the fee-free Token-2022 paths. Created before
        // create_market, which sizes each vault from its own mint's extension list. Created at
        // 0 bps: the fee is raised to T22_FEE_BPS below, once the leg's global exists.
        let t22_quote_mint = Self::mint_t22_with_fee(&mut ctx, QUOTE_DECIMALS, &payer.pubkey(), 0);
        let t22_market = Self::create_market(&mut ctx, &payer, &t22_base_mint, &t22_quote_mint)
            .expect("setup: create the Token-2022 market");
        let t22_base = Self::leg(&t22_market, t22_base_mint, SPL_TOKEN_2022_ID);
        let t22_quote = Self::leg(&t22_market, t22_quote_mint, SPL_TOKEN_2022_ID);

        // Global accounts for the four traded mints. A global is a per-MINT PDA, so GlobalCreate
        // can only succeed once per mint -- creating it here would disable that action forever
        // if these were the only mints, which is why the spare pair below exists.
        for leg in [&base, &quote, &t22_base, &t22_quote] {
            Self::create_global(&mut ctx, &payer, leg).expect("setup: create global account");
        }

        // Only now put the quote leg's fee in force. GlobalCreate refuses a mint whose transfer
        // fee is live (`is_global_mint_matchable`), so a global on a fee-bearing mint is reachable
        // one way only: the fee authority raises the fee after the global exists. That is also the
        // state worth fuzzing, because placement, swap quoting and matching must each re-check the
        // mint rather than trust that it was clean when the global was created.
        Self::set_t22_transfer_fee(&mut ctx, &t22_quote_mint, T22_FEE_BPS);

        // Mints with NO market and NO global, reserved for action_create_market and
        // action_global_create. Without them both instructions would be one-shot and already
        // spent, and the gate would have to record them as permanently blocked.
        let spare_base_mint = Self::mint(&mut ctx, BASE_DECIMALS, &payer.pubkey(), &SPL_TOKEN_ID);
        let spare_quote_mint = Self::mint(&mut ctx, QUOTE_DECIMALS, &payer.pubkey(), &SPL_TOKEN_ID);

        // Per-actor wallets on both markets. Distinct accounts per actor per leg.
        let mut wallets = Vec::with_capacity(N_ACTORS);
        let mut t22_wallets = Vec::with_capacity(N_ACTORS);
        for a in &actors {
            wallets.push([
                Self::wallet(&mut ctx, &base.mint, &a.pubkey(), &SPL_TOKEN_ID),
                Self::wallet(&mut ctx, &quote.mint, &a.pubkey(), &SPL_TOKEN_ID),
            ]);
            t22_wallets.push([
                Self::wallet(&mut ctx, &t22_base.mint, &a.pubkey(), &SPL_TOKEN_2022_ID),
                // The fee mint's token accounts need the TransferFeeAmount extension.
                Self::wallet_t22_with_fee(&mut ctx, &t22_quote.mint, &a.pubkey()),
            ]);
        }

        // Accounts the account-mutation engine should probe for missing validation.
        for k in [
            &market,
            &t22_market,
            &base.vault,
            &quote.vault,
            &base.global,
            &quote.global,
            &base.global_vault,
            &quote.global_vault,
        ] {
            ctx.track_account(*k);
        }

        let mut f = Self {
            ctx,
            program_id,
            actors,
            wallets,
            t22_wallets,
            market,
            base,
            quote,
            t22_market,
            t22_base,
            t22_quote,
            spare_base_mint,
            spare_quote_mint,
            supply_baseline: [0; 4],
            max_seen_sequence: 0,
        };
        // Taken AFTER every endowment: earlier would credit the adversary with its own starting
        // capital and make P-0004 unfalsifiable.
        f.supply_baseline = f.mint_totals();
        f
    }

    // -------------------------------------------------------------------------------- actions --

    /// ClaimSeat (tag 1). The precondition for Deposit, Withdraw and any order, so the gateway
    /// action. A second claim by the same trader is rejected with AlreadyClaimedSeat, which is a
    /// legitimate reject rather than a harness fault.
    pub fn action_claim_seat(&mut self, #[range(0..3)] actor: u8, on_t22: bool) -> bool {
        let a = self.actor(actor);
        let market = if on_t22 { self.t22_market } else { self.market };
        let ix = self.ix(
            manifest::instruction::ClaimSeat.data(),
            vec![
                AccountMeta::new(a.pubkey(), true),
                AccountMeta::new(market, false),
                AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            ],
        );
        self.send(ix, &[&a])
    }

    /// Deposit (tag 2). Six accounts, the mint included -- it is NOT optional in
    /// `DepositContext::load` -- and the trader token account must be owned by the payer.
    pub fn action_deposit(
        &mut self,
        #[range(0..3)] actor: u8,
        is_base: bool,
        #[range(0..1000000000000)] amount_atoms: u64,
        use_hint: bool,
    ) -> bool {
        let a = self.actor(actor);
        let leg = if is_base { self.base } else { self.quote };
        let wallet = self.wallets[actor as usize % N_ACTORS][if is_base { BASE } else { QUOTE }];
        let market = self.market;
        // A hint must be the block-aligned byte offset of the payer's OWN ClaimedSeat node, so a
        // raw fuzzer draw would be rejected at the alignment check on almost every input. Looking
        // the hint up keeps the happy path reachable; the interesting case -- a well-formed but
        // stale hint -- arises on its own as seats are claimed and the tree rotates under it.
        let hint = if use_hint { self.seat_index(&a.pubkey()) } else { None };
        let ix = self.ix(
            manifest::instruction::Deposit {
                params: manifest::types::DepositParams { amount_atoms, trader_index_hint: hint },
            }
            .data(),
            vec![
                AccountMeta::new(a.pubkey(), true),
                AccountMeta::new(market, false),
                AccountMeta::new(wallet, false),
                AccountMeta::new(leg.vault, false),
                AccountMeta::new_readonly(leg.token_program, false),
                AccountMeta::new_readonly(leg.mint, false),
            ],
        );
        self.send(ix, &[&a])
    }

    /// Withdraw (tag 3). Same six accounts as Deposit.
    ///
    /// Deliberately reachable with NO seat: `MarketRefMut::deposit` guards its trader index with
    /// `require!(is_not_nil!(..))` and `withdraw` does not (`state/market.rs`), so a withdraw by
    /// a seatless trader indexes the dynamic region at NIL. Whatever that does is worth seeing.
    pub fn action_withdraw(
        &mut self,
        #[range(0..3)] actor: u8,
        is_base: bool,
        #[range(0..1000000000000)] amount_atoms: u64,
        use_hint: bool,
        on_t22: bool,
    ) -> bool {
        let a = self.actor(actor);
        // Withdrawing from the Token-2022 market as well is what lets that vault SHRINK; with
        // deposits only, its solvency properties hold with slack and never bind.
        let (market, leg, wallet) = if on_t22 {
            let leg = if is_base { self.t22_base } else { self.t22_quote };
            let w = self.t22_wallets[actor as usize % N_ACTORS][if is_base { BASE } else { QUOTE }];
            (self.t22_market, leg, w)
        } else {
            let leg = if is_base { self.base } else { self.quote };
            let w = self.wallets[actor as usize % N_ACTORS][if is_base { BASE } else { QUOTE }];
            (self.market, leg, w)
        };
        let hint = if use_hint { self.seat_index_on(&market, &a.pubkey()) } else { None };
        let ix = self.ix(
            manifest::instruction::Withdraw {
                params: manifest::types::WithdrawParams { amount_atoms, trader_index_hint: hint },
            }
            .data(),
            vec![
                AccountMeta::new(a.pubkey(), true),
                AccountMeta::new(market, false),
                AccountMeta::new(wallet, false),
                AccountMeta::new(leg.vault, false),
                AccountMeta::new_readonly(leg.token_program, false),
                AccountMeta::new_readonly(leg.mint, false),
            ],
        );
        self.send(ix, &[&a])
    }

    /// Swap, the 13-account single-signer shape. Needs neither a seat nor a deposit -- just a
    /// funded wallet -- so it is the action that reaches matching logic from a cold fixture.
    /// `with_globals` appends the two optional global slots, which is what lets a swap match
    /// against a global order.
    pub fn action_swap(
        &mut self,
        #[range(0..3)] actor: u8,
        #[range(0..100000000000)] in_atoms: u64,
        #[range(0..100000000000)] out_atoms: u64,
        is_base_in: bool,
        is_exact_in: bool,
        with_globals: bool,
    ) -> bool {
        let a = self.actor(actor);
        let (base, quote) = (self.base, self.quote);
        let w = self.wallets[actor as usize % N_ACTORS];
        let market = self.market;
        let mut metas = vec![
            AccountMeta::new(a.pubkey(), true),
            AccountMeta::new(market, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            AccountMeta::new(w[BASE], false),
            AccountMeta::new(w[QUOTE], false),
            AccountMeta::new(base.vault, false),
            AccountMeta::new(quote.vault, false),
            AccountMeta::new_readonly(base.token_program, false),
        ];
        if with_globals {
            // The loader recognises these by content, and the side matters: gas prepayment is
            // indexed `global_order_counts[is_bid]` (`state/utils.rs`), so a global bid needs the
            // QUOTE global and a global ask the BASE global. Offer the side being taken from.
            let leg = if is_base_in { quote } else { base };
            metas.push(AccountMeta::new(leg.global, false));
            metas.push(AccountMeta::new(leg.global_vault, false));
        }
        let ix = self.ix(
            manifest::instruction::Swap {
                params: manifest::types::SwapParams { in_atoms, out_atoms, is_base_in, is_exact_in },
            }
            .data(),
            metas,
        );
        self.send(ix, &[&a])
    }

    /// The 14-account shape that separates the rent payer from the token-account owner.
    ///
    /// `SwapContext::load` selects it by finding account 1 NOT owned by the manifest program, so
    /// the two shapes are distinguished by CONTENT, not by tag -- and the program's own
    /// `swap_v2_instruction` builder emits tag 4, which means tag 13 is never exercised by any
    /// in-repo client. Both tags are driven here for that reason.
    pub fn action_swap_v2(
        &mut self,
        #[range(0..3)] payer_sel: u8,
        #[range(0..3)] owner_sel: u8,
        #[range(0..100000000000)] in_atoms: u64,
        #[range(0..100000000000)] out_atoms: u64,
        is_base_in: bool,
        is_exact_in: bool,
        use_tag_13: bool,
    ) -> bool {
        let payer = self.actor(payer_sel);
        let owner = self.actor(owner_sel);
        let (base, quote) = (self.base, self.quote);
        let w = self.wallets[owner_sel as usize % N_ACTORS];
        let market = self.market;
        let params = manifest::types::SwapParams { in_atoms, out_atoms, is_base_in, is_exact_in };
        let data = if use_tag_13 {
            manifest::instruction::SwapV2 { params }.data()
        } else {
            manifest::instruction::Swap { params }.data()
        };
        let ix = self.ix(
            data,
            vec![
                AccountMeta::new(payer.pubkey(), true),
                AccountMeta::new(owner.pubkey(), true),
                AccountMeta::new(market, false),
                AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
                AccountMeta::new(w[BASE], false),
                AccountMeta::new(w[QUOTE], false),
                AccountMeta::new(base.vault, false),
                AccountMeta::new(quote.vault, false),
                AccountMeta::new_readonly(base.token_program, false),
            ],
        );
        if payer.pubkey() == owner.pubkey() {
            return self.send(ix, &[&payer]);
        }
        self.send(ix, &[&payer, &owner])
    }

    /// Expand (tag 5), no-payload form: adds exactly one block, and only when the market does
    /// not already hold two free ones.
    pub fn action_expand(&mut self, #[range(0..3)] actor: u8) -> bool {
        let a = self.actor(actor);
        let market = self.market;
        let ix = self.ix(
            manifest::instruction::Expand.data(),
            vec![
                AccountMeta::new(a.pubkey(), true),
                AccountMeta::new(market, false),
                AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            ],
        );
        self.send(ix, &[&a])
    }

    /// Expand (tag 5), u32-payload form: batch-expand to `blocks` free blocks in one call.
    ///
    /// `Expand` is payload-polymorphic -- `processor/expand_market.rs` reads
    /// `data.first_chunk::<4>()` as a RAW little-endian u32, not as borsh -- and the shipped
    /// builder only ever emits the no-payload form, so this arm is reachable in production but
    /// unreachable through the program's own client. Hand-encoded for both reasons: the IDL
    /// records the instruction as taking no arguments, which is true of the builder and not of
    /// the processor.
    pub fn action_expand_n(&mut self, #[range(0..3)] actor: u8, #[range(0..64)] blocks: u32) -> bool {
        let a = self.actor(actor);
        let market = self.market;
        let mut data = vec![5u8];
        data.extend_from_slice(&blocks.to_le_bytes());
        let ix = self.ix(
            data,
            vec![
                AccountMeta::new(a.pubkey(), true),
                AccountMeta::new(market, false),
                AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            ],
        );
        self.send(ix, &[&a])
    }

    /// BatchUpdate (tag 6): place and cancel orders. The protocol's richest instruction and the
    /// only way to put resting orders on the book.
    ///
    /// The fuzzer drives the shape with scalars and the orders are synthesised from them, so one
    /// input can describe a crossing pair or a ladder rather than n copies of one order.
    /// `order_type` is a byte over a slightly wider range than the enum: the harness IDL types it
    /// `u8` rather than as a 6-variant enum precisely so values above the last named constant
    /// reach `OrderType::is_valid()` instead of being unreachable by construction. The range stops
    /// at 8 rather than spanning the whole byte because `require!(order_type.is_valid())` rejects
    /// the order BEFORE any placement, so an unbounded byte would weight that reject branch 40:1
    /// against every legal type -- measured, that difference is ~0.7% vs ~21% successful
    /// placements, and since crucible breaks an action chain on the first failure, a rejected
    /// batch update also truncates everything after it. 0..8 keeps 6 and 7 for the reject branch
    /// while leaving the six legal types the common case.
    ///
    /// `price_exponent` is mapped from a u8 into the program's legal [-18, 8] window plus a margin
    /// on each side, so the out-of-range branch in `quantities.rs` is reachable too.
    pub fn action_batch_update(
        &mut self,
        #[range(0..3)] actor: u8,
        use_hint: bool,
        #[range(0..4)] n_orders: u8,
        #[range(0..3)] n_cancels: u8,
        #[range(0..1000000000000)] base_atoms: u64,
        #[range(1..1000000000)] price_mantissa: u32,
        #[range(0..32)] exponent_sel: u8,
        is_bid: bool,
        #[range(0..8)] order_type: u8,
        #[range(0..4294967295)] last_valid_slot: u32,
        with_globals: bool,
        on_t22: bool,
    ) -> bool {
        let a = self.actor(actor);
        // Either market can hold resting orders. Without this the Token-2022 market's books stay
        // permanently empty, every t22 swap matches nothing, and its vault only ever grows -- so
        // the solvency properties over that leg hold with slack and can never bind.
        let (market, base, quote) = if on_t22 {
            (self.t22_market, self.t22_base, self.t22_quote)
        } else {
            (self.market, self.base, self.quote)
        };
        let hint = if use_hint { self.seat_index_on(&market, &a.pubkey()) } else { None };
        // [-20, 11]: the legal window is [-18, 8], so both rejection branches stay reachable.
        let price_exponent = (exponent_sel as i16 - 20) as i8;

        let mut orders = Vec::with_capacity(n_orders as usize);
        for k in 0..n_orders {
            let bump = u64::from(k) + 1;
            orders.push(manifest::types::PlaceOrderParams {
                base_atoms: base_atoms.saturating_mul(bump).max(1),
                price_mantissa: price_mantissa.saturating_add(u32::from(k)),
                price_exponent,
                // Alternate sides so one action can cross its own orders and reach matching.
                is_bid: if k % 2 == 0 { is_bid } else { !is_bid },
                last_valid_slot,
                order_type,
            });
        }

        let mine = self.my_order_sequences(&a.pubkey(), n_cancels as usize);
        let mut cancels = Vec::with_capacity(n_cancels as usize);
        for k in 0..n_cancels as usize {
            cancels.push(manifest::types::CancelOrderParams {
                order_sequence_number: mine.get(k).copied().unwrap_or(k as u64),
                order_index_hint: None,
            });
        }

        let mut metas = vec![
            AccountMeta::new(a.pubkey(), true),
            AccountMeta::new(market, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
        ];
        if with_globals {
            // Each present mint appends a 5-account block, base first then quote
            // (`instruction_builders/batch_update_instruction.rs`). A global order needs the
            // matching side's block and only that side, so supply both.
            for leg in [base, quote] {
                metas.push(AccountMeta::new_readonly(leg.mint, false));
                metas.push(AccountMeta::new(leg.global, false));
                metas.push(AccountMeta::new(leg.global_vault, false));
                metas.push(AccountMeta::new(leg.vault, false));
                metas.push(AccountMeta::new_readonly(leg.token_program, false));
            }
        }

        let ix = self.ix(
            manifest::instruction::BatchUpdate {
                params: manifest::types::BatchUpdateParams { trader_index_hint: hint, cancels, orders },
            }
            .data(),
            metas,
        );
        let ok = self.send(ix, &[&a]);
        if ok {
            self.note_sequence();
        }
        ok
    }

    /// GlobalCreate (tag 7), on the spare mints that setup deliberately left without a global.
    pub fn action_global_create(&mut self, #[range(0..3)] actor: u8, use_quote: bool) -> bool {
        let a = self.actor(actor);
        let mint = if use_quote { self.spare_quote_mint } else { self.spare_base_mint };
        let leg = Leg {
            mint,
            vault: Pubkey::default(),
            global: global_account(&mint),
            global_vault: global_vault(&mint),
            token_program: SPL_TOKEN_ID,
        };
        Self::create_global(&mut self.ctx, &a, &leg).is_ok()
    }

    /// GlobalAddTrader (tag 8). Charges a lamport seat fee and expands the global account. A
    /// second add for the same trader is rejected, which is a legitimate reject.
    pub fn action_global_add_trader(&mut self, #[range(0..3)] actor: u8, is_base: bool) -> bool {
        let a = self.actor(actor);
        let leg = if is_base { self.base } else { self.quote };
        let ix = self.ix(
            manifest::instruction::GlobalAddTrader.data(),
            vec![
                AccountMeta::new(a.pubkey(), true),
                AccountMeta::new(leg.global, false),
                AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            ],
        );
        self.send(ix, &[&a])
    }

    /// GlobalDeposit (tag 9). Requires the trader to have been added first.
    pub fn action_global_deposit(
        &mut self,
        #[range(0..3)] actor: u8,
        is_base: bool,
        #[range(0..1000000000000)] amount_atoms: u64,
    ) -> bool {
        let a = self.actor(actor);
        let leg = if is_base { self.base } else { self.quote };
        let wallet = self.wallets[actor as usize % N_ACTORS][if is_base { BASE } else { QUOTE }];
        let ix = self.ix(
            manifest::instruction::GlobalDeposit { params: manifest::types::GlobalDepositParams { amount_atoms } }
                .data(),
            vec![
                AccountMeta::new_readonly(a.pubkey(), true),
                AccountMeta::new(leg.global, false),
                AccountMeta::new_readonly(leg.mint, false),
                AccountMeta::new(leg.global_vault, false),
                AccountMeta::new(wallet, false),
                AccountMeta::new_readonly(leg.token_program, false),
            ],
        );
        self.send(ix, &[&a])
    }

    /// GlobalWithdraw (tag 10). Same shape as GlobalDeposit.
    pub fn action_global_withdraw(
        &mut self,
        #[range(0..3)] actor: u8,
        is_base: bool,
        #[range(0..1000000000000)] amount_atoms: u64,
    ) -> bool {
        let a = self.actor(actor);
        let leg = if is_base { self.base } else { self.quote };
        let wallet = self.wallets[actor as usize % N_ACTORS][if is_base { BASE } else { QUOTE }];
        let ix = self.ix(
            manifest::instruction::GlobalWithdraw { params: manifest::types::GlobalWithdrawParams { amount_atoms } }
                .data(),
            vec![
                AccountMeta::new_readonly(a.pubkey(), true),
                AccountMeta::new(leg.global, false),
                AccountMeta::new_readonly(leg.mint, false),
                AccountMeta::new(leg.global_vault, false),
                AccountMeta::new(wallet, false),
                AccountMeta::new_readonly(leg.token_program, false),
            ],
        );
        self.send(ix, &[&a])
    }

    /// GlobalEvict (tag 11). EIGHT accounts: the shank attributes declare seven, but
    /// `GlobalEvictContext::load` pulls a required `system_program` at index 7
    /// (`validation/loaders.rs`). gen-idl.py records the eighth with that citation, and its arity
    /// reconciliation turns any future divergence into a build failure.
    ///
    /// Eviction only becomes meaningful once the global seat cap is reached, and the staged
    /// default-feature program has `MAX_GLOBAL_SEATS = 999` rather than the 4 the `test` feature
    /// sets -- so with three actors the cap is not reachable and this action exercises the
    /// not-full rejection path. That is a known coverage limit of this fixture, recorded in the
    /// README rather than papered over by building the program with a different feature set.
    pub fn action_global_evict(
        &mut self,
        #[range(0..3)] actor: u8,
        #[range(0..3)] evictee: u8,
        is_base: bool,
        #[range(0..1000000000000)] amount_atoms: u64,
    ) -> bool {
        let a = self.actor(actor);
        let leg = if is_base { self.base } else { self.quote };
        let slot = if is_base { BASE } else { QUOTE };
        let wallet = self.wallets[actor as usize % N_ACTORS][slot];
        let evictee_wallet = self.wallets[evictee as usize % N_ACTORS][slot];
        let ix = self.ix(
            manifest::instruction::GlobalEvict { params: manifest::types::GlobalEvictParams { amount_atoms } }.data(),
            vec![
                AccountMeta::new(a.pubkey(), true),
                AccountMeta::new(leg.global, false),
                AccountMeta::new_readonly(leg.mint, false),
                AccountMeta::new(leg.global_vault, false),
                AccountMeta::new(wallet, false),
                AccountMeta::new(evictee_wallet, false),
                AccountMeta::new_readonly(leg.token_program, false),
                AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            ],
        );
        self.send(ix, &[&a])
    }

    /// GlobalClean (tag 12): remove an unfillable global order and claim its gas prepayment. A
    /// keeper instruction, so it must be an action -- the whole point is that it fires between
    /// other actors' actions.
    pub fn action_global_clean(&mut self, #[range(0..3)] actor: u8, is_base: bool, #[range(0..64)] block: u32) -> bool {
        let a = self.actor(actor);
        let leg = if is_base { self.base } else { self.quote };
        let market = self.market;
        // The processor indexes the dynamic region by byte offset, so an index that is not a
        // multiple of the block stride can never name a node. Scale the draw into the aligned
        // space rather than discarding almost all of it.
        let order_index = block.saturating_mul(MARKET_BLOCK_SIZE as u32);
        let ix = self.ix(
            manifest::instruction::GlobalClean { params: manifest::types::GlobalCleanParams { order_index } }.data(),
            vec![
                AccountMeta::new(a.pubkey(), true),
                AccountMeta::new(market, false),
                AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
                AccountMeta::new(leg.global, false),
            ],
        );
        self.send(ix, &[&a])
    }

    /// CreateMarket (tag 0) as an action, on the spare mints. Compound by necessity: the market
    /// account must be created by the system program in the same transaction, with the market
    /// key signing.
    ///
    /// `same_mint` drives the base == quote rejection, which `create_market.rs` raises as
    /// InvalidMarketParameters.
    pub fn action_create_market(&mut self, #[range(0..3)] actor: u8, same_mint: bool) -> bool {
        let a = self.actor(actor);
        let base_mint = self.spare_base_mint;
        let quote_mint = if same_mint { base_mint } else { self.spare_quote_mint };
        Self::create_market(&mut self.ctx, &a, &base_mint, &quote_mint).is_ok()
    }

    /// Drive the Token-2022 market, so `transfer_checked`, the optional-mint accounts and the
    /// 2022 vault sizing are exercised rather than only the legacy path. Both legs are 2022, so
    /// the optional mints are required by `transfer_checked` and recognised by owner.
    pub fn action_t22_swap(
        &mut self,
        #[range(0..3)] actor: u8,
        #[range(0..100000000000)] in_atoms: u64,
        #[range(0..100000000000)] out_atoms: u64,
        is_base_in: bool,
        is_exact_in: bool,
    ) -> bool {
        let a = self.actor(actor);
        let (base, quote) = (self.t22_base, self.t22_quote);
        let market = self.t22_market;
        let w = self.t22_wallets[actor as usize % N_ACTORS];
        let ix = self.ix(
            manifest::instruction::Swap {
                params: manifest::types::SwapParams { in_atoms, out_atoms, is_base_in, is_exact_in },
            }
            .data(),
            vec![
                AccountMeta::new(a.pubkey(), true),
                AccountMeta::new(market, false),
                AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
                AccountMeta::new(w[BASE], false),
                AccountMeta::new(w[QUOTE], false),
                AccountMeta::new(base.vault, false),
                AccountMeta::new(quote.vault, false),
                AccountMeta::new_readonly(base.token_program, false),
                AccountMeta::new_readonly(base.mint, false),
                AccountMeta::new_readonly(quote.token_program, false),
                AccountMeta::new_readonly(quote.mint, false),
            ],
        );
        self.send(ix, &[&a])
    }

    /// Deposit on the Token-2022 market, so the fee-aware credit-the-vault-delta path in
    /// `processor/deposit.rs` is reachable rather than only the legacy transfer.
    pub fn action_t22_deposit(
        &mut self,
        #[range(0..3)] actor: u8,
        is_base: bool,
        #[range(0..1000000000000)] amount_atoms: u64,
    ) -> bool {
        let a = self.actor(actor);
        let leg = if is_base { self.t22_base } else { self.t22_quote };
        let wallet = self.t22_wallets[actor as usize % N_ACTORS][if is_base { BASE } else { QUOTE }];
        let market = self.t22_market;
        let hint = self.seat_index_on(&market, &a.pubkey());
        let ix = self.ix(
            manifest::instruction::Deposit {
                params: manifest::types::DepositParams { amount_atoms, trader_index_hint: hint },
            }
            .data(),
            vec![
                AccountMeta::new(a.pubkey(), true),
                AccountMeta::new(market, false),
                AccountMeta::new(wallet, false),
                AccountMeta::new(leg.vault, false),
                AccountMeta::new_readonly(leg.token_program, false),
                AccountMeta::new_readonly(leg.mint, false),
            ],
        );
        self.send(ix, &[&a])
    }

    /// Advance the slot. Keeper-like, and mandatory as an action rather than a setup constant:
    /// order expiry (`last_valid_slot`) is only reachable if time moves BETWEEN two user actions,
    /// and freezing the clock silently deletes every expiry-related behaviour. The Clock sysvar
    /// is live in the staged build (the `no-clock` feature is off), so this is real.
    pub fn action_advance_slots(&mut self, #[range(1..64)] slots: u64) -> bool {
        let next = self.ctx.slot().saturating_add(slots);
        self.ctx.warp_to_slot(next);
        true
    }
}

// ---------------------------------------------------------------------------------------------
// Invariants
//
// Each is a NET over the whole protocol rather than a restatement of a check the program already
// makes: a property that mirrors a require!() can never fail, because the program built that
// wall itself. The interesting question is always where the rule SHOULD apply and does not.
// ---------------------------------------------------------------------------------------------

/// The property roster, as a plain function.
///
/// Kept separate from the `#[invariant_test]` entry point deliberately. That macro rewrites its
/// body into the per-action loop of a generated `fn(fixture, actions)` and runs it once per
/// executed action, so calling the generated function with an empty action list executes NO
/// property at all -- a test that drove the properties that way would pass while checking
/// nothing. The tests call this directly for exactly that reason.
fn run_all_properties(fixture: &ManifestFixture) {
    scout_check_session!();
    scout_run_property!("P-0001", invariant_p_0001(fixture));
    scout_run_property!("P-0002", invariant_p_0002(fixture));
    scout_run_property!("P-0003", invariant_p_0003(fixture));
    scout_run_property!("P-0004", invariant_p_0004(fixture));
    scout_run_property!("P-0005", invariant_p_0005(fixture));
    scout_run_property!("P-0006", invariant_p_0006(fixture));
    scout_run_property!("P-0007", invariant_p_0007(fixture));
    scout_run_property!("P-0008", invariant_p_0008(fixture));
    scout_run_property!("P-0009", invariant_p_0009(fixture));
    scout_run_property!("P-0010", invariant_p_0010(fixture));
    scout_run_property!("P-0011", invariant_p_0011(fixture));
}

#[invariant_test]
fn invariant_test(fixture: &mut ManifestFixture) {
    run_all_properties(fixture);
}

/// P-0001 BASE VAULT SOLVENCY. For each market, the base vault holds at least every claimed
/// seat's base withdrawable balance plus the base atoms locked in resting asks.
///
/// The program states this relationship itself, in a comment rather than in code:
/// `vault == withdrawable + orderbook` (`state/market.rs:1458`). Seat balances and resting orders
/// are therefore disjoint -- placing an order debits the seat -- so summing both is not a double
/// count. The aggregate `withdrawable_base_atoms` / `orderbook_base_atoms` fields that would have
/// made this cheap to read are `#[cfg(feature = "certora")]` ("informational only and not worth
/// the CU", `state/market.rs:192-204`) and so are absent from the fuzzed build; the sums are
/// walked out of the trees instead.
///
/// Asserted as `>=` rather than the equality the program states. Equality would be stronger but
/// would fire on dust the protocol legitimately strands in a vault -- a Token-2022 transfer fee
/// makes the deposited amount differ from the credited one (`processor/deposit.rs` credits the
/// observed vault delta for exactly that reason), and the reverse-order coalescing arithmetic at
/// `state/market.rs:1440-1480` can leave an atom behind. `>=` keeps the dangerous direction,
/// insolvency, and drops the harmless one.
///
/// Why a net rather than a mirror: `processor/withdraw.rs` checks one withdrawal against one
/// seat's balance and never the aggregate against the vault, so the relationship above is
/// maintained by an argument spread across deposit, withdraw, matching, global settlement and
/// eviction -- and nothing in the shipped build asserts it anywhere. Certora's verified suite
/// covers single-instruction funds checks; this is the cross-instruction statement it does not
/// make.
fn invariant_p_0001(f: &ManifestFixture) {
    for (label, market, leg) in [("spl", f.market, f.base), ("t22", f.t22_market, f.t22_base)] {
        let Some((d, budget, seats)) = f.seats(&market) else { continue };
        if seats.fault.is_some() {
            continue; // P-0005 owns structural faults; do not double-report them.
        }
        let tree = Tree::new(d, MARKET_FIXED_SIZE, MARKET_BLOCK_SIZE);
        let mut owed: u128 = 0;
        for &i in &seats.order {
            let Some(v) = tree.value(i) else { continue };
            owed = owed.saturating_add(u64at(v, seat::BASE_WITHDRAWABLE).unwrap_or(0) as u128);
        }
        // Resting asks lock base atoms. Bids lock quote, which the program books into the seat's
        // quote balance at placement time, so they add no base liability.
        //
        // GLOBAL asks are excluded: they rest in this book but are backed by the global account,
        // never by this vault, so counting one would report insolvency on a vault that was never
        // meant to cover it. See ORDER_TYPE_GLOBAL for the program's own accounting.
        let Some(asks_root) = u32at(d, market::ASKS_ROOT_INDEX) else { continue };
        let asks = tree.walk(asks_root, budget);
        if asks.fault.is_some() {
            continue;
        }
        for &i in &asks.order {
            let Some(v) = tree.value(i) else { continue };
            if u8at(v, order::ORDER_TYPE) == Some(ORDER_TYPE_GLOBAL) {
                continue;
            }
            owed = owed.saturating_add(u64at(v, order::NUM_BASE_ATOMS).unwrap_or(0) as u128);
        }
        let vault = f.token_balance(&leg.vault) as u128;
        prop_check!(
            "P-0001",
            "base-vault-solvency",
            vault >= owed,
            "{} base vault holds {} atoms but the market owes {} (seats + resting asks)",
            label,
            vault,
            owed
        );
    }
}

/// P-0002 QUOTE VAULT SOLVENCY, over seat balances only.
///
/// Resting bids are deliberately excluded. A bid's quote liability is a function of the 128-bit
/// fixed-point price and the base size, and reproducing the program's rounding direction wrong
/// would make this property fire on correct behaviour. Under-counting can only make the property
/// weaker, never wrong, which is the right way to be wrong here.
fn invariant_p_0002(f: &ManifestFixture) {
    for (label, market, leg) in [("spl", f.market, f.quote), ("t22", f.t22_market, f.t22_quote)] {
        let Some((d, _, seats)) = f.seats(&market) else { continue };
        if seats.fault.is_some() {
            continue;
        }
        let tree = Tree::new(d, MARKET_FIXED_SIZE, MARKET_BLOCK_SIZE);
        let mut owed: u128 = 0;
        for &i in &seats.order {
            let Some(v) = tree.value(i) else { continue };
            owed = owed.saturating_add(u64at(v, seat::QUOTE_WITHDRAWABLE).unwrap_or(0) as u128);
        }
        let vault = f.token_balance(&leg.vault) as u128;
        prop_check!(
            "P-0002",
            "quote-vault-solvency",
            vault >= owed,
            "{} quote vault holds {} atoms but seats claim {}",
            label,
            vault,
            owed
        );
    }
}

/// P-0003 GLOBAL VAULT SOLVENCY. A global vault covers the sum of every GlobalDeposit it backs.
///
/// Why a net: `global_deposit` and `global_withdraw` each check one trader's balance, while the
/// aggregate is also reduced by matching against global orders and by eviction, which pays an
/// evictee out of the same vault (`processor/global_evict.rs`). Nothing checks the total.
fn invariant_p_0003(f: &ManifestFixture) {
    for (label, leg) in [("base", f.base), ("quote", f.quote), ("t22-base", f.t22_base), ("t22-quote", f.t22_quote)] {
        let Some(d) = f.data(&leg.global) else { continue };
        if u64at(d, global::DISCRIMINANT) != Some(GLOBAL_FIXED_DISCRIMINANT) {
            continue;
        }
        let Some(allocated) = u32at(d, global::NUM_BYTES_ALLOCATED) else { continue };
        let tree = Tree::new(d, GLOBAL_FIXED_SIZE, GLOBAL_BLOCK_SIZE);
        let budget = tree.block_count(allocated);
        let Some(root) = u32at(d, global::DEPOSITS_ROOT_INDEX) else { continue };
        let walk = tree.walk(root, budget);
        if walk.fault.is_some() {
            continue;
        }
        let mut owed: u128 = 0;
        for &i in &walk.order {
            let Some(v) = tree.value(i) else { continue };
            owed = owed.saturating_add(u64at(v, gdeposit::BALANCE_ATOMS).unwrap_or(0) as u128);
        }
        let vault = f.token_balance(&leg.global_vault) as u128;
        prop_check!(
            "P-0003",
            "global-vault-solvency",
            vault >= owed,
            "{} global vault holds {} atoms but deposits total {}",
            label,
            vault,
            owed
        );
    }
}

/// P-0004 TOKEN CONSERVATION. No traded mint gains atoms across the accounts that hold it.
///
/// Tokens enter this fixture exactly once, in `setup()`, and nothing afterwards is allowed to
/// create more. So for each traded mint, the total across the three actors' wallets, the market
/// vault and the global vault may FALL -- a Token-2022 transfer fee moves atoms into the mint's
/// own withheld pool, which is not one of those accounts -- but must never RISE. A rise is atoms
/// appearing from nowhere.
///
/// Why this shape and not a per-actor one. The obvious strong net is "the designated adversary
/// cannot end better off", and it was tried: as a Pareto condition over the adversary's two legs.
/// It is unsound in a fuzzed world, because it cannot distinguish the adversary being robbed from
/// the adversary taking the other side of an order a counterparty signed -- and a fuzzer signs
/// economically degenerate orders constantly. The measured counterexample: actor 0 rested an ask
/// of 255 base atoms at 6.5536e-7 quote atoms per base atom, a notional of 0.000167 quote atoms,
/// i.e. below one atom. The adversary filled it for 0 quote and gained 255 base. The per-actor
/// property fired 324 times in three minutes on exactly that, with no value created anywhere: the
/// maker lost precisely what the taker gained. (Whether the program SHOULD reject an order whose
/// whole notional rounds below one atom is a real question about `OrderTooSmall`, recorded in the
/// README as a lead -- but it is not value creation, and a property that conflates the two is a
/// false-positive generator.)
///
/// Conservation keeps all the strength that matters. It is blind to transfers between actors,
/// which is what a trade is, and still violated by a double credit, a fee counted twice, a
/// deposit credited more than it transferred, or any arithmetic that mints balance -- the whole
/// class P-0001..P-0003 can only see once it has already reached a vault.
fn invariant_p_0004(f: &ManifestFixture) {
    let now = f.mint_totals();
    for (i, label) in ["spl-base", "spl-quote", "t22-base", "t22-quote"].iter().enumerate() {
        prop_check!(
            "P-0004",
            "token-conservation",
            now[i] <= f.supply_baseline[i],
            "{} atoms rose from {} to {} (+{}): atoms appeared from nowhere",
            label,
            f.supply_baseline[i],
            now[i],
            now[i].saturating_sub(f.supply_baseline[i])
        );
    }
}

/// P-0005 RED-BLACK TREE STRUCTURAL INTEGRITY, over every tree in every program-owned account.
///
/// Why a net: the program ships `validate_red_black_tree` but no processor calls it, and its
/// ordering check is wrong for repeated keys, so in practice nothing validates these trees
/// on-chain at all. A violation is not cosmetic: the orderbook walk climbs `parent` links, so a
/// wrong link silently truncates matching, and a red-red chain degrades a cancel into a walk long
/// enough to exhaust the compute budget -- a liveness bug reachable by anyone who can place
/// orders.
fn invariant_p_0005(f: &ManifestFixture) {
    for (label, key, fixed, stride, alloc_off, roots) in tree_roster(f) {
        let Some(d) = f.data(&key) else { continue };
        let Some(allocated) = u32at(d, alloc_off) else { continue };
        let tree = Tree::new(d, fixed, stride);
        let budget = tree.block_count(allocated);
        for (which, off) in roots {
            let Some(root) = u32at(d, off) else { continue };
            // A non-empty tree's root must be black: only the fix-up loops colour it, and they
            // end by blackening it.
            prop_check!(
                "P-0005",
                "rb-root-black",
                root == NIL || tree.color(root) == Some(0),
                "{} {} tree root {} is not black",
                label,
                which,
                root
            );
            let walk = tree.walk(root, budget);
            prop_check!(
                "P-0005",
                "rb-structure",
                walk.fault.is_none(),
                "{} {} tree is structurally invalid: {}",
                label,
                which,
                walk.fault.unwrap_or_default()
            );
        }
    }
}

/// P-0006 FREE LIST INTEGRITY. The free list is a terminating chain of in-range, correctly
/// aligned, untyped blocks, and it never overlaps a live tree.
///
/// Why a net: `FreeList::remove()` returns NIL on an empty list and none of its callers check it
/// -- `state/market_helpers.rs` feeds the result straight into `tree.insert` -- so the only thing
/// between an exhausted free list and a `data[4294967295..]` abort is a `has_free_block()` gate
/// at each of three separate call sites. A block simultaneously free and in a tree is the shape
/// that bug would take.
fn invariant_p_0006(f: &ManifestFixture) {
    for (label, key, fixed, stride, head_off, alloc_off, roots) in [
        (
            "market",
            f.market,
            MARKET_FIXED_SIZE,
            MARKET_BLOCK_SIZE,
            market::FREE_LIST_HEAD_INDEX,
            market::NUM_BYTES_ALLOCATED,
            vec![market::BIDS_ROOT_INDEX, market::ASKS_ROOT_INDEX, market::CLAIMED_SEATS_ROOT_INDEX],
        ),
        (
            "global-base",
            f.base.global,
            GLOBAL_FIXED_SIZE,
            GLOBAL_BLOCK_SIZE,
            global::FREE_LIST_HEAD_INDEX,
            global::NUM_BYTES_ALLOCATED,
            vec![global::TRADERS_ROOT_INDEX, global::DEPOSITS_ROOT_INDEX],
        ),
    ] {
        let Some(d) = f.data(&key) else { continue };
        let Some(allocated) = u32at(d, alloc_off) else { continue };
        let Some(head) = u32at(d, head_off) else { continue };
        let tree = Tree::new(d, fixed, stride);
        let budget = tree.block_count(allocated);
        let free = match tree.free_list(head, budget) {
            Ok(v) => v,
            Err(why) => {
                prop_check!("P-0006", "free-list-shape", false, "{} free list: {}", label, why);
                continue;
            }
        };

        // Freeing zeroes the payload, byte 13 included, and that is exactly what makes the
        // market processors' index-hint checks sound.
        //
        // MARKET ONLY. The global account's processors insert with a plain `tree.insert(..)` and
        // never set a payload type (`state/global.rs:609,625,686,699`), unlike the market, which
        // uses `set_payload_type(ClaimedSeat)` and `insert_with_payload_type::<RestingOrder>`
        // (`state/market.rs:1034,2104`). So byte 13 is 0 for a LIVE global node too, and the same
        // check there would hold for every block whether free or not -- true by construction and
        // therefore no evidence of anything.
        if label == "market" {
            for &i in &free {
                prop_check!(
                    "P-0006",
                    "free-block-untyped",
                    tree.payload_type(i) == Some(node_type::FREE),
                    "{} block {} is on the free list but is typed {:?}",
                    label,
                    i,
                    tree.payload_type(i)
                );
            }
        }

        let mut live = std::collections::HashSet::new();
        let mut sound = true;
        for off in roots {
            let Some(root) = u32at(d, off) else { continue };
            let walk = tree.walk(root, budget);
            if walk.fault.is_some() {
                sound = false;
            }
            live.extend(walk.order);
        }
        if sound {
            let freeset: std::collections::HashSet<u32> = free.iter().copied().collect();
            let both: Vec<u32> = live.intersection(&freeset).copied().collect();
            prop_check!(
                "P-0006",
                "free-and-live-disjoint",
                both.is_empty(),
                "{} blocks {:?} are on the free list AND reachable from a tree root",
                label,
                both
            );

            // Disjointness alone is only half the statement: a block that is on NEITHER list has
            // leaked -- the account paid rent for it and nothing can ever allocate it again. The
            // free list is the only way a released block returns to circulation
            // (`state/market_helpers.rs` free_list.add), so every allocated block must be in
            // exactly one of the two sets.
            let mut leaked: Vec<u32> = (0..budget)
                .map(|n| (n * stride) as u32)
                .filter(|i| !freeset.contains(i) && !live.contains(i))
                .collect();
            leaked.truncate(8);
            prop_check!(
                "P-0006",
                "every-block-accounted-for",
                leaked.is_empty(),
                "{} blocks {:?} are neither on the free list nor reachable from any tree root",
                label,
                leaked
            );
        }
    }
}

/// P-0007 ORDER SEQUENCE MONOTONICITY. `order_sequence_number` only ever increases.
///
/// Why a net: it is the name a cancel uses to identify an order, so a repeat makes two distinct
/// orders indistinguishable to a cancel. Nothing asserts monotonicity; it is a consequence of
/// every placement path incrementing it -- batch update, reverse-order re-placement and global
/// settlement included -- which is a statement about all of them at once.
fn invariant_p_0007(f: &ManifestFixture) {
    let Some(d) = f.data(&f.market) else { return };
    let Some(seq) = u64at(d, market::ORDER_SEQUENCE_NUMBER) else { return };
    prop_check!(
        "P-0007",
        "sequence-monotonic",
        seq >= f.max_seen_sequence,
        "market order_sequence_number went backwards: {} < {}",
        seq,
        f.max_seen_sequence
    );
}

/// P-0008 ALLOCATION COHERENCE. `num_bytes_allocated` describes the account the program holds.
///
/// Why a net: expansion writes the new block at the old `num_bytes_allocated` and then bumps it
/// (`state/market.rs`), while the account's real length changes through a separate `resize`. If
/// the two disagree, the next allocation writes outside the account or over live data, and no
/// single instruction is positioned to notice.
fn invariant_p_0008(f: &ManifestFixture) {
    for (label, key, fixed, stride, alloc_off, _) in tree_roster(f) {
        let Some(d) = f.data(&key) else { continue };
        let Some(allocated) = u32at(d, alloc_off) else { continue };
        let dynamic = d.len().saturating_sub(fixed);
        prop_check!(
            "P-0008",
            "allocation-fits",
            allocated as usize <= dynamic,
            "{} claims {} allocated bytes but its dynamic region is {}",
            label,
            allocated,
            dynamic
        );
        prop_check!(
            "P-0008",
            "allocation-aligned",
            allocated as usize % stride == 0,
            "{} claims {} allocated bytes, not a multiple of the {}-byte block",
            label,
            allocated,
            stride
        );
    }
}

/// P-0009 GLOBAL SEAT COUNTER INTEGRITY. `num_seats_claimed` equals the global trader tree's
/// node count.
///
/// Why a net: the counter gates `MAX_GLOBAL_SEATS` and the eviction path, and it is maintained by
/// `global_add_trader` and `global_evict` separately from the tree it describes. Drifting above
/// the tree bricks new seats; drifting below lets the cap be exceeded.
fn invariant_p_0009(f: &ManifestFixture) {
    for (label, key) in [("global-base", f.base.global), ("global-quote", f.quote.global)] {
        let Some(d) = f.data(&key) else { continue };
        if u64at(d, global::DISCRIMINANT) != Some(GLOBAL_FIXED_DISCRIMINANT) {
            continue;
        }
        let Some(allocated) = u32at(d, global::NUM_BYTES_ALLOCATED) else { continue };
        let Some(claimed) = u16at(d, global::NUM_SEATS_CLAIMED) else { continue };
        let tree = Tree::new(d, GLOBAL_FIXED_SIZE, GLOBAL_BLOCK_SIZE);
        let budget = tree.block_count(allocated);
        let Some(root) = u32at(d, global::TRADERS_ROOT_INDEX) else { continue };
        let walk = tree.walk(root, budget);
        if walk.fault.is_some() {
            continue;
        }
        prop_check!(
            "P-0009",
            "global-seat-counter",
            walk.order.len() == claimed as usize,
            "{} says {} seats claimed but the trader tree holds {} nodes",
            label,
            claimed,
            walk.order.len()
        );
    }
}

/// P-0010 BOOK ORDERING AND NODE TYPING. An in-order walk of a book yields non-decreasing
/// prices, every node in a book is typed as a resting order, and every node in the seat tree is
/// typed as a claimed seat.
///
/// Why a net: matching takes the best price from `best_index` and then walks the tree, so a
/// decreasing step means the engine fills at a worse price than the book actually contains --
/// value lost by whoever was matched. The ordering check is deliberately NON-STRICT: equal prices
/// are legal and common, and a strict check reproduces the false positive in the program's own
/// `validate_red_black_tree`.
fn invariant_p_0010(f: &ManifestFixture) {
    let Some((d, budget, seats)) = f.seats(&f.market) else { return };
    let tree = Tree::new(d, MARKET_FIXED_SIZE, MARKET_BLOCK_SIZE);

    for (which, off) in [("bids", market::BIDS_ROOT_INDEX), ("asks", market::ASKS_ROOT_INDEX)] {
        let Some(root) = u32at(d, off) else { continue };
        let walk = tree.walk(root, budget);
        if walk.fault.is_some() {
            continue;
        }
        let mut prices = Vec::with_capacity(walk.order.len());
        let mut all_orders = true;
        for &i in &walk.order {
            if tree.payload_type(i) != Some(node_type::RESTING_ORDER) {
                all_orders = false;
            }
            if let Some(v) = tree.value(i) {
                if let Some(p) = u128at(v, order::PRICE) {
                    prices.push(p);
                }
            }
        }
        prop_check!(
            "P-0010",
            "book-nodes-are-orders",
            all_orders,
            "a node in the {} tree is not typed as a resting order",
            which
        );
        // Bids ascend with price, asks descend: see first_out_of_order.
        let ascending = off == market::BIDS_ROOT_INDEX;
        let bad = first_out_of_order(&prices, ascending);
        prop_check!(
            "P-0010",
            "book-ordering",
            bad.is_none(),
            "{} tree is out of order at in-order position {:?} (expected prices to {})",
            which,
            bad,
            if ascending { "ascend" } else { "descend" }
        );
    }

    if seats.fault.is_none() {
        let all_seats = seats.order.iter().all(|&i| tree.payload_type(i) == Some(node_type::CLAIMED_SEAT));
        prop_check!(
            "P-0010",
            "seat-nodes-are-seats",
            all_seats,
            "a node in the claimed-seat tree is not typed as a claimed seat"
        );

        // The seat tree is keyed on the trader pubkey under compare_trader_keys, a total order
        // consistent with equality, and duplicate seats are rejected at insert
        // (`state/market.rs:1018-1022`). So its in-order walk must be STRICTLY increasing --
        // unlike a bookside, where equal prices are legal. A break here means `lookup_index`
        // cannot find a seat that is present, which reads to a trader as a lost balance.
        let keys: Vec<Pubkey> =
            seats.order.iter().filter_map(|&i| tree.value(i).and_then(|v| keyat(v, seat::TRADER))).collect();
        let bad = keys.windows(2).position(|w| compare_trader_keys(&w[0], &w[1]) != std::cmp::Ordering::Less);
        prop_check!(
            "P-0010",
            "seat-tree-strictly-ordered",
            bad.is_none(),
            "the claimed-seat tree is not strictly ordered by trader key at position {:?}",
            bad
        );
    }

    // The global accounts' two trees, with their own comparators: GlobalTrader is keyed on the
    // trader (strictly increasing, duplicates rejected at `state/global.rs:603-607`), while
    // GlobalDeposit is keyed on balance REVERSED so the tree max is the MINIMUM balance -- which
    // is what GlobalEvict uses to pick whom to displace (`state/global.rs:253-257`). Equal
    // balances are legal and common (three fresh seats all sit at 0), so that one is non-strict.
    for (label, leg) in [("base", f.base), ("quote", f.quote)] {
        let Some(d) = f.data(&leg.global) else { continue };
        if u64at(d, global::DISCRIMINANT) != Some(GLOBAL_FIXED_DISCRIMINANT) {
            continue;
        }
        let Some(allocated) = u32at(d, global::NUM_BYTES_ALLOCATED) else { continue };
        let gtree = Tree::new(d, GLOBAL_FIXED_SIZE, GLOBAL_BLOCK_SIZE);
        let gbudget = gtree.block_count(allocated);

        if let Some(root) = u32at(d, global::TRADERS_ROOT_INDEX) {
            let walk = gtree.walk(root, gbudget);
            if walk.fault.is_none() {
                let keys: Vec<Pubkey> =
                    walk.order.iter().filter_map(|&i| gtree.value(i).and_then(|v| keyat(v, 0))).collect();
                let bad = keys.windows(2).position(|w| compare_trader_keys(&w[0], &w[1]) != std::cmp::Ordering::Less);
                prop_check!(
                    "P-0010",
                    "global-trader-tree-strictly-ordered",
                    bad.is_none(),
                    "{} global trader tree is not strictly ordered by trader key at position {:?}",
                    label,
                    bad
                );
            }
        }

        if let Some(root) = u32at(d, global::DEPOSITS_ROOT_INDEX) {
            let walk = gtree.walk(root, gbudget);
            if walk.fault.is_none() {
                let balances: Vec<u128> = walk
                    .order
                    .iter()
                    .filter_map(|&i| gtree.value(i).and_then(|v| u64at(v, gdeposit::BALANCE_ATOMS)))
                    .map(u128::from)
                    .collect();
                let bad = first_out_of_order(&balances, false);
                prop_check!(
                    "P-0010",
                    "global-deposit-tree-ordered-by-balance",
                    bad.is_none(),
                    "{} global deposit tree balances do not descend in order at position {:?}",
                    label,
                    bad
                );
            }
        }
    }
}

/// P-0011 TOP-OF-BOOK CACHE COHERENCE. Each cached best/max index is the tree's actual maximum,
/// and is NIL exactly when the tree is empty.
///
/// Why a net: the cache is load-bearing twice over, and nothing validates it. Matching seeds its
/// walk from `fixed.asks_best_index` / `fixed.bids_best_index` (`state/market.rs:1403-1409`), so
/// a stale value makes the engine fill from the wrong end of the book -- at a worse price than the
/// book actually contains, which is value lost by whoever was matched. Worse,
/// `insert_with_payload_type` has an append-to-max fast path that links a new node as the right
/// child of the CACHED max whenever that max compares Less and its right link is NIL
/// (`lib/src/red_black_tree.rs:1252-1272`): with a stale cache that appends a greater key beneath
/// a non-maximal node and breaks the ordering outright. P-0010 would only see the damage after
/// that later insert; the stale cache itself -- the actual defect -- is what this catches.
///
/// An in-order walk of a search tree is its sorted key sequence, so the last node the walker
/// emits IS the maximum; this needs no extra traversal. Applied only to the three trees the
/// program actually caches a max for: the two booksides and the global deposits tree. The
/// claimed-seat and global-trader trees are deliberately opened with `max = NIL`
/// (`state/market.rs:1015`, `state/global.rs:600`), so there is no cache to check.
fn invariant_p_0011(f: &ManifestFixture) {
    let markets = [("spl", f.market), ("t22", f.t22_market)];
    for (label, key) in markets {
        let Some(d) = f.data(&key) else { continue };
        if u64at(d, market::DISCRIMINANT) != Some(MARKET_FIXED_DISCRIMINANT) {
            continue;
        }
        let Some(allocated) = u32at(d, market::NUM_BYTES_ALLOCATED) else { continue };
        let tree = Tree::new(d, MARKET_FIXED_SIZE, MARKET_BLOCK_SIZE);
        let budget = tree.block_count(allocated);
        for (which, root_off, best_off) in [
            ("bids", market::BIDS_ROOT_INDEX, market::BIDS_BEST_INDEX),
            ("asks", market::ASKS_ROOT_INDEX, market::ASKS_BEST_INDEX),
        ] {
            let (Some(root), Some(cached)) = (u32at(d, root_off), u32at(d, best_off)) else {
                continue;
            };
            let walk = tree.walk(root, budget);
            if walk.fault.is_some() {
                continue; // P-0005 owns structural faults.
            }
            let actual = walk.order.last().copied().unwrap_or(NIL);
            prop_check!(
                "P-0011",
                "book-best-index-is-the-max",
                cached == actual,
                "{} {} best index is {} but the tree's maximum is {}",
                label,
                which,
                cached,
                actual
            );
        }
    }

    for (label, leg) in [("base", f.base), ("quote", f.quote)] {
        let Some(d) = f.data(&leg.global) else { continue };
        if u64at(d, global::DISCRIMINANT) != Some(GLOBAL_FIXED_DISCRIMINANT) {
            continue;
        }
        let Some(allocated) = u32at(d, global::NUM_BYTES_ALLOCATED) else { continue };
        let (Some(root), Some(cached)) = (u32at(d, global::DEPOSITS_ROOT_INDEX), u32at(d, global::DEPOSITS_MAX_INDEX))
        else {
            continue;
        };
        let tree = Tree::new(d, GLOBAL_FIXED_SIZE, GLOBAL_BLOCK_SIZE);
        let walk = tree.walk(root, tree.block_count(allocated));
        if walk.fault.is_some() {
            continue;
        }
        let actual = walk.order.last().copied().unwrap_or(NIL);
        prop_check!(
            "P-0011",
            "global-deposits-max-index-is-the-max",
            cached == actual,
            "{} global deposits max index is {} but the tree's maximum is {}",
            label,
            cached,
            actual
        );
    }
}

/// Every program-owned account the structural properties apply to, with its layout and roots.
type TreeEntry = (&'static str, Pubkey, usize, usize, usize, Vec<(&'static str, usize)>);

fn tree_roster(f: &ManifestFixture) -> Vec<TreeEntry> {
    let market_roots = vec![
        ("bids", market::BIDS_ROOT_INDEX),
        ("asks", market::ASKS_ROOT_INDEX),
        ("seats", market::CLAIMED_SEATS_ROOT_INDEX),
    ];
    let global_roots = vec![("traders", global::TRADERS_ROOT_INDEX), ("deposits", global::DEPOSITS_ROOT_INDEX)];
    vec![
        ("market", f.market, MARKET_FIXED_SIZE, MARKET_BLOCK_SIZE, market::NUM_BYTES_ALLOCATED, market_roots.clone()),
        ("t22-market", f.t22_market, MARKET_FIXED_SIZE, MARKET_BLOCK_SIZE, market::NUM_BYTES_ALLOCATED, market_roots),
        (
            "global-base",
            f.base.global,
            GLOBAL_FIXED_SIZE,
            GLOBAL_BLOCK_SIZE,
            global::NUM_BYTES_ALLOCATED,
            global_roots.clone(),
        ),
        (
            "global-quote",
            f.quote.global,
            GLOBAL_FIXED_SIZE,
            GLOBAL_BLOCK_SIZE,
            global::NUM_BYTES_ALLOCATED,
            global_roots,
        ),
    ]
}

// ---------------------------------------------------------------------------------------------
// SCOUT:TESTS:BEGIN
//
// These run against the program staged in programs/, so they check the harness against the real
// binary rather than against a model of it. CI gates on them (`cargo test --locked --features
// invariant_test`), because the two ways this harness can rot are both invisible to a compiler:
// an action that no longer reaches the program (it returns false forever and the campaign is
// green and empty), and a property that can no longer fire (it is quiet, and quiet is
// indistinguishable from sound).
// ---------------------------------------------------------------------------------------------
#[cfg(test)]
mod harness_tests {
    use super::*;

    fn take_violation() -> Option<String> {
        __scout_crucible_test_context::take_violation()
    }

    fn clear_violation() {
        let _ = take_violation();
    }

    /// Run the invariant body and return the violation it recorded, if any.
    fn check(f: &mut ManifestFixture) -> Option<String> {
        clear_violation();
        run_all_properties(f);
        take_violation()
    }

    /// The constants and offsets this harness decodes with, re-derived from the live program
    /// rather than trusted. A mirrored constant that drifts turns every property into a statement
    /// about the wrong bytes, which is the one failure mode that produces confident nonsense.
    #[test]
    fn layout_constants_match_the_live_program() {
        let f = ManifestFixture::setup();

        assert_eq!(
            MANIFEST_ID.to_bytes(),
            manifest::ID.to_bytes(),
            "the hardcoded program id disagrees with the IDL's"
        );

        let d = f.data(&f.market).expect("market account exists");
        assert_eq!(
            u64at(d, market::DISCRIMINANT),
            Some(MARKET_FIXED_DISCRIMINANT),
            "MARKET_FIXED_DISCRIMINANT does not match the created market"
        );
        // CreateMarket sizes the account at MARKET_FIXED_SIZE and then expands once, so a fresh
        // market is exactly the header plus one block with that block on the free list.
        assert_eq!(
            d.len(),
            MARKET_FIXED_SIZE + MARKET_BLOCK_SIZE,
            "a fresh market is not header + one block; the fixed size or block size is wrong"
        );
        assert_eq!(
            u32at(d, market::NUM_BYTES_ALLOCATED),
            Some(MARKET_BLOCK_SIZE as u32),
            "num_bytes_allocated is not at the offset this harness reads"
        );
        // The mints must read back at the offsets the harness uses, which pins the whole first
        // half of MarketFixed.
        assert_eq!(keyat(d, market::BASE_MINT), Some(f.base.mint), "base_mint offset is wrong");
        assert_eq!(keyat(d, market::QUOTE_MINT), Some(f.quote.mint), "quote_mint offset is wrong");

        let g = f.data(&f.base.global).expect("global account exists");
        assert_eq!(
            u64at(g, global::DISCRIMINANT),
            Some(GLOBAL_FIXED_DISCRIMINANT),
            "GLOBAL_FIXED_DISCRIMINANT does not match the created global"
        );
        assert_eq!(g.len(), GLOBAL_FIXED_SIZE, "a fresh global is not exactly GLOBAL_FIXED_SIZE bytes");

        // The vaults the program created must be the PDAs this harness derives, or every
        // solvency property reads an unrelated account and is vacuously true.
        for (label, v) in [("base", f.base.vault), ("quote", f.quote.vault)] {
            let vd = f.data(&v).unwrap_or_else(|| panic!("{label} vault was not created"));
            assert!(token_amount(vd).is_some(), "{label} vault is not a token account");
        }
    }

    /// Stand up a realistic world: three seats, deposits on both legs, and a crossing pair of
    /// resting orders. Returns the fixture.
    fn stocked() -> ManifestFixture {
        let mut f = ManifestFixture::setup();
        for a in 0..N_ACTORS as u8 {
            assert!(f.action_claim_seat(a, false), "claim_seat({a}) failed");
            assert!(f.action_deposit(a, true, 1_000_000_000_000, false), "base deposit({a}) failed");
            assert!(f.action_deposit(a, false, 1_000_000_000, false), "quote deposit({a}) failed");
        }
        // A bid at 1e-1 and an ask above it: resting, not crossing, so both stay on the book.
        assert!(
            f.action_batch_update(0, false, 1, 0, 1_000_000, 9, 19, true, 0, 0, false, false),
            "resting bid failed"
        );
        assert!(
            f.action_batch_update(1, false, 1, 0, 1_000_000, 11, 19, false, 0, 0, false, false),
            "resting ask failed"
        );
        f
    }

    /// Every action must reach a SUCCESSFUL execution of the program at least once. A covered
    /// line is not a working action: an instruction can fail at account validation, never reach
    /// its handler, and still show covered lines, because the error branch is a line too.
    ///
    /// Two actions are listed as not-yet-reachable with this fixture rather than asserted, each
    /// for a stated structural reason. They are still driven, so a wiring break in them shows up
    /// as a changed outcome rather than as silence.
    #[test]
    fn every_action_reaches_a_successful_execution() {
        let mut f = stocked();
        let mut ok: Vec<(&str, bool)> = Vec::new();

        ok.push(("claim_seat", f.action_claim_seat(0, true))); // on the Token-2022 market
        ok.push(("deposit", f.action_deposit(0, true, 1_000_000, false)));
        ok.push(("deposit:hinted", f.action_deposit(0, true, 1_000_000, true)));
        ok.push(("withdraw", f.action_withdraw(0, true, 1_000, false, false)));
        ok.push(("swap", f.action_swap(2, 1_000_000, 0, true, true, false)));
        ok.push(("swap:globals", f.action_swap(2, 1_000, 0, true, true, true)));
        ok.push(("swap_v2:tag4", f.action_swap_v2(0, 1, 1_000, 0, true, true, false)));
        ok.push(("swap_v2:tag13", f.action_swap_v2(0, 1, 1_000, 0, true, true, true)));
        ok.push(("expand", f.action_expand(0)));
        ok.push(("expand_n", f.action_expand_n(0, 4)));
        ok.push(("batch_update", f.action_batch_update(0, false, 1, 0, 1_000, 5, 19, true, 0, 0, false, false)));
        ok.push(("batch_update:cancel", f.action_batch_update(0, false, 0, 1, 0, 1, 19, true, 0, 0, false, false)));
        // A Global order is only legal once the trader holds a seat on the matching global
        // account AND has deposited there to back it, so seat and fund both legs first.
        for is_base in [true, false] {
            assert!(f.action_global_add_trader(0, is_base), "global_add_trader({is_base}) failed");
            assert!(f.action_global_deposit(0, is_base, 10_000_000), "global_deposit({is_base}) failed");
        }
        ok.push(("batch_update:globals", f.action_batch_update(0, false, 1, 0, 1_000, 5, 19, true, 3, 0, true, false)));
        ok.push(("global_create", f.action_global_create(0, false)));
        ok.push(("global_add_trader", f.action_global_add_trader(1, true)));
        ok.push(("global_deposit", f.action_global_deposit(1, true, 1_000_000)));
        ok.push(("global_withdraw", f.action_global_withdraw(1, true, 1_000)));
        ok.push(("create_market", f.action_create_market(0, false)));
        ok.push(("t22_swap", f.action_t22_swap(0, 1_000, 0, true, true)));
        ok.push(("t22_deposit", f.action_t22_deposit(0, true, 1_000_000)));
        ok.push(("advance_slots", f.action_advance_slots(8)));

        // Driven but not asserted, with the reason:
        //
        // global_evict only does anything once the global seat cap is reached, and the staged
        // default-feature program has MAX_GLOBAL_SEATS = 999 (state/constants.rs; the 4 in the
        // table belongs to the `test` feature, which this build does not set). Three actors
        // cannot fill it, and building the program with `test` would fuzz a different binary.
        //
        // global_clean needs a global order placed AND its backing removed AND the order still
        // resting -- a multi-action sequence this roster cannot build. It IS reachable, and
        // global_clean_removes_an_unbacked_global_order asserts it directly.
        let evict = f.action_global_evict(0, 1, true, 2_000_000);
        let clean = f.action_global_clean(0, true, 1);
        eprintln!("not-asserted here: global_evict={evict} global_clean={clean}");

        let failed: Vec<&str> = ok.iter().filter(|(_, v)| !v).map(|(n, _)| *n).collect();
        for (name, v) in &ok {
            eprintln!("{:<24} {}", name, if *v { "ok" } else { "FAILED" });
        }
        assert!(
            failed.is_empty(),
            "these actions never reached a successful execution: {failed:?}. An action that \
             always fails contributes no coverage and no state, and a campaign over it is green \
             and empty."
        );
    }

    /// No property may fire on a valid sequence. A property with a false-positive rate floods the
    /// objective and buries every other property's first real finding; the campaign then looks
    /// productive while discovering nothing.
    #[test]
    fn properties_are_quiet_on_valid_sequences() {
        let mut f = stocked();
        assert_eq!(check(&mut f), None, "a property fired on the initial stocked state");

        // A long valid sequence touching every value-moving path, on BOTH markets, and placing
        // GLOBAL orders as well as ordinary ones -- a global order rests in the market's book but
        // is backed by the global account, so it is the case that distinguishes a correct solvency
        // property from one that reports insolvency on a vault that was never meant to cover it.
        for round in 0..6u8 {
            let actor = round % 3;
            let on_t22 = round % 3 == 2;
            f.action_claim_seat(actor, on_t22);
            f.action_deposit(actor, round % 2 == 0, 10_000_000, round % 2 == 0);
            f.action_t22_deposit(actor, round % 2 == 0, 10_000_000);
            // Ordinary limit orders.
            f.action_batch_update(
                actor,
                false,
                2,
                1,
                100_000,
                7 + u32::from(round),
                19,
                round % 2 == 0,
                0,
                0,
                false,
                on_t22,
            );
            // Global orders, both sides, with the global account blocks supplied.
            f.action_global_add_trader(actor, true);
            f.action_global_add_trader(actor, false);
            f.action_global_deposit(actor, true, 100_000);
            f.action_global_deposit(actor, false, 100_000);
            f.action_batch_update(actor, false, 1, 0, 100_000, 9, 19, false, 3, 0, true, false);
            f.action_batch_update(actor, false, 1, 0, 100_000, 9, 19, true, 3, 0, true, false);
            // Reverse orders, which unlike global ones ARE debited to the market.
            f.action_batch_update(actor, false, 1, 0, 100_000, 9, 19, false, 4, 0, false, false);
            f.action_swap(2, 50_000, 0, round % 2 == 0, true, false);
            f.action_swap(2, 50_000, 0, round % 2 == 0, true, true);
            f.action_t22_swap(actor, 50_000, 0, round % 2 == 0, true);
            f.action_withdraw(actor, round % 2 == 1, 1_000, false, false);
            f.action_withdraw(actor, round % 2 == 1, 1_000, false, true);
            f.action_global_withdraw(actor, true, 1_000);
            f.action_advance_slots(3);
            f.action_expand(0);
            assert_eq!(check(&mut f), None, "a property fired during valid round {round}");
        }
    }

    /// A GLOBAL resting ask must not be counted as a market-vault liability. It rests in the
    /// market's book but is backed by the global account: placement takes the global branch at
    /// `state/market.rs:1731-1747`, which never calls `update_balance`, so no atoms enter the
    /// market vault and no seat balance is debited.
    ///
    /// This is its own test because the sequence is specific -- a global seat, then an ASK with
    /// `order_type == Global` and the global blocks supplied -- and getting it wrong produces a
    /// confident, false insolvency report against the program on the flagship solvency property.
    #[test]
    fn a_global_resting_ask_is_not_a_market_liability() {
        let mut f = stocked();
        let vault_before = f.token_balance(&f.base.vault);
        assert!(f.action_global_add_trader(0, true), "global_add_trader failed");
        assert!(
            f.action_batch_update(0, false, 1, 0, 500_000_000_000, 11, 19, false, 3, 0, true, false),
            "placing a global ask failed"
        );
        assert_eq!(
            f.token_balance(&f.base.vault),
            vault_before,
            "a global ask must not move atoms into the market vault"
        );
        assert_eq!(check(&mut f), None, "a property fired on a correctly-placed global ask");
        // And again, to confirm it is not merely a first-step artefact: the state persists, so a
        // property that mis-counted it would re-report on every subsequent step.
        f.action_advance_slots(1);
        assert_eq!(check(&mut f), None, "a property fired on the persisted global-ask state");
    }

    /// Each property must FIRE when the quantity it is about is corrupted. A property that cannot
    /// fail is not evidence of anything, and nothing else in this repository would notice.
    ///
    /// The corruption is applied to the account bytes directly rather than through the program,
    /// which is the point: it manufactures exactly the state the property claims cannot exist.
    #[test]
    fn every_property_fires_when_its_subject_is_corrupted() {
        // P-0001 / P-0002 -- drain a vault below what the seats are owed.
        for (prop, vault) in [("P-0001", 0usize), ("P-0002", 1usize)] {
            let mut f = stocked();
            let key = if vault == 0 { f.base.vault } else { f.quote.vault };
            f.ctx
                .update_account(&key, |d| d[64..72].copy_from_slice(&0u64.to_le_bytes()))
                .expect("zero the vault balance");
            let v = check(&mut f);
            assert!(
                v.as_deref().unwrap_or_default().contains(prop),
                "{prop} did not fire on an emptied vault (got {v:?})"
            );
        }

        // P-0003 -- drain a global vault below the deposits it backs.
        {
            let mut f = stocked();
            assert!(f.action_global_add_trader(0, true));
            assert!(f.action_global_deposit(0, true, 5_000_000));
            f.ctx
                .update_account(&f.base.global_vault.clone(), |d| d[64..72].copy_from_slice(&0u64.to_le_bytes()))
                .expect("zero the global vault balance");
            let v = check(&mut f);
            assert!(
                v.as_deref().unwrap_or_default().contains("P-0003"),
                "P-0003 did not fire on an emptied global vault (got {v:?})"
            );
        }

        // P-0004 -- credit a wallet, i.e. atoms appearing from nowhere, and separately move atoms
        // BETWEEN accounts, which must stay quiet. The second half is the important one: it is
        // what a trade looks like, and a conservation property that fired on it would be useless.
        {
            let mut f = stocked();
            let wb = f.wallets[ADVERSARY][BASE];
            let before = f.token_balance(&wb);
            f.ctx
                .update_account(&wb, |d| d[64..72].copy_from_slice(&(before + 1_000_000).to_le_bytes()))
                .expect("mint atoms into the adversary's wallet");
            let v = check(&mut f);
            assert!(
                v.as_deref().unwrap_or_default().contains("P-0004"),
                "P-0004 did not fire when a mint's total rose (got {v:?})"
            );
        }
        {
            let mut f = stocked();
            let from = f.wallets[0][BASE];
            let to = f.wallets[ADVERSARY][BASE];
            let (a, b) = (f.token_balance(&from), f.token_balance(&to));
            f.ctx
                .update_account(&from, |d| d[64..72].copy_from_slice(&(a - 1_000_000).to_le_bytes()))
                .expect("debit one wallet");
            f.ctx
                .update_account(&to, |d| d[64..72].copy_from_slice(&(b + 1_000_000).to_le_bytes()))
                .expect("credit another by the same amount");
            assert_eq!(
                check(&mut f),
                None,
                "P-0004 fired on atoms moving between accounts, which is what a trade does"
            );
        }

        // P-0005 -- paint the seat tree's root red. Only the fix-up loops colour the root, and
        // they end by blackening it, so a red root is a real balance-invariant break.
        {
            let mut f = stocked();
            let root = {
                let d = f.data(&f.market).expect("market");
                u32at(d, market::CLAIMED_SEATS_ROOT_INDEX).expect("seats root")
            };
            assert_ne!(root, NIL, "the stocked fixture should have claimed seats");
            let at = MARKET_FIXED_SIZE + root as usize + 12;
            f.ctx.update_account(&f.market.clone(), |d| d[at] = 1).expect("redden the root");
            let v = check(&mut f);
            assert!(
                v.as_deref().unwrap_or_default().contains("P-0005"),
                "P-0005 did not fire on a red root (got {v:?})"
            );
        }

        // P-0006 -- point the free-list head at a live, typed node, so the free set and the live
        // set overlap.
        {
            let mut f = stocked();
            let root = {
                let d = f.data(&f.market).expect("market");
                u32at(d, market::CLAIMED_SEATS_ROOT_INDEX).expect("seats root")
            };
            f.ctx
                .update_account(&f.market.clone(), |d| {
                    d[market::FREE_LIST_HEAD_INDEX..market::FREE_LIST_HEAD_INDEX + 4]
                        .copy_from_slice(&root.to_le_bytes())
                })
                .expect("alias the free list onto a live node");
            let v = check(&mut f);
            assert!(
                v.as_deref().unwrap_or_default().contains("P-0006"),
                "P-0006 did not fire on a free list aliasing a live node (got {v:?})"
            );
        }

        // P-0007 -- wind the order sequence number backwards.
        {
            let mut f = stocked();
            f.note_sequence();
            assert!(f.max_seen_sequence > 0, "the stocked fixture should have placed orders");
            f.ctx
                .update_account(&f.market.clone(), |d| {
                    d[market::ORDER_SEQUENCE_NUMBER..market::ORDER_SEQUENCE_NUMBER + 8]
                        .copy_from_slice(&0u64.to_le_bytes())
                })
                .expect("rewind the sequence number");
            let v = check(&mut f);
            assert!(
                v.as_deref().unwrap_or_default().contains("P-0007"),
                "P-0007 did not fire on a rewound sequence number (got {v:?})"
            );
        }

        // P-0008 -- claim more allocated bytes than the account holds.
        {
            let mut f = stocked();
            f.ctx
                .update_account(&f.market.clone(), |d| {
                    d[market::NUM_BYTES_ALLOCATED..market::NUM_BYTES_ALLOCATED + 4]
                        .copy_from_slice(&1_000_000u32.to_le_bytes())
                })
                .expect("overstate the allocation");
            let v = check(&mut f);
            assert!(
                v.as_deref().unwrap_or_default().contains("P-0008"),
                "P-0008 did not fire on an overstated allocation (got {v:?})"
            );
        }

        // P-0009 -- desynchronise the global seat counter from its tree.
        {
            let mut f = stocked();
            assert!(f.action_global_add_trader(0, true));
            f.ctx
                .update_account(&f.base.global.clone(), |d| {
                    d[global::NUM_SEATS_CLAIMED..global::NUM_SEATS_CLAIMED + 2].copy_from_slice(&99u16.to_le_bytes())
                })
                .expect("inflate the seat counter");
            let v = check(&mut f);
            assert!(
                v.as_deref().unwrap_or_default().contains("P-0009"),
                "P-0009 did not fire on an inflated seat counter (got {v:?})"
            );
        }

        // P-0006 -- leak a block: truncate the free list to empty while a block is neither on it
        // nor in any tree. Disjointness alone would still hold, so this exercises the
        // every-block-accounted-for half specifically.
        {
            let mut f = stocked();
            let head = {
                let d = f.data(&f.market).expect("market");
                u32at(d, market::FREE_LIST_HEAD_INDEX).expect("free list head")
            };
            assert_ne!(head, NIL, "the stocked fixture should have a free block");
            f.ctx
                .update_account(&f.market.clone(), |d| {
                    d[market::FREE_LIST_HEAD_INDEX..market::FREE_LIST_HEAD_INDEX + 4]
                        .copy_from_slice(&NIL.to_le_bytes())
                })
                .expect("empty the free list, leaking the block it held");
            let v = check(&mut f);
            assert!(
                v.as_deref().unwrap_or_default().contains("P-0006"),
                "P-0006 did not fire on a leaked block (got {v:?})"
            );
        }

        // P-0010 -- break the seat tree's key order by overwriting a trader pubkey. The seat tree
        // is a total order with duplicates rejected at insert, so an out-of-order step means
        // lookup_index cannot find a seat that is present -- a lost balance, from the trader's
        // point of view.
        {
            let mut f = stocked();
            let (market, node) = {
                let (d, _, walk) = f.seats(&f.market).expect("seats");
                (f.market, walk.order.first().copied())
            };
            let node = node.expect("the stocked fixture should have claimed seats");
            // 0xff.. sorts above every real key under compare_trader_keys, so placing it at the
            // FIRST in-order position guarantees a descending step.
            let at = MARKET_FIXED_SIZE + node as usize + RB_HEADER + seat::TRADER;
            f.ctx
                .update_account(&market, |d| {
                    for b in d[at..at + 32].iter_mut() {
                        *b = 0xff;
                    }
                })
                .expect("overwrite a seat's trader key");
            let v = check(&mut f);
            assert!(
                v.as_deref().unwrap_or_default().contains("P-0010"),
                "P-0010 did not fire on an out-of-order seat tree (got {v:?})"
            );
        }

        // P-0011 -- stale the cached top of the bid book. The append-to-max fast path in
        // insert_with_payload_type trusts this value, so a stale one is the precursor to a real
        // ordering break rather than a cosmetic inconsistency.
        {
            let mut f = stocked();
            let root = {
                let d = f.data(&f.market).expect("market");
                u32at(d, market::BIDS_ROOT_INDEX).expect("bids root")
            };
            assert_ne!(root, NIL, "the stocked fixture should have a resting bid");
            f.ctx
                .update_account(&f.market.clone(), |d| {
                    d[market::BIDS_BEST_INDEX..market::BIDS_BEST_INDEX + 4].copy_from_slice(&NIL.to_le_bytes())
                })
                .expect("stale the cached best bid");
            let v = check(&mut f);
            assert!(
                v.as_deref().unwrap_or_default().contains("P-0011"),
                "P-0011 did not fire on a stale best-bid cache (got {v:?})"
            );
        }

        // P-0010 -- retype a resting order's node as a claimed seat. Node typing is what the
        // processors' index-hint checks rest on.
        {
            let mut f = stocked();
            let (market, node) = {
                let d = f.data(&f.market).expect("market");
                let allocated = u32at(d, market::NUM_BYTES_ALLOCATED).expect("allocated");
                let tree = Tree::new(d, MARKET_FIXED_SIZE, MARKET_BLOCK_SIZE);
                let budget = tree.block_count(allocated);
                let root = u32at(d, market::BIDS_ROOT_INDEX).expect("bids root");
                let node = tree.walk(root, budget).order.first().copied();
                (f.market, node)
            };
            let node = node.expect("the stocked fixture should have a resting bid");
            let at = MARKET_FIXED_SIZE + node as usize + 13;
            f.ctx.update_account(&market, |d| d[at] = node_type::CLAIMED_SEAT).expect("retype an order node");
            let v = check(&mut f);
            assert!(
                v.as_deref().unwrap_or_default().contains("P-0010"),
                "P-0010 did not fire on a retyped book node (got {v:?})"
            );
        }
    }

    /// The Token-2022 quote leg's transfer fee must actually BITE: a deposit of N atoms must
    /// credit the seat with strictly less than N.
    ///
    /// This is the fixture's only source of a transfer where the amount that arrives differs from
    /// the amount requested. If it did not diverge, `processor/deposit.rs`'s credit-the-observed-
    /// delta logic would be entered on every Token-2022 deposit and never do anything different
    /// from the legacy path -- and the stated reasons P-0001 is `>=` and P-0002 excludes resting
    /// bids would both be untestable. A plain 2022 mint with no extensions looks identical in
    /// coverage and carries none of that behaviour, so this asserts the behaviour, not the lines.
    #[test]
    fn the_token_2022_transfer_fee_actually_bites() {
        let mut f = ManifestFixture::setup();
        let who = f.actors[0].pubkey();
        assert!(f.action_claim_seat(0, true), "claim_seat on the Token-2022 market failed");

        const AMOUNT: u64 = 1_000_000_000;
        assert!(f.action_t22_deposit(0, false, AMOUNT), "deposit on the fee-bearing leg failed");

        let seat_idx =
            f.seat_index_on(&f.t22_market, &who).expect("the depositor should hold a seat on the Token-2022 market");
        let credited = {
            let d = f.data(&f.t22_market).expect("t22 market");
            let tree = Tree::new(d, MARKET_FIXED_SIZE, MARKET_BLOCK_SIZE);
            let v = tree.value(seat_idx).expect("seat payload");
            u64at(v, seat::QUOTE_WITHDRAWABLE).expect("quote withdrawable")
        };

        // 10% fee, rounded up by the token program, so the credit is at most 90% of the request.
        let expected = AMOUNT - AMOUNT * u64::from(T22_FEE_BPS) / 10_000;
        assert!(
            credited < AMOUNT,
            "a deposit of {AMOUNT} credited {credited}: the transfer fee did not bite, so the \
             fee-divergence paths are unreachable and P-0001's `>=` is untestable"
        );
        assert!(
            credited <= expected,
            "a deposit of {AMOUNT} credited {credited}, more than the {expected} a {T22_FEE_BPS}bp \
             fee allows"
        );
        // And the vault really holds what the seat is owed, i.e. the program credited the DELTA
        // rather than the requested amount.
        assert_eq!(
            f.token_balance(&f.t22_quote.vault),
            credited,
            "the vault balance and the credited amount must agree"
        );
        assert_eq!(check(&mut f), None, "a property fired on a fee-bearing deposit");
    }

    /// `GlobalClean` must be reachable, and it is the keeper path the protocol's anti-spam
    /// economics rest on: an unfillable global order can be removed by anyone, who collects its
    /// 5000-lamport gas prepayment (`state/constants.rs` GAS_DEPOSIT_LAMPORTS, and the rationale
    /// in the header comment of `programs/manifest/src/lib.rs`).
    ///
    /// It needs a sequence no single action can build -- a global order placed, then its backing
    /// withdrawn so the order becomes unfillable, then the clean -- which is why the roster test
    /// cannot reach it and a campaign can. Asserting it here keeps it from regressing into an
    /// action that never succeeds while the roster test stays green.
    #[test]
    fn global_clean_removes_an_unbacked_global_order() {
        let mut f = stocked();
        assert!(f.action_global_add_trader(0, true), "global_add_trader failed");
        assert!(f.action_global_deposit(0, true, 10_000_000), "global_deposit failed");
        // A global ASK is backed by the BASE global account, so that is the block it needs.
        assert!(
            f.action_batch_update(0, false, 1, 0, 1_000_000, 11, 19, false, 3, 0, true, false),
            "placing a global ask failed"
        );
        assert!(f.action_global_withdraw(0, true, 10_000_000), "withdrawing the backing failed");
        // Cleaned by a DIFFERENT actor, which is the point: anyone may collect the prepayment.
        // The order's block index is not known a priori, so scan the aligned space.
        let cleaned = (0..12u32).any(|block| f.action_global_clean(1, true, block));
        assert!(
            cleaned,
            "global_clean could not remove an unbacked global order; the keeper incentive path is \
             unreachable and global spam would have no remedy"
        );
        assert_eq!(check(&mut f), None, "a property fired while cleaning an unbacked global order");
    }


    /// SECURITY PROBE (throwaway): Withdraw by a trader with NO seat.
    ///
    /// `MarketRefMut::deposit` guards its trader index with `require!(is_not_nil!(trader_index))`
    /// (state/market.rs:1066); `withdraw` does not (state/market.rs:1076-1083) and passes it
    /// straight to `update_balance`. With no seat, `get_trader_index` yields NIL = 0xFFFF_FFFF.
    #[test]
    fn zz_probe_seatless_withdraw() {
        let mut f = ManifestFixture::setup(); // no seats claimed
        // Fund the vault from a DIFFERENT, seated actor, so the token transfer out of the vault
        // succeeds and execution reaches the balance bookkeeping rather than failing before it.
        assert!(f.action_claim_seat(0, false), "actor 0 claim_seat");
        assert!(f.action_deposit(0, true, 1_000_000_000, false), "actor 0 deposit");
        let who = f.actors[2].pubkey();
        assert!(f.seat_index(&who).is_none(), "actor 2 must have no seat for this probe");

        let a = f.actors[2].clone();
        let leg = f.base;
        let wallet = f.wallets[2][BASE];
        let market = f.market;
        let ix = f.ix(
            manifest::instruction::Withdraw {
                params: manifest::types::WithdrawParams { amount_atoms: 1, trader_index_hint: None },
            }
            .data(),
            vec![
                AccountMeta::new(a.pubkey(), true),
                AccountMeta::new(market, false),
                AccountMeta::new(wallet, false),
                AccountMeta::new(leg.vault, false),
                AccountMeta::new_readonly(leg.token_program, false),
                AccountMeta::new_readonly(leg.mint, false),
            ],
        );
        let outcome = f.ctx.raw_call(ix).signers(&[&*a]).send();
        match &outcome {
            Ok(o) => {
                println!("PROBE success={} err={:?}", o.is_success(), o.error_code());
                for l in o.logs() {
                    println!("PROBE LOG: {l}");
                }
            }
            Err(e) => println!("PROBE send error: {e:?}"),
        }
        // Did the market survive, and does any property notice?
        println!("PROBE market still decodable = {}", f.seats(&f.market).is_some());
        println!("PROBE violation = {:?}", check(&mut f));
    }

    /// The red-black walker must accept a tree with repeated keys. This is the trap the program's
    /// own `validate_red_black_tree` falls into -- it compares the right spine strictly, so three
    /// orders at one price make it report a violation -- and a harness that copied it would
    /// produce a flood of false findings on any real book.
    #[test]
    fn repeated_prices_are_not_an_ordering_violation() {
        let mut f = stocked();
        // Five orders at the SAME price on the same side.
        for _ in 0..5 {
            assert!(
                f.action_batch_update(0, false, 1, 0, 1_000, 7, 19, true, 0, 0, false, false),
                "placing an order at a repeated price failed"
            );
        }
        assert_eq!(check(&mut f), None, "a property fired on a book holding several orders at one price");
        assert_eq!(first_out_of_order(&[1, 1, 1, 2, 2], true), None, "equal keys are in order");
        assert_eq!(first_out_of_order(&[1, 2, 1], true), Some(1), "an ascending break is reported");
        assert_eq!(first_out_of_order(&[9, 9, 5, 5], false), None, "equal keys are in order descending");
        assert_eq!(first_out_of_order(&[9, 5, 9], false), Some(1), "a descending break is reported");
    }
}
// SCOUT:TESTS:END
