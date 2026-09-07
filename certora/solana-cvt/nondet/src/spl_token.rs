use crate::nondet;

impl crate::Nondet for spl_token::state::Mint {
    fn nondet() -> Self {
        Self {
            mint_authority: nondet(),
            supply: nondet(),
            decimals: nondet(),
            is_initialized: nondet(),
            freeze_authority: nondet(),
        }
    }
}

impl crate::Nondet for spl_token_2022::state::Mint {
    fn nondet() -> Self {
        Self {
            mint_authority: nondet(),
            supply: nondet(),
            decimals: nondet(),
            is_initialized: nondet(),
            freeze_authority: nondet(),
        }
    }
}
