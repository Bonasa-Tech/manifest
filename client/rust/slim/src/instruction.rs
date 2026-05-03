//! Instruction builders for Manifest operations.

use crate::constants::DataIndex;
use crate::constants::OrderType;
use crate::constants::MANIFEST_PROGRAM_ID;
use crate::constants::NO_EXPIRATION_LAST_VALID_SLOT;
use crate::constants::SYSTEM_PROGRAM_ID;
use crate::constants::TOKEN_PROGRAM_ID;
use solana_instruction::AccountMeta;
use solana_instruction::Instruction;
use solana_pubkey::Pubkey;

/// Manifest instruction discriminants.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestInstruction {
    CreateMarket = 0,
    ClaimSeat = 1,
    Deposit = 2,
    Withdraw = 3,
    Swap = 4,
    Expand = 5,
    BatchUpdate = 6,
    GlobalCreate = 7,
    GlobalAddTrader = 8,
    GlobalDeposit = 9,
    GlobalWithdraw = 10,
    GlobalEvict = 11,
    GlobalClean = 12,
    SwapV2 = 13,
}

/// Get the vault PDA for a market and mint.
pub fn get_vault_address(market: &Pubkey, mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"vault", market.as_ref(), mint.as_ref()],
        &MANIFEST_PROGRAM_ID,
    )
}

/// Get the global account PDA for a mint.
pub fn get_global_address(mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"global", mint.as_ref()], &MANIFEST_PROGRAM_ID)
}

/// Get the global vault PDA for a mint.
pub fn get_global_vault_address(mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"global-vault", mint.as_ref()], &MANIFEST_PROGRAM_ID)
}

/// Create a CreateMarket instruction.
///
/// # Accounts
/// 0. `[writable, signer]` payer - Pays for account creation
/// 1. `[writable]` market - The market account to create
/// 2. `[]` system_program - System program
/// 3. `[]` base_mint - Base token mint
/// 4. `[]` quote_mint - Quote token mint
/// 5. `[writable]` base_vault - Base token vault PDA
/// 6. `[writable]` quote_vault - Quote token vault PDA
/// 7. `[]` token_program - SPL Token program
/// 8. `[]` token_program_22 - SPL Token-2022 program
pub fn create_market_instruction(
    payer: Pubkey,
    market: Pubkey,
    base_mint: Pubkey,
    quote_mint: Pubkey,
    token_program: Pubkey,
    token_program_22: Pubkey,
) -> Instruction {
    let (base_vault, _) = get_vault_address(&market, &base_mint);
    let (quote_vault, _) = get_vault_address(&market, &quote_mint);

    Instruction::new_with_bytes(
        MANIFEST_PROGRAM_ID,
        &[ManifestInstruction::CreateMarket as u8],
        vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(market, true),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(token_program, false),
            AccountMeta::new_readonly(token_program_22, false),
        ],
    )
}

/// Create a ClaimSeat instruction.
///
/// # Accounts
/// 0. `[writable, signer]` payer - The trader claiming a seat
/// 1. `[writable]` market - The market account
/// 2. `[]` system_program - System program
pub fn claim_seat_instruction(payer: Pubkey, market: Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        MANIFEST_PROGRAM_ID,
        &[ManifestInstruction::ClaimSeat as u8],
        vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(market, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
        ],
    )
}

/// Parameters for deposit instruction.
#[derive(Debug, Clone)]
pub struct DepositParams {
    pub amount_atoms: u64,
    pub trader_index_hint: Option<DataIndex>,
}

impl DepositParams {
    pub fn new(amount_atoms: u64) -> Self {
        Self {
            amount_atoms,
            trader_index_hint: None,
        }
    }

    pub fn with_hint(amount_atoms: u64, trader_index_hint: DataIndex) -> Self {
        Self {
            amount_atoms,
            trader_index_hint: Some(trader_index_hint),
        }
    }

    fn serialize(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(13);
        data.extend_from_slice(&self.amount_atoms.to_le_bytes());
        match self.trader_index_hint {
            Some(hint) => {
                data.push(1); // Option::Some
                data.extend_from_slice(&hint.to_le_bytes());
            }
            None => {
                data.push(0); // Option::None
            }
        }
        data
    }
}

