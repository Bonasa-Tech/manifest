use std::{
    cell::RefCell, collections::HashMap, fs::read_to_string, mem::size_of, rc::Rc, str::FromStr,
};

use hypertree::{
    get_mut_helper, DataIndex, FreeList, HyperTreeReadOperations, HyperTreeWriteOperations, RBNode,
    RedBlackTree, NIL,
};
use manifest::{
    quantities::{BaseAtoms, QuoteAtomsPerBaseAtom, WrapperU64},
    state::{
        create_empty_market, AddOrderToMarketArgs, AddOrderToMarketResult, DynamicAccount,
        MarketFixed, MarketRefMut, OrderType, RestingOrder,
    },
    validation::MintAccountInfo,
};
use serde_json::{from_str, Value};
use solana_program_test::ProgramTest;
use solana_sdk::{
    account::Account, account_info::AccountInfo, pubkey::Pubkey, rent::Rent, signature::Keypair,
    signer::Signer,
};
use spl_token_2022::state::Mint;
use wrapper::{
    market_info::MarketInfo,
    open_order::WrapperOpenOrder,
    processors::shared::{expand_wrapper, MarketInfosTree, UnusedWrapperFreeListPadding},
    wrapper_state::{ManifestWrapperStateFixed, WRAPPER_FIXED_SIZE},
};

use super::{
    get_market_info_index_for_market, value_get_array, value_get_f64, value_get_obj,
    value_get_pubkey, value_get_str, value_get_u64, Obj,
};

/// The open orders of one market, in the layout every wrapper account starts
/// in. Built here rather than taken from `wrapper::processors::shared`, whose
/// alias for it went away when the program moved these to a linked list: the
/// program converts a market the first time it touches it, so this harness
/// writes what a wrapper on chain actually looks like before that happens and
/// works against a program from either side of the change.
type OpenOrdersTree<'a> = RedBlackTree<'a, WrapperOpenOrder>;

// Number of spare spaces to put at the end of the market to mimic steady state,
// so the first orders do not need to expand.
const NUM_EXTRA_NODES: i32 = 50;

/// Test fixture for benchmarking.
pub struct TestFixture {
    /// Solana program test
    pub program_test: ProgramTest,
    /// Public key for the market that is being tested.
    pub market: Pubkey,
    // Maps the new trader pubkey to their wrapper pubkey.
    pub trader_to_wrapper: HashMap<Pubkey, Pubkey>,
    // Maps the new trader pubkey to their data index in the market.
    pub trader_to_index_hint: HashMap<Pubkey, DataIndex>,
    // Maps the original pubkey to the new keypair that was used.
    pub keypair_map: HashMap<Pubkey, Keypair>,
    // Base lot size for the original PHX market in atoms per lot.
    pub phx_base_lot_size: u64,
    // Quote lot size for the original PHX market in atoms per lot.
    pub phx_quote_lot_size: u64,
}

pub const RUST_LOG_DEFAULT: &str = "solana_rbpf::vm=info,\
             solana_program_runtime::stable_log=debug,\
             solana_runtime::message_processor=debug,\
             solana_runtime::system_instruction_processor=info,\
             solana_program_test=info,\
             solana_bpf_loader_program=debug";

