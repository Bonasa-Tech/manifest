use cvt::{cvt_assert, cvt_assume};
use nondet::*;

use crate::{
    program::get_mut_dynamic_account,
    quantities::{BaseAtoms, QuoteAtoms, QuoteAtomsPerBaseAtom, WrapperU64},
    state::{
        cvt_assume_second_trader_has_seat, get_helper_order, is_ask_order_free, is_bid_order_free,
        second_trader_index, DynamicAccount, MarketRefMut, RestingOrder,
    },
    validation::{loaders::GlobalTradeAccounts, ManifestAccountInfo, Signer, TokenAccountInfo},
    *,
};
use hypertree::DataIndex;
use solana_cvt::token::spl_token_account_get_amount;
use solana_program::account_info::AccountInfo;
use state::{
    cvt_assume_has_global_seat, global_balance_atoms, is_ask_order_taken, is_bid_order_taken,
    main_ask_order_index, main_bid_order_index, modeled_global_deposits, GlobalFixed, OrderType,
};

#[derive(Clone, Copy)]
pub struct AllBalances {
    pub vault_base: u64,
    pub vault_quote: u64,
    pub withdrawable_base: u64,
    pub orderbook_base: u64,
    pub withdrawable_quote: u64,
    pub orderbook_quote: u64,
    pub trader_base: u64,
    pub trader_quote: u64,
    pub maker_trader_base: u64,
    pub maker_trader_quote: u64,
    pub maker_order_base: u64,
    pub maker_order_quote: u64,
}

impl AllBalances {
    pub fn new(
        vault_base: u64,
        vault_quote: u64,
        withdrawable_base: u64,
        orderbook_base: u64,
        withdrawable_quote: u64,
        orderbook_quote: u64,
        trader_base: u64,
        trader_quote: u64,
        maker_trader_base: u64,
        maker_trader_quote: u64,
        maker_order_base: u64,
        maker_order_quote: u64,
    ) -> Self {
        Self {
            vault_base,
            vault_quote,
            withdrawable_base,
            orderbook_base,
            withdrawable_quote,
            orderbook_quote,
            trader_base,
            trader_quote,
            maker_trader_base,
            maker_trader_quote,
            maker_order_base,
            maker_order_quote,
        }
    }
}

/// Extract all relevant balances from all accounts
pub fn record_all_balances_without_order(
    market: &AccountInfo,
    vault_base_token: &AccountInfo,
    vault_quote_token: &AccountInfo,
    trader: &AccountInfo,
    maker_trader: &AccountInfo,
) -> AllBalances {
    let (trader_base, trader_quote) = get_trader_balance!(market, trader.key);
    let (maker_trader_base, maker_trader_quote) = get_trader_balance!(market, maker_trader.key);

    let withdrawable_base: u64 = get_withdrawable_base_atoms!(market);
    let withdrawable_quote: u64 = get_withdrawable_quote_atoms!(market);

    let orderbook_base: u64 = get_orderbook_base_atoms!(market);
    let orderbook_quote: u64 = get_orderbook_quote_atoms!(market);

    let vault_base: u64 = spl_token_account_get_amount(vault_base_token);
    let vault_quote: u64 = spl_token_account_get_amount(vault_quote_token);

    AllBalances::new(
        vault_base,
        vault_quote,
        withdrawable_base,
        orderbook_base,
        withdrawable_quote,
        orderbook_quote,
        trader_base,
        trader_quote,
        maker_trader_base,
        maker_trader_quote,
        0,
        0,
    )
}

/// Extract all relevant balances from all accounts and a maker order
pub fn record_all_balances(
    market: &AccountInfo,
    vault_base_token: &AccountInfo,
    vault_quote_token: &AccountInfo,
    trader: &AccountInfo,
    maker_trader: &AccountInfo,
    maker_order_index: DataIndex,
) -> AllBalances {
    let mut all_balances = record_all_balances_without_order(
        market,
        vault_base_token,
        vault_quote_token,
        trader,
        maker_trader,
    );
    let (maker_order_base, maker_order_quote) = get_order_atoms!(maker_order_index);
    all_balances.maker_order_base = maker_order_base.as_u64();
    all_balances.maker_order_quote = maker_order_quote.as_u64();
    all_balances
}

// Very basic market pre-conditions
pub fn cvt_assume_basic_market_preconditions(
    market: &AccountInfo,
    trader: &AccountInfo,
    vault_base_token: &AccountInfo,
    vault_quote_token: &AccountInfo,
    maker_trader: &AccountInfo,
) {
    // -- assume both maker and taker traders have seats
    state::cvt_assume_main_trader_has_seat(trader.key);
    cvt_assume_second_trader_has_seat(maker_trader.key);

    // -- assume market has proper base and quote vaults
    let market_base_vault_pk: Pubkey = get_base_vault!(market);
    let market_quote_vault_pk: Pubkey = get_quote_vault!(market);
    cvt_assume!(vault_base_token.key == &market_base_vault_pk);
    cvt_assume!(vault_quote_token.key == &market_quote_vault_pk);
    // -- assume base and quote vaults are different
    cvt_assume!(market_base_vault_pk != market_quote_vault_pk);

    // -- maker and taker traders are distinct
    cvt_assume!(trader.key != maker_trader.key);
}

