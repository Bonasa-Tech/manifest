# user settings should be placed into .env file in some ancestor directory
set dotenv-load

# used by OSX, ignore otherwise
export CPATH := env_var_or_default("CPATH", "/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk/usr/include")

# features used when compiling target Rust code
export CARGO_FEATURES := env("CARGO_FEATURES", "")

doc:
	cargo doc --lib -F certora
test *TESTS:
	cargo test {{TESTS}} -- --nocapture
test-certora *TESTS:
	cargo test --features certora-test {{TESTS}} -- --nocapture

build-sbf extra_features="":
	cargo certora-sbf --tools-version v1.57 --features certora {{ extra_features }} ${CARGO_FEATURES}

build-sbf-llvm:
	env RUSTFLAGS="--emit=llvm-ir -C no-vectorize-slp -C opt-level=2" \
	cargo certora-sbf --tools-version v1.57 --features certora ${CARGO_FEATURES}

build:
	cargo build

cvlr-update:
	cargo update -p cvlr
	cargo update -p cvlr-solana	

clean:
	rm -f *.dot *.png *.svg
	rm -Rf emv-*
	rm -Rf .certora_internal
	rm -f log.txt
	rm -rf certora_out
