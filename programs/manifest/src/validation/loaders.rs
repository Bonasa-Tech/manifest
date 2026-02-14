use std::{cell::Ref, slice::Iter};

use hypertree::{get_helper, trace};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    program_error::ProgramError,
    pubkey::Pubkey,
    system_program,
};

use crate::{
    program::ManifestError,
    require,
    state::{GlobalFixed, MarketFixed},
    validation::{
        get_global_address, get_market_address, EmptyAccount, MintAccountInfo, Program, Signer,
        TokenAccountInfo,
    },
};

use super::{get_vault_address, ManifestAccountInfo, TokenProgram};

#[cfg(feature = "certora")]
use early_panic::early_panic;

/// CreateMarket account infos
pub(crate) struct CreateMarketContext<'a, 'info> {
    pub payer: Signer<'a, 'info>,
    pub market: EmptyAccount<'a, 'info>,
    pub quote_mint: MintAccountInfo<'a, 'info>,
    pub quote_vault: EmptyAccount<'a, 'info>,
    pub system_program: Program<'a, 'info>,
    pub token_program: TokenProgram<'a, 'info>,
    pub token_program_22: TokenProgram<'a, 'info>,
}

impl<'a, 'info> CreateMarketContext<'a, 'info> {
    pub fn load(accounts: &'a [AccountInfo<'info>]) -> Result<Self, ProgramError> {
        let account_iter: &mut Iter<AccountInfo<'info>> = &mut accounts.iter();

        let payer: Signer = Signer::new_payer(next_account_info(account_iter)?)?;
        let market: EmptyAccount = EmptyAccount::new(next_account_info(account_iter)?)?;
        let system_program: Program =
            Program::new(next_account_info(account_iter)?, &system_program::id())?;
        let quote_mint: MintAccountInfo = MintAccountInfo::new(next_account_info(account_iter)?)?;
        let quote_vault: EmptyAccount = EmptyAccount::new(next_account_info(account_iter)?)?;

        // PDA verification is done in the processor after params are parsed
        // (seeds depend on base_mint_index from params)

        let (expected_quote_vault, _quote_vault_bump) =
            get_vault_address(market.info.key, quote_mint.info.key);
        require!(
            expected_quote_vault == *quote_vault.info.key,
            ManifestError::IncorrectAccount,
            "Incorrect quote vault account",
        )?;

        let token_program: TokenProgram = TokenProgram::new(next_account_info(account_iter)?)?;
        let token_program_22: TokenProgram = TokenProgram::new(next_account_info(account_iter)?)?;

        Ok(Self {
            payer,
            market,
            quote_vault,
            quote_mint,
            token_program,
            token_program_22,
            system_program,
        })
    }
}

/// ClaimSeat account infos
pub(crate) struct ClaimSeatContext<'a, 'info> {
    pub payer: Signer<'a, 'info>,
    pub market: ManifestAccountInfo<'a, 'info, MarketFixed>,
    pub _system_program: Program<'a, 'info>,
}

impl<'a, 'info> ClaimSeatContext<'a, 'info> {
    #[cfg_attr(all(feature = "certora", not(feature = "certora-test")), early_panic)]
    pub fn load(accounts: &'a [AccountInfo<'info>]) -> Result<Self, ProgramError> {
        let account_iter: &mut Iter<AccountInfo<'info>> = &mut accounts.iter();

        let payer: Signer = Signer::new(next_account_info(account_iter)?)?;
        let market_info: &AccountInfo = next_account_info(account_iter)?;
        let market: ManifestAccountInfo<MarketFixed> =
            ManifestAccountInfo::<MarketFixed>::new(market_info)
                .or_else(|_| ManifestAccountInfo::<MarketFixed>::new_delegated(market_info))?;
        let _system_program: Program =
            Program::new(next_account_info(account_iter)?, &system_program::id())?;
        Ok(Self {
            payer,
            market,
            _system_program,
        })
    }
}

/// ExpandMarketContext account infos
pub(crate) struct ExpandMarketContext<'a, 'info> {
    pub payer: Signer<'a, 'info>,
    pub market: ManifestAccountInfo<'a, 'info, MarketFixed>,
    pub _system_program: Program<'a, 'info>,
}

