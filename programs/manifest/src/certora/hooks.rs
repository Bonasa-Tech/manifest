#[derive(Clone, Copy, Ord, Eq, PartialEq, PartialOrd)]
enum CvtManifestOrder {
    None,
    CancelOrder,
    CancelOrderByIndex,
    PlaceOrder,
}

/// Keep track of which order was executed
#[link_section = ".rodata.certora"]
static mut LAST_ORDER_EXECUTED: CvtManifestOrder = CvtManifestOrder::None;

/// Keep track of whether remove_order_from_tree_and_free was called
#[link_section = ".rodata.certora"]
static mut ORDER_REMOVED: bool = false;

// Initialization

pub fn initialize_hooks() {
    unsafe {
        LAST_ORDER_EXECUTED = CvtManifestOrder::None;
        ORDER_REMOVED = false;
    }
}

// Setters

pub fn cancel_order_was_called() {
    unsafe {
        LAST_ORDER_EXECUTED = CvtManifestOrder::CancelOrder;
    }
}

pub fn cancel_order_by_index_was_called() {
    unsafe {
        LAST_ORDER_EXECUTED = CvtManifestOrder::CancelOrderByIndex;
    }
}

pub fn place_order_was_called() {
    unsafe {
        LAST_ORDER_EXECUTED = CvtManifestOrder::PlaceOrder;
    }
}

pub fn remove_order_from_tree_and_free_was_called() {
    unsafe {
        ORDER_REMOVED = true;
    }
}

// Getters

pub fn last_called_cancel_order() -> bool {
    unsafe { LAST_ORDER_EXECUTED == CvtManifestOrder::CancelOrder }
}

pub fn last_called_cancel_order_by_index() -> bool {
    unsafe { LAST_ORDER_EXECUTED == CvtManifestOrder::CancelOrderByIndex }
}

pub fn last_called_place_order() -> bool {
    unsafe { LAST_ORDER_EXECUTED == CvtManifestOrder::PlaceOrder }
}

pub fn last_called_remove_order_from_tree_and_free() -> bool {
    unsafe { ORDER_REMOVED == true }
}
