pub mod fixtures;

use solana_program_test::ProgramTest;

pub use fixtures::*;

pub fn manifest_program_test() -> ProgramTest {
    // The program is always the compiled binary. Its entrypoint hands out
    // pinocchio account views, which are pointers into the runtime's input
    // buffer, so there is no native processor to register: the tests run what
    // gets deployed. Build with `cargo test-sbf`, or point `BPF_OUT_DIR` at a
    // build.
    let mut program: ProgramTest = ProgramTest::new("manifest", manifest::ID, None);
    // The SPL programs below are registered with native processors and should
    // keep using them; only this program has to come from its binary.
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