impl<'a, 'info> ExpandMarketContext<'a, 'info> {
    pub fn load(accounts: &'a [AccountInfo<'info>]) -> Result<Self, ProgramError> {
        let account_iter: &mut Iter<AccountInfo<'info>> = &mut accounts.iter();

        let payer: Signer = Signer::new_payer(next_account_info(account_iter)?)?;
        let market_info: &AccountInfo = next_account_info(account_iter)?;
        let market: ManifestAccountInfo<MarketFixed> =
            ManifestAccountInfo::<MarketFixed>::new(market_info)
                .or_else(|_| ManifestAccountInfo::<MarketFixed>::new_delegated(market_info))?;
        let _system_program: Program =
            Program::new(next_account_info(account_iter)?, &system_program::id())?;
        Ok(Self {
            payer,
            market,
            _system_program,
        })
    }
}

/// Deposit into a market account infos
pub(crate) struct DepositContext<'a, 'info> {
    pub payer: Signer<'a, 'info>,
    pub market: ManifestAccountInfo<'a, 'info, MarketFixed>,
    pub trader_token: TokenAccountInfo<'a, 'info>,
    pub vault: TokenAccountInfo<'a, 'info>,
    pub token_program: TokenProgram<'a, 'info>,
    pub mint: Option<MintAccountInfo<'a, 'info>>,
}

impl<'a, 'info> DepositContext<'a, 'info> {
    #[cfg_attr(all(feature = "certora", not(feature = "certora-test")), early_panic)]
    pub fn load(accounts: &'a [AccountInfo<'info>]) -> Result<Self, ProgramError> {
        let account_iter: &mut Iter<AccountInfo<'info>> = &mut accounts.iter();

        let payer: Signer = Signer::new(next_account_info(account_iter)?)?;
        let market_info: &AccountInfo = next_account_info(account_iter)?;
        let market: ManifestAccountInfo<MarketFixed> =
            ManifestAccountInfo::<MarketFixed>::new(market_info)
                .or_else(|_| ManifestAccountInfo::<MarketFixed>::new_delegated(market_info))?;

        let market_fixed: Ref<MarketFixed> = market.get_fixed()?;
        let quote_mint: Pubkey = *market_fixed.get_quote_mint();

        // Derive quote vault address on-the-fly
        let (expected_vault_address, _) = get_vault_address(market.info.key, &quote_mint);

        let token_account_info: &AccountInfo<'info> = next_account_info(account_iter)?;
        let is_ephemeral: bool =
            token_account_info.data_len() == super::token_checkers::EPHEMERAL_ATA_SIZE;

        // Only quote (USDC) deposits are allowed — verify the trader token is for quote mint
        let mint_offset: usize = if is_ephemeral { 32 } else { 0 };
        {
            let data = token_account_info.try_borrow_data()?;
            require!(
                &data[mint_offset..mint_offset + 32] == quote_mint.as_ref(),
                ManifestError::InvalidWithdrawAccounts,
                "Only quote mint deposits allowed",
            )?;
        }

        trace!("trader token account {:?}", token_account_info.key);
        let trader_token: TokenAccountInfo =
            TokenAccountInfo::new_with_owner(token_account_info, &quote_mint, payer.key)?;

        let vault_info: &AccountInfo<'info> = next_account_info(account_iter)?;
        let vault: TokenAccountInfo = if is_ephemeral {
            TokenAccountInfo::new(vault_info, &quote_mint)?
        } else {
            TokenAccountInfo::new_with_owner_and_key(
                vault_info,
                &quote_mint,
                &expected_vault_address,
                &expected_vault_address,
            )?
        };

        let token_program: TokenProgram = TokenProgram::new(next_account_info(account_iter)?)?;
        let mint: Option<MintAccountInfo> = if is_ephemeral {
            None
        } else {
            Some(MintAccountInfo::new(next_account_info(account_iter)?)?)
        };

        drop(market_fixed);
        Ok(Self {
            payer,
            market,
            trader_token,
            vault,
            token_program,
            mint,
        })
    }
}

/// Withdraw account infos
pub(crate) struct WithdrawContext<'a, 'info> {
    pub payer: Signer<'a, 'info>,
    pub market: ManifestAccountInfo<'a, 'info, MarketFixed>,
    pub trader_token: TokenAccountInfo<'a, 'info>,
    pub vault: TokenAccountInfo<'a, 'info>,
    pub token_program: TokenProgram<'a, 'info>,
    pub mint: Option<MintAccountInfo<'a, 'info>>,
}

