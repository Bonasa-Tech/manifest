#[macro_export]
macro_rules! create_empty_market {
    ($market_acc_info:expr) => {{
        let empty_market_fixed: MarketFixed = MarketFixed::new_nondet();
        let mut market_bytes: std::cell::RefMut<&mut [u8]> =
            $market_acc_info.data.try_borrow_mut().unwrap();
        *get_mut_helper::<MarketFixed>(*market_bytes, 0_u32) = empty_market_fixed;
    }};
}

#[macro_export]
/// Write a `GlobalFixed` with nondeterministic contents into the account, so
/// that it passes the manifest account checks.
macro_rules! create_global {
    ($global_acc_info:expr) => {{
        let global_fixed: crate::state::GlobalFixed = crate::state::GlobalFixed::new_nondet();
        let mut global_bytes: std::cell::RefMut<&mut [u8]> =
            $global_acc_info.data.try_borrow_mut().unwrap();
        *hypertree::get_mut_helper::<crate::state::GlobalFixed>(*global_bytes, 0_u32) =
            global_fixed;
    }};
}

#[macro_export]
/// Ghost sum of every balance held on the global account.
macro_rules! get_global_deposited_atoms {
    ($global_acc_info:expr) => {{
        let global_data: &mut std::cell::RefMut<&mut [u8]> =
            &mut $global_acc_info.try_borrow_mut_data().unwrap();
        let global_dynamic_account: crate::state::GlobalRefMut =
            crate::program::get_mut_dynamic_account(global_data);
        let res = global_dynamic_account
            .fixed
            .get_global_deposited_atoms()
            .as_u64();
        cvt_assume!(res <= nondet::<u64>());
        res
    }};
}

#[macro_export]
macro_rules! claim_seat {
    ($market_acc_info:expr, $trader_key: expr) => {{
        let market_data: &mut std::cell::RefMut<&mut [u8]> =
            &mut $market_acc_info.try_borrow_mut_data().unwrap();
        let mut dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);
        dynamic_account.claim_seat($trader_key).unwrap();
    }};
}

#[macro_export]
macro_rules! get_trader_index {
    ($market_acc_info:expr, $trader_key: expr) => {{
        let market_data: &mut std::cell::RefMut<&mut [u8]> =
            &mut $market_acc_info.try_borrow_mut_data().unwrap();
        let dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);
        dynamic_account.get_trader_index($trader_key)
    }};
}

#[macro_export]
/// Return a pair of (base_atoms, quote_atoms) as u64
macro_rules! get_trader_balance {
    ($market_acc_info:expr, $trader_key: expr) => {{
        let market_data: &mut std::cell::RefMut<&mut [u8]> =
            &mut $market_acc_info.try_borrow_mut_data().unwrap();
        let dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);
        let (base_atoms, quote_atoms) = dynamic_account.get_trader_balance($trader_key);
        let res = (u64::from(base_atoms), u64::from(quote_atoms));
        cvt_assume!(res.0 <= nondet::<u64>());
        cvt_assume!(res.1 <= nondet::<u64>());
        res
    }};
}

#[macro_export]
macro_rules! update_balance {
    ($market_acc_info:expr, $trader_index: expr, $is_base: expr, $is_increase: expr, $amount: expr) => {{
        let market_data: &mut std::cell::RefMut<&mut [u8]> =
            &mut $market_acc_info.try_borrow_mut_data().unwrap();
        let dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);
        let DynamicAccount { fixed, dynamic } = dynamic_account;
        crate::state::update_balance(
            fixed,
            dynamic,
            $trader_index,
            $is_base,
            $is_increase,
            $amount,
        )
        .unwrap();
    }};
}

#[macro_export]
macro_rules! cvt_assert_is_nil {
    ($e:expr) => {
        cvt_assert!(is_nil!($e))
    };
}

#[macro_export]
macro_rules! deposit {
    ($market_acc_info:expr, $trader_key: expr, $in_atoms: expr, $is_base_in: expr) => {{
        let market_data: &mut std::cell::RefMut<&mut [u8]> =
            &mut $market_acc_info.try_borrow_mut_data().unwrap();
        let mut dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);
        dynamic_account
            .deposit($trader_key, $in_atoms, $is_base_in)
            .unwrap();
    }};
}

