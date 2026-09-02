use crate::certora::hooks::*;
use hook_macro::cvt_hook_end;
use nondet::nondet;

use crate::{
    state::{AddOrderToMarketArgs, AddOrderToMarketResult, MarketRefMut},
    validation::loaders::GlobalTradeAccounts,
};
use hypertree::DataIndex;
use solana_program::{entrypoint::ProgramResult, program_error::ProgramError};

#[cfg_attr(feature = "certora", cvt_hook_end(cancel_order_was_called()))]
pub fn mock_cancel_order(
    _dynamic_account: &MarketRefMut,
    _trader_index: DataIndex,
    _order_sequence_number: u64,
    _global_trade_accounts_opts: &[Option<GlobalTradeAccounts>; 2],
) -> ProgramResult {
    Ok(())
}

#[cfg_attr(feature = "certora", cvt_hook_end(place_order_was_called()))]
pub fn mock_place_order(
    _dynamic_account: &MarketRefMut,
    _args: AddOrderToMarketArgs,
) -> Result<AddOrderToMarketResult, ProgramError> {
    Ok(AddOrderToMarketResult {
        order_sequence_number: nondet(),
        order_index: nondet(),
        base_atoms_traded: nondet(),
        quote_atoms_traded: nondet(),
    })
}
