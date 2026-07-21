# trios-mesh lint and NaN safety specification

## Scope
Bring `rings/RUST-13/trios-mesh` under the workspace clippy lints and eliminate a real panic surface in routing metric sorting.

## Invariants
1. `trios-mesh` must inherit `[lints] workspace = true` from the root `Cargo.toml`.
2. Production code must not trigger `clippy::unwrap_used` (`deny` in workspace).
3. Test code may use `unwrap` for infallible test-only assertions, but must be explicitly scoped under `cfg(test)`.
4. Sorting and selection by ETX metric must be total and must not panic on NaN.

## Interface changes
- `Cargo.toml`: add `[lints] workspace = true`.
- `src/lib.rs`: add `#![cfg_attr(test, allow(clippy::unwrap_used))]` with a comment explaining the exemption.
- `src/router.rs`: replace `sort_by` `partial_cmp` unwrap with a total order treating NaN as worse than finite.
- `src/routing.rs`: replace `min_by` `partial_cmp` unwrap with the same total order.
- `build.rs`: replace `unwrap` on `file_stem()` / `to_str()` with safe `Option` handling and UTF-8 validation.

## NaN handling
ETX is constructed to be finite, but defensive sorting uses:
```rust
a.1.partial_cmp(&b.1)
    .unwrap_or_else(|| a.1.is_nan().cmp(&b.1.is_nan()).reverse())
```
Finite values sort normally. NaN is treated as worse than any finite value.

## Tests
- `cargo clippy -p trios-mesh --all-targets --all-features` must pass.
- `cargo test -p trios-mesh --all-features` must pass.
- `cargo test --workspace` must pass.

## Change flow
All changes to this crate must be justified by this spec. Emergency hand edits require an `// AGENT-V-WAIVER:` block.
