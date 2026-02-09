use std::cell::RefMut;

use crate::{
    logs::{emit_stack, PlaceOrderLogV2},
    program::expand_market_if_needed,
    quantities::{BaseAtoms, QuoteAtoms, QuoteAtomsPerBaseAtom, WrapperU64},
    require,
    state::{
        AddOrderToMarketArgs, AddOrderToMarketResult, MarketRefMut, OrderType,
        NO_EXPIRATION_LAST_VALID_SLOT,
    },
    validation::loaders::SwapContext,
};
#[cfg(not(feature = "certora"))]
use crate::{
    market_vault_seeds_with_bump,
    program::{invoke, ManifestError},
};
use borsh::{BorshDeserialize, BorshSerialize};
use hypertree::{trace, DataIndex, NIL};
use solana_program::{account_info::AccountInfo, entrypoint::ProgramResult, pubkey::Pubkey};

use super::shared::get_mut_dynamic_account;

#[cfg(feature = "certora")]
use {
    crate::certora::summaries::place_order::place_fully_match_order_with_same_base_and_quote,
    early_panic::early_panic,
    solana_cvt::token::{spl_token_2022_transfer, spl_token_transfer},
};

use crate::validation::{MintAccountInfo, Signer, TokenAccountInfo, TokenProgram};
use solana_program::program_error::ProgramError;
#[cfg(not(feature = "certora"))]
use {
    crate::state::utils::get_now_epoch,
    spl_token_2022::{
        extension::{
            transfer_fee::TransferFeeConfig, BaseStateWithExtensions, StateWithExtensions,
        },
        state::Mint,
    },
};

#[derive(BorshDeserialize, BorshSerialize)]
pub struct SwapParams {
    pub in_atoms: u64,
    pub out_atoms: u64,
    pub is_base_in: bool,
    // Exact in is a technical term that doesnt actually mean exact. It is
    // desired. If not that much can be fulfilled, less will be allowed assuming
    // the min_out/max_in is satisfied.
    pub is_exact_in: bool,
}

impl SwapParams {
    pub fn new(in_atoms: u64, out_atoms: u64, is_base_in: bool, is_exact_in: bool) -> Self {
        SwapParams {
            in_atoms,
            out_atoms,
            is_base_in,
            is_exact_in,
        }
    }
}

pub(crate) fn process_swap(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let params = SwapParams::try_from_slice(data)?;
    process_swap_core(program_id, accounts, params)
}