#[macro_export]
/// Return the base token vault
macro_rules! get_base_vault {
    ($market_acc_info:expr) => {{
        let market_data: &mut std::cell::RefMut<&mut [u8]> =
            &mut $market_acc_info.try_borrow_mut_data().unwrap();
        let dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);
        let DynamicAccount { fixed, .. } = dynamic_account;
        *fixed.get_base_vault()
    }};
}
#[macro_export]

/// Return the quote token vault
macro_rules! get_quote_vault {
    ($market_acc_info:expr) => {{
        let market_data: &mut std::cell::RefMut<&mut [u8]> =
            &mut $market_acc_info.try_borrow_mut_data().unwrap();
        let dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);
        let DynamicAccount { fixed, .. } = dynamic_account;
        *fixed.get_quote_vault()
    }};
}

#[macro_export]
/// Return the withdrawable base token amount
macro_rules! get_withdrawable_base_atoms {
    ($market_acc_info:expr) => {{
        let market_data: &mut std::cell::RefMut<&mut [u8]> =
            &mut $market_acc_info.try_borrow_mut_data().unwrap();
        let dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);
        let DynamicAccount { fixed, .. } = dynamic_account;
        let res = fixed.get_withdrawable_base_atoms().as_u64();
        cvt_assume!(res <= nondet::<u64>());
        res
    }};
}
#[macro_export]
/// Return the withdrawable quote token amount
macro_rules! get_withdrawable_quote_atoms {
    ($market_acc_info:expr) => {{
        let market_data: &mut std::cell::RefMut<&mut [u8]> =
            &mut $market_acc_info.try_borrow_mut_data().unwrap();
        let dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);
        let DynamicAccount { fixed, .. } = dynamic_account;
        let res = fixed.get_withdrawable_quote_atoms().as_u64();
        cvt_assume!(res <= nondet::<u64>());
        res
    }};
}
#[macro_export]
/// Return the orderbook base token amount
macro_rules! get_orderbook_base_atoms {
    ($market_acc_info:expr) => {{
        let market_data: &mut std::cell::RefMut<&mut [u8]> =
            &mut $market_acc_info.try_borrow_mut_data().unwrap();
        let dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);
        let DynamicAccount { fixed, .. } = dynamic_account;
        let res = fixed.get_orderbook_base_atoms().as_u64();
        cvt_assume!(res <= nondet::<u64>());
        res
    }};
}
#[macro_export]
/// Return the orderbook quote token amount
macro_rules! get_orderbook_quote_atoms {
    ($market_acc_info:expr) => {{
        let market_data: &mut std::cell::RefMut<&mut [u8]> =
            &mut $market_acc_info.try_borrow_mut_data().unwrap();
        let dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);
        let DynamicAccount { fixed, .. } = dynamic_account;
        let res = fixed.get_orderbook_quote_atoms().as_u64();
        cvt_assume!(res <= nondet::<u64>());
        res
    }};
}

#[macro_export]
macro_rules! get_order_atoms {
    ($index:expr) => {{
        let dynamic: [u8; 8] = [0u8; 8];
        let order: &RestingOrder = get_helper_order(&dynamic, $index).get_value();
        order.get_orderbook_atoms().unwrap()
    }};
}

#[macro_export]
macro_rules! rest_remaining {
    ($market_acc_info:expr,
    $args:expr,
    $remaining_base_atoms: expr,
    $order_sequence_number: expr,
    $total_base_atoms_traded: expr,
    $total_quote_atoms_traded: expr) => {{
        let market_data: &mut std::cell::RefMut<&mut [u8]> =
            &mut $market_acc_info.try_borrow_mut_data().unwrap();
        let mut dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);
        // let DynamicAccount { fixed, .. } = dynamic_account;
        dynamic_account
            .certora_rest_remaining(
                $args,
                $remaining_base_atoms,
                $order_sequence_number,
                $total_base_atoms_traded,
                $total_quote_atoms_traded,
            )
            .unwrap()
    }};
}

