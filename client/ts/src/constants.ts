export const FIXED_MANIFEST_HEADER_SIZE: number = 256;
export const FIXED_GLOBAL_HEADER_SIZE: number = 96;
export const FIXED_WRAPPER_HEADER_SIZE: number = 64;
// Seed used to derive a trader's wrapper account address via
// `PublicKey.createWithSeed`. Deriving the address (instead of using a freshly
// generated Keypair) lets the wrapper account be created with
// `SystemProgram.createAccountWithSeed`, which requires no extra signer beyond
// the trader. This keeps setup compatible with multisig / smart-account wallets
// (e.g. Squads), whose only available signer is the trader account itself.
export const WRAPPER_SEED: string = 'manifest-wrapper';
export const NIL: number = 4_294_967_295;
export const NO_EXPIRATION_LAST_VALID_SLOT = 0;
export const U32_MAX = 4_294_967_295;
export const PRICE_MIN_EXP = -18;
export const PRICE_MAX_EXP = 8;
