# HyperTree sBPF Assembly Optimizations

This directory contains sBPF (Solana BPF) assembly implementations of performance-critical Red-Black tree operations.

## Prerequisites

1. Install the sbpf SDK:
   ```bash
   cargo install --git https://github.com/blueshift-gg/sbpf.git
   ```

## Building

```bash
make
```

This will:
1. Assemble all `.s` files in `src/`
2. Link them into `build/hypertree_asm.so`

## Files

| File | Description |
|------|-------------|
| `src/rotate_left.s` | Left rotation around a node |
| `src/rotate_right.s` | Right rotation around a node |
| `src/insert_fix.s` | Fix tree after insertion |
| `src/remove_fix.s` | Fix tree after removal |

## Memory Layout

The assembly code assumes the following RBNode layout (must match Rust struct):

```
Offset  Size  Field
0x00    4     left (DataIndex)
0x04    4     right (DataIndex)
0x08    4     parent (DataIndex)
0x0C    1     color (0=Black, 1=Red)
0x0D    1     payload_type
0x0E    2     _unused_padding
0x10    N     value (Payload)
```

## NIL Handling

**Important:** The NIL sentinel value differs between Certora and production builds:
- Certora: `0x7FFFFFFF`
- Production: `0xFFFFFFFF`

The assembly files use the production NIL value. For Certora verification, the Rust implementation must be used.

## Integration

To use these optimizations:

1. Enable `opt-asm` or `opt-full` when building for SBF. The production tree
   fixup loops call the assembly functions on SBF targets and use the optimized
   Rust path for native tests.
2. The Rust bindings include these files with `global_asm!` for SBF builds, so
   the assembly symbols are linked into the program directly.

## Testing

```bash
make test
```

## Cleaning

```bash
make clean
```
