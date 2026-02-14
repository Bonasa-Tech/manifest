use std::{cell::Ref, mem::size_of};

use crate::{
    logs::{emit_stack, CreateMarketLog},
    program::{expand_market_if_needed, invoke},
    require,
    state::MarketFixed,
    utils::create_account,
    validation::{
        get_market_address, get_vault_address, loaders::CreateMarketContext, ManifestAccountInfo,
    },
};
use borsh::{BorshDeserialize, BorshSerialize};
use hypertree::{get_mut_helper, trace};
use solana_program::{
    account_info::AccountInfo, entrypoint::ProgramResult, program_pack::Pack, pubkey::Pubkey,
    rent::Rent, sysvar::Sysvar,
};
use spl_token_2022::{
    extension::{
        mint_close_authority::MintCloseAuthority, permanent_delegate::PermanentDelegate,
        BaseStateWithExtensions, ExtensionType, PodStateWithExtensions, StateWithExtensions,
    },
    pod::PodMint,
    state::{Account, Mint},
};

#[derive(BorshDeserialize, BorshSerialize)]
pub struct CreateMarketParams {
    pub base_mint_index: u8,
    pub base_mint_decimals: u8,
    pub initial_margin_bps: u64,
    pub maintenance_margin_bps: u64,
    pub pyth_feed_account: Pubkey,
}

impl CreateMarketParams {
    pub fn new(
        base_mint_index: u8,
        base_mint_decimals: u8,
        initial_margin_bps: u64,
        maintenance_margin_bps: u64,
        pyth_feed_account: Pubkey,
    ) -> Self {
        CreateMarketParams {
            base_mint_index,
            base_mint_decimals,
            initial_margin_bps,
            maintenance_margin_bps,
            pyth_feed_account,
        }
    }
}