impl<'a, 'info> WithdrawContext<'a, 'info> {
    pub fn load(accounts: &'a [AccountInfo<'info>]) -> Result<Self, ProgramError> {
        let account_iter: &mut Iter<AccountInfo<'info>> = &mut accounts.iter();

        let payer: Signer = Signer::new(next_account_info(account_iter)?)?;
        let market_info: &AccountInfo = next_account_info(account_iter)?;
        let market: ManifestAccountInfo<MarketFixed> =
            ManifestAccountInfo::<MarketFixed>::new(market_info)
                .or_else(|_| ManifestAccountInfo::<MarketFixed>::new_delegated(market_info))?;

        let market_fixed: Ref<MarketFixed> = market.get_fixed()?;
        let quote_mint: Pubkey = *market_fixed.get_quote_mint();

        // Derive quote vault address on-the-fly
        let (expected_vault_address, _) = get_vault_address(market.info.key, &quote_mint);

        let token_account_info: &AccountInfo<'info> = next_account_info(account_iter)?;
        let is_ephemeral: bool =
            token_account_info.data_len() == super::token_checkers::EPHEMERAL_ATA_SIZE;

        // Only quote (USDC) withdrawals are allowed
        let mint_offset: usize = if is_ephemeral { 32 } else { 0 };
        {
            let data = token_account_info.try_borrow_data()?;
            require!(
                &data[mint_offset..mint_offset + 32] == quote_mint.as_ref(),
                ManifestError::InvalidWithdrawAccounts,
                "Only quote mint withdrawals allowed",
            )?;
        }

        let trader_token: TokenAccountInfo =
            TokenAccountInfo::new_with_owner(token_account_info, &quote_mint, payer.key)?;

        let vault_info: &AccountInfo<'info> = next_account_info(account_iter)?;
        let vault: TokenAccountInfo = if is_ephemeral {
            TokenAccountInfo::new(vault_info, &quote_mint)?
        } else {
            TokenAccountInfo::new_with_owner_and_key(
                vault_info,
                &quote_mint,
                &expected_vault_address,
                &expected_vault_address,
            )?
        };

        let token_program: TokenProgram = TokenProgram::new(next_account_info(account_iter)?)?;
        let mint: Option<MintAccountInfo> = if is_ephemeral {
            None
        } else {
            Some(MintAccountInfo::new(next_account_info(account_iter)?)?)
        };

        drop(market_fixed);
        Ok(Self {
            payer,
            market,
            trader_token,
            vault,
            token_program,
            mint,
        })
    }
}

/// Swap account infos (perps: only quote vault/token needed)
pub(crate) struct SwapContext<'a, 'info> {
    pub payer: AccountInfo<'info>,
    pub owner: Signer<'a, 'info>,
    pub market: ManifestAccountInfo<'a, 'info, MarketFixed>,
    pub trader_quote: TokenAccountInfo<'a, 'info>,
    pub quote_vault: TokenAccountInfo<'a, 'info>,
    pub token_program_quote: TokenProgram<'a, 'info>,
    pub quote_mint: Option<MintAccountInfo<'a, 'info>>,

    // One for each side. First is base, then is quote.
    pub global_trade_accounts_opts: [Option<GlobalTradeAccounts<'a, 'info>>; 2],
}

impl<'a, 'info> SwapContext<'a, 'info> {
    #[cfg_attr(all(feature = "certora", not(feature = "certora-test")), early_panic)]
    pub fn load(accounts: &'a [AccountInfo<'info>]) -> Result<Self, ProgramError> {
        let account_iter: &mut Iter<AccountInfo<'info>> = &mut accounts.iter();

        let payer: &AccountInfo = next_account_info(account_iter)?;

        let owner_or_market: &'a AccountInfo<'info> = next_account_info(account_iter)?;
        let (owner, market): (Signer, ManifestAccountInfo<MarketFixed>) = {
            if let Ok(market) = ManifestAccountInfo::<MarketFixed>::new(owner_or_market) {
                (Signer::new(payer)?, market)
            } else if let Ok(market) =
                ManifestAccountInfo::<MarketFixed>::new_delegated(owner_or_market)
            {
                (Signer::new(payer)?, market)
            } else {
                let market_info: &AccountInfo = next_account_info(account_iter)?;
                (
                    Signer::new(owner_or_market)?,
                    ManifestAccountInfo::<MarketFixed>::new(market_info)
                        .or_else(|_| {
                            ManifestAccountInfo::<MarketFixed>::new_delegated(market_info)
                        })?,
                )
            }
        };

        let _system_program: Program =
            Program::new(next_account_info(account_iter)?, &system_program::id())?;

        let market_fixed: Ref<MarketFixed> = market.get_fixed()?;
        let quote_mint_key: Pubkey = *market_fixed.get_quote_mint();

        // Derive quote vault on-the-fly
        let (quote_vault_address, _) = get_vault_address(market.info.key, &quote_mint_key);

        let trader_quote: TokenAccountInfo =
            TokenAccountInfo::new(next_account_info(account_iter)?, &quote_mint_key)?;

        let quote_vault_info: &AccountInfo<'info> = next_account_info(account_iter)?;
        let quote_vault: TokenAccountInfo = if trader_quote.is_ephemeral() {
            TokenAccountInfo::new(quote_vault_info, &quote_mint_key)?
        } else {
            TokenAccountInfo::new_with_owner_and_key(
                quote_vault_info,
                &quote_mint_key,
                &quote_vault_address,
                &quote_vault_address,
            )?
        };
        drop(market_fixed);

        let token_program_quote: TokenProgram = TokenProgram::new(next_account_info(account_iter)?)?;
        let mut quote_mint: Option<MintAccountInfo> = None;
        let global_trade_accounts_opts: [Option<GlobalTradeAccounts<'a, 'info>>; 2] =
            [None, None];

        let ephemeral_token_id = super::token_checkers::ephemeral_spl_token::id();

        let mut current_account_info_or: Result<&AccountInfo<'info>, ProgramError> =
            next_account_info(account_iter);

        // Possibly includes quote mint if the token program is token22.
        if current_account_info_or
            .as_ref()
            .is_ok_and(|f| *f.owner == spl_token::id() || *f.owner == spl_token_2022::id())
        {
            let current_account_info: &AccountInfo<'info> = current_account_info_or?;
            quote_mint = Some(MintAccountInfo::new(current_account_info)?);
            let _ = next_account_info(account_iter);
        }

        Ok(Self {
            payer: payer.clone(),
            owner,
            market,
            trader_quote,
            quote_vault,
            token_program_quote,
            quote_mint,
            global_trade_accounts_opts,
        })
    }
}