/// Basic market pre-conditions. The maker order resting on the book is any
/// order type except global.
pub fn cvt_assume_market_preconditions<const IS_BID: bool>(
    market: &AccountInfo,
    trader: &AccountInfo,
    vault_base_token: &AccountInfo,
    vault_quote_token: &AccountInfo,
    maker_trader: &AccountInfo,
) -> DataIndex {
    cvt_assume_market_preconditions_gen::<IS_BID, false /* MAKER_IS_GLOBAL */>(
        market,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    )
}

/// Same as `cvt_assume_market_preconditions` but the maker order resting on the
/// book is a global order.
pub fn cvt_assume_global_market_preconditions<const IS_BID: bool>(
    market: &AccountInfo,
    trader: &AccountInfo,
    vault_base_token: &AccountInfo,
    vault_quote_token: &AccountInfo,
    maker_trader: &AccountInfo,
) -> DataIndex {
    cvt_assume_market_preconditions_gen::<IS_BID, true /* MAKER_IS_GLOBAL */>(
        market,
        trader,
        vault_base_token,
        vault_quote_token,
        maker_trader,
    )
}

/// The claimed seat nodes carry the pubkeys of the traders sitting in them.
/// `claim_seat` writes the trader key into the node, so this is an invariant of
/// any real market; the havoced mock state has to be told. The matching code
/// reads the maker's key back out of the seat node to know whose global
/// deposit pays for a global order, so without this a counterexample can trade
/// against one trader's order while drawing down another trader's deposit.
pub fn cvt_assume_seat_pubkeys(trader: &AccountInfo, maker_trader: &AccountInfo) {
    let dynamic: &[u8; 8] = &[0; 8];
    cvt_assume!(
        state::get_helper_seat(dynamic, state::main_trader_index())
            .get_value()
            .trader
            == *trader.key
    );
    cvt_assume!(
        state::get_helper_seat(dynamic, second_trader_index())
            .get_value()
            .trader
            == *maker_trader.key
    );
}

/// Basic market pre-conditions.
///
/// `IS_BID` is the side of the taker, so the maker order sits on the opposite
/// book. `MAKER_IS_GLOBAL` picks whether that maker order is a global order,
/// which is what decides where its funds come from.
pub fn cvt_assume_market_preconditions_gen<const IS_BID: bool, const MAKER_IS_GLOBAL: bool>(
    market: &AccountInfo,
    trader: &AccountInfo,
    vault_base_token: &AccountInfo,
    vault_quote_token: &AccountInfo,
    maker_trader: &AccountInfo,
) -> DataIndex {
    // -- assume both maker and taker traders have seats, and that the seat
    // -- nodes carry their pubkeys
    crate::state::cvt_assume_main_trader_has_seat(trader.key);
    crate::state::cvt_assume_second_trader_has_seat(maker_trader.key);
    cvt_assume_seat_pubkeys(trader, maker_trader);

    // -- assume market has proper base and quote vaults
    let market_base_vault_pk: Pubkey = get_base_vault!(market);
    let market_quote_vault_pk: Pubkey = get_quote_vault!(market);
    cvt_assume!(vault_base_token.key == &market_base_vault_pk);
    cvt_assume!(vault_quote_token.key == &market_quote_vault_pk);
    // -- assume base and quote vaults are different
    cvt_assume!(market_base_vault_pk != market_quote_vault_pk);

    // -- maker and taker traders are distinct
    cvt_assume!(trader.key != maker_trader.key);

    let maker_trader_index: DataIndex = second_trader_index();

    // we assume that the slot into which our new order could rest is free,
    // while the slot in the other book that we want to try and match with is filled
    if IS_BID {
        cvt_assume!(is_bid_order_free());
        cvt_assume!(is_ask_order_taken());
    } else {
        cvt_assume!(is_ask_order_free());
        cvt_assume!(is_bid_order_taken());
    }

    // -- get index of the maker order, based on the book we expect it to be in
    let maker_order_index: DataIndex = if IS_BID {
        main_ask_order_index()
    } else {
        main_bid_order_index()
    };

    // -- assume maker order is sane
    let dynamic: &mut [u8; 8] = &mut [0; 8];
    let maker_order: &RestingOrder = get_helper_order(dynamic, maker_order_index).get_value();
    cvt_assume!(maker_order.get_is_bid() == !IS_BID);
    if MAKER_IS_GLOBAL {
        cvt_assume!(maker_order.get_order_type() == OrderType::Global);
    } else {
        cvt_assume!(maker_order.get_order_type() != OrderType::Global);
    }
    cvt_assume!(maker_order.get_trader_index() == maker_trader_index);
    cvt_assume!(maker_order.get_num_base_atoms() == BaseAtoms::new(nondet()));
    // An order with no atoms left is removed from the book instead of matched,
    // so it never rests.
    cvt_assume!(maker_order.get_num_base_atoms() > BaseAtoms::ZERO);
    cvt_assume!(maker_order.get_price() == QuoteAtomsPerBaseAtom::nondet_price_u32());

    maker_order_index
}

/// The maker order is not a reverse order.
///
/// Matching a reverse order puts a new order back on the other side of the
/// book, which debits the maker a second time and reserves those funds on the
/// orderbook. Rules that pin down the exact balance deltas of a plain match
/// have to rule that out. No loss of funds still holds either way, see
/// `order_type_checks`.
pub fn cvt_assume_maker_not_reversible(maker_order_index: DataIndex) {
    let dynamic: &mut [u8; 8] = &mut [0; 8];
    let maker_order: &RestingOrder = get_helper_order(dynamic, maker_order_index).get_value();
    cvt_assume!(!maker_order.get_order_type().is_reversible());
}

