use std::mem::size_of;

use borsh::{BorshDeserialize, BorshSerialize};
use hypertree::{
    get_helper, get_mut_helper, trace, DataIndex, FreeList, HyperTreeReadOperations,
    HyperTreeWriteOperations, RBNode, NIL,
};
use manifest::{
    program::{
        batch_update::{BatchUpdateParams, BatchUpdateReturn, PlaceOrderParams},
        claim_seat_instruction, deposit_instruction, expand_market_instruction,
        get_dynamic_account, get_mut_dynamic_account, invoke, ManifestInstruction,
    },
    quantities::{BaseAtoms, QuoteAtoms, QuoteAtomsPerBaseAtom, WrapperU64},
    require,
    state::{claimed_seat::ClaimedSeat, DynamicAccount, MarketFixed, MarketRef, OrderType},
    validation::{next_account_info, AccountViewExt, ManifestAccountInfo, Program, Signer},
};
use pinocchio::{
    account::{AccountView, Ref, RefMut},
    error::ProgramError,
    sysvars::Sysvar,
    ProgramResult,
};
use solana_program::{
    instruction::{AccountMeta, Instruction},
    program::get_return_data,
    pubkey::Pubkey,
    system_program,
};
use spl_token_2022::{
    extension::{
        transfer_fee::TransferFeeConfig, transfer_hook::TransferHook, BaseStateWithExtensions,
        StateWithExtensions,
    },
    state::Mint,
};

use crate::{
    error::ManifestWrapperError::InvalidDepositAccounts, market_info::MarketInfo,
    open_order::WrapperOpenOrder, wrapper_user::ManifestWrapperUserFixed,
};

use super::shared::{
    check_signer, expand_wrapper_if_needed, get_market_info_index_for_market, sync_fast,
    MarketInfosTree, OpenOrdersTree, UnusedWrapperFreeListPadding, WrapperStateAccountInfo,
};

#[derive(BorshDeserialize, BorshSerialize, Clone)]
pub struct WrapperPlaceOrderParams {
    client_order_id: u64,
    base_atoms: u64,
    price_mantissa: u32,
    price_exponent: i8,
    is_bid: bool,
    last_valid_slot: u32,
    order_type: OrderType,
}
impl WrapperPlaceOrderParams {
    pub fn new(
        client_order_id: u64,
        base_atoms: u64,
        price_mantissa: u32,
        price_exponent: i8,
        is_bid: bool,
        last_valid_slot: u32,
        order_type: OrderType,
    ) -> Self {
        WrapperPlaceOrderParams {
            client_order_id,
            base_atoms,
            price_mantissa,
            price_exponent,
            is_bid,
            last_valid_slot,
            order_type,
        }
    }
}

impl Into<PlaceOrderParams> for WrapperPlaceOrderParams {
    fn into(self) -> PlaceOrderParams {
        PlaceOrderParams::new(
            self.base_atoms,
            self.price_mantissa,
            self.price_exponent,
            self.is_bid,
            self.order_type,
            self.last_valid_slot,
        )
    }
}

// Call expand so core has enough free space and owner doesn't get charged
// rent on a subsequent operation. This allows to keep payer and owner
// separate in the case of PDA owners.
fn expand_market_if_needed<'a>(
    market: &ManifestAccountInfo<'a, MarketFixed>,
    payer: &Signer<'a>,
    manifest_program: &Program<'a>,
    system_program: &Program<'a>,
) -> ProgramResult {
    let market_data: Ref<[u8]> = market.try_borrow()?;
    let dynamic_account: MarketRef = get_dynamic_account(&market_data);
    // Check for two free blocks, bc. there needs to be always one free block
    // after every operation.
    if !dynamic_account.has_two_free_blocks() {
        drop(market_data);
        invoke(
            &expand_market_instruction(market.pubkey(), payer.pubkey()),
            &[
                manifest_program.info,
                payer.info,
                market.info,
                system_program.info,
            ],
        )?
    }
    Ok(())
}