/// Accounts needed to make a global trade. Scope is beyond just crate so
/// clients can place orders on markets in testing.
pub struct GlobalTradeAccounts<'a, 'info> {
    /// Required if this is a token22 token.
    pub mint_opt: Option<MintAccountInfo<'a, 'info>>,
    pub global: ManifestAccountInfo<'a, 'info, GlobalFixed>,

    // These are required when matching a global order, not necessarily when
    // cancelling since tokens dont move in that case.
    pub global_vault_opt: Option<TokenAccountInfo<'a, 'info>>,
    pub market_vault_opt: Option<TokenAccountInfo<'a, 'info>>,
    pub token_program_opt: Option<TokenProgram<'a, 'info>>,

    pub system_program: Option<Program<'a, 'info>>,

    // Trader is sending or cancelling the order. They are the one who will pay
    // or receive gas prepayments.
    pub gas_payer_opt: Option<Signer<'a, 'info>>,
    pub gas_receiver_opt: Option<Signer<'a, 'info>>,
    pub market: Pubkey,
}

/// BatchUpdate account infos
pub(crate) struct BatchUpdateContext<'a, 'info> {
    pub payer: Signer<'a, 'info>,
    pub market: ManifestAccountInfo<'a, 'info, MarketFixed>,
    pub _system_program: Program<'a, 'info>,

    // One for each side. First is base, then is quote.
    pub global_trade_accounts_opts: [Option<GlobalTradeAccounts<'a, 'info>>; 2],
}

