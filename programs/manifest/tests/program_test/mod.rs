pub mod fixtures;

use solana_program_test::ProgramTest;

pub use fixtures::*;

pub fn manifest_program_test() -> ProgramTest {
    let mut program = {
        #[cfg(feature = "test-sbf")]
        {
            ProgramTest::new("manifest", manifest::ID, None)
        }

        #[cfg(not(feature = "test-sbf"))]
        {
            ProgramTest::new(
                "manifest",
                manifest::ID,
                solana_program_test::processor!(manifest::process_instruction),
            )
        }
    };

    #[cfg(feature = "test-sbf")]
    program.prefer_bpf(false);

    program.add_program(
        "spl_token",
        spl_token::ID,
        solana_program_test::processor!(spl_token::processor::Processor::process),
    );
    program.add_program(
        "spl_token_2022",
        spl_token_2022::ID,
        solana_program_test::processor!(spl_token_2022::processor::Processor::process),
    );

    #[cfg(feature = "test-sbf")]
    program.prefer_bpf(true);

    program
}