/// Preconditions for the reverse-coalesce rules.
///
/// The maker order on the opposite book is a reverse order (`ReverseTight`
/// when `IS_TIGHT`), and the taker-side slot already holds a resting order of
/// the same maker within one price increment of the price the reverse order
/// comes back at -- the window `RestingOrder::eq` tolerates. That makes the
/// reverse placement coalesce into the existing order instead of resting a
/// fresh one.
///
/// Returns `(maker_order_index, coalesce_order_index)`.
pub fn cvt_assume_reverse_coalesce_preconditions<const IS_BID: bool, const IS_TIGHT: bool>(
    market: &AccountInfo,
    trader: &AccountInfo,
    vault_base_token: &AccountInfo,
    vault_quote_token: &AccountInfo,
    maker_trader: &AccountInfo,
) -> (DataIndex, DataIndex) {
    // -- assume both maker and taker traders have seats, and that the seat
    // -- nodes carry their pubkeys
    crate::state::cvt_assume_main_trader_has_seat(trader.key);
    crate::state::cvt_assume_second_trader_has_seat(maker_trader.key);
    cvt_assume_seat_pubkeys(trader, maker_trader);

    // -- assume market has proper base and quote vaults
    let market_base_vault_pk: Pubkey = get_base_vault!(market);
    let market_quote_vault_pk: Pubkey = get_quote_vault!(market);
    cvt_assume!(vault_base_token.key == &market_base_vault_pk);
    cvt_assume!(vault_quote_token.key == &market_quote_vault_pk);
    cvt_assume!(market_base_vault_pk != market_quote_vault_pk);

    // -- maker and taker traders are distinct
    cvt_assume!(trader.key != maker_trader.key);

    // -- unlike the plain matching preconditions, BOTH book slots are taken:
    // -- the maker order to match against, and the maker's own order on the
    // -- taker side that the reverse placement will coalesce into
    cvt_assume!(is_ask_order_taken());
    cvt_assume!(is_bid_order_taken());

    let maker_order_index: DataIndex = if IS_BID {
        main_ask_order_index()
    } else {
        main_bid_order_index()
    };
    let coalesce_order_index: DataIndex = if IS_BID {
        main_bid_order_index()
    } else {
        main_ask_order_index()
    };

    let maker_trader_index: DataIndex = second_trader_index();

    let reverse_order_type: OrderType = if IS_TIGHT {
        OrderType::ReverseTight
    } else {
        OrderType::Reverse
    };

    // -- the maker order is a live reverse order
    let dynamic: &mut [u8; 8] = &mut [0; 8];
    let maker_order: &RestingOrder = get_helper_order(dynamic, maker_order_index).get_value();
    cvt_assume!(maker_order.get_is_bid() == !IS_BID);
    cvt_assume!(maker_order.get_order_type() == reverse_order_type);
    cvt_assume!(maker_order.get_trader_index() == maker_trader_index);
    cvt_assume!(maker_order.get_num_base_atoms() == BaseAtoms::new(nondet()));
    cvt_assume!(maker_order.get_num_base_atoms() > BaseAtoms::ZERO);
    cvt_assume!(maker_order.get_price() == QuoteAtomsPerBaseAtom::nondet_price_u32());
    // -- Pin the spread field to a genuine u16, the same havoc idiom the other
    // -- field assumptions above use (e.g. num_base_atoms == new(nondet())).
    // -- The spread is read out of havoced mock memory and the prover does not
    // -- recover its 2-byte width, so `base - spread` in reverse_price can
    // -- underflow with a spread far larger than any u16. Equating the field
    // -- with a fresh u16 nondet constrains the memory the matching code
    // -- re-reads, not just a derived value. The funds rules do not need this
    // -- because they unwrap reverse_price and prune the error path; the
    // -- no-revert rules keep it alive.
    cvt_assume!(maker_order.get_reverse_spread() == nondet::<u16>());

    // -- the price the maker comes back at. Deterministic, so the same value
    // -- is recomputed inside the matching code.
    let price_reverse: QuoteAtomsPerBaseAtom = maker_order.reverse_price().unwrap();

    // -- the maker's resting order on the taker side sits within one price
    // -- increment of that price, so RestingOrder::eq matches and the
    // -- placement coalesces. The maker's debit is computed at the coalesce
    // -- target's own price, which is what keeps the funds invariant exact
    // -- even when the two prices differ by an increment.
    let coalesce_order: &RestingOrder = get_helper_order(dynamic, coalesce_order_index).get_value();
    cvt_assume!(coalesce_order.get_is_bid() == IS_BID);
    cvt_assume!(coalesce_order.get_order_type() == reverse_order_type);
    cvt_assume!(coalesce_order.get_trader_index() == maker_trader_index);
    cvt_assume!(coalesce_order.get_num_base_atoms() == BaseAtoms::new(nondet()));
    cvt_assume!(coalesce_order.get_reverse_spread() == nondet::<u16>());
    // Constrain the stored price fields directly rather than assuming the
    // whole price equals a constructed value. The prover does not propagate a
    // struct equality against a constructed value back into the mock's memory,
    // so the matching code re-loads a price unrelated to the one assumed here,
    // RestingOrder::eq then fails to match, and the come-back order takes the
    // fresh-insert path into an occupied slot. Constraining the limbs keeps the
    // relation on the memory the code actually reads.
    let coalesce_price: QuoteAtomsPerBaseAtom = coalesce_order.get_price();
    cvt_assume!(coalesce_price.inner[1] == 0);
    cvt_assume!(coalesce_price.inner[0] >= price_reverse.inner[0].saturating_sub(1));
    cvt_assume!(coalesce_price.inner[0] <= price_reverse.inner[0].saturating_add(1));

    (maker_order_index, coalesce_order_index)
}

