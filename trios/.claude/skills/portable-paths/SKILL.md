---
name: portable-paths
description: Centralize and harden project root resolution in trios so the repo works on any checkout/CI without hardcoded absolute paths.
argument-hint: [ring]
---

# Portable Paths Skill (trios)

Makes trios root resolution environment-portable and eliminates hardcoded developer-path fallbacks.

## When to Invoke

- Adding a new Rust ring that needs the project root.
- Finding `/Users/playra/` or any absolute home path in source/build files.
- Preparing the repo for CI or another developer machine.
- Refactoring runtime state out of `/tmp` into project-relative directories.

## Canonical Resolver

### Rust
The single source of truth is `trios-config::project_dir()` in `rings/RUST-00/trios-config/src/lib.rs`:

```rust
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

Rings declare the dependency and delegate:

```toml
[dependencies]
trios-config = { path = "../../RUST-00/trios-config" }
```

```rust
fn project_dir() -> String { trios_config::project_dir() }
```

### Swift
`BR-OUTPUT/ProjectPaths.swift` resolves in priority order:

1. `ProcessInfo.processInfo.environment["TRIOS_ROOT"]`
2. `Bundle.main.resourcePath` parent (when running from `.app`)
3. `FileManager.default.currentDirectoryPath`

## Migration Recipe

1. If the ring lacks a `trios-config` dependency, add it to `Cargo.toml`.
2. Delete the local `project_dir()` function.
3. Replace all calls with `trios_config::project_dir()`.
4. Delete any `const DEFAULT_TRIOS_ROOT: &str = "/Users/..."`.
5. Run `cargo test -p {ring}` and `cargo clippy -p {ring}`.
6. Run `grep -RIn '/Users/playra/' rings/ BR-OUTPUT/` - must be empty.

## Runtime State Migration

Move persistent state from `/tmp/` to `.trinity/` subdirs:

| Old path | New path |
| --- | --- |
| `/tmp/trios_e2e` | `{project_dir()}/.trinity/e2e` |
| `/tmp/trios_screenshot.png` | `{project_dir()}/.trinity/e2e/trios_screenshot.png` |
| `/tmp/clade-rollback` | `{project_dir()}/.trinity/rollback` |
| `/tmp/clade-dev` | `{project_dir()}/.trinity/dev` |
| `/tmp/mesh.drop` | `{project_dir()}/.trinity/run/mesh.drop` (P1 backlog) |

Ensure directories are created with restricted permissions (`0o700`) where security matters.

## Tests

- `cargo test --workspace` passes.
- `cargo clippy --workspace --all-targets --all-features` is clean (except pre-existing approved debt).
- `grep -RIn '/Users/playra/' rings/ BR-OUTPUT/` returns zero matches.
- Running `./build.sh` without `TRIOS_ROOT` still produces `trios.app`.

## Rules

- Never hardcode a developer home path as a fallback.
- Never use `/tmp` for persistent runtime state in production code.
- Always prefer `TRIOS_ROOT` env override; fall back to `current_dir()` or bundle path.
- Keep all changed files ASCII-only (L3 PURITY).