/// Load the initial state from a json file.
pub(crate) async fn load_initial_state(initial_state_filepath: String) -> TestFixture {
    let market_keypair: Keypair = Keypair::new();
    let default_keypair: Keypair = Keypair::new();

    let initial_data_str: String =
        read_to_string(initial_state_filepath).expect("Unable to read file");
    let initial_data_json: Value = from_str(&initial_data_str).expect("Unable to parse");

    let initial_state: &Value = initial_data_json
        .get("initialState")
        .expect("Has initial state");

    // Create a program test with both the wrapper and core programs in it.
    let mut program_test: ProgramTest = ProgramTest::default();
    program_test.add_program("wrapper", wrapper::ID.to_bytes().into(), None);
    program_test.add_program("manifest", manifest::ID.to_bytes().into(), None);
    solana_logger::setup_with_default(RUST_LOG_DEFAULT);

    let base_mint: &str = value_get_str(initial_state, "baseMint");
    let quote_mint: &str = value_get_str(initial_state, "quoteMint");
    let base_decimals: u8 = value_get_u64(initial_state, "baseDecimals") as u8;
    let quote_decimals: u8 = value_get_u64(initial_state, "quoteDecimals") as u8;
    let decimal_adjustment: f64 = 10f64.powi(quote_decimals as i32 - base_decimals as i32);
    let phx_base_lot_size: u64 = value_get_u64(initial_state, "baseLotSize");
    let phx_quote_lot_size: u64 = value_get_u64(initial_state, "quoteLotSize");

    let mut market_fixed: MarketFixed = create_empty_market(
        base_mint,
        quote_mint,
        base_decimals,
        quote_decimals,
        &default_keypair.pubkey().to_bytes().into(),
        &market_keypair.pubkey().to_bytes().into(),
    );
    // Just make the dynamic way bigger than it needs to be.
    let mut market_dynamic_bytes: [u8; 1_000_000] = [0_u8; 1_000_000];

    let mut dynamic_account: MarketRefMut = DynamicAccount {
        fixed: &mut market_fixed,
        dynamic: &mut market_dynamic_bytes,
    };
    let mut keypair_map: HashMap<Pubkey, Keypair> = HashMap::new();
    let mut wrapper_bytes_by_pubkey: HashMap<Pubkey, Vec<u8>> = HashMap::new();

    // First pass is just seats and balances and new keypairs.
    let traders_by_index: &Obj = value_get_obj(initial_state, "tradersByIndex");
    for (_index, trader) in traders_by_index {
        let base_atoms: u64 = value_get_u64(trader, "baseAtomsDeposited");
        let quote_atoms: u64 = value_get_u64(trader, "quoteAtomsDeposited");
        let original_pubkey: Pubkey = value_get_pubkey(trader, "key");

        let new_keypair: Keypair = Keypair::new();
        let new_pubkey: Pubkey = new_keypair.pubkey();
        keypair_map.insert(original_pubkey, new_keypair);

        // Transfer gas
        fund_wallet(&mut program_test, &new_pubkey);

        // Create wrapper
        create_and_save_empty_wrapper(&new_pubkey, &mut wrapper_bytes_by_pubkey);

        // Claim seat on the market
        dynamic_account
            .market_expand()
            .expect("Expand before claim seat");
        dynamic_account
            .claim_seat(&new_pubkey.to_bytes().into())
            .expect("Claim seat");
        extend_wrapper(&mut wrapper_bytes_by_pubkey, &new_pubkey);
        // Claim seat on the wrapper
        claim_seat_on_wrapper(
            &mut dynamic_account,
            wrapper_bytes_by_pubkey
                .get_mut(&new_pubkey)
                .unwrap()
                .as_mut_slice(),
            &new_pubkey,
            &market_keypair.pubkey(),
        );

        // Deposit on the market.
        let trader_index: DataIndex =
            dynamic_account.get_trader_index(&new_pubkey.to_bytes().into());
        dynamic_account
            .deposit(trader_index, base_atoms, true)
            .unwrap();
        dynamic_account
            .deposit(trader_index, quote_atoms, false)
            .unwrap();
        // No need to deposit on the wrapper. Will be handled by a sync
    }

    for (key, is_bid) in [("bids", true), ("asks", false)] {
        for order in value_get_array(initial_state, key) {
            let price_float: f64 = value_get_f64(order, "priceFloat");
            let order_sequence_number: u64 = value_get_u64(order, "orderSequenceNumber");
            let trader_index: u64 = value_get_u64(order, "traderIndex");
            let num_base_atoms: u64 = value_get_u64(order, "numBaseAtoms");
            let last_valid_slot: u32 = value_get_u64(order, "lastValidSlot") as u32;
            let trader: &Value = traders_by_index
                .get(&trader_index.to_string())
                .unwrap()
                .as_object()
                .unwrap()
                .get("key")
                .unwrap();
            let original_pubkey: Pubkey = Pubkey::from_str(trader.as_str().unwrap()).unwrap();
            let mapped_pubkey: Pubkey = keypair_map.get(&original_pubkey).unwrap().pubkey();
            if !dynamic_account.fixed.has_free_block() {
                dynamic_account.market_expand().unwrap();
            }
            let trader_index: DataIndex =
                dynamic_account.get_trader_index(&mapped_pubkey.to_bytes().into());

            let market_pubkey = market_keypair.pubkey().to_bytes().into();
            let result: AddOrderToMarketResult = dynamic_account
                .place_order(AddOrderToMarketArgs {
                    market: &market_pubkey,
                    trader_index,
                    num_base_atoms: BaseAtoms::new(num_base_atoms),
                    price: QuoteAtomsPerBaseAtom::try_from(price_float * decimal_adjustment)
                        .unwrap(),
                    is_bid,
                    last_valid_slot: 0,
                    order_type: OrderType::PostOnly,
                    // No global orders in competitors
                    global_trade_accounts_opts: &[None, None],
                    current_slot: None,
                })
                .unwrap();

            // Set seuqence number to be the phoenix sequence number. Will
            // likely not be used since our cancelling works on client order ids
            // through the wrapper, but can be used at the start.
            get_mut_helper::<RBNode<RestingOrder>>(dynamic_account.dynamic, result.order_index)
                .get_mut_value()
                .set_sequence_number(order_sequence_number);
            // Use the phoenix expiration. Note that in tests, this will likely
            // be wrong.
            get_mut_helper::<RBNode<RestingOrder>>(dynamic_account.dynamic, result.order_index)
                .get_mut_value()
                .set_last_valid_slot(last_valid_slot);

            // Extend the wrapper and place order.
            extend_wrapper(&mut wrapper_bytes_by_pubkey, &mapped_pubkey);
            place_order_on_wrapper(
                &market_keypair.pubkey(),
                wrapper_bytes_by_pubkey
                    .get_mut(&mapped_pubkey)
                    .unwrap()
                    .as_mut_slice(),
                order_sequence_number,
                order_sequence_number,
                QuoteAtomsPerBaseAtom::try_from(price_float * decimal_adjustment).unwrap(),
                num_base_atoms,
                last_valid_slot,
                result.order_index,
                is_bid,
            );
        }
    }

    // Initialize all the trader accounts with full gas.
    dynamic_account.market_expand().unwrap();
    let mut trader_to_wrapper: HashMap<Pubkey, Pubkey> = HashMap::new();
    let mut trader_to_index_hint: HashMap<Pubkey, DataIndex> = HashMap::new();
    for (pubkey, bytes) in wrapper_bytes_by_pubkey {
        let wrapper_keypair: Keypair = Keypair::new();
        let mut account: Account = Account::new(
            Rent::default().minimum_balance(bytes.len()),
            bytes.len(),
            &wrapper::ID.to_bytes().into(),
        );
        account.data = bytes;
        program_test.add_account(wrapper_keypair.pubkey(), account);
        trader_to_wrapper.insert(pubkey, wrapper_keypair.pubkey());
        trader_to_index_hint.insert(
            pubkey,
            dynamic_account.get_trader_index(&pubkey.to_bytes().into()),
        );
    }

    // Start with some space already allocated.
    for _ in 0..NUM_EXTRA_NODES {
        dynamic_account.market_expand().unwrap();
    }

    // Initialize the market account.
    let mut fixed_bytes: [u8; size_of::<MarketFixed>()] = [0_u8; size_of::<MarketFixed>()];
    *get_mut_helper::<MarketFixed>(&mut fixed_bytes, 0) = *dynamic_account.fixed;
    let mut market_account_data_vec: Vec<u8> = Vec::new();
    market_account_data_vec.extend_from_slice(&fixed_bytes);

    // TODO: Trim to only have what is needed, not the extra that was
    // initialized. This isnt a major issue since the first realloc will
    // actually reduce the size to what we want.
    market_account_data_vec.append(&mut dynamic_account.dynamic.to_vec());
    let mut market_account: Account = Account::new(
        Rent::default().minimum_balance(market_account_data_vec.len()),
        market_account_data_vec.len(),
        &manifest::ID.to_bytes().into(),
    );
    market_account.data = market_account_data_vec;
    program_test.add_account(market_keypair.pubkey(), market_account);

    TestFixture {
        program_test,
        market: market_keypair.pubkey(),
        trader_to_wrapper,
        trader_to_index_hint,
        keypair_map,
        phx_base_lot_size,
        phx_quote_lot_size,
    }
}