/// Everything the global account side of the funds invariant needs.
///
/// A global order is not backed by the market vault. Its funds sit in the
/// global vault, a token account shared by every market that trades the mint,
/// and are only moved into the market vault at the moment the order is matched.
/// That makes the global vault a second source of funds, with its own
/// no-loss-of-funds property.
#[derive(Clone, Copy, Default)]
pub struct GlobalBalances {
    /// Token amount held by the global vault.
    pub global_vault: u64,
    /// Ghost sum of the balances of every global depositor, including the ones
    /// the mock does not model.
    pub global_deposits: u64,
    /// The maker's own balance in the global account.
    pub maker_deposit: u64,
}

/// Read the balances of the global account and its vault.
pub fn record_global_balances(
    global: &AccountInfo,
    global_vault_token: &AccountInfo,
    maker_trader: &AccountInfo,
) -> GlobalBalances {
    let global_vault: u64 = spl_token_account_get_amount(global_vault_token);
    let global_deposits: u64 = get_global_deposited_atoms!(global);
    let maker_deposit: u64 = global_balance_atoms(maker_trader.key);
    cvt_assume!(maker_deposit <= nondet::<u64>());

    GlobalBalances {
        global_vault,
        global_deposits,
        maker_deposit,
    }
}

/// The no-loss-of-funds invariant for the global account: the global vault
/// holds exactly what the depositors are owed.
pub fn cvt_assume_global_funds_invariants(balances: GlobalBalances) {
    let GlobalBalances {
        global_vault,
        global_deposits,
        maker_deposit,
    } = balances;

    // -- every modeled deposit is part of the aggregate
    cvt_assume!(modeled_global_deposits() <= global_deposits);
    cvt_assume!(maker_deposit <= global_deposits);

    // -- the vault covers all deposits
    cvt_assume!(global_vault == global_deposits);
}

pub fn cvt_assert_global_funds_invariants(balances: GlobalBalances) {
    let GlobalBalances {
        global_vault,
        global_deposits,
        maker_deposit,
    } = balances;

    // -- the vault still covers all deposits
    cvt_assert!(global_vault == global_deposits);
    // -- and the maker cannot be owed more than the total
    cvt_assert!(maker_deposit <= global_deposits);
}

/// Nothing left the global account.
pub fn cvt_assert_global_funds_unchanged(old: GlobalBalances, new: GlobalBalances) {
    cvt_assert!(old.global_vault == new.global_vault);
    cvt_assert!(old.global_deposits == new.global_deposits);
    cvt_assert!(old.maker_deposit == new.maker_deposit);
}

/// Tokens that leave the global account land in the market vault, and nowhere
/// else. `amount` is what the trade took out of the maker's global deposit.
pub fn cvt_assert_global_funds_moved_to_market(
    global_old: GlobalBalances,
    global_new: GlobalBalances,
    market_vault_old: u64,
    market_vault_new: u64,
    amount: u64,
) {
    // -- the maker paid for the trade out of their global deposit
    cvt_assert!(global_new.maker_deposit == global_old.maker_deposit.saturating_sub(amount));
    cvt_assert!(amount <= global_old.maker_deposit);

    // -- and the aggregate and the vault went down by the same amount
    cvt_assert!(global_new.global_deposits == global_old.global_deposits.saturating_sub(amount));
    cvt_assert!(global_new.global_vault == global_old.global_vault.saturating_sub(amount));

    // -- exactly those tokens showed up in the market vault
    cvt_assert!(market_vault_new == market_vault_old.saturating_add(amount));
    cvt_assert!(
        market_vault_new.saturating_sub(market_vault_old)
            == global_old
                .global_vault
                .saturating_sub(global_new.global_vault)
    );
}

/// Same as `cvt_assume_global_trade_accounts`, with a system program present so
/// that removing a global order counts a gas prepayment refund for the gas
/// receiver (the trader).
pub fn cvt_assume_global_trade_accounts_with_gas<'a>(
    market: &AccountInfo,
    trader: &'a AccountInfo<'static>,
    maker_trader: &AccountInfo,
    global: &'a AccountInfo<'static>,
    global_vault_token: &'a AccountInfo<'static>,
    market_vault_token: &'a AccountInfo<'static>,
    system_program: &'a AccountInfo<'static>,
    is_global_base: bool,
) -> [Option<GlobalTradeAccounts<'a, 'static>>; 2] {
    cvt_assume!(system_program.key == &solana_program::system_program::id());
    cvt_assume_global_trade_accounts_gen(
        market,
        trader,
        maker_trader,
        global,
        global_vault_token,
        market_vault_token,
        Some(system_program),
        is_global_base,
    )
}