impl<'a, 'info> BatchUpdateContext<'a, 'info> {
    pub fn load(accounts: &'a [AccountInfo<'info>]) -> Result<Self, ProgramError> {
        let account_iter: &mut Iter<AccountInfo<'info>> = &mut accounts.iter();

        // Does not have to be writable, but this ix will fail if removing a
        // global or requiring expanding.
        let payer: Signer = Signer::new(next_account_info(account_iter)?)?;
        let market_info: &AccountInfo = next_account_info(account_iter)?;
        let market: ManifestAccountInfo<MarketFixed> =
            ManifestAccountInfo::<MarketFixed>::new(market_info)
                .or_else(|_| ManifestAccountInfo::<MarketFixed>::new_delegated(market_info))?;
        let system_program: Program =
            Program::new(next_account_info(account_iter)?, &system_program::id())?;
        // Certora version is not mutable.
        #[cfg(feature = "certora")]
        let global_trade_accounts_opts: [Option<GlobalTradeAccounts<'a, 'info>>; 2] = [None, None];
        #[cfg(not(feature = "certora"))]
        let mut global_trade_accounts_opts: [Option<GlobalTradeAccounts<'a, 'info>>; 2] =
            [None, None];

        #[cfg(not(feature = "certora"))]
        {
            let market_fixed: Ref<MarketFixed> = market.get_fixed()?;
            let quote_mint: Pubkey = *market_fixed.get_quote_mint();
            let (quote_vault, _) = get_vault_address(market.info.key, &quote_mint);
            drop(market_fixed);

            for _ in 0..2 {
                let next_account_info_or: Result<&AccountInfo<'info>, ProgramError> =
                    next_account_info(account_iter);
                if next_account_info_or.is_ok() {
                    let mint: MintAccountInfo<'a, 'info> =
                        MintAccountInfo::new(next_account_info_or?)?;
                    // In perps, only quote mint is used for global trade accounts
                    require!(
                        quote_mint == *mint.info.key,
                        ManifestError::MissingGlobal,
                        "Unexpected global mint",
                    )?;
                    let (index, expected_market_vault_address) = (1, &quote_vault);

                    let global_or: Result<
                        ManifestAccountInfo<'a, 'info, GlobalFixed>,
                        ProgramError,
                    > = ManifestAccountInfo::<GlobalFixed>::new(next_account_info(account_iter)?);

                    // If a client blindly fills in the global account and vault,
                    // then handle that case and allow them to try to work without
                    // the global accounts.
                    if global_or.is_err() {
                        let _global_vault: Result<&AccountInfo<'info>, ProgramError> =
                            next_account_info(account_iter);
                        let _market_vault: Result<&AccountInfo<'info>, ProgramError> =
                            next_account_info(account_iter);
                        let _token_program: Result<&AccountInfo<'info>, ProgramError> =
                            next_account_info(account_iter);
                        continue;
                    }
                    let global: ManifestAccountInfo<'a, 'info, GlobalFixed> = global_or.unwrap();
                    let global_data: Ref<&mut [u8]> = global.data.borrow();
                    let global_fixed: &GlobalFixed = get_helper::<GlobalFixed>(&global_data, 0_u32);
                    let expected_global_vault_address: &Pubkey = global_fixed.get_vault();

                    let global_mint_key: &Pubkey = global_fixed.get_mint();
                    let (expected_global_key, _global_bump) = get_global_address(global_mint_key);
                    require!(
                        expected_global_key == *global.info.key,
                        ManifestError::MissingGlobal,
                        "Unexpected global accounts",
                    )?;

                    let global_vault: TokenAccountInfo<'a, 'info> =
                        TokenAccountInfo::new_with_owner_and_key(
                            next_account_info(account_iter)?,
                            mint.info.key,
                            &expected_global_vault_address,
                            &expected_global_vault_address,
                        )?;
                    drop(global_data);

                    let market_vault: TokenAccountInfo<'a, 'info> =
                        TokenAccountInfo::new_with_owner_and_key(
                            next_account_info(account_iter)?,
                            mint.info.key,
                            &expected_market_vault_address,
                            &expected_market_vault_address,
                        )?;
                    let token_program: TokenProgram<'a, 'info> =
                        TokenProgram::new(next_account_info(account_iter)?)?;

                    global_trade_accounts_opts[index] = Some(GlobalTradeAccounts {
                        mint_opt: Some(mint),
                        global,
                        global_vault_opt: Some(global_vault),
                        market_vault_opt: Some(market_vault),
                        token_program_opt: Some(token_program),
                        system_program: Some(system_program.clone()),
                        gas_payer_opt: Some(payer.clone()),
                        gas_receiver_opt: Some(payer.clone()),
                        market: *market.info.key,
                    })
                };
            }
        }

        Ok(Self {
            payer,
            market,
            _system_program: system_program,
            global_trade_accounts_opts,
        })
    }
}

/// Global create
pub(crate) struct GlobalCreateContext<'a, 'info> {
    pub payer: Signer<'a, 'info>,
    pub global: EmptyAccount<'a, 'info>,
    pub system_program: Program<'a, 'info>,
    pub global_mint: MintAccountInfo<'a, 'info>,
    pub global_vault: EmptyAccount<'a, 'info>,
    pub token_program: TokenProgram<'a, 'info>,
}

impl<'a, 'info> GlobalCreateContext<'a, 'info> {
    pub fn load(accounts: &'a [AccountInfo<'info>]) -> Result<Self, ProgramError> {
        let account_iter: &mut Iter<AccountInfo<'info>> = &mut accounts.iter();

        let payer: Signer = Signer::new_payer(next_account_info(account_iter)?)?;
        let global: EmptyAccount = EmptyAccount::new(next_account_info(account_iter)?)?;
        let system_program: Program =
            Program::new(next_account_info(account_iter)?, &system_program::id())?;
        let global_mint: MintAccountInfo = MintAccountInfo::new(next_account_info(account_iter)?)?;
        let global_vault: EmptyAccount = EmptyAccount::new(next_account_info(account_iter)?)?;

        let (expected_global_key, _global_bump) = get_global_address(global_mint.info.key);
        assert_eq!(expected_global_key, *global.info.key);

        let token_program: TokenProgram = TokenProgram::new(next_account_info(account_iter)?)?;
        Ok(Self {
            payer,
            global,
            system_program,
            global_mint,
            global_vault,
            token_program,
        })
    }
}

