use crate::{
    quantities::{BaseAtoms, QuoteAtoms, WrapperU64},
    state::{
        main_trader_index, second_trader_index, update_balance, AddOrderToMarketArgs,
        AddOrderToMarketResult, MarketRefMut,
    },
};
use hypertree::DataIndex;
use nondet::nondet;
use solana_program::program_error::ProgramError;

/// This summary for place_order assumes that there is a matched order with a trader,
/// and its price is 1:1
#[cfg(feature = "certora")]
pub fn place_fully_match_order_with_same_base_and_quote(
    market: &mut MarketRefMut,
    args: AddOrderToMarketArgs,
) -> Result<AddOrderToMarketResult, ProgramError> {
    let is_bid: bool = args.is_bid;
    let num_base_atoms: BaseAtoms = args.num_base_atoms;
    let base_traded: u64 = nondet();
    let quote_traded: u64 = nondet();
    let base_atoms_traded: BaseAtoms = BaseAtoms::new(base_traded);
    let quote_atoms_traded: QuoteAtoms = QuoteAtoms::new(quote_traded);

    ////////////////////////////////////////
    // -- Assumptions for this summary
    ////////////////////////////////////////
    //
    // Any summary must satisfy this condition
    cvt::cvt_assume!(base_atoms_traded <= num_base_atoms);
    // Avoid underflow our ghost variables
    cvt::cvt_assume!(market.fixed.orderbook_base_atoms >= base_atoms_traded);
    cvt::cvt_assume!(market.fixed.orderbook_quote_atoms >= quote_atoms_traded);
    // Condition specific to this summary: we fix price 1:1
    cvt::cvt_assume!(base_traded == quote_traded);

    let trader_index: DataIndex = main_trader_index();
    let maker_trader_index: DataIndex = second_trader_index();

    update_balance(
        market.fixed,
        market.dynamic,
        trader_index,
        !is_bid,
        false,
        if is_bid {
            quote_atoms_traded.into()
        } else {
            base_atoms_traded.into()
        },
    )?;

    update_balance(
        market.fixed,
        market.dynamic,
        maker_trader_index,
        !is_bid,
        true,
        if is_bid {
            quote_atoms_traded.into()
        } else {
            base_atoms_traded.into()
        },
    )?;

    update_balance(
        market.fixed,
        market.dynamic,
        trader_index,
        is_bid,
        true,
        if is_bid {
            base_atoms_traded.into()
        } else {
            quote_atoms_traded.into()
        },
    )?;

    // This code depends on the price. This is fine for 1:1 price
    // Otherwise, the amount in quote should be the amount in base multiplied by price
    if is_bid {
        market.fixed.orderbook_base_atoms = market
            .fixed
            .get_orderbook_base_atoms()
            .saturating_sub(base_atoms_traded);
    } else {
        market.fixed.orderbook_quote_atoms = market
            .fixed
            .get_orderbook_quote_atoms()
            .saturating_sub(quote_atoms_traded);
    }

    let order_sequence_number: u64 = nondet();
    let order_index: DataIndex = nondet();
    Ok(AddOrderToMarketResult {
        order_sequence_number,
        order_index,
        base_atoms_traded,
        quote_atoms_traded,
    })
}