#[cfg_attr(all(feature = "certora", not(feature = "certora-test")), early_panic)]
pub(crate) fn process_swap_core(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: SwapParams,
) -> ProgramResult {
    let swap_context: SwapContext = SwapContext::load(accounts)?;

    let SwapContext {
        market,
        payer,
        owner,
        trader_base: trader_base_account,
        trader_quote: trader_quote_account,
        base_vault,
        quote_vault,
        token_program_base,
        token_program_quote,
        base_mint,
        quote_mint,
        global_trade_accounts_opts,
    } = swap_context;

    let (existing_seat_index, trader_index, initial_base_atoms, initial_quote_atoms) = {
        let market_data: &mut RefMut<&mut [u8]> = &mut market.try_borrow_mut_data()?;
        let mut dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);

        // Claim seat if needed
        let existing_seat_index: DataIndex = dynamic_account.get_trader_index(owner.key);
        if existing_seat_index == NIL {
            dynamic_account.claim_seat(owner.key)?;
        }
        let trader_index: DataIndex = dynamic_account.get_trader_index(owner.key);

        let (initial_base_atoms, initial_quote_atoms) =
            dynamic_account.get_trader_balance(owner.key);

        (
            existing_seat_index,
            trader_index,
            initial_base_atoms,
            initial_quote_atoms,
        )
    };

    // Might need a free list spot for both the temporary claimed seat as well
    // as for a partially filled reverse order. The temporary claimed seat has
    // already been taken, so this is just checking if there is an additional
    // free block for the reverse order.
    expand_market_if_needed(&payer, &market)?;

    let market_data: &mut RefMut<&mut [u8]> = &mut market.try_borrow_mut_data()?;
    let mut dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);

    let SwapParams {
        in_atoms,
        out_atoms,
        is_base_in,
        is_exact_in,
    } = params;

    // Calculate in_atoms_after_transfer_fees after transfer fees for Token-2022 tokens.
    // This is the amount that will actually arrive at the vault.
    let in_atoms_after_transfer_fees: u64 = calculate_post_fee_amount(
        in_atoms,
        is_base_in,
        &token_program_base,
        &token_program_quote,
        &base_mint,
        &quote_mint,
    )?;

    // For exact_out (!is_exact_in), calculate how much the market needs to give
    // so that after transfer fees, the trader receives their desired out_atoms.
    let out_atoms_after_transfer_fees: u64 = if !is_exact_in {
        calculate_pre_fee_amount(
            out_atoms,
            is_base_in, // Output is opposite of input: if base_in, output is quote; if quote_in, output is base
            &token_program_base,
            &token_program_quote,
            &base_mint,
            &quote_mint,
        )?
    } else {
        out_atoms
    };

    trace!("swap in_atoms:{in_atoms} in_atoms_after_transfer_fees:{in_atoms_after_transfer_fees} out_atoms:{out_atoms} out_atoms_after_transfer_fees:{out_atoms_after_transfer_fees} is_base_in:{is_base_in} is_exact_in:{is_exact_in}");

    // This check is redundant with the check that will be done within token
    // program on deposit, but it is done here to future proof in case we later
    // remove checked math.
    // This actually adds a new restriction that the wallet can fully fund the
    // swap instead of a combination of wallet and existing withdrawable
    // balance.
    if is_exact_in {
        if is_base_in {
            require!(
                in_atoms_after_transfer_fees <= trader_base_account.get_balance_atoms(),
                ManifestError::Overflow,
                "Insufficient base in atoms for swap has: {} requires: {}",
                trader_base_account.get_balance_atoms(),
                in_atoms_after_transfer_fees,
            )?;
        } else {
            require!(
                in_atoms_after_transfer_fees <= trader_quote_account.get_balance_atoms(),
                ManifestError::Overflow,
                "Insufficient quote in atoms for swap has: {} requires: {}",
                trader_quote_account.get_balance_atoms(),
                in_atoms_after_transfer_fees,
            )?;
        }
    }

    // this is a virtual credit to ensure matching always proceeds
    // net token transfers will be handled later
    // Use in_atoms_after_transfer_fees (post-fee) to account for Token-2022 transfer fees
    dynamic_account.deposit(trader_index, in_atoms_after_transfer_fees, is_base_in)?;

    // 4 cases:
    // 1. Exact in base. Simplest case, just use the base atoms given.
    // 2. Exact in quote. Search the asks for the number of base atoms in bids to match.
    // 3. Exact out quote. Search the bids for the number of base atoms needed to match to get the right quote out.
    // 4. Exact out base. Use the number of out atoms as the number of atoms to place_order against.
    // Note: For exact_in cases, we use in_atoms_after_transfer_fees (post-fee) since that's what's available to trade.
    let base_atoms: BaseAtoms = if is_exact_in {
        if is_base_in {
            // input=desired(base) output=min(quote)
            BaseAtoms::new(in_atoms_after_transfer_fees)
        } else {
            // input=desired(quote)* output=min(base)
            // round down base amount to not cross quote limit
            dynamic_account.impact_base_atoms(
                true,
                QuoteAtoms::new(in_atoms_after_transfer_fees),
                &global_trade_accounts_opts,
            )?
        }
    } else {
        if is_base_in {
            // input=max(base) output=desired(quote)
            // round up base amount to ensure not staying below quote limit
            // Use out_atoms_after_transfer_fees to account for transfer fees on output
            dynamic_account.impact_base_atoms(
                false,
                QuoteAtoms::new(out_atoms_after_transfer_fees),
                &global_trade_accounts_opts,
            )?
        } else {
            // input=max(quote) output=desired(base)
            // Use out_atoms_after_transfer_fees to account for transfer fees on output
            BaseAtoms::new(out_atoms_after_transfer_fees)
        }
    };

    // Note that in the case of fully exhausting the book, exact in/out will not
    // be respected. It should be treated as a desired in/out. This pushes the
    // burden of checking the results onto the caller program.

    // Example case is exact quote in. User wants exact quote in of 1_000_000
    // and min base out of 1_000. Suppose they fully exhaust the book and get
    // out 2_000 but that is not enough to fully use the entire 1_000_000. In
    // this case the ix will succeed.

    // Another interesting case is exact quote out. Suppose the user is doing
    // exact quote out 1_000_000 with max_base_in of 1_000. If it fully exhausts
    // the book without using the entire max_base_in and that is still not
    // enough for the exact quote amount, the transaction will still succeed.

    let price: QuoteAtomsPerBaseAtom = if is_base_in {
        QuoteAtomsPerBaseAtom::MIN
    } else {
        QuoteAtomsPerBaseAtom::MAX
    };
    let last_valid_slot: u32 = NO_EXPIRATION_LAST_VALID_SLOT;
    let order_type: OrderType = OrderType::ImmediateOrCancel;

    trace!("swap in:{in_atoms} out:{out_atoms} base/quote:{is_base_in} in/out:{is_exact_in} base:{base_atoms} price:{price}",);

    let AddOrderToMarketResult {
        base_atoms_traded,
        quote_atoms_traded,
        order_sequence_number,
        order_index,
        ..
    } = place_order(
        &mut dynamic_account,
        AddOrderToMarketArgs {
            market: *market.key,
            trader_index,
            num_base_atoms: base_atoms,
            price,
            is_bid: !is_base_in,
            last_valid_slot,
            order_type,
            global_trade_accounts_opts: &global_trade_accounts_opts,
            current_slot: None,
        },
    )?;

    if is_exact_in {
        let out_atoms_traded: u64 = if is_base_in {
            quote_atoms_traded.as_u64()
        } else {
            base_atoms_traded.as_u64()
        };
        // Note that we define the spec as the out amount verified against is
        // the amount taken from the market, not the amount actually received.
        // These are the same except when there are transfer fees.
        require!(
            out_atoms <= out_atoms_traded,
            ManifestError::InsufficientOut,
            "Insufficient out atoms returned. Minimum: {} Actual: {}",
            out_atoms,
            out_atoms_traded
        )?;
    } else {
        let in_atoms_traded = if is_base_in {
            base_atoms_traded.as_u64()
        } else {
            quote_atoms_traded.as_u64()
        };
        require!(
            in_atoms >= in_atoms_traded,
            ManifestError::InsufficientOut,
            "Excessive in atoms charged. Maximum: {} Actual: {}",
            in_atoms,
            in_atoms_traded
        )?;
    }

    let (end_base_atoms, end_quote_atoms) = dynamic_account.get_trader_balance(owner.key);

    let extra_base_atoms: BaseAtoms = end_base_atoms.checked_sub(initial_base_atoms)?;
    let extra_quote_atoms: QuoteAtoms = end_quote_atoms.checked_sub(initial_quote_atoms)?;

    // Transfer tokens
    if is_base_in {
        // Trader is depositing base.

        // Summary: This takes extra on edge case of is_exact_in=false and
        // transfer fee != 0.
        // Because of rounding, it is difficult to calculate efficiently the
        // amount that should be taken when is_exact_in = false and transfer
        // fees. In that case, the we calculated the matching with all of the
        // input after fees. The result here is that we will take extra from the
        // user and be lost to the vault in the amount of the fees on
        // extra_base_atoms. This cannot be simply subtracted due to rounding
        // issues possibly going against the vault and resulting in loss of
        // funds.
        let base_atoms_to_transfer: u64 = in_atoms.saturating_sub(extra_base_atoms.as_u64());

        if *token_program_base.key == spl_token_2022::id() {
            spl_token_2022_transfer_from_trader_to_vault(
                &token_program_base,
                &trader_base_account,
                base_mint,
                dynamic_account.fixed.get_base_mint(),
                &base_vault,
                &owner,
                base_atoms_to_transfer,
                dynamic_account.fixed.get_base_mint_decimals(),
            )?;
        } else {
            spl_token_transfer_from_trader_to_vault(
                &token_program_base,
                &trader_base_account,
                &base_vault,
                &owner,
                base_atoms_to_transfer,
            )?;
        }

        // Give all but what started there.
        let quote_vault_bump: u8 = dynamic_account.fixed.get_quote_vault_bump();
        if *token_program_quote.key == spl_token_2022::id() {
            spl_token_2022_transfer_from_vault_to_trader(
                &token_program_quote,
                quote_mint,
                dynamic_account.fixed.get_quote_mint(),
                &quote_vault,
                &trader_quote_account,
                extra_quote_atoms.as_u64(),
                dynamic_account.fixed.get_quote_mint_decimals(),
                market.key,
                quote_vault_bump,
            )?;
        } else {
            spl_token_transfer_from_vault_to_trader(
                &token_program_quote,
                &quote_vault,
                &trader_quote_account,
                extra_quote_atoms.as_u64(),
                market.key,
                quote_vault_bump,
                dynamic_account.fixed.get_quote_mint(),
            )?;
        }
    } else {
        // Trader is depositing quote.

        // Same comment as above for why this takes extra on edge case of
        // is_exact_in=false and transfer fee != 0.
        let quote_atoms_to_transfer: u64 = in_atoms.saturating_sub(extra_quote_atoms.as_u64());

        if *token_program_quote.key == spl_token_2022::id() {
            spl_token_2022_transfer_from_trader_to_vault(
                &token_program_quote,
                &trader_quote_account,
                quote_mint,
                dynamic_account.fixed.get_quote_mint(),
                &quote_vault,
                &owner,
                quote_atoms_to_transfer,
                dynamic_account.fixed.get_quote_mint_decimals(),
            )?;
        } else {
            spl_token_transfer_from_trader_to_vault(
                &token_program_quote,
                &trader_quote_account,
                &quote_vault,
                &owner,
                quote_atoms_to_transfer,
            )?;
        }

        // Give all but what started there.
        let base_vault_bump: u8 = dynamic_account.fixed.get_base_vault_bump();
        if *token_program_base.key == spl_token_2022::id() {
            spl_token_2022_transfer_from_vault_to_trader(
                &token_program_base,
                base_mint,
                dynamic_account.get_base_mint(),
                &base_vault,
                &trader_base_account,
                extra_base_atoms.as_u64(),
                dynamic_account.fixed.get_base_mint_decimals(),
                market.key,
                base_vault_bump,
            )?;
        } else {
            spl_token_transfer_from_vault_to_trader(
                &token_program_base,
                &base_vault,
                &trader_base_account,
                extra_base_atoms.as_u64(),
                market.key,
                base_vault_bump,
                dynamic_account.get_base_mint(),
            )?;
        }
    }

    if existing_seat_index == NIL {
        dynamic_account.release_seat(owner.key)?;
    } else {
        // Withdraw in case there already was a seat so it doesnt mess with their
        // balances. Need to withdraw base and quote in case the order wasnt fully
        // filled.
        dynamic_account.withdraw(trader_index, extra_base_atoms.as_u64(), true)?;
        dynamic_account.withdraw(trader_index, extra_quote_atoms.as_u64(), false)?;
    }
    // Verify that there wasnt a reverse order that took the only spare block.
    require!(
        dynamic_account.has_free_block(),
        ManifestError::InvalidFreeList,
        "Cannot swap against a reverse order unless there is a free block"
    )?;

    emit_stack(PlaceOrderLogV2 {
        market: *market.key,
        trader: *owner.key,
        payer: *payer.key,
        base_atoms,
        price,
        order_type,
        is_bid: (!is_base_in).into(),
        _padding: [0; 6],
        order_sequence_number,
        order_index,
        last_valid_slot,
    })?;

    Ok(())
}

