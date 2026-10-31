use manifest::validation::{get_global_address, get_global_vault_address, get_vault_address};
use pinocchio::error::ProgramError;
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

use crate::state::{get_position_address, get_trader_address};

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionInstruction {
    /// Accounts: 0 owner [signer, writable, pays rent], 1 position PDA
    /// [writable], 2 trader PDA [writable], 3 Manifest market [writable],
    /// 4 system program, 5 Manifest program.
    /// Data: created_at i64, opening price_e9 u64, then the ladder bytes to
    /// the end, all little-endian.
    ///
    /// Creates the position and its trader, funds the trader to the rent
    /// minimum and claims the trader's seat on the market (the owner pays
    /// for the market's growth).
    CreatePosition = 0,
    /// Accounts: see [`TradingAccounts`], followed by 16 base global
    /// [writable], 17 base global vault [writable], 18 quote global
    /// [writable], 19 quote global vault [writable] (Manifest's global
    /// accounts of each mint, which may not exist).
    /// Data: u16 order indices, little-endian, to the end.
    ///
    /// Places these orders of the ladder: pulls exactly what they need from
    /// the owner's token accounts into the seat and lays them as reverse
    /// orders. An order already placed fails the whole instruction, and so
    /// does one that traded more than 1% of its size against the book on the
    /// way in (`ERROR_ORDER_CROSSED`). Refused once closing began (a
    /// CancelOrders recorded the closing price).
    PlaceOrders = 1,
    /// Accounts: see [`TradingAccounts`].
    /// Data: closing price_e9 u64, nonzero; ignored when a CancelOrders
    /// already recorded one.
    ///
    /// Cancels every order of the seat, withdraws the seat's balances to the
    /// owner, closes the trader's token accounts and returns the trader's
    /// lamports, then writes the closing snapshot. Final.
    ClosePosition = 2,
    /// Accounts: 0 owner [signer], 1 position [writable], 2 trader
    /// [writable], 3 market [writable], 4 system program, 5 Manifest program.
    /// Data: closing price_e9 u64, nonzero, then u32 market indices of the
    /// seat's orders, strictly increasing, little-endian, to the end.
    ///
    /// Cancels the seat's orders at these indices, for ladders too long to
    /// cancel inside one ClosePosition. Its cost follows the list, not the
    /// book; indices that no longer hold an order of the seat are skipped.
    /// The first one records the closing price, which later CancelOrders and
    /// ClosePosition keep.
    CancelOrders = 3,
}

impl TryFrom<u8> for PositionInstruction {
    type Error = ProgramError;
    fn try_from(tag: u8) -> Result<Self, Self::Error> {
        match tag {
            0 => Ok(Self::CreatePosition),
            1 => Ok(Self::PlaceOrders),
            2 => Ok(Self::ClosePosition),
            3 => Ok(Self::CancelOrders),
            _ => Err(ProgramError::InvalidInstructionData),
        }
    }
}

/// created_at + price_e9, before the ladder.
pub const CREATE_FIXED_DATA_LEN: usize = 16;

pub fn create_position_instruction(
    owner: &Pubkey,
    market: &Pubkey,
    created_at: i64,
    price_e9: u64,
    ladder: &[u8],
) -> Instruction {
    let (position, _) = get_position_address(owner, market, created_at);
    let (trader, _) = get_trader_address(&position);
    let mut data = vec![PositionInstruction::CreatePosition as u8];
    data.extend_from_slice(&created_at.to_le_bytes());
    data.extend_from_slice(&price_e9.to_le_bytes());
    data.extend_from_slice(ladder);
    Instruction {
        program_id: crate::ID,
        accounts: vec![
            AccountMeta::new(*owner, true),
            AccountMeta::new(position, false),
            AccountMeta::new(trader, false),
            AccountMeta::new(*market, false),
            AccountMeta::new_readonly(solana_sdk_ids::system_program::id(), false),
            AccountMeta::new_readonly(manifest::ID, false),
        ],
        data,
    }
}

