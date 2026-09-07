pub mod fixtures;

use solana_program_test::ProgramTest;

pub use fixtures::*;

pub fn manifest_program_test() -> ProgramTest {
    // Always the compiled binary. The entrypoint hands out pinocchio account
    // views, which are pointers into the runtime's serialized input, and off
    // the SBF target pinocchio compiles its syscalls to no-ops: a native
    // processor would skip every CPI and the tests would pass without testing
    // anything. Run with `cargo test-sbf`, or point BPF_OUT_DIR at a build.
    let mut program: ProgramTest = ProgramTest::new("manifest", manifest::ID, None);
    // The SPL programs registered below keep their native processors.
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