#[cfg(not(feature = "certora"))]
fn place_order(
    dynamic_account: &mut MarketRefMut,
    args: AddOrderToMarketArgs,
) -> Result<AddOrderToMarketResult, ProgramError> {
    dynamic_account.place_order(args)
}

#[cfg(feature = "certora")]
fn place_order(
    market: &mut MarketRefMut,
    args: AddOrderToMarketArgs,
) -> Result<AddOrderToMarketResult, ProgramError> {
    place_fully_match_order_with_same_base_and_quote(market, args)
}

/** Transfer from base (quote) trader to base (quote) vault using SPL Token **/
#[cfg(not(feature = "certora"))]
fn spl_token_transfer_from_trader_to_vault<'a, 'info>(
    token_program: &TokenProgram<'a, 'info>,
    trader_account: &TokenAccountInfo<'a, 'info>,
    vault: &TokenAccountInfo<'a, 'info>,
    owner: &Signer<'a, 'info>,
    amount: u64,
) -> ProgramResult {
    invoke(
        &spl_token::instruction::transfer(
            token_program.key,
            trader_account.key,
            vault.key,
            owner.key,
            &[],
            amount,
        )?,
        &[
            token_program.as_ref().clone(),
            trader_account.as_ref().clone(),
            vault.as_ref().clone(),
            owner.as_ref().clone(),
        ],
    )
}
#[cfg(feature = "certora")]
/** (Summary) Transfer from base (quote) trader to base (quote) vault using SPL Token **/
fn spl_token_transfer_from_trader_to_vault<'a, 'info>(
    _token_program: &TokenProgram<'a, 'info>,
    trader_account: &TokenAccountInfo<'a, 'info>,
    vault: &TokenAccountInfo<'a, 'info>,
    owner: &Signer<'a, 'info>,
    amount: u64,
) -> ProgramResult {
    spl_token_transfer(trader_account.info, vault.info, owner.info, amount)
}