/// Global add trader
pub(crate) struct GlobalAddTraderContext<'a, 'info> {
    pub payer: Signer<'a, 'info>,
    pub global: ManifestAccountInfo<'a, 'info, GlobalFixed>,
    pub _system_program: Program<'a, 'info>,
}

impl<'a, 'info> GlobalAddTraderContext<'a, 'info> {
    pub fn load(accounts: &'a [AccountInfo<'info>]) -> Result<Self, ProgramError> {
        let account_iter: &mut Iter<AccountInfo<'info>> = &mut accounts.iter();

        let payer: Signer = Signer::new_payer(next_account_info(account_iter)?)?;
        let global: ManifestAccountInfo<GlobalFixed> =
            ManifestAccountInfo::<GlobalFixed>::new(next_account_info(account_iter)?)?;

        let global_data: Ref<&mut [u8]> = global.data.borrow();
        let global_fixed: &GlobalFixed = get_helper::<GlobalFixed>(&global_data, 0_u32);
        let global_mint_key: &Pubkey = global_fixed.get_mint();
        let (expected_global_key, _global_bump) = get_global_address(global_mint_key);
        require!(
            expected_global_key == *global.info.key,
            ManifestError::MissingGlobal,
            "Unexpected global accounts",
        )?;
        drop(global_data);

        let _system_program: Program =
            Program::new(next_account_info(account_iter)?, &system_program::id())?;
        Ok(Self {
            payer,
            global,
            _system_program,
        })
    }
}

/// Global deposit
pub(crate) struct GlobalDepositContext<'a, 'info> {
    pub payer: Signer<'a, 'info>,
    pub global: ManifestAccountInfo<'a, 'info, GlobalFixed>,
    pub mint: MintAccountInfo<'a, 'info>,
    pub global_vault: TokenAccountInfo<'a, 'info>,
    pub trader_token: TokenAccountInfo<'a, 'info>,
    pub token_program: TokenProgram<'a, 'info>,
}

impl<'a, 'info> GlobalDepositContext<'a, 'info> {
    pub fn load(accounts: &'a [AccountInfo<'info>]) -> Result<Self, ProgramError> {
        let account_iter: &mut Iter<AccountInfo<'info>> = &mut accounts.iter();

        let payer: Signer = Signer::new(next_account_info(account_iter)?)?;
        let global: ManifestAccountInfo<GlobalFixed> =
            ManifestAccountInfo::<GlobalFixed>::new(next_account_info(account_iter)?)?;

        let mint: MintAccountInfo = MintAccountInfo::new(next_account_info(account_iter)?)?;

        let global_data: Ref<&mut [u8]> = global.data.borrow();
        let global_fixed: &GlobalFixed = get_helper::<GlobalFixed>(&global_data, 0_u32);

        let global_mint_key: &Pubkey = global_fixed.get_mint();
        let (expected_global_key, _global_bump) = get_global_address(global_mint_key);
        require!(
            expected_global_key == *global.info.key,
            ManifestError::MissingGlobal,
            "Unexpected global accounts",
        )?;

        let expected_global_vault_address: &Pubkey = global_fixed.get_vault();

        let global_vault: TokenAccountInfo = TokenAccountInfo::new_with_owner_and_key(
            next_account_info(account_iter)?,
            mint.info.key,
            &expected_global_vault_address,
            &expected_global_vault_address,
        )?;
        drop(global_data);

        let token_account_info: &AccountInfo<'info> = next_account_info(account_iter)?;
        let trader_token: TokenAccountInfo =
            TokenAccountInfo::new_with_owner(token_account_info, mint.info.key, payer.key)?;
        let token_program: TokenProgram = TokenProgram::new(next_account_info(account_iter)?)?;
        Ok(Self {
            payer,
            global,
            mint,
            global_vault,
            trader_token,
            token_program,
        })
    }
}

/// Global withdraw
pub(crate) struct GlobalWithdrawContext<'a, 'info> {
    pub payer: Signer<'a, 'info>,
    pub global: ManifestAccountInfo<'a, 'info, GlobalFixed>,
    pub mint: MintAccountInfo<'a, 'info>,
    pub global_vault: TokenAccountInfo<'a, 'info>,
    pub trader_token: TokenAccountInfo<'a, 'info>,
    pub token_program: TokenProgram<'a, 'info>,
}