#[macro_export]
macro_rules! cancel_order_by_index {
    (
        $market_acc_info:expr,
        $order_index:expr
    ) => {{
        $crate::cancel_order_by_index!($market_acc_info, $order_index, &[None, None])
    }};
    (
        $market_acc_info:expr,
        $order_index:expr,
        $global_trade_accounts_opts:expr
    ) => {{
        let market_data: &mut std::cell::RefMut<&mut [u8]> =
            &mut $market_acc_info.try_borrow_mut_data().unwrap();
        let mut dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);
        dynamic_account
            .cancel_order_by_index($order_index, $global_trade_accounts_opts)
            .unwrap();
    }};
}

#[macro_export]
macro_rules! place_single_order {
    (
        $market_acc_info:expr,
        $args:expr,
        $remaining_base_atoms: expr,
        $now_slot: expr,
        $current_order_index: expr
    ) => {{
        let (res, base, quote) = $crate::place_single_order_res!(
            $market_acc_info,
            $args,
            $remaining_base_atoms,
            $now_slot,
            $current_order_index
        );
        (res.unwrap(), base, quote)
    }};
}

#[macro_export]
/// One matching step against a global maker, followed by the settlement that
/// moves the tokens out of the global vault.
///
/// Matching only draws down the maker's balance on the global account.
/// `place_order` batches the token transfer and does it once, after the loop,
/// so a single step is only a faithful model of `place_order` when it is
/// settled the same way. Returns the atoms that were moved on top of what
/// `place_single_order!` returns.
macro_rules! place_single_order_and_settle_global {
    (
        $market_acc_info:expr,
        $args:expr,
        $remaining_base_atoms: expr,
        $now_slot: expr,
        $current_order_index: expr
    ) => {{
        let market_data: &mut std::cell::RefMut<&mut [u8]> =
            &mut $market_acc_info.try_borrow_mut_data().unwrap();
        let dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);
        let DynamicAccount { fixed, dynamic } = dynamic_account;

        let mut ctx: AddSingleOrderCtx =
            AddSingleOrderCtx::new($args, fixed, dynamic, $remaining_base_atoms, $now_slot);

        let res: AddOrderToMarketInnerResult =
            ctx.place_single_order($current_order_index).unwrap();

        let global_atoms_to_transfer: crate::quantities::GlobalAtoms = ctx.global_atoms_to_transfer;
        let global_trade_accounts_opt = if ctx.args.is_bid {
            &ctx.args.global_trade_accounts_opts[0]
        } else {
            &ctx.args.global_trade_accounts_opts[1]
        };
        crate::state::utils::transfer_global_tokens(
            global_trade_accounts_opt,
            global_atoms_to_transfer,
        )
        .unwrap();

        (
            res,
            ctx.total_base_atoms_traded,
            ctx.total_quote_atoms_traded,
            global_atoms_to_transfer,
        )
    }};
}

#[macro_export]
/// Like `place_single_order!` but hands back the `Result` so a rule can say
/// something about the orders that are rejected.
macro_rules! place_single_order_res {
    (
        $market_acc_info:expr,
        $args:expr,
        $remaining_base_atoms: expr,
        $now_slot: expr,
        $current_order_index: expr
    ) => {{
        let market_data: &mut std::cell::RefMut<&mut [u8]> =
            &mut $market_acc_info.try_borrow_mut_data().unwrap();
        let dynamic_account: MarketRefMut = get_mut_dynamic_account(market_data);
        let DynamicAccount { fixed, dynamic } = dynamic_account;

        let mut ctx: AddSingleOrderCtx =
            AddSingleOrderCtx::new($args, fixed, dynamic, $remaining_base_atoms, $now_slot);

        let res: Result<AddOrderToMarketInnerResult, solana_program::program_error::ProgramError> =
            ctx.place_single_order($current_order_index);
        (
            res,
            ctx.total_base_atoms_traded,
            ctx.total_quote_atoms_traded,
        )
    }};
}

extern "C" {
    fn memhavoc_c(data: *mut u8, sz: usize) -> ();
}
pub fn memhavoc(data: *mut u8, size: usize) {
    unsafe {
        memhavoc_c(data, size);
    }
}

pub fn alloc_havoced<T: Sized>() -> *mut T {
    use std::alloc::Layout;
    let layout = Layout::new::<T>();
    unsafe {
        let ptr = std::alloc::alloc(layout);
        memhavoc(ptr, layout.size());
        ptr as *mut T
    }
}