/** Transfer from base (quote) trader to base (quote) vault using SPL Token 2022 **/
#[cfg(not(feature = "certora"))]
fn spl_token_2022_transfer_from_trader_to_vault<'a, 'info>(
    token_program: &TokenProgram<'a, 'info>,
    trader_account: &TokenAccountInfo<'a, 'info>,
    mint: Option<MintAccountInfo<'a, 'info>>,
    mint_pubkey: &Pubkey,
    vault: &TokenAccountInfo<'a, 'info>,
    owner: &Signer<'a, 'info>,
    amount: u64,
    decimals: u8,
) -> ProgramResult {
    invoke(
        &spl_token_2022::instruction::transfer_checked(
            token_program.key,
            trader_account.key,
            mint_pubkey,
            vault.key,
            owner.key,
            &[],
            amount,
            decimals,
        )?,
        &[
            token_program.as_ref().clone(),
            trader_account.as_ref().clone(),
            vault.as_ref().clone(),
            mint.unwrap().as_ref().clone(),
            owner.as_ref().clone(),
        ],
    )
}

#[cfg(feature = "certora")]
/** (Summary) Transfer from base (quote) trader to base (quote) vault using SPL Token 2022 **/
fn spl_token_2022_transfer_from_trader_to_vault<'a, 'info>(
    _token_program: &TokenProgram<'a, 'info>,
    trader_account: &TokenAccountInfo<'a, 'info>,
    _mint: Option<MintAccountInfo<'a, 'info>>,
    _mint_pubkey: &Pubkey,
    vault: &TokenAccountInfo<'a, 'info>,
    owner: &Signer<'a, 'info>,
    amount: u64,
    _decimals: u8,
) -> ProgramResult {
    spl_token_2022_transfer(trader_account.info, vault.info, owner.info, amount)
}