/// Set up the global account and build the `GlobalTradeAccounts` for the mint a
/// global order is backed with.
///
/// The program keeps the base global at index 0 and the quote global at index
/// 1, so `is_global_base` picks the slot. A global bid is backed with quote, a
/// global ask with base.
pub fn cvt_assume_global_trade_accounts<'a>(
    market: &AccountInfo,
    trader: &'a AccountInfo<'static>,
    maker_trader: &AccountInfo,
    global: &'a AccountInfo<'static>,
    global_vault_token: &'a AccountInfo<'static>,
    market_vault_token: &'a AccountInfo<'static>,
    is_global_base: bool,
) -> [Option<GlobalTradeAccounts<'a, 'static>>; 2] {
    cvt_assume_global_trade_accounts_gen(
        market,
        trader,
        maker_trader,
        global,
        global_vault_token,
        market_vault_token,
        None,
        is_global_base,
    )
}

#[allow(clippy::too_many_arguments)]
fn cvt_assume_global_trade_accounts_gen<'a>(
    market: &AccountInfo,
    trader: &'a AccountInfo<'static>,
    maker_trader: &AccountInfo,
    global: &'a AccountInfo<'static>,
    global_vault_token: &'a AccountInfo<'static>,
    market_vault_token: &'a AccountInfo<'static>,
    system_program: Option<&'a AccountInfo<'static>>,
    is_global_base: bool,
) -> [Option<GlobalTradeAccounts<'a, 'static>>; 2] {
    // -- the global account is a manifest account holding a GlobalFixed
    cvt_assume!(global.owner == &crate::id());
    create_global!(global);

    // -- the maker has a seat on the global account, otherwise their global
    // -- order could not have been placed
    cvt_assume_has_global_seat(maker_trader.key);

    // -- the global vault is a token account of its own, distinct from the
    // -- market vaults, otherwise a transfer between them would be a no-op
    cvt_assume!(global_vault_token.key != market_vault_token.key);
    cvt_assume!(global_vault_token.key != global.key);

    let global_trade_accounts: GlobalTradeAccounts<'a, 'static> = GlobalTradeAccounts {
        // Token-2022 extensions are summarized away, so the mint is never read.
        mint_opt: None,
        global: ManifestAccountInfo::<GlobalFixed>::new(global).unwrap(),
        global_vault_opt: Some(TokenAccountInfo {
            info: global_vault_token,
        }),
        market_vault_opt: Some(TokenAccountInfo {
            info: market_vault_token,
        }),
        // The transfers are summarized, so the token program is never read.
        token_program_opt: None,
        // Without a system program there are no gas prepayment refunds, which
        // keeps lamports out of the token funds invariants. The gas rules pass
        // one to bring the refunds in.
        system_program: system_program.map(|info| crate::validation::Program { info }),
        gas_payer_opt: Some(Signer { info: trader }),
        gas_receiver_opt: Some(Signer { info: trader }),
        market: *market.key,
        num_deferred_gas_refunds: std::cell::Cell::new(0),
    };

    if is_global_base {
        [Some(global_trade_accounts), None]
    } else {
        [None, Some(global_trade_accounts)]
    }
}

pub fn cvt_assume_funds_invariants(balances: AllBalances) {
    let AllBalances {
        vault_base,
        vault_quote,
        withdrawable_base,
        orderbook_base,
        withdrawable_quote,
        orderbook_quote,
        trader_base,
        trader_quote,
        maker_trader_base,
        maker_trader_quote,
        maker_order_base,
        maker_order_quote,
    } = balances;

    // -- the sum of the trader amounts is less than aggregates
    cvt_assume!(trader_base.checked_add(maker_trader_base).unwrap() <= withdrawable_base);
    cvt_assume!(trader_quote.checked_add(maker_trader_quote).unwrap() <= withdrawable_quote);

    // -- maker order amounts are less than aggregates
    cvt_assume!(maker_order_base <= orderbook_base);
    cvt_assume!(maker_order_quote <= orderbook_quote);

    // -- vaults have enough funds to cover all obligations
    cvt_assume!(vault_base == withdrawable_base.checked_add(orderbook_base).unwrap());
    cvt_assume!(vault_quote == withdrawable_quote.checked_add(orderbook_quote).unwrap());
}

pub fn cvt_assert_funds_invariants(balances: AllBalances) {
    let AllBalances {
        vault_base,
        vault_quote,
        withdrawable_base,
        orderbook_base,
        withdrawable_quote,
        orderbook_quote,
        trader_base,
        trader_quote,
        maker_trader_base,
        maker_trader_quote,
        maker_order_base: _,
        maker_order_quote: _,
    } = balances;

    // using non-checked arithmetic in the assertion to not hide any potentially bad executions

    // -- the sum of the trader amounts is less than aggregates
    cvt_assert!(trader_base.saturating_add(maker_trader_base) <= withdrawable_base);
    cvt_assert!(trader_quote.saturating_add(maker_trader_quote) <= withdrawable_quote);

    // -- vaults have enough funds to cover all obligations
    cvt_assert!(vault_base == withdrawable_base.saturating_add(orderbook_base));
    cvt_assert!(vault_quote == withdrawable_quote.saturating_add(orderbook_quote));
}