pub(crate) fn process_create_market(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let params: CreateMarketParams = CreateMarketParams::try_from_slice(data)?;
    trace!("process_create_market accs={accounts:?}");
    let create_market_context: CreateMarketContext = CreateMarketContext::load(accounts)?;

    let CreateMarketContext {
        market,
        payer,
        quote_mint,
        quote_vault,
        system_program,
        token_program,
        token_program_22,
    } = create_market_context;

    // Check token-2022 extensions on quote mint
    if *quote_mint.as_ref().owner == spl_token_2022::id() {
        let mint_data: Ref<'_, &mut [u8]> = quote_mint.as_ref().data.borrow();
        let pool_mint: StateWithExtensions<'_, Mint> =
            StateWithExtensions::<Mint>::unpack(&mint_data)?;
        if let Ok(extension) = pool_mint.get_extension::<MintCloseAuthority>() {
            let close_authority: Option<Pubkey> = extension.close_authority.into();
            if close_authority.is_some() {
                solana_program::msg!(
                    "Warning, you are creating a market with a close authority."
                );
            }
        }
        if let Ok(extension) = pool_mint.get_extension::<PermanentDelegate>() {
            let permanent_delegate: Option<Pubkey> = extension.delegate.into();
            if permanent_delegate.is_some() {
                solana_program::msg!(
                    "Warning, you are creating a market with a permanent delegate. There is no loss of funds protection for funds on this market"
                );
            }
        }
    }

    {
        let rent: Rent = Rent::get()?;

        // Create the market PDA account using base_mint_index seed
        let (_market_key, market_bump) =
            get_market_address(params.base_mint_index, quote_mint.info.key);
        require!(
            _market_key == *market.info.key,
            crate::program::ManifestError::InvalidMarketPubkey,
            "Market account is not at expected PDA address",
        )?;

        let market_seeds: Vec<Vec<u8>> = vec![
            b"market".to_vec(),
            vec![params.base_mint_index],
            quote_mint.info.key.as_ref().to_vec(),
            vec![market_bump],
        ];
        create_account(
            payer.as_ref(),
            market.info,
            system_program.as_ref(),
            &crate::id(),
            &rent,
            size_of::<MarketFixed>() as u64,
            market_seeds,
        )?;

        // Create the quote vault only (no base vault — base is virtual in perps)
        {
            let mint = quote_mint.as_ref();
            let is_mint_22: bool = *mint.owner == spl_token_2022::id();
            let token_program_for_mint: Pubkey = if is_mint_22 {
                spl_token_2022::id()
            } else {
                spl_token::id()
            };

            let (_vault_key, bump) = get_vault_address(market.info.key, mint.key);
            let seeds: Vec<Vec<u8>> = vec![
                b"vault".to_vec(),
                market.info.key.as_ref().to_vec(),
                mint.key.as_ref().to_vec(),
                vec![bump],
            ];

            if is_mint_22 {
                let mint_data: Ref<'_, &mut [u8]> = mint.data.borrow();
                let mint_with_extension: PodStateWithExtensions<'_, PodMint> =
                    PodStateWithExtensions::<PodMint>::unpack(&mint_data).unwrap();
                let mint_extensions: Vec<ExtensionType> =
                    mint_with_extension.get_extension_types()?;
                let required_extensions: Vec<ExtensionType> =
                    ExtensionType::get_required_init_account_extensions(&mint_extensions);
                let space: usize =
                    ExtensionType::try_calculate_account_len::<Account>(&required_extensions)?;
                create_account(
                    payer.as_ref(),
                    quote_vault.as_ref(),
                    system_program.as_ref(),
                    &token_program_for_mint,
                    &rent,
                    space as u64,
                    seeds,
                )?;
                invoke(
                    &spl_token_2022::instruction::initialize_account3(
                        &token_program_for_mint,
                        quote_vault.as_ref().key,
                        mint.key,
                        quote_vault.as_ref().key,
                    )?,
                    &[
                        payer.as_ref().clone(),
                        quote_vault.as_ref().clone(),
                        mint.clone(),
                        token_program_22.as_ref().clone(),
                    ],
                )?;
            } else {
                let space: usize = spl_token::state::Account::LEN;
                create_account(
                    payer.as_ref(),
                    quote_vault.as_ref(),
                    system_program.as_ref(),
                    &token_program_for_mint,
                    &rent,
                    space as u64,
                    seeds,
                )?;
                invoke(
                    &spl_token::instruction::initialize_account3(
                        &token_program_for_mint,
                        quote_vault.as_ref().key,
                        mint.key,
                        quote_vault.as_ref().key,
                    )?,
                    &[
                        payer.as_ref().clone(),
                        quote_vault.as_ref().clone(),
                        mint.clone(),
                        token_program.as_ref().clone(),
                    ],
                )?;
            }
        }

        // Setup the empty market
        let mut empty_market_fixed: MarketFixed =
            MarketFixed::new_empty(params.base_mint_index, params.base_mint_decimals, &quote_mint);

        // Configure margin params
        empty_market_fixed.set_perps_params(
            params.initial_margin_bps,
            params.maintenance_margin_bps,
        );

        // Set the Pyth oracle feed account
        empty_market_fixed.set_pyth_feed(params.pyth_feed_account);

        assert_eq!(market.info.data_len(), size_of::<MarketFixed>());

        let market_bytes: &mut [u8] = &mut market.info.try_borrow_mut_data()?[..];
        *get_mut_helper::<MarketFixed>(market_bytes, 0_u32) = empty_market_fixed;

        emit_stack(CreateMarketLog {
            market: *market.info.key,
            creator: *payer.key,
            base_mint: Pubkey::default(),
            quote_mint: *quote_mint.info.key,
        })?;
    }

    // Now that the market is created and initialized, construct ManifestAccountInfo for expand
    let market_manifest: ManifestAccountInfo<MarketFixed> =
        ManifestAccountInfo::<MarketFixed>::new(market.info)?;
    expand_market_if_needed(&payer, &market_manifest)?;

    Ok(())
}