/// The accounts PlaceOrders and ClosePosition take, in this order:
/// 0 owner [signer, writable], 1 position [writable], 2 trader [writable],
/// 3 market [writable], 4 system program, 5 Manifest program, 6 base mint,
/// 7 quote mint, 8 owner base token [writable], 9 owner quote token
/// [writable], 10 trader base token [writable], 11 trader quote token
/// [writable], 12 base vault [writable], 13 quote vault [writable], 14 base
/// token program, 15 quote token program.
///
/// The trader's token accounts are any token accounts the trader owns, in
/// practice its associated ones, created by the client.
pub struct TradingAccounts {
    pub owner: Pubkey,
    pub position: Pubkey,
    pub market: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub owner_base: Pubkey,
    pub owner_quote: Pubkey,
    pub trader_base: Pubkey,
    pub trader_quote: Pubkey,
    pub base_token_program: Pubkey,
    pub quote_token_program: Pubkey,
}

impl TradingAccounts {
    fn metas(&self) -> Vec<AccountMeta> {
        let (trader, _) = get_trader_address(&self.position);
        vec![
            AccountMeta::new(self.owner, true),
            AccountMeta::new(self.position, false),
            AccountMeta::new(trader, false),
            AccountMeta::new(self.market, false),
            AccountMeta::new_readonly(solana_sdk_ids::system_program::id(), false),
            AccountMeta::new_readonly(manifest::ID, false),
            AccountMeta::new_readonly(self.base_mint, false),
            AccountMeta::new_readonly(self.quote_mint, false),
            AccountMeta::new(self.owner_base, false),
            AccountMeta::new(self.owner_quote, false),
            AccountMeta::new(self.trader_base, false),
            AccountMeta::new(self.trader_quote, false),
            AccountMeta::new(get_vault_address(&self.market, &self.base_mint).0, false),
            AccountMeta::new(get_vault_address(&self.market, &self.quote_mint).0, false),
            AccountMeta::new_readonly(self.base_token_program, false),
            AccountMeta::new_readonly(self.quote_token_program, false),
        ]
    }
}

/// PlaceOrders, with each mint's Manifest global account and global vault
/// after the trading accounts.
pub fn place_orders_instruction(accounts: &TradingAccounts, orders: &[u16]) -> Instruction {
    let mut data = vec![PositionInstruction::PlaceOrders as u8];
    for k in orders {
        data.extend_from_slice(&k.to_le_bytes());
    }
    let mut metas = accounts.metas();
    for mint in [&accounts.base_mint, &accounts.quote_mint] {
        metas.push(AccountMeta::new(get_global_address(mint).0, false));
        metas.push(AccountMeta::new(get_global_vault_address(mint).0, false));
    }
    Instruction {
        program_id: crate::ID,
        accounts: metas,
        data,
    }
}

pub fn close_position_instruction(accounts: &TradingAccounts, price_e9: u64) -> Instruction {
    let mut data = vec![PositionInstruction::ClosePosition as u8];
    data.extend_from_slice(&price_e9.to_le_bytes());
    Instruction {
        program_id: crate::ID,
        accounts: accounts.metas(),
        data,
    }
}

pub fn cancel_orders_instruction(
    owner: &Pubkey,
    position: &Pubkey,
    market: &Pubkey,
    price_e9: u64,
    indices: &[u32],
) -> Instruction {
    let (trader, _) = get_trader_address(position);
    let mut data = vec![PositionInstruction::CancelOrders as u8];
    data.extend_from_slice(&price_e9.to_le_bytes());
    for i in indices {
        data.extend_from_slice(&i.to_le_bytes());
    }
    Instruction {
        program_id: crate::ID,
        accounts: vec![
            AccountMeta::new_readonly(*owner, true),
            AccountMeta::new(*position, false),
            AccountMeta::new(trader, false),
            AccountMeta::new(*market, false),
            AccountMeta::new_readonly(solana_sdk_ids::system_program::id(), false),
            AccountMeta::new_readonly(manifest::ID, false),
        ],
        data,
    }
}