pub fn cvt_assert_place_single_order_canceled_extra<const IS_BID: bool>(
    balances_old: AllBalances,
    balances_new: AllBalances,
) {
    let AllBalances {
        vault_base: vault_base_old,
        vault_quote: vault_quote_old,
        withdrawable_base: withdrawable_base_old,
        orderbook_base: orderbook_base_old,
        withdrawable_quote: withdrawable_quote_old,
        orderbook_quote: orderbook_quote_old,
        trader_base: _trader_base_old,
        trader_quote: _trader_quote_old,
        maker_trader_base: _maker_trader_base_old,
        maker_trader_quote: _maker_trader_quote_old,
        maker_order_base: _maker_order_base_old,
        maker_order_quote: _maker_order_quote_old,
    } = balances_old;

    let AllBalances {
        vault_base: vault_base_new,
        vault_quote: vault_quote_new,
        withdrawable_base: withdrawable_base_new,
        orderbook_base: orderbook_base_new,
        withdrawable_quote: withdrawable_quote_new,
        orderbook_quote: orderbook_quote_new,
        trader_base: _trader_base_new,
        trader_quote: _trader_quote_new,
        maker_trader_base: _maker_trader_base_new,
        maker_trader_quote: _maker_trader_quote_new,
        maker_order_base: _maker_order_base_new,
        maker_order_quote: _maker_order_quote_new,
    } = balances_new;

    // -- additional asserts
    cvt_assert!(vault_base_old == vault_base_new);
    cvt_assert!(vault_quote_old == vault_quote_new);
    cvt_assert!(
        withdrawable_base_old.saturating_add(orderbook_base_old)
            == withdrawable_base_new.saturating_add(orderbook_base_new)
    );
    cvt_assert!(
        withdrawable_quote_old.saturating_add(orderbook_quote_old)
            == withdrawable_quote_new.saturating_add(orderbook_quote_new)
    );
}

pub fn cvt_assert_place_single_order_unmatched_extra<const IS_BID: bool>(
    balances_old: AllBalances,
    balances_new: AllBalances,
) {
    let AllBalances {
        vault_base: vault_base_old,
        vault_quote: vault_quote_old,
        withdrawable_base: withdrawable_base_old,
        orderbook_base: orderbook_base_old,
        withdrawable_quote: withdrawable_quote_old,
        orderbook_quote: orderbook_quote_old,
        trader_base: _trader_base_old,
        trader_quote: _trader_quote_old,
        maker_trader_base: _maker_trader_base_old,
        maker_trader_quote: _maker_trader_quote_old,
        maker_order_base: _maker_order_base_old,
        maker_order_quote: _maker_order_quote_old,
    } = balances_old;

    let AllBalances {
        vault_base: vault_base_new,
        vault_quote: vault_quote_new,
        withdrawable_base: withdrawable_base_new,
        orderbook_base: orderbook_base_new,
        withdrawable_quote: withdrawable_quote_new,
        orderbook_quote: orderbook_quote_new,
        trader_base: _trader_base_new,
        trader_quote: _trader_quote_new,
        maker_trader_base: _maker_trader_base_new,
        maker_trader_quote: _maker_trader_quote_new,
        maker_order_base: _maker_order_base_new,
        maker_order_quote: _maker_order_quote_new,
    } = balances_new;

    // -- additional asserts
    cvt_assert!(withdrawable_base_new == withdrawable_base_old);
    cvt_assert!(withdrawable_quote_new == withdrawable_quote_old);
    cvt_assert!(orderbook_base_new == orderbook_base_old);
    cvt_assert!(orderbook_quote_new == orderbook_quote_old);
    cvt_assert!(vault_base_old == vault_base_new);
    cvt_assert!(vault_quote_old == vault_quote_new);
    cvt_assert!(
        withdrawable_base_old.saturating_add(orderbook_base_old)
            == withdrawable_base_new.saturating_add(orderbook_base_new)
    );
    cvt_assert!(
        withdrawable_quote_old.saturating_add(orderbook_quote_old)
            == withdrawable_quote_new.saturating_add(orderbook_quote_new)
    );
}