/** Transfer from base (quote) vault to base (quote) trader using SPL Token **/
#[cfg(not(feature = "certora"))]
fn spl_token_transfer_from_vault_to_trader<'a, 'info>(
    token_program: &TokenProgram<'a, 'info>,
    vault: &TokenAccountInfo<'a, 'info>,
    trader_account: &TokenAccountInfo<'a, 'info>,
    amount: u64,
    market_key: &Pubkey,
    vault_bump: u8,
    mint_pubkey: &Pubkey,
) -> ProgramResult {
    solana_program::program::invoke_signed(
        &spl_token::instruction::transfer(
            token_program.key,
            vault.key,
            trader_account.key,
            vault.key,
            &[],
            amount,
        )?,
        &[
            token_program.as_ref().clone(),
            vault.as_ref().clone(),
            trader_account.as_ref().clone(),
        ],
        market_vault_seeds_with_bump!(market_key, mint_pubkey, vault_bump),
    )
}

#[cfg(feature = "certora")]
/** (Summary) Transfer from base (quote) vault to base (quote) trader using SPL Token **/
fn spl_token_transfer_from_vault_to_trader<'a, 'info>(
    _token_program: &TokenProgram<'a, 'info>,
    vault: &TokenAccountInfo<'a, 'info>,
    trader_account: &TokenAccountInfo<'a, 'info>,
    amount: u64,
    _market_key: &Pubkey,
    _vault_bump: u8,
    _mint_pubkey: &Pubkey,
) -> ProgramResult {
    spl_token_transfer(vault.info, trader_account.info, vault.info, amount)
}