/// Create a Deposit instruction.
///
/// # Accounts
/// 0. `[writable, signer]` payer - The trader depositing
/// 1. `[writable]` market - The market account
/// 2. `[writable]` trader_token - Trader's token account
/// 3. `[writable]` vault - Market vault PDA
/// 4. `[]` token_program - Token program
/// 5. `[]` mint - Token mint (required for token-2022)
pub fn deposit_instruction(
    payer: Pubkey,
    market: Pubkey,
    trader_token: Pubkey,
    mint: Pubkey,
    token_program: Pubkey,
    params: DepositParams,
) -> Instruction {
    let (vault, _) = get_vault_address(&market, &mint);

    let mut data = vec![ManifestInstruction::Deposit as u8];
    data.extend_from_slice(&params.serialize());

    Instruction::new_with_bytes(
        MANIFEST_PROGRAM_ID,
        &data,
        vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(market, false),
            AccountMeta::new(trader_token, false),
            AccountMeta::new(vault, false),
            AccountMeta::new_readonly(token_program, false),
            AccountMeta::new_readonly(mint, false),
        ],
    )
}

/// Parameters for withdraw instruction.
#[derive(Debug, Clone)]
pub struct WithdrawParams {
    pub amount_atoms: u64,
    pub trader_index_hint: Option<DataIndex>,
}

impl WithdrawParams {
    pub fn new(amount_atoms: u64) -> Self {
        Self {
            amount_atoms,
            trader_index_hint: None,
        }
    }

    pub fn with_hint(amount_atoms: u64, trader_index_hint: DataIndex) -> Self {
        Self {
            amount_atoms,
            trader_index_hint: Some(trader_index_hint),
        }
    }

    fn serialize(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(13);
        data.extend_from_slice(&self.amount_atoms.to_le_bytes());
        match self.trader_index_hint {
            Some(hint) => {
                data.push(1);
                data.extend_from_slice(&hint.to_le_bytes());
            }
            None => {
                data.push(0);
            }
        }
        data
    }
}

/// Create a Withdraw instruction.
///
/// # Accounts
/// 0. `[writable, signer]` payer - The trader withdrawing
/// 1. `[writable]` market - The market account
/// 2. `[writable]` trader_token - Trader's token account
/// 3. `[writable]` vault - Market vault PDA
/// 4. `[]` token_program - Token program
/// 5. `[]` mint - Token mint
pub fn withdraw_instruction(
    payer: Pubkey,
    market: Pubkey,
    trader_token: Pubkey,
    mint: Pubkey,
    token_program: Pubkey,
    params: WithdrawParams,
) -> Instruction {
    let (vault, _) = get_vault_address(&market, &mint);

    let mut data = vec![ManifestInstruction::Withdraw as u8];
    data.extend_from_slice(&params.serialize());

    Instruction::new_with_bytes(
        MANIFEST_PROGRAM_ID,
        &data,
        vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(market, false),
            AccountMeta::new(trader_token, false),
            AccountMeta::new(vault, false),
            AccountMeta::new_readonly(token_program, false),
            AccountMeta::new_readonly(mint, false),
        ],
    )
}

/// Parameters for swap instruction.
#[derive(Debug, Clone)]
pub struct SwapParams {
    pub in_atoms: u64,
    pub out_atoms: u64,
    pub is_base_in: bool,
    pub is_exact_in: bool,
}

impl SwapParams {
    pub fn new(in_atoms: u64, out_atoms: u64, is_base_in: bool, is_exact_in: bool) -> Self {
        Self {
            in_atoms,
            out_atoms,
            is_base_in,
            is_exact_in,
        }
    }

    fn serialize(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(18);
        data.extend_from_slice(&self.in_atoms.to_le_bytes());
        data.extend_from_slice(&self.out_atoms.to_le_bytes());
        data.push(self.is_base_in as u8);
        data.push(self.is_exact_in as u8);
        data
    }
}