impl<'a, 'info> GlobalWithdrawContext<'a, 'info> {
    pub fn load(accounts: &'a [AccountInfo<'info>]) -> Result<Self, ProgramError> {
        let account_iter: &mut Iter<AccountInfo<'info>> = &mut accounts.iter();

        let payer: Signer = Signer::new(next_account_info(account_iter)?)?;
        let global: ManifestAccountInfo<GlobalFixed> =
            ManifestAccountInfo::<GlobalFixed>::new(next_account_info(account_iter)?)?;

        let mint: MintAccountInfo = MintAccountInfo::new(next_account_info(account_iter)?)?;

        let global_data: Ref<&mut [u8]> = global.data.borrow();
        let global_fixed: &GlobalFixed = get_helper::<GlobalFixed>(&global_data, 0_u32);

        let global_mint_key: &Pubkey = global_fixed.get_mint();
        let (expected_global_key, _global_bump) = get_global_address(global_mint_key);
        require!(
            expected_global_key == *global.info.key,
            ManifestError::MissingGlobal,
            "Unexpected global accounts",
        )?;

        let expected_global_vault_address: &Pubkey = global_fixed.get_vault();

        let global_vault: TokenAccountInfo = TokenAccountInfo::new_with_owner_and_key(
            next_account_info(account_iter)?,
            mint.info.key,
            &expected_global_vault_address,
            &expected_global_vault_address,
        )?;
        drop(global_data);

        let token_account_info: &AccountInfo<'info> = next_account_info(account_iter)?;
        let trader_token: TokenAccountInfo =
            TokenAccountInfo::new_with_owner(token_account_info, mint.info.key, payer.key)?;
        let token_program: TokenProgram = TokenProgram::new(next_account_info(account_iter)?)?;
        Ok(Self {
            payer,
            global,
            mint,
            global_vault,
            trader_token,
            token_program,
        })
    }
}

/// Global evict
pub(crate) struct GlobalEvictContext<'a, 'info> {
    pub payer: Signer<'a, 'info>,
    pub global: ManifestAccountInfo<'a, 'info, GlobalFixed>,
    pub mint: MintAccountInfo<'a, 'info>,
    pub global_vault: TokenAccountInfo<'a, 'info>,
    pub trader_token: TokenAccountInfo<'a, 'info>,
    pub evictee_token: TokenAccountInfo<'a, 'info>,
    pub token_program: TokenProgram<'a, 'info>,
}

impl<'a, 'info> GlobalEvictContext<'a, 'info> {
    pub fn load(accounts: &'a [AccountInfo<'info>]) -> Result<Self, ProgramError> {
        let account_iter: &mut Iter<AccountInfo<'info>> = &mut accounts.iter();

        let payer: Signer = Signer::new_payer(next_account_info(account_iter)?)?;
        let global: ManifestAccountInfo<GlobalFixed> =
            ManifestAccountInfo::<GlobalFixed>::new(next_account_info(account_iter)?)?;

        let mint: MintAccountInfo = MintAccountInfo::new(next_account_info(account_iter)?)?;

        let global_data: Ref<&mut [u8]> = global.data.borrow();
        let global_fixed: &GlobalFixed = get_helper::<GlobalFixed>(&global_data, 0_u32);

        let global_mint_key: &Pubkey = global_fixed.get_mint();
        let (expected_global_key, _global_bump) = get_global_address(global_mint_key);
        require!(
            expected_global_key == *global.info.key,
            ManifestError::MissingGlobal,
            "Unexpected global accounts",
        )?;

        let expected_global_vault_address: &Pubkey = global_fixed.get_vault();

        let global_vault: TokenAccountInfo = TokenAccountInfo::new_with_owner_and_key(
            next_account_info(account_iter)?,
            mint.info.key,
            &expected_global_vault_address,
            &expected_global_vault_address,
        )?;
        drop(global_data);

        let token_account_info: &AccountInfo<'info> = next_account_info(account_iter)?;
        let trader_token: TokenAccountInfo =
            TokenAccountInfo::new_with_owner(token_account_info, mint.info.key, payer.key)?;
        let token_account_info: &AccountInfo<'info> = next_account_info(account_iter)?;
        let evictee_token: TokenAccountInfo =
            TokenAccountInfo::new(token_account_info, mint.info.key)?;
        let token_program: TokenProgram = TokenProgram::new(next_account_info(account_iter)?)?;
        Ok(Self {
            payer,
            global,
            mint,
            global_vault,
            trader_token,
            evictee_token,
            token_program,
        })
    }
}