fn fund_wallet(program: &mut ProgramTest, pubkey: &Pubkey) {
    program.add_account(
        *pubkey,
        Account::new(u32::MAX as u64, 0, &Pubkey::default()),
    );
}

fn extend_wrapper(wrapper_bytes_by_pubkey: &mut HashMap<Pubkey, Vec<u8>>, pubkey: &Pubkey) {
    wrapper_bytes_by_pubkey
        .get_mut(pubkey)
        .unwrap()
        .extend_from_slice(&[0_u8; size_of::<RBNode<MarketInfo>>()]);
}

fn create_and_save_empty_wrapper(
    pubkey: &Pubkey,
    wrapper_bytes_by_pubkey: &mut HashMap<Pubkey, Vec<u8>>,
) {
    let bytes: [u8; WRAPPER_FIXED_SIZE] = [0_u8; size_of::<ManifestWrapperStateFixed>()];
    let mut wrapper_bytes_vec: Vec<u8> = bytes.to_vec();
    let wrapper_bytes_slice: &mut [u8] = wrapper_bytes_vec.as_mut_slice();
    let wrapper_fixed: ManifestWrapperStateFixed =
        ManifestWrapperStateFixed::new_empty(&pubkey.to_bytes().into());
    *get_mut_helper(wrapper_bytes_slice, 0) = wrapper_fixed;
    wrapper_bytes_by_pubkey.insert(*pubkey, wrapper_bytes_vec);
}