pub fn cvt_assert_place_single_order_full_match_extra<const IS_BID: bool>(
    balances_old: AllBalances,
    balances_new: AllBalances,
    total_base_atoms_traded: BaseAtoms,
    total_quote_atoms_traded: QuoteAtoms,
) {
    let AllBalances {
        vault_base: vault_base_old,
        vault_quote: vault_quote_old,
        withdrawable_base: withdrawable_base_old,
        orderbook_base: orderbook_base_old,
        withdrawable_quote: withdrawable_quote_old,
        orderbook_quote: orderbook_quote_old,
        trader_base: _trader_base_old,
        trader_quote: _trader_quote_old,
        maker_trader_base: _maker_trader_base_old,
        maker_trader_quote: _maker_trader_quote_old,
        maker_order_base: _maker_order_base_old,
        maker_order_quote: _maker_order_quote_old,
    } = balances_old;

    let AllBalances {
        vault_base: vault_base_new,
        vault_quote: vault_quote_new,
        withdrawable_base: withdrawable_base_new,
        orderbook_base: orderbook_base_new,
        withdrawable_quote: withdrawable_quote_new,
        orderbook_quote: orderbook_quote_new,
        trader_base: _trader_base_new,
        trader_quote: _trader_quote_new,
        maker_trader_base: _maker_trader_base_new,
        maker_trader_quote: _maker_trader_quote_new,
        maker_order_base: _maker_order_base_new,
        maker_order_quote: _maker_order_quote_new,
    } = balances_new;

    if IS_BID {
        cvt_assert!(total_base_atoms_traded.as_u64() <= orderbook_base_old);
        cvt_assert!(orderbook_base_new <= orderbook_base_old);
        cvt_assert!(
            orderbook_base_old.saturating_sub(orderbook_base_new)
                == total_base_atoms_traded.as_u64()
        );
        cvt_assert!(withdrawable_base_new >= withdrawable_base_old);
        cvt_assert!(
            withdrawable_base_new.saturating_sub(withdrawable_base_old)
                == orderbook_base_old.saturating_sub(orderbook_base_new)
        );
        cvt_assert!(withdrawable_quote_old == withdrawable_quote_new);
        cvt_assert!(orderbook_quote_old == orderbook_quote_new);
    } else {
        cvt_assert!(total_quote_atoms_traded.as_u64() <= orderbook_quote_old);
        cvt_assert!(orderbook_quote_new <= orderbook_quote_old);
        cvt_assert!(
            orderbook_quote_old.saturating_sub(orderbook_quote_new)
                <= total_quote_atoms_traded.as_u64().saturating_add(1)
        );
        cvt_assert!(
            orderbook_quote_old.saturating_sub(orderbook_quote_new)
                >= total_quote_atoms_traded.as_u64()
        );
        cvt_assert!(withdrawable_quote_new >= withdrawable_quote_old);
        cvt_assert!(
            withdrawable_quote_new.saturating_sub(withdrawable_quote_old)
                == orderbook_quote_old.saturating_sub(orderbook_quote_new)
        );
        cvt_assert!(withdrawable_base_old == withdrawable_base_new);
        cvt_assert!(orderbook_base_old == orderbook_base_new);
    }
    cvt_assert!(vault_base_old == vault_base_new);
    cvt_assert!(vault_quote_old == vault_quote_new);
    cvt_assert!(
        withdrawable_base_old.saturating_add(orderbook_base_old)
            == withdrawable_base_new.saturating_add(orderbook_base_new)
    );
    cvt_assert!(
        withdrawable_quote_old.saturating_add(orderbook_quote_old)
            == withdrawable_quote_new.saturating_add(orderbook_quote_new)
    );
}

pub fn cvt_assert_place_single_order_partial_match_extra<const IS_BID: bool>(
    balances_old: AllBalances,
    balances_new: AllBalances,
    total_base_atoms_traded: BaseAtoms,
    total_quote_atoms_traded: QuoteAtoms,
) {
    let AllBalances {
        vault_base: vault_base_old,
        vault_quote: vault_quote_old,
        withdrawable_base: withdrawable_base_old,
        orderbook_base: orderbook_base_old,
        withdrawable_quote: withdrawable_quote_old,
        orderbook_quote: orderbook_quote_old,
        trader_base: _trader_base_old,
        trader_quote: _trader_quote_old,
        maker_trader_base: _maker_trader_base_old,
        maker_trader_quote: _maker_trader_quote_old,
        maker_order_base: _maker_order_base_old,
        maker_order_quote: _maker_order_quote_old,
    } = balances_old;

    let AllBalances {
        vault_base: vault_base_new,
        vault_quote: vault_quote_new,
        withdrawable_base: withdrawable_base_new,
        orderbook_base: orderbook_base_new,
        withdrawable_quote: withdrawable_quote_new,
        orderbook_quote: orderbook_quote_new,
        trader_base: _trader_base_new,
        trader_quote: _trader_quote_new,
        maker_trader_base: _maker_trader_base_new,
        maker_trader_quote: _maker_trader_quote_new,
        maker_order_base: _maker_order_base_new,
        maker_order_quote: _maker_order_quote_new,
    } = balances_new;

    if IS_BID {
        // -- additional assertions
        cvt_assert!(total_base_atoms_traded.as_u64() <= orderbook_base_old);
        cvt_assert!(orderbook_base_new <= orderbook_base_old);
        cvt_assert!(
            orderbook_base_old.saturating_sub(orderbook_base_new)
                == total_base_atoms_traded.as_u64()
        );
        cvt_assert!(withdrawable_base_new >= withdrawable_base_old);
        cvt_assert!(
            withdrawable_base_new.saturating_sub(withdrawable_base_old)
                == orderbook_base_old.saturating_sub(orderbook_base_new)
        );
        cvt_assert!(withdrawable_quote_old == withdrawable_quote_new);
        cvt_assert!(orderbook_quote_old == orderbook_quote_new);
    } else {
        // -- additional assertions
        cvt_assert!(total_quote_atoms_traded.as_u64() <= orderbook_quote_old);
        cvt_assert!(orderbook_quote_new <= orderbook_quote_old);
        cvt_assert!(
            orderbook_quote_old.saturating_sub(orderbook_quote_new)
                <= total_quote_atoms_traded.as_u64().saturating_add(1)
        );
        cvt_assert!(
            orderbook_quote_old.saturating_sub(orderbook_quote_new)
                >= total_quote_atoms_traded.as_u64()
        );
        cvt_assert!(withdrawable_quote_new >= withdrawable_quote_old);
        cvt_assert!(
            withdrawable_quote_new.saturating_sub(withdrawable_quote_old)
                == orderbook_quote_old.saturating_sub(orderbook_quote_new)
        );
        cvt_assert!(withdrawable_base_old == withdrawable_base_new);
        cvt_assert!(orderbook_base_old == orderbook_base_new);
    }
    cvt_assert!(vault_base_old == vault_base_new);
    cvt_assert!(vault_quote_old == vault_quote_new);
    cvt_assert!(
        withdrawable_base_old.saturating_add(orderbook_base_old)
            == withdrawable_base_new.saturating_add(orderbook_base_new)
    );
    cvt_assert!(
        withdrawable_quote_old.saturating_add(orderbook_quote_old)
            == withdrawable_quote_new.saturating_add(orderbook_quote_new)
    );
}

