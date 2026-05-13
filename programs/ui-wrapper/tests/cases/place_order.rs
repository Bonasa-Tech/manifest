use std::rc::Rc;

use borsh::BorshSerialize;
use hypertree::{
    get_helper, DataIndex, HyperTreeReadOperations, HyperTreeValueIteratorTrait, RBNode, NIL,
};
use manifest::{
    quantities::QuoteAtomsPerBaseAtom,
    state::{constants::NO_EXPIRATION_LAST_VALID_SLOT, OrderType, RestingOrder},
    validation::{get_global_address, get_global_vault_address, get_vault_address},
};
use solana_account::Account;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_program::{
    instruction::AccountMeta, program_pack::Pack, pubkey::Pubkey, system_instruction::transfer,
    system_program,
};
use solana_program_test::tokio;
use solana_signer::Signer;
use spl_token;
use spl_token_2022::extension::StateWithExtensions;
use ui_wrapper::{
    self,
    instruction::ManifestWrapperInstruction,
    instruction_builders::create_wrapper_instructions,
    market_info::MarketInfo,
    open_order::WrapperOpenOrder,
    processors::{
        cancel_order::WrapperCancelOrderParams,
        place_order::WrapperPlaceOrderParams,
        settle_funds::WrapperSettleFundsParams,
        shared::{MarketInfosTreeReadOnly, OpenOrdersTreeReadOnly},
    },
};

use crate::{
    send_tx_with_retry, TestFixture, Token, WrapperFixture, SOL_UNIT_SIZE, USDC_UNIT_SIZE,
};

