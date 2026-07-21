# Portable root resolution specification

## Scope
Remove every hardcoded `/Users/playra/BrowserOS-full/trios` fallback from Rust rings and Swift `BR-OUTPUT/ProjectPaths.swift`. Centralize root resolution in `trios-config::project_dir()` and make all rings derive project paths from that single source of truth.

## Invariants
1. No Rust ring or Swift file may contain the literal `/Users/playra/` path as a fallback.
2. `TRIOS_ROOT` env var is the primary override.
3. Fallback is `std::env::current_dir()` (Rust) or `FileManager.default.currentDirectoryPath` / bundle path (Swift), not a hardcoded home directory.
4. If both `TRIOS_ROOT` and current directory resolution fail, the program must log a clear error and exit instead of silently using a wrong path.
5. `trios-config::project_dir()` is the canonical resolver; rings should import `trios-config` and call it.
6. L3 PURITY: no non-ASCII characters in changed files.
7. L7 UNITY: no new `.sh` scripts introduced.

## Interface

### Rust
```rust
// trios-config/src/lib.rs
pub fn project_dir() -> String {
    std::env::var("TRIOS_ROOT").unwrap_or_else(|_| {
        match std::env::current_dir() {
            Ok(p) => p.to_string_lossy().into_owned(),
            Err(e) => {
                eprintln!("[FAIL] TRIOS_ROOT not set and current_dir unavailable: {}", e);
                std::process::exit(1);
            }
        }
    })
}
```

Rings import:
```rust
use trios_config::project_dir;
```

### Swift
```swift
// BR-OUTPUT/ProjectPaths.swift
static var root: String {
    if let env = ProcessInfo.processInfo.environment["TRIOS_ROOT"], !env.isEmpty {
        return env
    }
    return FileManager.default.currentDirectoryPath
}
```

## Algorithm
1. Read `TRIOS_ROOT` env var.
2. If unset/empty, get current working directory.
3. If current directory unavailable, print `[FAIL] TRIOS_ROOT not set and current_dir unavailable: {error}` and exit 1.
4. Canonicalize if possible, but return the resolved string even if canonicalization fails (e.g. path does not yet exist).
5. All rings replace their local `project_dir()` with `trios_config::project_dir()`.

## Affected files
- `rings/RUST-00/trios-config/src/lib.rs` - canonical resolver.
- `rings/RUST-01/clade-build/src/main.rs`
- `rings/RUST-03/clade-rollback/src/main.rs`
- `rings/RUST-06/clade-dashboard/src/main.rs`
- `rings/RUST-07/clade-experience/src/main.rs`
- `rings/RUST-08/clade-promote/src/main.rs`
- `rings/RUST-09/clade-launchd/src/main.rs`
- `rings/RUST-10/clade-worktree/src/main.rs`
- `rings/RUST-12/clade-audit/src/main.rs`
- `rings/RUST-14/clade-tablecloth/src/main.rs`
- `rings/RUST-04/clade-improve/src/variant.rs`
- `BR-OUTPUT/ProjectPaths.swift`

For rings that do not already depend on `trios-config`, add the dependency in their `Cargo.toml`.

## Tests
- `cargo test --workspace` passes.
- `cargo clippy --workspace --all-targets --all-features` is clean.
- `grep -RIn '/Users/playra/' rings/ BR-OUTPUT/*.swift` returns zero matches.
- `./build.sh` passes.
- Running any ring binary without `TRIOS_ROOT` from within the trios directory works.

## Change flow
Spec-first. Each ring change is a mechanical replacement of local `project_dir()` with `trios_config::project_dir()`. Add `trios-config` dependency where missing. Land after `t27-verifier` confirms zero hardcoded path violations.