fn claim_seat_on_wrapper(
    dynamic_account: &mut MarketRefMut,
    wrapper_data: &mut [u8],
    pubkey: &Pubkey,
    market_key: &Pubkey,
) {
    // Expand wrapper first to claim a seat.
    expand_wrapper(wrapper_data);
    let (fixed_data, wrapper_dynamic_data) =
        wrapper_data.split_at_mut(size_of::<ManifestWrapperStateFixed>());
    let wrapper_fixed: &mut ManifestWrapperStateFixed = get_mut_helper(fixed_data, 0);

    // Get the free block and setup the new MarketInfo there
    let trader_index: DataIndex = dynamic_account.get_trader_index(&pubkey.to_bytes().into());
    let market_info: MarketInfo = MarketInfo::new_empty(market_key.to_bytes().into(), trader_index);

    // Put that market_info at the free list head
    let mut free_list: FreeList<UnusedWrapperFreeListPadding> =
        unsafe { FreeList::new(wrapper_dynamic_data, wrapper_fixed.free_list_head_index) };
    let free_address: DataIndex = free_list.remove();
    wrapper_fixed.free_list_head_index = free_list.get_head();

    // Insert into the MarketInfosTree
    let mut market_infos_tree: MarketInfosTree = unsafe {
        MarketInfosTree::new(
            wrapper_dynamic_data,
            wrapper_fixed.market_infos_root_index,
            NIL,
        )
    };
    market_infos_tree.insert(free_address, market_info);
    wrapper_fixed.market_infos_root_index = market_infos_tree.get_root_index();
}

fn place_order_on_wrapper(
    market_key: &Pubkey,
    wrapper_data: &mut [u8],
    client_order_id: u64,
    order_sequence_number: u64,
    price: QuoteAtomsPerBaseAtom,
    num_base_atoms: u64,
    last_valid_slot: u32,
    core_new_order_index: DataIndex,
    is_bid: bool,
) {
    expand_wrapper(wrapper_data);
    let market_info_index: DataIndex = get_market_info_index_for_market(wrapper_data, market_key);

    // Add to client order id tree.
    let (fixed_data, dynamic_data) =
        wrapper_data.split_at_mut(size_of::<ManifestWrapperStateFixed>());
    let wrapper_orders_root_index: DataIndex = {
        let market_info: &mut MarketInfo =
            get_mut_helper::<RBNode<MarketInfo>>(dynamic_data, market_info_index).get_mut_value();
        market_info.orders_root_index
    };

    let wrapper_fixed: &mut ManifestWrapperStateFixed = get_mut_helper(fixed_data, 0);
    let wrapper_new_order_index: DataIndex = {
        let mut free_list: FreeList<UnusedWrapperFreeListPadding> =
            unsafe { FreeList::new(dynamic_data, wrapper_fixed.free_list_head_index) };
        let new_index: DataIndex = free_list.remove();
        wrapper_fixed.free_list_head_index = free_list.get_head();
        new_index
    };

    let order: WrapperOpenOrder = WrapperOpenOrder::new(
        client_order_id,
        order_sequence_number,
        price,
        BaseAtoms::new(num_base_atoms).into(),
        last_valid_slot,
        core_new_order_index,
        is_bid,
        OrderType::Limit,
    );

    let new_root_index: DataIndex = {
        let mut open_orders_tree: OpenOrdersTree =
            unsafe { OpenOrdersTree::new(dynamic_data, wrapper_orders_root_index, NIL) };
        open_orders_tree.insert(wrapper_new_order_index, order);
        open_orders_tree.get_root_index()
    };
    let market_info: &mut MarketInfo =
        get_mut_helper::<RBNode<MarketInfo>>(dynamic_data, market_info_index).get_mut_value();

    market_info.orders_root_index = new_root_index;
}