#[tokio::test]
async fn wrapper_place_order_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;

    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
    let (base_mint, trader_token_account_base) = test_fixture
        .fund_trader_wallet(&payer_keypair, Token::SOL, 1)
        .await;
    let (quote_mint, trader_token_account_quote) = test_fixture
        .fund_trader_wallet(&payer_keypair, Token::USDC, 1)
        .await;

    let platform_token_account = test_fixture.fund_token_account(&quote_mint, &payer).await;
    let referred_token_account = test_fixture.fund_token_account(&quote_mint, &payer).await;

    let (quote_vault, _) = get_vault_address(&test_fixture.market.key, &quote_mint);
    let (base_vault, _) = get_vault_address(&test_fixture.market.key, &base_mint);
    let (global_base, _) = get_global_address(&base_mint);
    let (global_quote, _) = get_global_address(&quote_mint);
    let (global_base_vault, _) = get_global_vault_address(&base_mint);
    let (global_quote_vault, _) = get_global_vault_address(&quote_mint);

    // place order
    let place_order_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(test_fixture.wrapper.key, false),
            AccountMeta::new(payer, true),
            AccountMeta::new(trader_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(payer, true),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new(global_base, false),
            AccountMeta::new(global_base_vault, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new(global_quote, false),
            AccountMeta::new(global_quote_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: [
            ManifestWrapperInstruction::PlaceOrder.to_vec(),
            WrapperPlaceOrderParams::new(
                1,
                1,
                1,
                0,
                true,
                NO_EXPIRATION_LAST_VALID_SLOT,
                OrderType::Limit,
            )
            .try_to_vec()
            .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[place_order_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // verify order is on book
    test_fixture.market.reload().await;
    let trader_index = test_fixture.market.market.get_trader_index(&payer);

    let bids = test_fixture.market.market.get_bids();
    let found: Option<(DataIndex, &RestingOrder)> = bids
        .iter::<RestingOrder>()
        .find(|(_, o)| o.get_trader_index() == trader_index);
    assert!(found.is_some());
    let (core_index, order) = found.unwrap();
    assert_eq!(order.get_is_bid(), true);
    assert_eq!(order.get_num_base_atoms(), 1);

    // verify order is correctly tracked on wrapper
    test_fixture.wrapper.reload().await;

    let open_order: WrapperOpenOrder = {
        let market_infos_tree: MarketInfosTreeReadOnly = MarketInfosTreeReadOnly::new(
            &test_fixture.wrapper.wrapper.dynamic,
            test_fixture.wrapper.wrapper.fixed.market_infos_root_index,
            NIL,
        );

        let market_info_index: DataIndex =
            market_infos_tree.lookup_index(&MarketInfo::new_empty(test_fixture.market.key, NIL));

        let market_info: &MarketInfo = get_helper::<RBNode<MarketInfo>>(
            &test_fixture.wrapper.wrapper.dynamic,
            market_info_index,
        )
        .get_value();

        get_helper::<RBNode<WrapperOpenOrder>>(
            &test_fixture.wrapper.wrapper.dynamic,
            market_info.orders_root_index,
        )
        .get_value()
        .clone()
    };

    assert_eq!(open_order.get_is_bid(), true);
    assert_eq!(open_order.get_client_order_id(), 1);
    assert_eq!(open_order.get_num_base_atoms(), 1);
    assert_eq!(
        open_order.get_price(),
        QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(1, 0).unwrap()
    );
    assert_eq!(open_order.get_market_data_index(), core_index);

    // cancel the same order

    let cancel_order_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(test_fixture.wrapper.key, false),
            AccountMeta::new(payer, true),
            AccountMeta::new(trader_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
        ],
        data: [
            ManifestWrapperInstruction::CancelOrder.to_vec(),
            WrapperCancelOrderParams::new(1).try_to_vec().unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[cancel_order_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // verify order is no longer on book
    test_fixture.market.reload().await;
    let trader_index = test_fixture.market.market.get_trader_index(&payer);
    let bids = test_fixture.market.market.get_bids();
    let found: Option<(DataIndex, &RestingOrder)> = bids
        .iter::<RestingOrder>()
        .find(|(_, o)| o.get_trader_index() == trader_index);
    assert!(found.is_none());

    // verify order is no longer tracked on wrapper
    test_fixture.wrapper.reload().await;

    let market_info_index: DataIndex = {
        let market_infos_tree: MarketInfosTreeReadOnly = MarketInfosTreeReadOnly::new(
            &test_fixture.wrapper.wrapper.dynamic,
            test_fixture.wrapper.wrapper.fixed.market_infos_root_index,
            NIL,
        );
        market_infos_tree.lookup_index(&MarketInfo::new_empty(test_fixture.market.key, NIL))
    };

    let orders_root_index = {
        let market_info: &MarketInfo = get_helper::<RBNode<MarketInfo>>(
            &test_fixture.wrapper.wrapper.dynamic,
            market_info_index,
        )
        .get_value();

        market_info.orders_root_index
    };

    let open_orders_tree: OpenOrdersTreeReadOnly = OpenOrdersTreeReadOnly::new(
        &test_fixture.wrapper.wrapper.dynamic,
        orders_root_index,
        NIL,
    );
    let found = open_orders_tree
        .iter::<WrapperOpenOrder>()
        .find(|(_, o)| o.get_client_order_id() == 1);
    assert!(found.is_none());

    // release funds
    let settle_funds_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(test_fixture.wrapper.key, false),
            AccountMeta::new(payer, true),
            AccountMeta::new(trader_token_account_base, false),
            AccountMeta::new(trader_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(platform_token_account, false),
            AccountMeta::new(referred_token_account, false),
        ],
        data: [
            ManifestWrapperInstruction::SettleFunds.to_vec(),
            WrapperSettleFundsParams::new(1_000_000_000, 50)
                .try_to_vec()
                .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[settle_funds_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // verify no fees were charged and user has deposit back in his wallet
    let trader_token_account_quote: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(trader_token_account_quote)
        .await
        .unwrap()
        .unwrap();

    let trader_token_account_quote =
        spl_token::state::Account::unpack(&trader_token_account_quote.data)?;
    assert_eq!(trader_token_account_quote.amount, 1);

    Ok(())
}

#[tokio::test]
async fn wrapper_place_order_with_broke_owner_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;

    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
    let owner: Pubkey = test_fixture.second_keypair.pubkey();
    let owner_keypair: Keypair = test_fixture.second_keypair.insecure_clone();

    // setup wrapper for owner
    let mut wrapper_fixture: WrapperFixture = {
        let wrapper_keypair = Keypair::new();

        let create_wrapper_ixs: Vec<Instruction> =
            create_wrapper_instructions(&payer, &owner, &wrapper_keypair.pubkey()).unwrap();

        send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &create_wrapper_ixs,
            Some(&payer),
            &[&payer_keypair, &owner_keypair, &wrapper_keypair],
        )
        .await
        .unwrap();

        WrapperFixture::new(Rc::clone(&test_fixture.context), wrapper_keypair.pubkey()).await
    };

    let (base_mint, trader_token_account_base) = test_fixture
        .fund_trader_wallet(&owner_keypair, Token::SOL, 0)
        .await;
    let (quote_mint, trader_token_account_quote) = test_fixture
        .fund_trader_wallet(&owner_keypair, Token::USDC, 1)
        .await;

    // deplete all gas of owner
    {
        let owner_balance = test_fixture
            .context
            .borrow_mut()
            .banks_client
            .get_balance(owner)
            .await?;
        let ix = transfer(&owner, &payer, owner_balance);
        send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &[ix],
            Some(&payer),
            &[&payer_keypair, &owner_keypair],
        )
        .await
        .unwrap();
    }

    let platform_token_account = test_fixture.fund_token_account(&quote_mint, &payer).await;
    let referred_token_account = test_fixture.fund_token_account(&quote_mint, &payer).await;
    let (base_vault, _) = get_vault_address(&test_fixture.market.key, &base_mint);

    let (quote_vault, _) = get_vault_address(&test_fixture.market.key, &quote_mint);
    let (global_base, _) = get_global_address(&base_mint);
    let (global_quote, _) = get_global_address(&quote_mint);
    let (global_base_vault, _) = get_global_vault_address(&base_mint);
    let (global_quote_vault, _) = get_global_vault_address(&quote_mint);

    // place order
    let place_order_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(wrapper_fixture.key, false),
            AccountMeta::new(owner, true),
            AccountMeta::new(trader_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(payer, true),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new(global_base, false),
            AccountMeta::new(global_base_vault, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new(global_quote, false),
            AccountMeta::new(global_quote_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: [
            ManifestWrapperInstruction::PlaceOrder.to_vec(),
            WrapperPlaceOrderParams::new(
                1,
                1,
                1,
                0,
                true,
                NO_EXPIRATION_LAST_VALID_SLOT,
                OrderType::Limit,
            )
            .try_to_vec()
            .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[place_order_ix],
        Some(&payer),
        &[&payer_keypair, &owner_keypair],
    )
    .await?;

    // verify order is on book
    test_fixture.market.reload().await;
    let trader_index = test_fixture.market.market.get_trader_index(&owner);
    let bids = test_fixture.market.market.get_bids();
    let found: Option<(DataIndex, &RestingOrder)> = bids
        .iter::<RestingOrder>()
        .find(|(_, o)| o.get_trader_index() == trader_index);
    assert!(found.is_some());
    let (core_index, order) = found.unwrap();
    assert_eq!(order.get_is_bid(), true);
    assert_eq!(order.get_num_base_atoms(), 1);

    // verify order is correctly tracked on wrapper
    wrapper_fixture.reload().await;

    let open_order: WrapperOpenOrder = {
        let market_infos_tree: MarketInfosTreeReadOnly = MarketInfosTreeReadOnly::new(
            &wrapper_fixture.wrapper.dynamic,
            wrapper_fixture.wrapper.fixed.market_infos_root_index,
            NIL,
        );

        let market_info_index: DataIndex =
            market_infos_tree.lookup_index(&MarketInfo::new_empty(test_fixture.market.key, NIL));

        let market_info: &MarketInfo =
            get_helper::<RBNode<MarketInfo>>(&wrapper_fixture.wrapper.dynamic, market_info_index)
                .get_value();

        get_helper::<RBNode<WrapperOpenOrder>>(
            &wrapper_fixture.wrapper.dynamic,
            market_info.orders_root_index,
        )
        .get_value()
        .clone()
    };

    assert_eq!(open_order.get_is_bid(), true);
    assert_eq!(open_order.get_client_order_id(), 1);
    assert_eq!(open_order.get_num_base_atoms(), 1);
    assert_eq!(
        open_order.get_price(),
        QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(1, 0).unwrap()
    );
    assert_eq!(open_order.get_market_data_index(), core_index);

    // cancel the same order

    let cancel_order_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(wrapper_fixture.key, false),
            AccountMeta::new(owner, true),
            AccountMeta::new(trader_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
        ],
        data: [
            ManifestWrapperInstruction::CancelOrder.to_vec(),
            WrapperCancelOrderParams::new(1).try_to_vec().unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[cancel_order_ix],
        Some(&payer),
        &[&payer_keypair, &owner_keypair],
    )
    .await?;

    // verify order is no longer on book
    test_fixture.market.reload().await;
    let trader_index = test_fixture.market.market.get_trader_index(&payer);

    let bids = test_fixture.market.market.get_bids();
    let found: Option<(DataIndex, &RestingOrder)> = bids
        .iter::<RestingOrder>()
        .find(|(_, o)| o.get_trader_index() == trader_index);
    assert!(found.is_none());

    // verify order is no longer tracked on wrapper
    wrapper_fixture.reload().await;

    let market_info_index: DataIndex = {
        let market_infos_tree: MarketInfosTreeReadOnly = MarketInfosTreeReadOnly::new(
            &wrapper_fixture.wrapper.dynamic,
            wrapper_fixture.wrapper.fixed.market_infos_root_index,
            NIL,
        );
        market_infos_tree.lookup_index(&MarketInfo::new_empty(test_fixture.market.key, NIL))
    };

    let orders_root_index = {
        let market_info: &MarketInfo =
            get_helper::<RBNode<MarketInfo>>(&wrapper_fixture.wrapper.dynamic, market_info_index)
                .get_value();

        market_info.orders_root_index
    };

    let open_orders_tree: OpenOrdersTreeReadOnly =
        OpenOrdersTreeReadOnly::new(&wrapper_fixture.wrapper.dynamic, orders_root_index, NIL);
    let found = open_orders_tree
        .iter::<WrapperOpenOrder>()
        .find(|(_, o)| o.get_client_order_id() == 1);
    assert!(found.is_none());

    // release funds
    let settle_funds_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(wrapper_fixture.key, false),
            AccountMeta::new(owner, true),
            AccountMeta::new(trader_token_account_base, false),
            AccountMeta::new(trader_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(platform_token_account, false),
            AccountMeta::new(referred_token_account, false),
        ],
        data: [
            ManifestWrapperInstruction::SettleFunds.to_vec(),
            WrapperSettleFundsParams::new(1_000_000_000, 50)
                .try_to_vec()
                .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[settle_funds_ix],
        Some(&payer),
        &[&payer_keypair, &owner_keypair],
    )
    .await?;

    // verify no fees were charged and user has deposit back in his wallet
    let trader_token_account_quote: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(trader_token_account_quote)
        .await
        .unwrap()
        .unwrap();

    let trader_token_account_quote =
        spl_token::state::Account::unpack(&trader_token_account_quote.data)?;
    assert_eq!(trader_token_account_quote.amount, 1);

    Ok(())
}

#[tokio::test]
async fn wrapper_place_order_without_globals_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;

    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
    let (base_mint, trader_token_account_base) = test_fixture
        .fund_trader_wallet(&payer_keypair, Token::SOL, 1)
        .await;
    let (quote_mint, trader_token_account_quote) = test_fixture
        .fund_trader_wallet(&payer_keypair, Token::USDC, 1)
        .await;

    let platform_token_account = test_fixture.fund_token_account(&quote_mint, &payer).await;
    let referred_token_account = test_fixture.fund_token_account(&quote_mint, &payer).await;

    let (quote_vault, _) = get_vault_address(&test_fixture.market.key, &quote_mint);
    let (base_vault, _) = get_vault_address(&test_fixture.market.key, &base_mint);
    let (_global_base, _) = get_global_address(&base_mint);
    let (_global_quote, _) = get_global_address(&quote_mint);
    let (_global_base_vault, _) = get_global_vault_address(&base_mint);
    let (_global_quote_vault, _) = get_global_vault_address(&quote_mint);

    // place order
    let place_order_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(test_fixture.wrapper.key, false),
            AccountMeta::new(payer, true),
            AccountMeta::new(trader_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(payer, true),
        ],
        data: [
            ManifestWrapperInstruction::PlaceOrder.to_vec(),
            WrapperPlaceOrderParams::new(
                1,
                1,
                1,
                0,
                true,
                NO_EXPIRATION_LAST_VALID_SLOT,
                OrderType::Limit,
            )
            .try_to_vec()
            .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[place_order_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // verify order is on book
    test_fixture.market.reload().await;
    let trader_index = test_fixture.market.market.get_trader_index(&payer);

    let bids = test_fixture.market.market.get_bids();
    let found: Option<(DataIndex, &RestingOrder)> = bids
        .iter::<RestingOrder>()
        .find(|(_, o)| o.get_trader_index() == trader_index);
    assert!(found.is_some());
    let (core_index, order) = found.unwrap();
    assert_eq!(order.get_is_bid(), true);
    assert_eq!(order.get_num_base_atoms(), 1);

    // verify order is correctly tracked on wrapper
    test_fixture.wrapper.reload().await;

    let open_order: WrapperOpenOrder = {
        let market_infos_tree: MarketInfosTreeReadOnly = MarketInfosTreeReadOnly::new(
            &test_fixture.wrapper.wrapper.dynamic,
            test_fixture.wrapper.wrapper.fixed.market_infos_root_index,
            NIL,
        );

        let market_info_index: DataIndex =
            market_infos_tree.lookup_index(&MarketInfo::new_empty(test_fixture.market.key, NIL));

        let market_info: &MarketInfo = get_helper::<RBNode<MarketInfo>>(
            &test_fixture.wrapper.wrapper.dynamic,
            market_info_index,
        )
        .get_value();

        get_helper::<RBNode<WrapperOpenOrder>>(
            &test_fixture.wrapper.wrapper.dynamic,
            market_info.orders_root_index,
        )
        .get_value()
        .clone()
    };

    assert_eq!(open_order.get_is_bid(), true);
    assert_eq!(open_order.get_client_order_id(), 1);
    assert_eq!(open_order.get_num_base_atoms(), 1);
    assert_eq!(
        open_order.get_price(),
        QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(1, 0).unwrap()
    );
    assert_eq!(open_order.get_market_data_index(), core_index);

    // cancel the same order

    let cancel_order_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(test_fixture.wrapper.key, false),
            AccountMeta::new(payer, true),
            AccountMeta::new(trader_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
        ],
        data: [
            ManifestWrapperInstruction::CancelOrder.to_vec(),
            WrapperCancelOrderParams::new(1).try_to_vec().unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[cancel_order_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // verify order is no longer on book
    test_fixture.market.reload().await;
    let trader_index = test_fixture.market.market.get_trader_index(&payer);
    let bids = test_fixture.market.market.get_bids();
    let found: Option<(DataIndex, &RestingOrder)> = bids
        .iter::<RestingOrder>()
        .find(|(_, o)| o.get_trader_index() == trader_index);
    assert!(found.is_none());

    // verify order is no longer tracked on wrapper
    test_fixture.wrapper.reload().await;

    let market_info_index: DataIndex = {
        let market_infos_tree: MarketInfosTreeReadOnly = MarketInfosTreeReadOnly::new(
            &test_fixture.wrapper.wrapper.dynamic,
            test_fixture.wrapper.wrapper.fixed.market_infos_root_index,
            NIL,
        );
        market_infos_tree.lookup_index(&MarketInfo::new_empty(test_fixture.market.key, NIL))
    };

    let orders_root_index = {
        let market_info: &MarketInfo = get_helper::<RBNode<MarketInfo>>(
            &test_fixture.wrapper.wrapper.dynamic,
            market_info_index,
        )
        .get_value();

        market_info.orders_root_index
    };

    let open_orders_tree: OpenOrdersTreeReadOnly = OpenOrdersTreeReadOnly::new(
        &test_fixture.wrapper.wrapper.dynamic,
        orders_root_index,
        NIL,
    );
    let found = open_orders_tree
        .iter::<WrapperOpenOrder>()
        .find(|(_, o)| o.get_client_order_id() == 1);
    assert!(found.is_none());

    // release funds
    let settle_funds_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(test_fixture.wrapper.key, false),
            AccountMeta::new(payer, true),
            AccountMeta::new(trader_token_account_base, false),
            AccountMeta::new(trader_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(platform_token_account, false),
            AccountMeta::new(referred_token_account, false),
        ],
        data: [
            ManifestWrapperInstruction::SettleFunds.to_vec(),
            WrapperSettleFundsParams::new(1_000_000_000, 50)
                .try_to_vec()
                .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[settle_funds_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // verify no fees were charged and user has deposit back in his wallet
    let trader_token_account_quote: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(trader_token_account_quote)
        .await
        .unwrap()
        .unwrap();

    let trader_token_account_quote =
        spl_token::state::Account::unpack(&trader_token_account_quote.data)?;
    assert_eq!(trader_token_account_quote.amount, 1);

    Ok(())
}

#[tokio::test]
async fn wrapper_place_order_with_mixed_up_mint_ask() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;

    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
    let (base_mint, _trader_token_account_base) = test_fixture
        .fund_trader_wallet(&payer_keypair, Token::SOL, 1)
        .await;
    let (quote_mint, trader_token_account_quote) = test_fixture
        .fund_trader_wallet(&payer_keypair, Token::USDC, 1)
        .await;

    let _platform_token_account = test_fixture.fund_token_account(&quote_mint, &payer).await;
    let _referred_token_account = test_fixture.fund_token_account(&quote_mint, &payer).await;

    let (quote_vault, _) = get_vault_address(&test_fixture.market.key, &quote_mint);
    let (_base_vault, _) = get_vault_address(&test_fixture.market.key, &base_mint);
    let (_global_base, _) = get_global_address(&base_mint);
    let (_global_quote, _) = get_global_address(&quote_mint);
    let (_global_base_vault, _) = get_global_vault_address(&base_mint);
    let (_global_quote_vault, _) = get_global_vault_address(&quote_mint);

    // place order as ask, but passing quote currency should fail
    let place_order_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(test_fixture.wrapper.key, false),
            AccountMeta::new(payer, true),
            AccountMeta::new(trader_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(payer, true),
        ],
        data: [
            ManifestWrapperInstruction::PlaceOrder.to_vec(),
            WrapperPlaceOrderParams::new(
                1,
                1,
                1,
                0,
                false,
                NO_EXPIRATION_LAST_VALID_SLOT,
                OrderType::Limit,
            )
            .try_to_vec()
            .unwrap(),
        ]
        .concat(),
    };
    let result = send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[place_order_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await;

    assert!(result.is_err());
    Ok(())
}

#[tokio::test]
async fn wrapper_place_order_with_mixed_up_mint_bid() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;

    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
    let (base_mint, trader_token_account_base) = test_fixture
        .fund_trader_wallet(&payer_keypair, Token::SOL, 1)
        .await;
    let (quote_mint, _trader_token_account_quote) = test_fixture
        .fund_trader_wallet(&payer_keypair, Token::USDC, 1)
        .await;

    let _platform_token_account = test_fixture.fund_token_account(&quote_mint, &payer).await;
    let _referred_token_account = test_fixture.fund_token_account(&quote_mint, &payer).await;

    let (_quote_vault, _) = get_vault_address(&test_fixture.market.key, &quote_mint);
    let (base_vault, _) = get_vault_address(&test_fixture.market.key, &base_mint);
    let (_global_base, _) = get_global_address(&base_mint);
    let (_global_quote, _) = get_global_address(&quote_mint);
    let (_global_base_vault, _) = get_global_vault_address(&base_mint);
    let (_global_quote_vault, _) = get_global_vault_address(&quote_mint);

    // place order as ask, but passing quote currency should fail
    let place_order_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(test_fixture.wrapper.key, false),
            AccountMeta::new(payer, true),
            AccountMeta::new(trader_token_account_base, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(payer, true),
        ],
        data: [
            ManifestWrapperInstruction::PlaceOrder.to_vec(),
            WrapperPlaceOrderParams::new(
                1,
                1,
                1,
                0,
                true,
                NO_EXPIRATION_LAST_VALID_SLOT,
                OrderType::Limit,
            )
            .try_to_vec()
            .unwrap(),
        ]
        .concat(),
    };
    let result = send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[place_order_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await;

    assert!(result.is_err());
    Ok(())
}

#[tokio::test]
async fn wrapper_fill_order_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;

    let taker: Pubkey = test_fixture.payer();
    let taker_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
    let mut taker_wrapper_fixture: WrapperFixture = test_fixture.wrapper.clone();

    let maker: Pubkey = test_fixture.second_keypair.pubkey();
    let maker_keypair: Keypair = test_fixture.second_keypair.insecure_clone();

    // setup wrapper for maker
    let mut maker_wrapper_fixture: WrapperFixture = {
        let wrapper_keypair = Keypair::new();

        let create_wrapper_ixs: Vec<Instruction> =
            create_wrapper_instructions(&maker, &maker, &wrapper_keypair.pubkey())?;

        send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &create_wrapper_ixs,
            Some(&maker),
            &[&maker_keypair, &wrapper_keypair],
        )
        .await?;

        WrapperFixture::new(Rc::clone(&test_fixture.context), wrapper_keypair.pubkey()).await
    };

    // setup token accounts for taker, maker, platform & referrer
    let (_, taker_token_account_base) = test_fixture
        .fund_trader_wallet(&taker_keypair, Token::SOL, 1 * SOL_UNIT_SIZE)
        .await;
    let (_, taker_token_account_quote) = test_fixture
        .fund_trader_wallet(&taker_keypair, Token::USDC, 1 * USDC_UNIT_SIZE)
        .await;

    let (base_mint, maker_token_account_base) = test_fixture
        .fund_trader_wallet(&maker_keypair, Token::SOL, 1 * SOL_UNIT_SIZE)
        .await;
    let (quote_mint, maker_token_account_quote) = test_fixture
        .fund_trader_wallet(&maker_keypair, Token::USDC, 1 * USDC_UNIT_SIZE)
        .await;
    let platform_token_account = test_fixture.fund_token_account(&quote_mint, &taker).await;
    let referred_token_account = test_fixture.fund_token_account(&quote_mint, &taker).await;

    let (base_vault, _) = get_vault_address(&test_fixture.market.key, &base_mint);
    let (quote_vault, _) = get_vault_address(&test_fixture.market.key, &quote_mint);
    let (global_base, _) = get_global_address(&base_mint);
    let (global_quote, _) = get_global_address(&quote_mint);
    let (global_base_vault, _) = get_global_vault_address(&base_mint);
    let (global_quote_vault, _) = get_global_vault_address(&quote_mint);

    // maker buys 1 sol @ 1000 USDC
    let maker_order_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(maker_wrapper_fixture.key, false),
            AccountMeta::new(maker, true),
            AccountMeta::new(maker_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(maker, true),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new(global_base, false),
            AccountMeta::new(global_base_vault, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new(global_quote, false),
            AccountMeta::new(global_quote_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: [
            ManifestWrapperInstruction::PlaceOrder.to_vec(),
            WrapperPlaceOrderParams::new(
                1,
                1 * SOL_UNIT_SIZE,
                1,
                -3,
                true,
                NO_EXPIRATION_LAST_VALID_SLOT,
                OrderType::Limit,
            )
            .try_to_vec()
            .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[maker_order_ix],
        Some(&maker),
        &[&maker_keypair],
    )
    .await?;

    // verify order is on book
    test_fixture.market.reload().await;
    let maker_index = test_fixture.market.market.get_trader_index(&maker);

    let bids = test_fixture.market.market.get_bids();
    let found: Option<(DataIndex, &RestingOrder)> = bids
        .iter::<RestingOrder>()
        .find(|(_, o)| o.get_trader_index() == maker_index);
    assert!(found.is_some());
    let (core_index, order) = found.unwrap();
    assert_eq!(order.get_is_bid(), true);
    assert_eq!(order.get_num_base_atoms(), 1 * SOL_UNIT_SIZE);

    // verify order is correctly tracked on wrapper
    maker_wrapper_fixture.reload().await;

    let open_order: WrapperOpenOrder = {
        let market_infos_tree: MarketInfosTreeReadOnly = MarketInfosTreeReadOnly::new(
            &maker_wrapper_fixture.wrapper.dynamic,
            maker_wrapper_fixture.wrapper.fixed.market_infos_root_index,
            NIL,
        );

        let market_info_index: DataIndex =
            market_infos_tree.lookup_index(&MarketInfo::new_empty(test_fixture.market.key, NIL));

        let market_info: &MarketInfo = get_helper::<RBNode<MarketInfo>>(
            &maker_wrapper_fixture.wrapper.dynamic,
            market_info_index,
        )
        .get_value();

        get_helper::<RBNode<WrapperOpenOrder>>(
            &maker_wrapper_fixture.wrapper.dynamic,
            market_info.orders_root_index,
        )
        .get_value()
        .clone()
    };

    assert_eq!(open_order.get_is_bid(), true);
    assert_eq!(open_order.get_client_order_id(), 1);
    assert_eq!(open_order.get_num_base_atoms(), SOL_UNIT_SIZE);
    assert_eq!(
        open_order.get_price(),
        QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(1, -3).unwrap()
    );
    assert_eq!(open_order.get_market_data_index(), core_index);

    // taker buys 1 sol @ 1000 USDC
    let taker_order_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(taker_wrapper_fixture.key, false),
            AccountMeta::new(taker, true),
            AccountMeta::new(taker_token_account_base, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(taker, true),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new(global_base, false),
            AccountMeta::new(global_base_vault, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new(global_quote, false),
            AccountMeta::new(global_quote_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: [
            ManifestWrapperInstruction::PlaceOrder.to_vec(),
            WrapperPlaceOrderParams::new(
                1,
                1 * SOL_UNIT_SIZE,
                1,
                -3,
                false,
                NO_EXPIRATION_LAST_VALID_SLOT,
                OrderType::Limit,
            )
            .try_to_vec()
            .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[taker_order_ix],
        Some(&taker),
        &[&taker_keypair],
    )
    .await?;

    // verify book is cleared
    test_fixture.market.reload().await;
    let asks = test_fixture.market.market.get_asks();
    assert_eq!(asks.iter::<RestingOrder>().next(), None);
    let bids = test_fixture.market.market.get_bids();
    assert_eq!(bids.iter::<RestingOrder>().next(), None);

    // verify order is correctly not-tracked on wrapper
    taker_wrapper_fixture.reload().await;
    {
        let market_infos_tree: MarketInfosTreeReadOnly = MarketInfosTreeReadOnly::new(
            &taker_wrapper_fixture.wrapper.dynamic,
            taker_wrapper_fixture.wrapper.fixed.market_infos_root_index,
            NIL,
        );

        let market_info_index: DataIndex =
            market_infos_tree.lookup_index(&MarketInfo::new_empty(test_fixture.market.key, NIL));

        let market_info: &MarketInfo = get_helper::<RBNode<MarketInfo>>(
            &taker_wrapper_fixture.wrapper.dynamic,
            market_info_index,
        )
        .get_value();

        assert_eq!(market_info.orders_root_index, NIL);
    };

    // settle & pay fees
    let settle_taker_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(taker_wrapper_fixture.key, false),
            AccountMeta::new(taker, true),
            AccountMeta::new(taker_token_account_base, false),
            AccountMeta::new(taker_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(platform_token_account, false),
        ],
        data: [
            ManifestWrapperInstruction::SettleFunds.to_vec(),
            WrapperSettleFundsParams::new(500_000_000, 50)
                .try_to_vec()
                .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[settle_taker_ix],
        Some(&taker),
        &[&taker_keypair],
    )
    .await?;

    // taker sold 1/1 SOL, expect 0
    let taker_token_account_base: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(taker_token_account_base)
        .await
        .unwrap()
        .unwrap();

    let taker_token_account_base =
        spl_token::state::Account::unpack(&taker_token_account_base.data)?;
    assert_eq!(taker_token_account_base.amount, 0);

    // user has proceeds of trade in his wallet, but 50% fees were charged
    let taker_token_account_quote: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(taker_token_account_quote)
        .await
        .unwrap()
        .unwrap();

    let taker_token_account_quote =
        spl_token::state::Account::unpack(&taker_token_account_quote.data)?;
    assert_eq!(taker_token_account_quote.amount, USDC_UNIT_SIZE * 3 / 2);

    // verify the remaining 50% was paid to platform not referrer
    let platform_token_account_quote: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(platform_token_account)
        .await
        .unwrap()
        .unwrap();

    let platform_token_account_quote =
        spl_token::state::Account::unpack(&platform_token_account_quote.data)?;
    assert_eq!(platform_token_account_quote.amount, USDC_UNIT_SIZE / 2);

    let referred_token_account_quote: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(referred_token_account)
        .await
        .unwrap()
        .unwrap();

    let referred_token_account_quote =
        spl_token::state::Account::unpack(&referred_token_account_quote.data)?;
    assert_eq!(referred_token_account_quote.amount, 0);

    let settle_maker_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(maker_wrapper_fixture.key, false),
            AccountMeta::new(maker, true),
            AccountMeta::new(maker_token_account_base, false),
            AccountMeta::new(maker_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(platform_token_account, false),
        ],
        data: [
            ManifestWrapperInstruction::SettleFunds.to_vec(),
            WrapperSettleFundsParams::new(500_000_000, 50)
                .try_to_vec()
                .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[settle_maker_ix],
        Some(&maker),
        &[&maker_keypair],
    )
    .await
    .expect_err("should fail due to lack of USDC balance to pay fees");

    // maker has 1 SOL & bought 1 SOL, but couldn't settle
    let maker_token_account_base: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(maker_token_account_base)
        .await
        .unwrap()
        .unwrap();

    let maker_token_account_base =
        spl_token::state::Account::unpack(&maker_token_account_base.data)?;
    assert_eq!(maker_token_account_base.amount, SOL_UNIT_SIZE);

    Ok(())
}

#[tokio::test]
async fn wrapper_fill_order_without_referral_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;

    let taker: Pubkey = test_fixture.payer();
    let taker_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
    let mut taker_wrapper_fixture: WrapperFixture = test_fixture.wrapper.clone();

    let maker: Pubkey = test_fixture.second_keypair.pubkey();
    let maker_keypair: Keypair = test_fixture.second_keypair.insecure_clone();

    // setup wrapper for maker
    let mut maker_wrapper_fixture: WrapperFixture = {
        let wrapper_keypair = Keypair::new();

        let create_wrapper_ixs: Vec<Instruction> =
            create_wrapper_instructions(&maker, &maker, &wrapper_keypair.pubkey())?;

        send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &create_wrapper_ixs,
            Some(&maker),
            &[&maker_keypair, &wrapper_keypair],
        )
        .await?;

        WrapperFixture::new(Rc::clone(&test_fixture.context), wrapper_keypair.pubkey()).await
    };

    // setup token accounts for taker, maker, platform & referrer
    let (_, taker_token_account_base) = test_fixture
        .fund_trader_wallet(&taker_keypair, Token::SOL, 1 * SOL_UNIT_SIZE)
        .await;
    let (_, taker_token_account_quote) = test_fixture
        .fund_trader_wallet(&taker_keypair, Token::USDC, 1 * USDC_UNIT_SIZE)
        .await;

    let (base_mint, maker_token_account_base) = test_fixture
        .fund_trader_wallet(&maker_keypair, Token::SOL, 1 * SOL_UNIT_SIZE)
        .await;
    let (quote_mint, maker_token_account_quote) = test_fixture
        .fund_trader_wallet(&maker_keypair, Token::USDC, 1 * USDC_UNIT_SIZE)
        .await;
    let platform_token_account = test_fixture.fund_token_account(&quote_mint, &taker).await;
    let referred_token_account = test_fixture.fund_token_account(&quote_mint, &taker).await;

    let (base_vault, _) = get_vault_address(&test_fixture.market.key, &base_mint);
    let (quote_vault, _) = get_vault_address(&test_fixture.market.key, &quote_mint);
    let (global_base, _) = get_global_address(&base_mint);
    let (global_quote, _) = get_global_address(&quote_mint);
    let (global_base_vault, _) = get_global_vault_address(&base_mint);
    let (global_quote_vault, _) = get_global_vault_address(&quote_mint);

    // maker buys 1 sol @ 1000 USDC
    let maker_order_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(maker_wrapper_fixture.key, false),
            AccountMeta::new(maker, true),
            AccountMeta::new(maker_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(maker, true),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new(global_base, false),
            AccountMeta::new(global_base_vault, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new(global_quote, false),
            AccountMeta::new(global_quote_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: [
            ManifestWrapperInstruction::PlaceOrder.to_vec(),
            WrapperPlaceOrderParams::new(
                1,
                1 * SOL_UNIT_SIZE,
                1,
                -3,
                true,
                NO_EXPIRATION_LAST_VALID_SLOT,
                OrderType::Limit,
            )
            .try_to_vec()
            .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[maker_order_ix],
        Some(&maker),
        &[&maker_keypair],
    )
    .await?;

    // verify order is on book
    test_fixture.market.reload().await;
    let maker_index = test_fixture.market.market.get_trader_index(&maker);

    let bids = test_fixture.market.market.get_bids();
    let found: Option<(DataIndex, &RestingOrder)> = bids
        .iter::<RestingOrder>()
        .find(|(_, o)| o.get_trader_index() == maker_index);
    assert!(found.is_some());
    let (core_index, order) = found.unwrap();
    assert_eq!(order.get_is_bid(), true);
    assert_eq!(order.get_num_base_atoms(), 1 * SOL_UNIT_SIZE);

    // verify order is correctly tracked on wrapper
    maker_wrapper_fixture.reload().await;

    let open_order: WrapperOpenOrder = {
        let market_infos_tree: MarketInfosTreeReadOnly = MarketInfosTreeReadOnly::new(
            &maker_wrapper_fixture.wrapper.dynamic,
            maker_wrapper_fixture.wrapper.fixed.market_infos_root_index,
            NIL,
        );

        let market_info_index: DataIndex =
            market_infos_tree.lookup_index(&MarketInfo::new_empty(test_fixture.market.key, NIL));

        let market_info: &MarketInfo = get_helper::<RBNode<MarketInfo>>(
            &maker_wrapper_fixture.wrapper.dynamic,
            market_info_index,
        )
        .get_value();

        get_helper::<RBNode<WrapperOpenOrder>>(
            &maker_wrapper_fixture.wrapper.dynamic,
            market_info.orders_root_index,
        )
        .get_value()
        .clone()
    };

    assert_eq!(open_order.get_is_bid(), true);
    assert_eq!(open_order.get_client_order_id(), 1);
    assert_eq!(open_order.get_num_base_atoms(), SOL_UNIT_SIZE);
    assert_eq!(
        open_order.get_price(),
        QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(1, -3).unwrap()
    );
    assert_eq!(open_order.get_market_data_index(), core_index);

    // taker buys 1 sol @ 1000 USDC
    let taker_order_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(taker_wrapper_fixture.key, false),
            AccountMeta::new(taker, true),
            AccountMeta::new(taker_token_account_base, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(taker, true),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new(global_base, false),
            AccountMeta::new(global_base_vault, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new(global_quote, false),
            AccountMeta::new(global_quote_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: [
            ManifestWrapperInstruction::PlaceOrder.to_vec(),
            WrapperPlaceOrderParams::new(
                1,
                1 * SOL_UNIT_SIZE,
                1,
                -3,
                false,
                NO_EXPIRATION_LAST_VALID_SLOT,
                OrderType::Limit,
            )
            .try_to_vec()
            .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[taker_order_ix],
        Some(&taker),
        &[&taker_keypair],
    )
    .await?;

    // verify book is cleared
    test_fixture.market.reload().await;
    let asks = test_fixture.market.market.get_asks();
    assert_eq!(asks.iter::<RestingOrder>().next(), None);
    let bids = test_fixture.market.market.get_bids();
    assert_eq!(bids.iter::<RestingOrder>().next(), None);

    // verify order is correctly not-tracked on wrapper
    taker_wrapper_fixture.reload().await;
    {
        let market_infos_tree: MarketInfosTreeReadOnly = MarketInfosTreeReadOnly::new(
            &taker_wrapper_fixture.wrapper.dynamic,
            taker_wrapper_fixture.wrapper.fixed.market_infos_root_index,
            NIL,
        );

        let market_info_index: DataIndex =
            market_infos_tree.lookup_index(&MarketInfo::new_empty(test_fixture.market.key, NIL));

        let market_info: &MarketInfo = get_helper::<RBNode<MarketInfo>>(
            &taker_wrapper_fixture.wrapper.dynamic,
            market_info_index,
        )
        .get_value();

        assert_eq!(market_info.orders_root_index, NIL);
    };

    // settle & pay fees
    let settle_taker_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(taker_wrapper_fixture.key, false),
            AccountMeta::new(taker, true),
            AccountMeta::new(taker_token_account_base, false),
            AccountMeta::new(taker_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(platform_token_account, false),
            AccountMeta::new(referred_token_account, false),
        ],
        data: [
            ManifestWrapperInstruction::SettleFunds.to_vec(),
            WrapperSettleFundsParams::new(500_000_000, 50)
                .try_to_vec()
                .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[settle_taker_ix],
        Some(&taker),
        &[&taker_keypair],
    )
    .await?;

    // taker sold 1/1 SOL, expect 0
    let taker_token_account_base: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(taker_token_account_base)
        .await
        .unwrap()
        .unwrap();

    let taker_token_account_base =
        spl_token::state::Account::unpack(&taker_token_account_base.data)?;
    assert_eq!(taker_token_account_base.amount, 0);

    // user has proceeds of trade in his wallet, but 50% fees were charged
    let taker_token_account_quote: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(taker_token_account_quote)
        .await
        .unwrap()
        .unwrap();

    let taker_token_account_quote =
        spl_token::state::Account::unpack(&taker_token_account_quote.data)?;
    assert_eq!(taker_token_account_quote.amount, USDC_UNIT_SIZE * 3 / 2);

    // verify the remaining 50% was split 50/50 between platform & referrer
    let platform_token_account_quote: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(platform_token_account)
        .await
        .unwrap()
        .unwrap();

    let platform_token_account_quote =
        spl_token::state::Account::unpack(&platform_token_account_quote.data)?;
    assert_eq!(platform_token_account_quote.amount, USDC_UNIT_SIZE / 4);

    let referred_token_account_quote: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(referred_token_account)
        .await
        .unwrap()
        .unwrap();

    let referred_token_account_quote =
        spl_token::state::Account::unpack(&referred_token_account_quote.data)?;
    assert_eq!(referred_token_account_quote.amount, USDC_UNIT_SIZE / 4);

    let settle_maker_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(maker_wrapper_fixture.key, false),
            AccountMeta::new(maker, true),
            AccountMeta::new(maker_token_account_base, false),
            AccountMeta::new(maker_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(platform_token_account, false),
            AccountMeta::new(referred_token_account, false),
        ],
        data: [
            ManifestWrapperInstruction::SettleFunds.to_vec(),
            WrapperSettleFundsParams::new(500_000_000, 50)
                .try_to_vec()
                .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[settle_maker_ix],
        Some(&maker),
        &[&maker_keypair],
    )
    .await
    .expect_err("should fail due to lack of USDC balance to pay fees");

    // maker has 1 SOL & bought 1 SOL, but couldn't settle
    let maker_token_account_base: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(maker_token_account_base)
        .await
        .unwrap()
        .unwrap();

    let maker_token_account_base =
        spl_token::state::Account::unpack(&maker_token_account_base.data)?;
    assert_eq!(maker_token_account_base.amount, SOL_UNIT_SIZE);

    Ok(())
}

#[tokio::test]
async fn wrapper_fill_order_with_transfer_fees_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new_with_extensions(true, true).await;

    let taker: Pubkey = test_fixture.payer();
    let taker_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
    let mut taker_wrapper_fixture: WrapperFixture = test_fixture.wrapper.clone();

    let maker: Pubkey = test_fixture.second_keypair.pubkey();
    let maker_keypair: Keypair = test_fixture.second_keypair.insecure_clone();

    // setup wrapper for maker
    let mut maker_wrapper_fixture: WrapperFixture = {
        let wrapper_keypair = Keypair::new();

        let create_wrapper_ixs: Vec<Instruction> =
            create_wrapper_instructions(&maker, &maker, &wrapper_keypair.pubkey())?;

        send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &create_wrapper_ixs,
            Some(&maker),
            &[&maker_keypair, &wrapper_keypair],
        )
        .await?;

        WrapperFixture::new(Rc::clone(&test_fixture.context), wrapper_keypair.pubkey()).await
    };

    // setup token accounts for taker, maker, platform & referrer
    let (_, taker_token_account_base) = test_fixture
        .fund_trader_wallet(&taker_keypair, Token::SOL, 1 * SOL_UNIT_SIZE)
        .await;
    let (_, taker_token_account_quote) = test_fixture
        .fund_trader_wallet_2022(&taker_keypair, Token::USDC, 1 * USDC_UNIT_SIZE + 100)
        .await;

    let (base_mint, maker_token_account_base) = test_fixture
        .fund_trader_wallet(&maker_keypair, Token::SOL, 1 * SOL_UNIT_SIZE)
        .await;
    let (quote_mint, maker_token_account_quote) = test_fixture
        .fund_trader_wallet_2022(&maker_keypair, Token::USDC, 1 * USDC_UNIT_SIZE + 100)
        .await;
    let platform_token_account = test_fixture
        .fund_token_account_2022(&quote_mint, &taker)
        .await;
    let referred_token_account = test_fixture
        .fund_token_account_2022(&quote_mint, &taker)
        .await;

    let (base_vault, _) = get_vault_address(&test_fixture.market.key, &base_mint);
    let (quote_vault, _) = get_vault_address(&test_fixture.market.key, &quote_mint);
    let (global_base, _) = get_global_address(&base_mint);
    let (global_quote, _) = get_global_address(&quote_mint);
    let (global_base_vault, _) = get_global_vault_address(&base_mint);
    let (global_quote_vault, _) = get_global_vault_address(&quote_mint);

    // maker buys 1 sol @ 1000 USDC
    let maker_order_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(maker_wrapper_fixture.key, false),
            AccountMeta::new(maker, true),
            AccountMeta::new(maker_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_token_2022::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(maker, true),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new(global_base, false),
            AccountMeta::new(global_base_vault, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new(global_quote, false),
            AccountMeta::new(global_quote_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(spl_token_2022::id(), false),
        ],
        data: [
            ManifestWrapperInstruction::PlaceOrder.to_vec(),
            WrapperPlaceOrderParams::new(
                1,
                1 * SOL_UNIT_SIZE,
                1,
                -3,
                true,
                NO_EXPIRATION_LAST_VALID_SLOT,
                OrderType::Limit,
            )
            .try_to_vec()
            .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[maker_order_ix],
        Some(&maker),
        &[&maker_keypair],
    )
    .await?;

    // verify order is on book
    test_fixture.market.reload().await;
    let maker_index = test_fixture.market.market.get_trader_index(&maker);

    let bids = test_fixture.market.market.get_bids();
    let found: Option<(DataIndex, &RestingOrder)> = bids
        .iter::<RestingOrder>()
        .find(|(_, o)| o.get_trader_index() == maker_index);
    assert!(found.is_some());
    let (core_index, order) = found.unwrap();
    assert_eq!(order.get_is_bid(), true);
    assert_eq!(order.get_num_base_atoms(), 1 * SOL_UNIT_SIZE);

    // verify order is correctly tracked on wrapper
    maker_wrapper_fixture.reload().await;

    let open_order: WrapperOpenOrder = {
        let market_infos_tree: MarketInfosTreeReadOnly = MarketInfosTreeReadOnly::new(
            &maker_wrapper_fixture.wrapper.dynamic,
            maker_wrapper_fixture.wrapper.fixed.market_infos_root_index,
            NIL,
        );

        let market_info_index: DataIndex =
            market_infos_tree.lookup_index(&MarketInfo::new_empty(test_fixture.market.key, NIL));

        let market_info: &MarketInfo = get_helper::<RBNode<MarketInfo>>(
            &maker_wrapper_fixture.wrapper.dynamic,
            market_info_index,
        )
        .get_value();

        get_helper::<RBNode<WrapperOpenOrder>>(
            &maker_wrapper_fixture.wrapper.dynamic,
            market_info.orders_root_index,
        )
        .get_value()
        .clone()
    };

    assert_eq!(open_order.get_is_bid(), true);
    assert_eq!(open_order.get_client_order_id(), 1);
    assert_eq!(open_order.get_num_base_atoms(), SOL_UNIT_SIZE);
    assert_eq!(
        open_order.get_price(),
        QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(1, -3).unwrap()
    );
    assert_eq!(open_order.get_market_data_index(), core_index);

    // taker sells 1 sol @ 1000 USDC
    let taker_order_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(taker_wrapper_fixture.key, false),
            AccountMeta::new(taker, true),
            AccountMeta::new(taker_token_account_base, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(taker, true),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new(global_base, false),
            AccountMeta::new(global_base_vault, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new(global_quote, false),
            AccountMeta::new(global_quote_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(spl_token_2022::id(), false),
        ],
        data: [
            ManifestWrapperInstruction::PlaceOrder.to_vec(),
            WrapperPlaceOrderParams::new(
                1,
                1 * SOL_UNIT_SIZE,
                1,
                -3,
                false,
                NO_EXPIRATION_LAST_VALID_SLOT,
                OrderType::Limit,
            )
            .try_to_vec()
            .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[taker_order_ix],
        Some(&taker),
        &[&taker_keypair],
    )
    .await?;

    // verify book is cleared
    test_fixture.market.reload().await;
    let asks = test_fixture.market.market.get_asks();
    assert_eq!(asks.iter::<RestingOrder>().next(), None);
    let bids = test_fixture.market.market.get_bids();
    assert_eq!(bids.iter::<RestingOrder>().next(), None);

    // verify order is correctly not-tracked on wrapper
    taker_wrapper_fixture.reload().await;
    {
        let market_infos_tree: MarketInfosTreeReadOnly = MarketInfosTreeReadOnly::new(
            &taker_wrapper_fixture.wrapper.dynamic,
            taker_wrapper_fixture.wrapper.fixed.market_infos_root_index,
            NIL,
        );

        let market_info_index: DataIndex =
            market_infos_tree.lookup_index(&MarketInfo::new_empty(test_fixture.market.key, NIL));

        let market_info: &MarketInfo = get_helper::<RBNode<MarketInfo>>(
            &taker_wrapper_fixture.wrapper.dynamic,
            market_info_index,
        )
        .get_value();

        assert_eq!(market_info.orders_root_index, NIL);
    };

    // settle & pay fees
    let settle_taker_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(taker_wrapper_fixture.key, false),
            AccountMeta::new(taker, true),
            AccountMeta::new(taker_token_account_base, false),
            AccountMeta::new(taker_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(spl_token_2022::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(platform_token_account, false),
            AccountMeta::new(referred_token_account, false),
        ],
        data: [
            ManifestWrapperInstruction::SettleFunds.to_vec(),
            WrapperSettleFundsParams::new(500_000_000, 50)
                .try_to_vec()
                .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[settle_taker_ix],
        Some(&taker),
        &[&taker_keypair],
    )
    .await?;

    // taker sold 1/1 SOL, expect 0
    let taker_token_account_base: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(taker_token_account_base)
        .await
        .unwrap()
        .unwrap();

    let taker_token_account_base =
        spl_token::state::Account::unpack(&taker_token_account_base.data)?;
    assert_eq!(taker_token_account_base.amount, 0);

    // user has proceeds of trade in his wallet, but 50% fees were charged
    let taker_token_account_quote: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(taker_token_account_quote)
        .await
        .unwrap()
        .unwrap();

    let taker_token_account_quote = StateWithExtensions::<spl_token_2022::state::Account>::unpack(
        &taker_token_account_quote.data,
    )?;
    assert_eq!(
        taker_token_account_quote.base.amount,
        USDC_UNIT_SIZE * 3 / 2
    );

    // verify the remaining 50% was paid to platform not referrer
    // transfer fees of 100 applied on the settled amount
    let platform_token_account_quote: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(platform_token_account)
        .await
        .unwrap()
        .unwrap();

    let platform_token_account_quote =
        StateWithExtensions::<spl_token_2022::state::Account>::unpack(
            &platform_token_account_quote.data,
        )?;
    assert_eq!(
        platform_token_account_quote.base.amount,
        USDC_UNIT_SIZE / 4 - 100
    );

    let referred_token_account_quote: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(referred_token_account)
        .await
        .unwrap()
        .unwrap();

    let referred_token_account_quote =
        StateWithExtensions::<spl_token_2022::state::Account>::unpack(
            &referred_token_account_quote.data,
        )?;
    assert_eq!(
        referred_token_account_quote.base.amount,
        USDC_UNIT_SIZE / 4 - 100
    );

    let settle_maker_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(maker_wrapper_fixture.key, false),
            AccountMeta::new(maker, true),
            AccountMeta::new(maker_token_account_base, false),
            AccountMeta::new(maker_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(spl_token_2022::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(platform_token_account, false),
            AccountMeta::new(referred_token_account, false),
        ],
        data: [
            ManifestWrapperInstruction::SettleFunds.to_vec(),
            WrapperSettleFundsParams::new(500_000_000, 50)
                .try_to_vec()
                .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[settle_maker_ix],
        Some(&maker),
        &[&maker_keypair],
    )
    .await
    .expect_err("should fail due to lack of USDC balance to pay fees");

    // maker has 1 SOL & bought 1 SOL, but couldn't settle
    let maker_token_account_base: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(maker_token_account_base)
        .await
        .unwrap()
        .unwrap();

    let maker_token_account_base =
        spl_token::state::Account::unpack(&maker_token_account_base.data)?;
    assert_eq!(maker_token_account_base.amount, SOL_UNIT_SIZE);

    Ok(())
}

#[tokio::test]
async fn wrapper_fill_order_with_transfer_fees_without_referral_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new_with_extensions(true, true).await;

    let taker: Pubkey = test_fixture.payer();
    let taker_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
    let mut taker_wrapper_fixture: WrapperFixture = test_fixture.wrapper.clone();

    let maker: Pubkey = test_fixture.second_keypair.pubkey();
    let maker_keypair: Keypair = test_fixture.second_keypair.insecure_clone();

    // setup wrapper for maker
    let mut maker_wrapper_fixture: WrapperFixture = {
        let wrapper_keypair = Keypair::new();

        let create_wrapper_ixs: Vec<Instruction> =
            create_wrapper_instructions(&maker, &maker, &wrapper_keypair.pubkey())?;

        send_tx_with_retry(
            Rc::clone(&test_fixture.context),
            &create_wrapper_ixs,
            Some(&maker),
            &[&maker_keypair, &wrapper_keypair],
        )
        .await?;

        WrapperFixture::new(Rc::clone(&test_fixture.context), wrapper_keypair.pubkey()).await
    };

    // setup token accounts for taker, maker, platform & referrer
    let (_, taker_token_account_base) = test_fixture
        .fund_trader_wallet(&taker_keypair, Token::SOL, 1 * SOL_UNIT_SIZE)
        .await;
    let (_, taker_token_account_quote) = test_fixture
        .fund_trader_wallet_2022(&taker_keypair, Token::USDC, 1 * USDC_UNIT_SIZE + 100)
        .await;

    let (base_mint, maker_token_account_base) = test_fixture
        .fund_trader_wallet(&maker_keypair, Token::SOL, 1 * SOL_UNIT_SIZE)
        .await;
    let (quote_mint, maker_token_account_quote) = test_fixture
        .fund_trader_wallet_2022(&maker_keypair, Token::USDC, 1 * USDC_UNIT_SIZE + 100)
        .await;
    let platform_token_account = test_fixture
        .fund_token_account_2022(&quote_mint, &taker)
        .await;

    let (base_vault, _) = get_vault_address(&test_fixture.market.key, &base_mint);
    let (quote_vault, _) = get_vault_address(&test_fixture.market.key, &quote_mint);
    let (global_base, _) = get_global_address(&base_mint);
    let (global_quote, _) = get_global_address(&quote_mint);
    let (global_base_vault, _) = get_global_vault_address(&base_mint);
    let (global_quote_vault, _) = get_global_vault_address(&quote_mint);

    // maker buys 1 sol @ 1000 USDC
    let maker_order_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(maker_wrapper_fixture.key, false),
            AccountMeta::new(maker, true),
            AccountMeta::new(maker_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_token_2022::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(maker, true),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new(global_base, false),
            AccountMeta::new(global_base_vault, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new(global_quote, false),
            AccountMeta::new(global_quote_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(spl_token_2022::id(), false),
        ],
        data: [
            ManifestWrapperInstruction::PlaceOrder.to_vec(),
            WrapperPlaceOrderParams::new(
                1,
                1 * SOL_UNIT_SIZE,
                1,
                -3,
                true,
                NO_EXPIRATION_LAST_VALID_SLOT,
                OrderType::Limit,
            )
            .try_to_vec()
            .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[maker_order_ix],
        Some(&maker),
        &[&maker_keypair],
    )
    .await?;

    // verify order is on book
    test_fixture.market.reload().await;
    let maker_index = test_fixture.market.market.get_trader_index(&maker);

    let bids = test_fixture.market.market.get_bids();
    let found: Option<(DataIndex, &RestingOrder)> = bids
        .iter::<RestingOrder>()
        .find(|(_, o)| o.get_trader_index() == maker_index);
    assert!(found.is_some());
    let (core_index, order) = found.unwrap();
    assert_eq!(order.get_is_bid(), true);
    assert_eq!(order.get_num_base_atoms(), 1 * SOL_UNIT_SIZE);

    // verify order is correctly tracked on wrapper
    maker_wrapper_fixture.reload().await;

    let open_order: WrapperOpenOrder = {
        let market_infos_tree: MarketInfosTreeReadOnly = MarketInfosTreeReadOnly::new(
            &maker_wrapper_fixture.wrapper.dynamic,
            maker_wrapper_fixture.wrapper.fixed.market_infos_root_index,
            NIL,
        );

        let market_info_index: DataIndex =
            market_infos_tree.lookup_index(&MarketInfo::new_empty(test_fixture.market.key, NIL));

        let market_info: &MarketInfo = get_helper::<RBNode<MarketInfo>>(
            &maker_wrapper_fixture.wrapper.dynamic,
            market_info_index,
        )
        .get_value();

        get_helper::<RBNode<WrapperOpenOrder>>(
            &maker_wrapper_fixture.wrapper.dynamic,
            market_info.orders_root_index,
        )
        .get_value()
        .clone()
    };

    assert_eq!(open_order.get_is_bid(), true);
    assert_eq!(open_order.get_client_order_id(), 1);
    assert_eq!(open_order.get_num_base_atoms(), SOL_UNIT_SIZE);
    assert_eq!(
        open_order.get_price(),
        QuoteAtomsPerBaseAtom::try_from_mantissa_and_exponent(1, -3).unwrap()
    );
    assert_eq!(open_order.get_market_data_index(), core_index);

    // taker sells 1 sol @ 1000 USDC
    let taker_order_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(taker_wrapper_fixture.key, false),
            AccountMeta::new(taker, true),
            AccountMeta::new(taker_token_account_base, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(taker, true),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new(global_base, false),
            AccountMeta::new(global_base_vault, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new(global_quote, false),
            AccountMeta::new(global_quote_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(spl_token_2022::id(), false),
        ],
        data: [
            ManifestWrapperInstruction::PlaceOrder.to_vec(),
            WrapperPlaceOrderParams::new(
                1,
                1 * SOL_UNIT_SIZE,
                1,
                -3,
                false,
                NO_EXPIRATION_LAST_VALID_SLOT,
                OrderType::Limit,
            )
            .try_to_vec()
            .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[taker_order_ix],
        Some(&taker),
        &[&taker_keypair],
    )
    .await?;

    // verify book is cleared
    test_fixture.market.reload().await;
    let asks = test_fixture.market.market.get_asks();
    assert_eq!(asks.iter::<RestingOrder>().next(), None);
    let bids = test_fixture.market.market.get_bids();
    assert_eq!(bids.iter::<RestingOrder>().next(), None);

    // verify order is correctly not-tracked on wrapper
    taker_wrapper_fixture.reload().await;
    {
        let market_infos_tree: MarketInfosTreeReadOnly = MarketInfosTreeReadOnly::new(
            &taker_wrapper_fixture.wrapper.dynamic,
            taker_wrapper_fixture.wrapper.fixed.market_infos_root_index,
            NIL,
        );

        let market_info_index: DataIndex =
            market_infos_tree.lookup_index(&MarketInfo::new_empty(test_fixture.market.key, NIL));

        let market_info: &MarketInfo = get_helper::<RBNode<MarketInfo>>(
            &taker_wrapper_fixture.wrapper.dynamic,
            market_info_index,
        )
        .get_value();

        assert_eq!(market_info.orders_root_index, NIL);
    };

    // settle & pay fees
    let settle_taker_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(taker_wrapper_fixture.key, false),
            AccountMeta::new(taker, true),
            AccountMeta::new(taker_token_account_base, false),
            AccountMeta::new(taker_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(spl_token_2022::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(platform_token_account, false),
        ],
        data: [
            ManifestWrapperInstruction::SettleFunds.to_vec(),
            WrapperSettleFundsParams::new(500_000_000, 50)
                .try_to_vec()
                .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[settle_taker_ix],
        Some(&taker),
        &[&taker_keypair],
    )
    .await?;

    // taker sold 1/1 SOL, expect 0
    let taker_token_account_base: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(taker_token_account_base)
        .await
        .unwrap()
        .unwrap();

    let taker_token_account_base =
        spl_token::state::Account::unpack(&taker_token_account_base.data)?;
    assert_eq!(taker_token_account_base.amount, 0);

    // user has proceeds of trade in his wallet, but 50% fees were charged
    let taker_token_account_quote: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(taker_token_account_quote)
        .await
        .unwrap()
        .unwrap();

    let taker_token_account_quote = StateWithExtensions::<spl_token_2022::state::Account>::unpack(
        &taker_token_account_quote.data,
    )?;
    assert_eq!(
        taker_token_account_quote.base.amount,
        USDC_UNIT_SIZE * 3 / 2
    );

    // verify the remaining 50% was not split with referrer
    // transfer fees of 100 applied on the settled amount
    let platform_token_account_quote: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(platform_token_account)
        .await
        .unwrap()
        .unwrap();

    let platform_token_account_quote =
        StateWithExtensions::<spl_token_2022::state::Account>::unpack(
            &platform_token_account_quote.data,
        )?;
    assert_eq!(
        platform_token_account_quote.base.amount,
        USDC_UNIT_SIZE / 2 - 100
    );

    let settle_maker_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(maker_wrapper_fixture.key, false),
            AccountMeta::new(maker, true),
            AccountMeta::new(maker_token_account_base, false),
            AccountMeta::new(maker_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(spl_token_2022::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(platform_token_account, false),
        ],
        data: [
            ManifestWrapperInstruction::SettleFunds.to_vec(),
            WrapperSettleFundsParams::new(500_000_000, 50)
                .try_to_vec()
                .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[settle_maker_ix],
        Some(&maker),
        &[&maker_keypair],
    )
    .await
    .expect_err("should fail due to lack of USDC balance to pay fees");

    // maker has 1 SOL & bought 1 SOL, but couldn't settle
    let maker_token_account_base: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(maker_token_account_base)
        .await
        .unwrap()
        .unwrap();

    let maker_token_account_base =
        spl_token::state::Account::unpack(&maker_token_account_base.data)?;
    assert_eq!(maker_token_account_base.amount, SOL_UNIT_SIZE);

    Ok(())
}

#[tokio::test]
async fn wrapper_self_trade_test() -> anyhow::Result<()> {
    let mut test_fixture: TestFixture = TestFixture::new().await;

    let payer: Pubkey = test_fixture.payer();
    let payer_keypair: Keypair = test_fixture.payer_keypair().insecure_clone();
    let (base_mint, trader_token_account_base) = test_fixture
        .fund_trader_wallet(&payer_keypair, Token::SOL, 1)
        .await;
    let (quote_mint, trader_token_account_quote) = test_fixture
        .fund_trader_wallet(&payer_keypair, Token::USDC, 1)
        .await;

    let platform_token_account = test_fixture.fund_token_account(&quote_mint, &payer).await;
    let referred_token_account = test_fixture.fund_token_account(&quote_mint, &payer).await;

    let (quote_vault, _) = get_vault_address(&test_fixture.market.key, &quote_mint);
    let (base_vault, _) = get_vault_address(&test_fixture.market.key, &base_mint);
    let (global_base, _) = get_global_address(&base_mint);
    let (global_quote, _) = get_global_address(&quote_mint);
    let (global_base_vault, _) = get_global_vault_address(&base_mint);
    let (global_quote_vault, _) = get_global_vault_address(&quote_mint);

    // place orders
    let place_order_1_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(test_fixture.wrapper.key, false),
            AccountMeta::new(payer, true),
            AccountMeta::new(trader_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(payer, true),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new(global_base, false),
            AccountMeta::new(global_base_vault, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new(global_quote, false),
            AccountMeta::new(global_quote_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: [
            ManifestWrapperInstruction::PlaceOrder.to_vec(),
            WrapperPlaceOrderParams::new(
                1,
                1,
                1,
                0,
                true,
                NO_EXPIRATION_LAST_VALID_SLOT,
                OrderType::Limit,
            )
            .try_to_vec()
            .unwrap(),
        ]
        .concat(),
    };
    let place_order_2_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(test_fixture.wrapper.key, false),
            AccountMeta::new(payer, true),
            AccountMeta::new(trader_token_account_base, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(payer, true),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new(global_base, false),
            AccountMeta::new(global_base_vault, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new(global_quote, false),
            AccountMeta::new(global_quote_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: [
            ManifestWrapperInstruction::PlaceOrder.to_vec(),
            WrapperPlaceOrderParams::new(
                2,
                1,
                1,
                0,
                false,
                NO_EXPIRATION_LAST_VALID_SLOT,
                OrderType::Limit,
            )
            .try_to_vec()
            .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[place_order_1_ix, place_order_2_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // verify both orders are no longer tracked on wrapper:
    test_fixture.wrapper.reload().await;

    let market_info_index: DataIndex = {
        let market_infos_tree: MarketInfosTreeReadOnly = MarketInfosTreeReadOnly::new(
            &test_fixture.wrapper.wrapper.dynamic,
            test_fixture.wrapper.wrapper.fixed.market_infos_root_index,
            NIL,
        );
        market_infos_tree.lookup_index(&MarketInfo::new_empty(test_fixture.market.key, NIL))
    };

    let orders_root_index = {
        let market_info: &MarketInfo = get_helper::<RBNode<MarketInfo>>(
            &test_fixture.wrapper.wrapper.dynamic,
            market_info_index,
        )
        .get_value();

        market_info.orders_root_index
    };

    let open_orders_tree: OpenOrdersTreeReadOnly = OpenOrdersTreeReadOnly::new(
        &test_fixture.wrapper.wrapper.dynamic,
        orders_root_index,
        NIL,
    );
    let found = open_orders_tree
        .iter::<WrapperOpenOrder>()
        .find(|(_, o)| o.get_client_order_id() == 1 || o.get_client_order_id() == 2);
    assert!(found.is_none());

    // release funds
    let settle_funds_ix = Instruction {
        program_id: ui_wrapper::id(),
        accounts: vec![
            AccountMeta::new(test_fixture.wrapper.key, false),
            AccountMeta::new(payer, true),
            AccountMeta::new(trader_token_account_base, false),
            AccountMeta::new(trader_token_account_quote, false),
            AccountMeta::new(test_fixture.market.key, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(manifest::id(), false),
            AccountMeta::new(platform_token_account, false),
            AccountMeta::new(referred_token_account, false),
        ],
        data: [
            ManifestWrapperInstruction::SettleFunds.to_vec(),
            WrapperSettleFundsParams::new(500_000_000, 50)
                .try_to_vec()
                .unwrap(),
        ]
        .concat(),
    };
    send_tx_with_retry(
        Rc::clone(&test_fixture.context),
        &[settle_funds_ix],
        Some(&payer),
        &[&payer_keypair],
    )
    .await?;

    // verify fees were charged and user did not receive quote tokens
    let trader_token_account_quote: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(trader_token_account_quote)
        .await
        .unwrap()
        .unwrap();

    let trader_token_account_quote =
        spl_token::state::Account::unpack(&trader_token_account_quote.data)?;
    assert_eq!(trader_token_account_quote.amount, 0);

    // verify base token was received back
    let trader_token_account_base: Account = test_fixture
        .context
        .borrow_mut()
        .banks_client
        .get_account(trader_token_account_base)
        .await
        .unwrap()
        .unwrap();

    let trader_token_account_base =
        spl_token::state::Account::unpack(&trader_token_account_base.data)?;
    assert_eq!(trader_token_account_base.amount, 1);

    Ok(())
}
