use manifest::validation::{next_account_info, AccountViewExt};
use pinocchio::{account::Ref, ProgramResult};

use borsh::{BorshDeserialize, BorshSerialize};
use hypertree::DataIndex;
use manifest::{
    program::{invoke, withdraw_instruction},
    state::MarketFixed,
    validation::{ManifestAccountInfo, MintAccountInfo, TokenProgram},
};

use manifest::validation::{Program, Signer};
use pinocchio::account::AccountView;
use solana_program::pubkey::Pubkey;

use crate::loader::{check_signer, WrapperStateAccountInfo};

use super::shared::{get_trader_index_hint_for_market, sync};

#[derive(BorshDeserialize, BorshSerialize)]
pub struct WrapperWithdrawParams {
    pub amount_atoms: u64,
}

impl WrapperWithdrawParams {
    pub fn new(amount_atoms: u64) -> Self {
        WrapperWithdrawParams { amount_atoms }
    }
}

pub(crate) fn process_withdraw(
    _program_id: &Pubkey,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let account_iter: &mut std::slice::Iter<AccountView> = &mut accounts.iter();
    let manifest_program: Program =
        Program::new(next_account_info(account_iter)?, &manifest::id())?;
    let owner: Signer = Signer::new(next_account_info(account_iter)?)?;
    let market: ManifestAccountInfo<MarketFixed> =
        ManifestAccountInfo::<MarketFixed>::new(next_account_info(account_iter)?)?;
    let trader_token_account: &AccountView = next_account_info(account_iter)?;
    let vault: &AccountView = next_account_info(account_iter)?;
    let token_program: TokenProgram = TokenProgram::new(next_account_info(account_iter)?)?;
    let wrapper_state: WrapperStateAccountInfo =
        WrapperStateAccountInfo::new(next_account_info(account_iter)?)?;
    check_signer(&wrapper_state, owner.pubkey());
    let mint_account_info: MintAccountInfo =
        MintAccountInfo::new(next_account_info(account_iter)?)?;

    let mint: Pubkey = {
        let market_fixed: Ref<MarketFixed> = market.get_fixed()?;
        let base_mint: &Pubkey = market_fixed.get_base_mint();
        let quote_mint: &Pubkey = market_fixed.get_quote_mint();
        if &trader_token_account.try_borrow()?[0..32] == base_mint.as_ref() {
            *base_mint
        } else {
            *quote_mint
        }
    };

    // Params are a direct pass through.
    let WrapperWithdrawParams { amount_atoms } = WrapperWithdrawParams::try_from_slice(data)
        .map_err(manifest::validation::io_to_program_error)?;

    let trader_index_hint: Option<DataIndex> =
        get_trader_index_hint_for_market(&wrapper_state, &market.info.pubkey())?;

    // Call the withdraw CPI
    invoke(
        &withdraw_instruction(
            market.pubkey(),
            owner.pubkey(),
            &mint,
            amount_atoms,
            trader_token_account.pubkey(),
            *token_program.pubkey(),
            trader_index_hint,
        ),
        &[
            manifest_program.info,
            owner.info,
            market.info,
            trader_token_account,
            vault,
            token_program.info,
            mint_account_info.info,
        ],
    )?;

    // Sync
    sync(&wrapper_state, &market)?;

    Ok(())
}