/// Create a Swap instruction.
///
/// # Accounts
/// 0. `[signer]` payer - Payer for potential expansion
/// 1. `[writable]` market - The market account
/// 2. `[]` system_program - System program
/// 3. `[writable]` trader_base - Trader's base token account
/// 4. `[writable]` trader_quote - Trader's quote token account
/// 5. `[writable]` base_vault - Market base vault PDA
/// 6. `[writable]` quote_vault - Market quote vault PDA
/// 7. `[]` token_program_base - Token program for base
/// 8. `[]` base_mint (optional, for token-2022)
/// 9. `[]` token_program_quote (optional, if different from base)
/// 10. `[]` quote_mint (optional, for token-2022)
pub fn swap_instruction(
    payer: Pubkey,
    market: Pubkey,
    trader_base: Pubkey,
    trader_quote: Pubkey,
    base_mint: Pubkey,
    quote_mint: Pubkey,
    token_program_base: Pubkey,
    token_program_quote: Option<Pubkey>,
    include_base_mint: bool,
    include_quote_mint: bool,
    params: SwapParams,
) -> Instruction {
    let (base_vault, _) = get_vault_address(&market, &base_mint);
    let (quote_vault, _) = get_vault_address(&market, &quote_mint);

    let mut accounts = vec![
        AccountMeta::new_readonly(payer, true),
        AccountMeta::new(market, false),
        AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
        AccountMeta::new(trader_base, false),
        AccountMeta::new(trader_quote, false),
        AccountMeta::new(base_vault, false),
        AccountMeta::new(quote_vault, false),
        AccountMeta::new_readonly(token_program_base, false),
    ];

    // Add optional accounts based on token types
    if include_base_mint {
        accounts.push(AccountMeta::new_readonly(base_mint, false));
    }

    if let Some(quote_program) = token_program_quote {
        accounts.push(AccountMeta::new_readonly(quote_program, false));
    }

    if include_quote_mint {
        accounts.push(AccountMeta::new_readonly(quote_mint, false));
    }

    let mut data = vec![ManifestInstruction::Swap as u8];
    data.extend_from_slice(&params.serialize());

    Instruction::new_with_bytes(MANIFEST_PROGRAM_ID, &data, accounts)
}

/// Parameters for placing a single order.
#[derive(Debug, Clone)]
pub struct PlaceOrderParams {
    pub base_atoms: u64,
    pub price_mantissa: u32,
    pub price_exponent: i8,
    pub is_bid: bool,
    pub last_valid_slot: u32,
    pub order_type: OrderType,
}

impl PlaceOrderParams {
    pub fn new(
        base_atoms: u64,
        price_mantissa: u32,
        price_exponent: i8,
        is_bid: bool,
        order_type: OrderType,
    ) -> Self {
        Self {
            base_atoms,
            price_mantissa,
            price_exponent,
            is_bid,
            last_valid_slot: NO_EXPIRATION_LAST_VALID_SLOT,
            order_type,
        }
    }

    pub fn with_expiration(mut self, last_valid_slot: u32) -> Self {
        self.last_valid_slot = last_valid_slot;
        self
    }

    fn serialize(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(16);
        data.extend_from_slice(&self.base_atoms.to_le_bytes());
        data.extend_from_slice(&self.price_mantissa.to_le_bytes());
        data.push(self.price_exponent as u8);
        data.push(self.is_bid as u8);
        data.extend_from_slice(&self.last_valid_slot.to_le_bytes());
        data.push(self.order_type as u8);
        data
    }
}

/// Parameters for cancelling an order.
#[derive(Debug, Clone)]
pub struct CancelOrderParams {
    pub order_sequence_number: u64,
    pub order_index_hint: Option<DataIndex>,
}

impl CancelOrderParams {
    pub fn new(order_sequence_number: u64) -> Self {
        Self {
            order_sequence_number,
            order_index_hint: None,
        }
    }

    pub fn with_hint(order_sequence_number: u64, order_index_hint: DataIndex) -> Self {
        Self {
            order_sequence_number,
            order_index_hint: Some(order_index_hint),
        }
    }

    fn serialize(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(13);
        data.extend_from_slice(&self.order_sequence_number.to_le_bytes());
        match self.order_index_hint {
            Some(hint) => {
                data.push(1);
                data.extend_from_slice(&hint.to_le_bytes());
            }
            None => {
                data.push(0);
            }
        }
        data
    }
}

/// Parameters for batch update (place orders and cancel orders).
#[derive(Debug, Clone, Default)]
pub struct BatchUpdateParams {
    pub trader_index_hint: Option<DataIndex>,
    pub cancels: Vec<CancelOrderParams>,
    pub orders: Vec<PlaceOrderParams>,
}

impl BatchUpdateParams {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_hint(mut self, trader_index_hint: DataIndex) -> Self {
        self.trader_index_hint = Some(trader_index_hint);
        self
    }

