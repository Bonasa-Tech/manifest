use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=src/certora/verification.ld");
    if env::var_os("CARGO_FEATURE_CERTORA").is_some()
        && env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("solana")
    {
        assert_eq!(
            env::var("TARGET").unwrap(),
            "sbpfv3-solana-solana",
            "Certora verification must compile sBPF v3"
        );
        let script: PathBuf = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
            .join("src/certora/verification.ld");
        println!("cargo:rustc-link-arg=-T{}", script.display());
    }
}