/** Transfer from base (quote) vault to base (quote) trader using SPL Token 2022 **/
#[cfg(not(feature = "certora"))]
fn spl_token_2022_transfer_from_vault_to_trader<'a, 'info>(
    token_program: &TokenProgram<'a, 'info>,
    mint: Option<MintAccountInfo<'a, 'info>>,
    mint_pubkey: &Pubkey,
    vault: &TokenAccountInfo<'a, 'info>,
    trader_account: &TokenAccountInfo<'a, 'info>,
    amount: u64,
    decimals: u8,
    market_key: &Pubkey,
    vault_bump: u8,
) -> ProgramResult {
    solana_program::program::invoke_signed(
        &spl_token_2022::instruction::transfer_checked(
            token_program.key,
            vault.key,
            mint_pubkey,
            trader_account.key,
            vault.key,
            &[],
            amount,
            decimals,
        )?,
        &[
            token_program.as_ref().clone(),
            vault.as_ref().clone(),
            mint.unwrap().as_ref().clone(),
            trader_account.as_ref().clone(),
        ],
        market_vault_seeds_with_bump!(market_key, mint_pubkey, vault_bump),
    )
}

#[cfg(feature = "certora")]
/** (Summary) Transfer from base (quote) vault to base (quote) trader using SPL Token 2022 **/
fn spl_token_2022_transfer_from_vault_to_trader<'a, 'info>(
    _token_program: &TokenProgram<'a, 'info>,
    _mint: Option<MintAccountInfo<'a, 'info>>,
    _mint_pubkey: &Pubkey,
    vault: &TokenAccountInfo<'a, 'info>,
    trader_account: &TokenAccountInfo<'a, 'info>,
    amount: u64,
    _decimals: u8,
    _market_key: &Pubkey,
    _vault_bump: u8,
) -> ProgramResult {
    spl_token_2022_transfer(vault.info, trader_account.info, vault.info, amount)
}