pub fn cvt_assert_deposit_extra<const IS_BASE: bool>(
    balances_old: AllBalances,
    balances_new: AllBalances,
    amount: u64,
) {
    let AllBalances {
        vault_base: vault_base_old,
        vault_quote: vault_quote_old,
        withdrawable_base: withdrawable_base_old,
        orderbook_base: orderbook_base_old,
        withdrawable_quote: withdrawable_quote_old,
        orderbook_quote: orderbook_quote_old,
        trader_base: trader_base_old,
        trader_quote: trader_quote_old,
        maker_trader_base: _maker_trader_base_old,
        maker_trader_quote: _maker_trader_quote_old,
        maker_order_base: _maker_order_base_old,
        maker_order_quote: _maker_order_quote_old,
    } = balances_old;

    let AllBalances {
        vault_base: vault_base_new,
        vault_quote: vault_quote_new,
        withdrawable_base: withdrawable_base_new,
        orderbook_base: orderbook_base_new,
        withdrawable_quote: withdrawable_quote_new,
        orderbook_quote: orderbook_quote_new,
        trader_base: trader_base_new,
        trader_quote: trader_quote_new,
        maker_trader_base: _maker_trader_base_new,
        maker_trader_quote: _maker_trader_quote_new,
        maker_order_base: _maker_order_base_new,
        maker_order_quote: _maker_order_quote_new,
    } = balances_new;

    cvt_assert!(orderbook_base_old == orderbook_base_new);
    cvt_assert!(orderbook_quote_old == orderbook_quote_new);
    if IS_BASE {
        cvt_assert!(trader_quote_new == trader_quote_old);
        cvt_assert!(withdrawable_quote_new == withdrawable_quote_old);
        cvt_assert!(vault_quote_new == vault_quote_old);
        cvt_assert!(trader_base_old.saturating_add(amount) == trader_base_new);
        cvt_assert!(vault_base_old.saturating_add(amount) == vault_base_new);
    } else {
        cvt_assert!(trader_base_new == trader_base_old);
        cvt_assert!(withdrawable_base_new == withdrawable_base_old);
        cvt_assert!(vault_base_new == vault_base_old);
        cvt_assert!(trader_quote_old.saturating_add(amount) == trader_quote_new);
        cvt_assert!(vault_quote_old.saturating_add(amount) == vault_quote_new);
    }
}

pub fn cvt_assert_withdraw_extra<const IS_BASE: bool>(
    balances_old: AllBalances,
    balances_new: AllBalances,
    amount: u64,
) {
    let AllBalances {
        vault_base: vault_base_old,
        vault_quote: vault_quote_old,
        withdrawable_base: withdrawable_base_old,
        orderbook_base: orderbook_base_old,
        withdrawable_quote: withdrawable_quote_old,
        orderbook_quote: orderbook_quote_old,
        trader_base: trader_base_old,
        trader_quote: trader_quote_old,
        maker_trader_base: _maker_trader_base_old,
        maker_trader_quote: _maker_trader_quote_old,
        maker_order_base: _maker_order_base_old,
        maker_order_quote: _maker_order_quote_old,
    } = balances_old;

    let AllBalances {
        vault_base: vault_base_new,
        vault_quote: vault_quote_new,
        withdrawable_base: withdrawable_base_new,
        orderbook_base: orderbook_base_new,
        withdrawable_quote: withdrawable_quote_new,
        orderbook_quote: orderbook_quote_new,
        trader_base: trader_base_new,
        trader_quote: trader_quote_new,
        maker_trader_base: _maker_trader_base_new,
        maker_trader_quote: _maker_trader_quote_new,
        maker_order_base: _maker_order_base_new,
        maker_order_quote: _maker_order_quote_new,
    } = balances_new;

    cvt_assert!(orderbook_base_old == orderbook_base_new);
    cvt_assert!(orderbook_quote_old == orderbook_quote_new);
    if IS_BASE {
        cvt_assert!(trader_quote_new == trader_quote_old);
        cvt_assert!(withdrawable_quote_new == withdrawable_quote_old);
        cvt_assert!(vault_quote_new == vault_quote_old);
        cvt_assert!(trader_base_old.saturating_sub(amount) == trader_base_new);
        cvt_assert!(vault_base_old.saturating_sub(amount) == vault_base_new);
    } else {
        cvt_assert!(trader_base_new == trader_base_old);
        cvt_assert!(withdrawable_base_new == withdrawable_base_old);
        cvt_assert!(vault_base_new == vault_base_old);
        cvt_assert!(trader_quote_old.saturating_sub(amount) == trader_quote_new);
        cvt_assert!(vault_quote_old.saturating_sub(amount) == vault_quote_new);
    }
}
