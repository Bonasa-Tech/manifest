use std::cell::RefMut;

use borsh::{BorshDeserialize, BorshSerialize};
use hypertree::{get_helper, trace, DataIndex, RBNode};
use solana_program::{account_info::AccountInfo, entrypoint::ProgramResult, pubkey::Pubkey};

use crate::{
    program::{batch_update::MarketDataTreeNodeType, get_mut_dynamic_account},
    quantities::{GlobalAtoms, WrapperU64},
    require,
    state::{utils::get_now_slot, GlobalRefMut, MarketRefMut, RestingOrder, MARKET_BLOCK_SIZE},
    validation::loaders::{GlobalCleanContext, GlobalTradeAccounts},
};

#[derive(BorshDeserialize, BorshSerialize)]
pub struct GlobalCleanParams {
    pub order_index: DataIndex,
}

impl GlobalCleanParams {
    pub fn new(order_index: DataIndex) -> Self {
        GlobalCleanParams { order_index }
    }
}

pub(crate) fn process_global_clean(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    trace!("process_global_clean accs={accounts:?}");
    let global_clean_context: GlobalCleanContext = GlobalCleanContext::load(accounts)?;

    let GlobalCleanContext {
        payer,
        market,
        global,
        system_program,
    } = global_clean_context;

    let global_trade_accounts: GlobalTradeAccounts = GlobalTradeAccounts {
        mint_opt: None,
        global: global.clone(),
        global_vault_opt: None,
        market_vault_opt: None,
        token_program_opt: None,
        system_program: Some(system_program),
        market: *market.key,
        gas_payer_opt: None,
        gas_receiver_opt: Some(payer),
    };

    let GlobalCleanParams { order_index } = GlobalCleanParams::try_from_slice(data)?;

    let market_data: &mut RefMut<&mut [u8]> = &mut market.try_borrow_mut_data()?;
    let mut market_dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);

    let global_data: &mut RefMut<&mut [u8]> = &mut global.try_borrow_mut_data()?;
    let global_dynamic_account: GlobalRefMut = get_mut_dynamic_account(global_data);

    // Get the resting order and do some checks to make sure the order index is
    // valid and that the global account is correct.
    let resting_order: &RestingOrder = {
        // Sanity check on the order index
        require!(
            order_index % (MARKET_BLOCK_SIZE as DataIndex) == 0,
            crate::program::ManifestError::WrongIndexHintParams,
            "Invalid order index {}",
            order_index,
        )?;
        let resting_order_node: &RBNode<RestingOrder> =
            get_helper::<RBNode<RestingOrder>>(&market_dynamic_account.dynamic, order_index);
        require!(
            resting_order_node.get_payload_type() == MarketDataTreeNodeType::RestingOrder as u8,
            crate::program::ManifestError::WrongIndexHintParams,
            "Invalid order index {}",
            order_index,
        )?;

        let resting_order: &RestingOrder = resting_order_node.get_value();
        let expected_global_mint: &Pubkey = if resting_order.get_is_bid() {
            market_dynamic_account.get_quote_mint()
        } else {
            market_dynamic_account.get_base_mint()
        };
        let global_mint: &Pubkey = global_dynamic_account.fixed.get_mint();

        // Verify that the resting order uses the global account given.
        require!(
            *expected_global_mint == *global_mint,
            crate::program::ManifestError::InvalidClean,
            "Wrong global provided",
        )?;

        // Do not need to require that the order is global. This ix is useful
        // for cleaning up markets with all expired orders.

        resting_order
    };
    let maker_index: DataIndex = resting_order.get_trader_index();
    let maker: &Pubkey = market_dynamic_account.get_trader_key_by_index(maker_index);

    // Verify that the RestingOrder is clean eligible
    let is_expired: bool = resting_order.is_expired(get_now_slot());
    // Balance is zero when evicted.
    let maker_global_balance: GlobalAtoms = global_dynamic_account.get_balance_atoms(maker);
    let required_global_atoms: u64 = if resting_order.get_is_bid() {
        resting_order
            .get_num_base_atoms()
            .checked_mul(resting_order.get_price(), true)
            .unwrap()
            .as_u64()
    } else {
        resting_order.get_num_base_atoms().as_u64()
    };

    require!(
        is_expired
            || (resting_order.is_global() && maker_global_balance.as_u64() < required_global_atoms),
        crate::program::ManifestError::InvalidClean,
        "Ineligible clean order index {}",
        order_index,
    )?;

    // Do the actual clean on the market.
    let global_trade_accounts: [Option<GlobalTradeAccounts>; 2] = if resting_order.get_is_bid() {
        [None, Some(global_trade_accounts)]
    } else {
        [Some(global_trade_accounts), None]
    };

    // Should drop global, but cancel_order_by_index actually does not need to
    // borrow in this case.

    market_dynamic_account.cancel_order_by_index(order_index, &global_trade_accounts)?;

    // The global account itself only accounting on remove_order is that it
    // tracks unclaimed gas deposits for informational purposes and this is
    // claiming anyways.

    Ok(())
}