/// Calculate the amount that will arrive at the vault after transfer fees.
/// For Token-2022 tokens with TransferFeeConfig, this accounts for the fee.
/// For regular SPL tokens or Token-2022 without fees, returns the original amount.
#[cfg(not(feature = "certora"))]
fn calculate_post_fee_amount<'a, 'info>(
    amount: u64,
    is_base_in: bool,
    token_program_base: &TokenProgram<'a, 'info>,
    token_program_quote: &TokenProgram<'a, 'info>,
    base_mint: &Option<MintAccountInfo<'a, 'info>>,
    quote_mint: &Option<MintAccountInfo<'a, 'info>>,
) -> Result<u64, ProgramError> {
    let (token_program, mint_opt) = if is_base_in {
        (token_program_base, base_mint)
    } else {
        (token_program_quote, quote_mint)
    };

    // Only Token-2022 can have transfer fees
    if *token_program.key != spl_token_2022::id() {
        return Ok(amount);
    }

    // If no mint info provided, assume no fee
    let Some(mint_info) = mint_opt else {
        return Ok(amount);
    };

    let mint_data = mint_info.info.data.borrow();
    let mint_state = StateWithExtensions::<Mint>::unpack(&mint_data)?;

    // Check for TransferFeeConfig extension
    if let Ok(fee_config) = mint_state.get_extension::<TransferFeeConfig>() {
        let epoch_fee = fee_config.get_epoch_fee(get_now_epoch());
        // calculate_post_fee_amount returns the amount after fee is deducted
        if let Some(post_fee_amount) = epoch_fee.calculate_post_fee_amount(amount) {
            return Ok(post_fee_amount);
        }
    }

    Ok(amount)
}

#[cfg(feature = "certora")]
fn calculate_post_fee_amount<'a, 'info>(
    amount: u64,
    _is_base_in: bool,
    _token_program_base: &TokenProgram<'a, 'info>,
    _token_program_quote: &TokenProgram<'a, 'info>,
    _base_mint: &Option<MintAccountInfo<'a, 'info>>,
    _quote_mint: &Option<MintAccountInfo<'a, 'info>>,
) -> Result<u64, ProgramError> {
    // For certora verification, assume no transfer fees
    Ok(amount)
}

/// Calculate the amount that must be sent for a specific amount to arrive after transfer fees.
/// This is used for the output side of swaps to ensure the trader receives their desired amount.
/// For Token-2022 tokens with TransferFeeConfig, this accounts for the fee.
/// For regular SPL tokens or Token-2022 without fees, returns the original amount.
/// Note: is_base_in refers to input direction; output is the opposite (if base_in, output is quote).
#[cfg(not(feature = "certora"))]
fn calculate_pre_fee_amount<'a, 'info>(
    desired_amount: u64,
    is_base_in: bool,
    token_program_base: &TokenProgram<'a, 'info>,
    token_program_quote: &TokenProgram<'a, 'info>,
    base_mint: &Option<MintAccountInfo<'a, 'info>>,
    quote_mint: &Option<MintAccountInfo<'a, 'info>>,
) -> Result<u64, ProgramError> {
    // Output token is opposite of input: if is_base_in, output is quote; otherwise output is base
    let (token_program, mint_opt) = if is_base_in {
        (token_program_quote, quote_mint)
    } else {
        (token_program_base, base_mint)
    };

    // Only Token-2022 can have transfer fees
    if *token_program.key != spl_token_2022::id() {
        return Ok(desired_amount);
    }

    // If no mint info provided, assume no fee
    let Some(mint_info) = mint_opt else {
        return Ok(desired_amount);
    };

    let mint_data = mint_info.info.data.borrow();
    let mint_state = StateWithExtensions::<Mint>::unpack(&mint_data)?;

    // Check for TransferFeeConfig extension
    if let Ok(fee_config) = mint_state.get_extension::<TransferFeeConfig>() {
        let epoch_fee = fee_config.get_epoch_fee(get_now_epoch());
        // calculate_pre_fee_amount returns the amount to send to get desired_amount after fee
        if let Some(pre_fee_amount) = epoch_fee.calculate_pre_fee_amount(desired_amount) {
            return Ok(pre_fee_amount);
        }
    }

    Ok(desired_amount)
}

#[cfg(feature = "certora")]
fn calculate_pre_fee_amount<'a, 'info>(
    desired_amount: u64,
    _is_base_in: bool,
    _token_program_base: &TokenProgram<'a, 'info>,
    _token_program_quote: &TokenProgram<'a, 'info>,
    _base_mint: &Option<MintAccountInfo<'a, 'info>>,
    _quote_mint: &Option<MintAccountInfo<'a, 'info>>,
) -> Result<u64, ProgramError> {
    // For certora verification, assume no transfer fees
    Ok(desired_amount)
}