fn get_or_create_trader_index<'a>(
    market: &ManifestAccountInfo<'a, MarketFixed>,
    owner: &Signer<'a>,
    payer: &Signer<'a>,
    manifest_program: &Program<'a>,
    system_program: &Program<'a>,
) -> Result<DataIndex, ProgramError> {
    let trader_index: DataIndex = {
        let market_data: &Ref<[u8]> = &market.try_borrow()?;
        let dynamic_account: MarketRef = get_dynamic_account(market_data);
        dynamic_account.get_trader_index(owner.pubkey())
    };

    if trader_index != NIL {
        // If core seat was already initialized, nothing to do here.
        Ok(trader_index)
    } else {
        // Need to intialize a new seat on core.
        expand_market_if_needed(market, payer, manifest_program, system_program)?;
        invoke(
            &claim_seat_instruction(market.pubkey(), owner.pubkey()),
            &[
                manifest_program.info,
                owner.info,
                market.info,
                system_program.info,
            ],
        )?;

        // Fetch newly assigned trader index after claiming core seat.
        let market_data: &Ref<[u8]> = &mut market.try_borrow()?;
        let dynamic_account: MarketRef = get_dynamic_account(market_data);
        Ok(dynamic_account.get_trader_index(owner.pubkey()))
    }
}

fn get_or_create_market_info<'a>(
    wrapper_state: &WrapperStateAccountInfo<'a>,
    market: &ManifestAccountInfo<'a, MarketFixed>,
    payer: &Signer<'a>,
    system_program: &Program<'a>,
    trader_index: u32,
) -> Result<(MarketInfo, DataIndex), ProgramError> {
    let market_info_index: DataIndex =
        get_market_info_index_for_market(&wrapper_state, market.pubkey());
    if market_info_index != NIL {
        // Do an initial sync to get all existing orders and balances fresh. This is
        // needed for modifying user orders for insufficient funds.
        sync_fast(&wrapper_state, &market, market_info_index)?;

        let wrapper_data: Ref<[u8]> = wrapper_state.info.try_borrow()?;
        let (_fixed_data, wrapper_dynamic_data) =
            wrapper_data.split_at(size_of::<ManifestWrapperUserFixed>());

        let market_info: MarketInfo =
            *get_helper::<RBNode<MarketInfo>>(wrapper_dynamic_data, market_info_index).get_value();

        Ok((market_info, market_info_index))
    } else {
        // Market info not found, create a new one in wrapper.
        expand_wrapper_if_needed(&wrapper_state, &payer, &system_program)?;

        // Load the market_infos tree and insert a new one.
        let wrapper_state_info: &AccountView = wrapper_state.info;
        let mut wrapper_data: RefMut<[u8]> = wrapper_state_info.try_borrow_mut()?;
        let (fixed_data, wrapper_dynamic_data) =
            wrapper_data.split_at_mut(size_of::<ManifestWrapperUserFixed>());
        let wrapper_fixed: &mut ManifestWrapperUserFixed = get_mut_helper(fixed_data, 0);
        let mut market_info: MarketInfo = MarketInfo::new_empty(*market.pubkey(), trader_index);
        market_info.quote_volume = {
            // Sync volume from core seat to prevent double billing if seat
            // existed before wrapper invocation
            let market_data: &Ref<[u8]> = &market.try_borrow()?;
            let dynamic_account: MarketRef = get_dynamic_account(market_data);
            let claimed_seat: &ClaimedSeat =
                get_helper::<RBNode<ClaimedSeat>>(dynamic_account.dynamic, trader_index)
                    .get_value();
            claimed_seat.quote_volume
        };

        // Put that market_info at the free list head.
        let mut free_list: FreeList<UnusedWrapperFreeListPadding> =
            FreeList::new(wrapper_dynamic_data, wrapper_fixed.free_list_head_index);
        let market_info_index: DataIndex = free_list.remove();
        wrapper_fixed.free_list_head_index = free_list.get_head();

        // Insert into the MarketInfosTree.
        let mut market_infos_tree: MarketInfosTree = MarketInfosTree::new(
            wrapper_dynamic_data,
            wrapper_fixed.market_infos_root_index,
            NIL,
        );
        market_infos_tree.insert(market_info_index, market_info);
        wrapper_fixed.market_infos_root_index = market_infos_tree.get_root_index();

        Ok((market_info, market_info_index))
    }
}

