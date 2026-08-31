//! The slim client re-declares `MarketFixed` so that it does not have to
//! depend on the program crate. Nothing makes the two definitions agree, and
//! when they drift the account still parses, because the size and the
//! discriminant are unchanged, while every field after the drift silently
//! reads the wrong bytes. That has happened: a field that only ever existed
//! in a stale size comment was added here, which moved `free_list_head_index`
//! by four bytes and `quote_volume` by eight.
//!
//! These tests pin the two layouts together by filling a program
//! `MarketFixed` with a pattern that makes every byte position
//! distinguishable and reading it back through the slim definition.

use bytemuck::Zeroable;
use manifest::{quantities::WrapperU64, state::MarketFixed as CoreMarketFixed};
use manifest_client::{MarketFixed as SlimMarketFixed, MARKET_FIXED_DISCRIMINANT};
use solana_pubkey::Pubkey;

/// A program `MarketFixed` whose bytes are all distinct, so that a field read
/// at the wrong offset cannot coincidentally match.
fn patterned_core_fixed() -> CoreMarketFixed {
    let mut fixed: CoreMarketFixed = CoreMarketFixed::zeroed();
    for (i, byte) in bytemuck::bytes_of_mut(&mut fixed).iter_mut().enumerate() {
        // Never zero, so that an unset field is distinguishable from a set one.
        *byte = (i as u8) | 1;
    }
    fixed.discriminant = MARKET_FIXED_DISCRIMINANT;
    fixed.set_base_global(Pubkey::new_from_array([7_u8; 32]));
    fixed.set_quote_global(Pubkey::new_from_array([9_u8; 32]));
    fixed
}

#[test]
fn slim_market_fixed_matches_the_program_layout() {
    assert_eq!(
        std::mem::size_of::<SlimMarketFixed>(),
        std::mem::size_of::<CoreMarketFixed>(),
        "the two MarketFixed definitions must stay the same size",
    );

    let core: CoreMarketFixed = patterned_core_fixed();
    let slim: SlimMarketFixed = SlimMarketFixed::try_from_bytes(bytemuck::bytes_of(&core))
        .expect("slim parses a program market header");

    assert_eq!(slim.get_base_mint(), *core.get_base_mint(), "base_mint");
    assert_eq!(slim.get_quote_mint(), *core.get_quote_mint(), "quote_mint");
    assert_eq!(slim.get_base_vault(), *core.get_base_vault(), "base_vault");
    assert_eq!(
        slim.get_quote_vault(),
        *core.get_quote_vault(),
        "quote_vault"
    );
    assert_eq!(
        slim.base_mint_decimals,
        core.get_base_mint_decimals(),
        "base_mint_decimals",
    );
    assert_eq!(
        slim.quote_mint_decimals,
        core.get_quote_mint_decimals(),
        "quote_mint_decimals",
    );
    assert_eq!(
        slim.base_vault_bump,
        core.get_base_vault_bump(),
        "base_vault_bump",
    );
    assert_eq!(
        slim.quote_vault_bump,
        core.get_quote_vault_bump(),
        "quote_vault_bump",
    );
    // The field the last drift moved. Everything between the tree indices and
    // this one is pinned by it: an extra or resized field before it shifts
    // this read.
    assert_eq!(
        slim.quote_volume,
        core.get_quote_volume().as_u64(),
        "quote_volume",
    );
    assert_eq!(
        slim.get_base_global(),
        Some(*core.get_base_global()),
        "base_global",
    );
    assert_eq!(
        slim.get_quote_global(),
        Some(*core.get_quote_global()),
        "quote_global",
    );
}

#[test]
fn slim_reports_no_cached_globals_on_an_old_market() {
    let mut core: CoreMarketFixed = CoreMarketFixed::zeroed();
    core.discriminant = MARKET_FIXED_DISCRIMINANT;
    let slim: SlimMarketFixed =
        SlimMarketFixed::try_from_bytes(bytemuck::bytes_of(&core)).expect("slim parses");
    assert_eq!(slim.get_base_global(), None, "base_global uncached");
    assert_eq!(slim.get_quote_global(), None, "quote_global uncached");
}
