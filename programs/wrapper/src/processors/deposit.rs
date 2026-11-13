use manifest::validation::{next_account_info, AccountViewExt};
use pinocchio::{account::Ref, ProgramResult};

use borsh::{BorshDeserialize, BorshSerialize};
use hypertree::{DataIndex, NIL};
use manifest::{
    program::{deposit_instruction, invoke},
    state::MarketFixed,
    validation::{ManifestAccountInfo, MintAccountInfo, TokenProgram},
};

use manifest::validation::{Program, Signer};
use pinocchio::account::AccountView;
use solana_program::pubkey::Pubkey;
use solana_sdk_ids::system_program;

use crate::loader::{check_signer, WrapperStateAccountInfo};

use super::shared::{get_trader_index_hint_for_market, restore_seat_if_reclaimed, sync};

#[derive(BorshDeserialize, BorshSerialize)]
pub struct WrapperDepositParams {
    pub amount_atoms: u64,
}

impl WrapperDepositParams {
    pub fn new(amount_atoms: u64) -> Self {
        WrapperDepositParams { amount_atoms }
    }
}

pub(crate) fn process_deposit(
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
    // Optional so deposits from older clients still work. Anything else in
    // this slot is ignored rather than rejected, since it used to be.
    let system_program_opt: Option<Program> = match next_account_info(account_iter) {
        Ok(account) if account.pubkey() == &system_program::id() => {
            Some(Program::new(account, &system_program::id())?)
        }
        _ => None,
    };

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

    let WrapperDepositParams { amount_atoms } = WrapperDepositParams::try_from_slice(data)
        .map_err(manifest::validation::io_to_program_error)?;

    // Defrag can move the seat, so the cached index below may be stale. Sync
    // before reading it, the same way withdraw does: the core validates the
    // hint and would fail the whole deposit on one that no longer matches.
    sync(&wrapper_state, &market)?;
    let mut trader_index_hint: Option<DataIndex> =
        get_trader_index_hint_for_market(&wrapper_state, &market.info.pubkey())?;

    // NIL means the seat was reclaimed, not moved. A deposit wants it back,
    // and defrag only reclaims empty seats, so nothing was lost with it.
    // Repairs the core side only; a missing wrapper entry is ClaimSeat's job.
    if trader_index_hint == Some(NIL) {
        if let Some(restored) = restore_seat_if_reclaimed(
            &wrapper_state,
            &market,
            &owner,
            &manifest_program,
            system_program_opt.as_ref(),
        )? {
            trader_index_hint = Some(restored);
        }
    }

    // Call the deposit CPI.
    invoke(
        &deposit_instruction(
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

    sync(&wrapper_state, &market)?;

    Ok(())
}