pub(crate) fn process_place_order(
    _program_id: &Pubkey,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let account_iter: &mut std::slice::Iter<AccountView> = &mut accounts.iter();
    let wrapper_state: WrapperStateAccountInfo =
        WrapperStateAccountInfo::new(next_account_info(account_iter)?)?;
    let owner: Signer = Signer::new(next_account_info(account_iter)?)?;
    let trader_token_account: &AccountView = next_account_info(account_iter)?;
    let market: ManifestAccountInfo<MarketFixed> =
        ManifestAccountInfo::<MarketFixed>::new(next_account_info(account_iter)?)?;
    let vault: &AccountView = next_account_info(account_iter)?;
    let mint: &AccountView = next_account_info(account_iter)?;
    let system_program: Program =
        Program::new(next_account_info(account_iter)?, &system_program::id())?;
    let token_program: &AccountView = next_account_info(account_iter)?;
    let manifest_program: Program =
        Program::new(next_account_info(account_iter)?, &manifest::id())?;
    let payer: Signer = Signer::new(next_account_info(account_iter)?)?;

    check_signer(&wrapper_state, owner.pubkey());

    // Ensure ClaimedSeat in core and MarketInfo in wrapper are allocated.
    // Syncs MarketInfo from ClaimedSeat to calculate required deposits.
    let trader_index =
        get_or_create_trader_index(&market, &owner, &payer, &manifest_program, &system_program)?;
    let (market_info, market_info_index) = get_or_create_market_info(
        &wrapper_state,
        &market,
        &payer,
        &system_program,
        trader_index,
    )?;
    let remaining_base_atoms: BaseAtoms = market_info.base_balance;
    let remaining_quote_atoms: QuoteAtoms = market_info.quote_balance;

    let order = WrapperPlaceOrderParams::try_from_slice(data)
        .map_err(manifest::validation::io_to_program_error)?;
    let base_atoms = BaseAtoms::new(order.base_atoms);
    let price = QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(
        order.price_mantissa,
        order.price_exponent,
    )?;

    let missing_amount_atoms: u64 = if order.is_bid {
        // Core CPI verifies token account / vault consistency with mint.
        require!(
            mint.pubkey().eq(market.get_fixed()?.get_quote_mint()),
            InvalidDepositAccounts,
            "expected market.quote_mint as deposit mint"
        )?;
        let required_quote_atoms = base_atoms.checked_mul(price, true)?;
        required_quote_atoms
            .saturating_sub(remaining_quote_atoms)
            .as_u64()
    } else {
        // Core CPI verifies token account / vault consistency with mint.
        require!(
            mint.pubkey().eq(market.get_fixed()?.get_base_mint()),
            InvalidDepositAccounts,
            "expected market.base_mint as deposit mint"
        )?;
        base_atoms.saturating_sub(remaining_base_atoms).as_u64()
    };

    // Adjust deposited amount for TransferFee if possible.
    let deposit_amount_atoms = if *mint.owner_pubkey() == spl_token_2022::id() {
        let mint_data: Ref<[u8]> = mint.try_borrow()?;
        let deposit_mint: StateWithExtensions<'_, Mint> =
            StateWithExtensions::<Mint>::unpack(&mint_data)
                .map_err(manifest::validation::to_program_error)?;

        if let Ok(extension) = deposit_mint.get_extension::<TransferHook>() {
            if !extension.program_id.0.eq(&Pubkey::default()) {
                solana_program::msg!(
                    "Warning, you are placing an order while using TransferHook. There is no accurate way to estimate deposits required for this trade. You might need to manually deposit using the core instruction before placing orders."
                );
            }
        }

        if let Ok(extension) = deposit_mint.get_extension::<TransferFeeConfig>() {
            let epoch_fee = extension.get_epoch_fee(pinocchio::sysvars::clock::Clock::get()?.epoch);
            epoch_fee
                .calculate_pre_fee_amount(missing_amount_atoms)
                .unwrap()
        } else {
            missing_amount_atoms
        }
    } else {
        missing_amount_atoms
    };

    trace!(
        "deposit amount:{deposit_amount_atoms} to cover missing: {missing_amount_atoms} mint:{:?}",
        mint.pubkey()
    );
    if deposit_amount_atoms > 0 {
        invoke(
            &deposit_instruction(
                market.pubkey(),
                owner.pubkey(),
                mint.pubkey(),
                deposit_amount_atoms,
                trader_token_account.pubkey(),
                *token_program.pubkey(),
                Some(trader_index),
            ),
            &[
                manifest_program.info,
                owner.info,
                market.info,
                trader_token_account,
                vault,
                token_program,
                mint,
            ],
        )?;
    }

    expand_market_if_needed(&market, &payer, &manifest_program, &system_program)?;

    // Call batch update and pass unparsed accounts without verifying them
    {
        let core_place: PlaceOrderParams = order.clone().into();
        trace!("cpi place {core_place:?}");

        let mut account_metas = Vec::with_capacity(13);
        account_metas.extend_from_slice(&[
            AccountMeta::new(*owner.pubkey(), true),
            AccountMeta::new(*market.pubkey(), false),
            AccountMeta::new_readonly(system_program::id(), false),
        ]);
        account_metas.extend(accounts[10..].iter().map(|ai| {
            if ai.is_writable() {
                AccountMeta::new(*ai.pubkey(), ai.is_signer())
            } else {
                AccountMeta::new_readonly(*ai.pubkey(), ai.is_signer())
            }
        }));

        let ix: Instruction = Instruction {
            program_id: manifest::id(),
            accounts: account_metas,
            data: [
                ManifestInstruction::BatchUpdate.to_vec(),
                BatchUpdateParams::new(Some(trader_index), vec![], vec![core_place])
                    .try_to_vec()
                    .map_err(manifest::validation::io_to_program_error)?,
            ]
            .concat(),
        };

        let mut account_infos = Vec::with_capacity(18);
        account_infos.extend_from_slice(&[
            system_program.info,
            manifest_program.info,
            owner.info,
            market.info,
        ]);
        account_infos.extend(accounts[10..].iter());

        invoke(&ix, &account_infos)?;
    }

    // Process the order result

    let cpi_return_data: Option<(Pubkey, Vec<u8>)> = get_return_data();
    let BatchUpdateReturn {
        orders: batch_update_orders,
    } = BatchUpdateReturn::try_from_slice(&cpi_return_data.unwrap().1[..])
        .map_err(manifest::validation::io_to_program_error)?;

    trace!("cpi return orders:{batch_update_orders:?}");

    let (order_sequence_number, order_index) = batch_update_orders[0];
    // Order index is NIL when it did not rest. In that case, do not need to store in wrapper.
    if order_index != NIL {
        expand_wrapper_if_needed(&wrapper_state, &payer, &system_program)?;

        let mut wrapper_data: RefMut<[u8]> = wrapper_state.info.try_borrow_mut().unwrap();
        let wrapper: DynamicAccount<&mut ManifestWrapperUserFixed, &mut [u8]> =
            get_mut_dynamic_account(&mut wrapper_data);

        let orders_root_index: DataIndex = {
            let market_info: &mut MarketInfo =
                get_mut_helper::<RBNode<MarketInfo>>(wrapper.dynamic, market_info_index)
                    .get_mut_value();
            market_info.orders_root_index
        };

        let wrapper_new_order_index: DataIndex = {
            let mut free_list: FreeList<UnusedWrapperFreeListPadding> =
                FreeList::new(wrapper.dynamic, wrapper.fixed.free_list_head_index);
            let new_index: DataIndex = free_list.remove();
            wrapper.fixed.free_list_head_index = free_list.get_head();
            new_index
        };

        let wrapper_order: WrapperOpenOrder = WrapperOpenOrder::new(
            order.client_order_id,
            order_sequence_number,
            price,
            // Base atoms can be wrong, will be fixed in the sync.
            order.base_atoms,
            order.last_valid_slot,
            order_index,
            order.is_bid,
            order.order_type,
        );

        let mut open_orders_tree: OpenOrdersTree =
            OpenOrdersTree::new(wrapper.dynamic, orders_root_index, NIL);
        open_orders_tree.insert(wrapper_new_order_index, wrapper_order);
        let new_root_index: DataIndex = open_orders_tree.get_root_index();
        let market_info: &mut MarketInfo =
            get_mut_helper::<RBNode<MarketInfo>>(wrapper.dynamic, market_info_index)
                .get_mut_value();
        market_info.orders_root_index = new_root_index;
    }

    // Sync to get the balance correct and remove any expired orders.
    sync_fast(&wrapper_state, &market, market_info_index)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::WrapperPlaceOrderParams;
    use manifest::{
        program::batch_update::PlaceOrderParams,
        quantities::{BaseAtoms, QuoteAtoms, WrapperU64},
        state::OrderType,
    };

    #[test]
    fn test_pass_order_params_to_core() {
        let wrapper_order =
            WrapperPlaceOrderParams::new(1, 2, 3, 4, true, 5, OrderType::ImmediateOrCancel);
        assert_eq!(wrapper_order.client_order_id, 1);

        let core_order: PlaceOrderParams = wrapper_order.into();
        assert_eq!(core_order.base_atoms(), 2);
        assert_eq!(
            core_order
                .try_price()
                .unwrap()
                .checked_quote_for_base(BaseAtoms::new(1), false)
                .unwrap(),
            QuoteAtoms::new(30_000)
        );
        assert_eq!(core_order.is_bid(), true);
        assert_eq!(core_order.order_type(), OrderType::ImmediateOrCancel);
        assert_eq!(core_order.last_valid_slot(), 5);
    }
}
