use pinocchio::{account::AccountView, ProgramResult};
use solana_program::pubkey::Pubkey;

use crate::{
    program::{batch_expand_market, get_dynamic_account},
    state::MarketRef,
    validation::loaders::ExpandMarketContext,
};

use super::expand_market;

pub(crate) fn process_expand_market(
    _program_id: &Pubkey,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let expand_market_context: ExpandMarketContext = ExpandMarketContext::load(accounts)?;
    let ExpandMarketContext { market, payer, .. } = expand_market_context;

    match data.first_chunk::<4>() {
        Some(data) => {
            let num_free_blocks_required = u32::from_le_bytes(*data);
            if let Some(blocks_missing) = {
                let market_data: pinocchio::account::Ref<[u8]> = market.try_borrow()?;
                let dynamic_account: MarketRef = get_dynamic_account(&market_data);
                dynamic_account.free_blocks_short_of_n(num_free_blocks_required)
            } {
                batch_expand_market(&payer, &market, blocks_missing)
            } else {
                Ok(())
            }
        }
        None => {
            let has_two_free_blocks: bool = {
                let market_data: pinocchio::account::Ref<[u8]> = market.try_borrow()?;
                let dynamic_account: MarketRef = get_dynamic_account(&market_data);
                dynamic_account.has_two_free_blocks()
            };

            if !has_two_free_blocks {
                expand_market(&payer, &market)
            } else {
                Ok(())
            }
        }
    }
}
