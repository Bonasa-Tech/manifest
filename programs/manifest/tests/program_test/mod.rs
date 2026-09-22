pub mod fixtures;

use solana_program_test::ProgramTest;

pub use fixtures::*;

pub fn manifest_program_test() -> ProgramTest {
    // Always the compiled binary. The entrypoint hands out pinocchio account
    // views, which are pointers into the runtime's serialized input, and off
    // the SBF target pinocchio compiles its syscalls to no-ops: a native
    // processor would skip every CPI and the tests would pass without testing
    // anything. Run with `cargo test-sbf`, or point BPF_OUT_DIR at a build.
    // ProgramTest 4.2.2 bundles p-token 1.0.0 and Token-2022 10.0.0. These
    // exercise our CPI interface, not a claim about mainnet's deployed versions;
    // the upgrade replay captures its token binaries from the selected RPC.
    // Using native Rust token processors would cross two
    // solana-sysvar crate generations and make CPI sysvar reads fail with
    // UnsupportedSysvar.
    ProgramTest::new("manifest", manifest::ID, None)
}