/// Global clean
pub(crate) struct GlobalCleanContext<'a, 'info> {
    pub payer: Signer<'a, 'info>,
    pub market: ManifestAccountInfo<'a, 'info, MarketFixed>,
    pub system_program: Program<'a, 'info>,
    pub global: ManifestAccountInfo<'a, 'info, GlobalFixed>,
}

impl<'a, 'info> GlobalCleanContext<'a, 'info> {
    pub fn load(accounts: &'a [AccountInfo<'info>]) -> Result<Self, ProgramError> {
        let account_iter: &mut Iter<AccountInfo<'info>> = &mut accounts.iter();

        let payer: Signer = Signer::new_payer(next_account_info(account_iter)?)?;
        let market_info: &AccountInfo = next_account_info(account_iter)?;
        let market: ManifestAccountInfo<MarketFixed> =
            ManifestAccountInfo::<MarketFixed>::new(market_info)
                .or_else(|_| ManifestAccountInfo::<MarketFixed>::new_delegated(market_info))?;
        let system_program: Program =
            Program::new(next_account_info(account_iter)?, &system_program::id())?;
        let global: ManifestAccountInfo<GlobalFixed> =
            ManifestAccountInfo::<GlobalFixed>::new(next_account_info(account_iter)?)?;

        let global_data: Ref<&mut [u8]> = global.data.borrow();
        let global_fixed: &GlobalFixed = get_helper::<GlobalFixed>(&global_data, 0_u32);
        let global_mint_key: &Pubkey = global_fixed.get_mint();
        let (expected_global_key, _global_bump) = get_global_address(global_mint_key);
        require!(
            expected_global_key == *global.info.key,
            ManifestError::MissingGlobal,
            "Unexpected global accounts",
        )?;
        drop(global_data);

        Ok(Self {
            payer,
            market,
            system_program,
            global,
        })
    }
}

/// CrankFunding account infos
pub(crate) struct CrankFundingContext<'a, 'info> {
    pub payer: Signer<'a, 'info>,
    pub market: ManifestAccountInfo<'a, 'info, MarketFixed>,
    pub pyth_price_feed: &'a AccountInfo<'info>,
}

impl<'a, 'info> CrankFundingContext<'a, 'info> {
    pub fn load(accounts: &'a [AccountInfo<'info>]) -> Result<Self, ProgramError> {
        let account_iter: &mut Iter<AccountInfo<'info>> = &mut accounts.iter();

        let payer: Signer = Signer::new_payer(next_account_info(account_iter)?)?;
        let market_info: &'a AccountInfo<'info> = next_account_info(account_iter)?;
        let market: ManifestAccountInfo<MarketFixed> =
            ManifestAccountInfo::<MarketFixed>::new(market_info)
                .or_else(|_| ManifestAccountInfo::<MarketFixed>::new_delegated(market_info))?;

        let pyth_price_feed: &'a AccountInfo<'info> = next_account_info(account_iter)?;

        // Validate pyth feed matches the one stored on the market
        {
            let market_fixed: std::cell::Ref<MarketFixed> = market.get_fixed()?;
            require!(
                *pyth_price_feed.key == *market_fixed.get_pyth_feed(),
                ManifestError::IncorrectAccount,
                "Pyth feed account does not match market's oracle",
            )?;
            require!(
                *market_fixed.get_pyth_feed() != Pubkey::default(),
                ManifestError::InvalidPerpsOperation,
                "Market has no oracle configured",
            )?;
        }

        Ok(Self {
            payer,
            market,
            pyth_price_feed,
        })
    }
}

/// Liquidate account infos
pub(crate) struct LiquidateContext<'a, 'info> {
    pub liquidator: Signer<'a, 'info>,
    pub market: ManifestAccountInfo<'a, 'info, MarketFixed>,
}

impl<'a, 'info> LiquidateContext<'a, 'info> {
    pub fn load(accounts: &'a [AccountInfo<'info>]) -> Result<Self, ProgramError> {
        let account_iter: &mut std::slice::Iter<AccountInfo<'info>> = &mut accounts.iter();

        let liquidator: Signer = Signer::new_payer(next_account_info(account_iter)?)?;
        let market_info: &'a AccountInfo<'info> = next_account_info(account_iter)?;
        let market: ManifestAccountInfo<MarketFixed> =
            ManifestAccountInfo::<MarketFixed>::new(market_info)
                .or_else(|_| ManifestAccountInfo::<MarketFixed>::new_delegated(market_info))?;
        // system_program is optional, just consume it
        let _system_program = next_account_info(account_iter).ok();

        Ok(Self {
            liquidator,
            market,
        })
    }
}
