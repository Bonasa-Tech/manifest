# Manifest Client

This module implements the `Amm` trait defined [here](https://github.com/jup-ag/rust-amm-implementation).

There is a swap_v2 instruction that can be swapped in for the swap instruction
if required. The call data is the same, the only difference is that the swap_v2
allows a separate owner of the user token accounts from the gas/rent payer of
the tx. This is useful when a router has intermediate token accounts owned by
their program and not the user. If that is not the case, the normal swap ix
should suffice.

### Testing

```
cargo test -- --nocapture
```