    pub fn add_cancel(mut self, cancel: CancelOrderParams) -> Self {
        self.cancels.push(cancel);
        self
    }

    pub fn add_order(mut self, order: PlaceOrderParams) -> Self {
        self.orders.push(order);
        self
    }

    fn serialize(&self) -> Vec<u8> {
        let mut data = Vec::new();

        // trader_index_hint
        match self.trader_index_hint {
            Some(hint) => {
                data.push(1);
                data.extend_from_slice(&hint.to_le_bytes());
            }
            None => {
                data.push(0);
            }
        }

        // cancels (Vec<CancelOrderParams>)
        let cancels_len = self.cancels.len() as u32;
        data.extend_from_slice(&cancels_len.to_le_bytes());
        for cancel in &self.cancels {
            data.extend_from_slice(&cancel.serialize());
        }

        // orders (Vec<PlaceOrderParams>)
        let orders_len = self.orders.len() as u32;
        data.extend_from_slice(&orders_len.to_le_bytes());
        for order in &self.orders {
            data.extend_from_slice(&order.serialize());
        }

        data
    }
}

/// Create a BatchUpdate instruction for placing and cancelling orders.
///
/// # Accounts
/// 0. `[writable, signer]` payer - The trader
/// 1. `[writable]` market - The market account
/// 2. `[]` system_program - System program
/// 3-7. Optional global accounts for base if placing global orders
/// 8-12. Optional global accounts for quote if placing global orders
pub fn batch_update_instruction(
    payer: Pubkey,
    market: Pubkey,
    params: BatchUpdateParams,
) -> Instruction {
    let mut data = vec![ManifestInstruction::BatchUpdate as u8];
    data.extend_from_slice(&params.serialize());

    Instruction::new_with_bytes(
        MANIFEST_PROGRAM_ID,
        &data,
        vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(market, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
        ],
    )
}

/// Create a BatchUpdate instruction with global accounts for global orders.
pub fn batch_update_with_global_instruction(
    payer: Pubkey,
    market: Pubkey,
    base_mint: Option<Pubkey>,
    quote_mint: Option<Pubkey>,
    base_token_program: Option<Pubkey>,
    quote_token_program: Option<Pubkey>,
    params: BatchUpdateParams,
) -> Instruction {
    let mut accounts = vec![
        AccountMeta::new(payer, true),
        AccountMeta::new(market, false),
        AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
    ];

    // Add base global accounts if needed
    if let Some(mint) = base_mint {
        let (global, _) = get_global_address(&mint);
        let (global_vault, _) = get_global_vault_address(&mint);
        let (market_vault, _) = get_vault_address(&market, &mint);
        let token_program = base_token_program.unwrap_or(TOKEN_PROGRAM_ID);

        accounts.push(AccountMeta::new_readonly(mint, false));
        accounts.push(AccountMeta::new(global, false));
        accounts.push(AccountMeta::new_readonly(global_vault, false));
        accounts.push(AccountMeta::new_readonly(market_vault, false));
        accounts.push(AccountMeta::new_readonly(token_program, false));
    }

    // Add quote global accounts if needed
    if let Some(mint) = quote_mint {
        let (global, _) = get_global_address(&mint);
        let (global_vault, _) = get_global_vault_address(&mint);
        let (market_vault, _) = get_vault_address(&market, &mint);
        let token_program = quote_token_program.unwrap_or(TOKEN_PROGRAM_ID);

        accounts.push(AccountMeta::new_readonly(mint, false));
        accounts.push(AccountMeta::new(global, false));
        accounts.push(AccountMeta::new_readonly(global_vault, false));
        accounts.push(AccountMeta::new_readonly(market_vault, false));
        accounts.push(AccountMeta::new_readonly(token_program, false));
    }

    let mut data = vec![ManifestInstruction::BatchUpdate as u8];
    data.extend_from_slice(&params.serialize());

    Instruction::new_with_bytes(MANIFEST_PROGRAM_ID, &data, accounts)
}

/// Create an Expand instruction to pre-allocate space on a market.
///
/// # Accounts
/// 0. `[writable, signer]` payer - Pays for expansion
/// 1. `[writable]` market - The market account
/// 2. `[]` system_program - System program
pub fn expand_instruction(payer: Pubkey, market: Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        MANIFEST_PROGRAM_ID,
        &[ManifestInstruction::Expand as u8],
        vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(market, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
        ],
    )
}
