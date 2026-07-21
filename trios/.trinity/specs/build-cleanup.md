# Build script cleanup specification

## Scope
Make `build.sh` and `rings/RUST-01/clade-build/src/main.rs` environment-portable and ASCII-only so the trios build works from any checkout directory without hardcoded user paths.

## Invariants
1. No hardcoded `/Users/playra/...` paths in build orchestration.
2. `build.sh` derives `PROJECT_DIR` from its own location or from `TRIOS_ROOT`.
3. `clade-build` derives its project root from `TRIOS_ROOT` with a fallback to `std::env::current_dir()`.
4. Build logs are written under `.trinity/logs/` with deterministic names, never to `/tmp`.
5. L3 PURITY: no emoji or non-ASCII status markers; use `[OK]` and `[FAIL]`.
6. `./build.sh` must still produce a working `trios.app` and exit 0 on success.

## Interface
```bash
# build.sh
PROJECT_DIR="${TRIOS_ROOT:-$(cd "$(dirname "$0")" && pwd)}"
LOG_DIR="$PROJECT_DIR/.trinity/logs"
LOG_FILE="$LOG_DIR/build_$(date +%Y%m%d_%H%M%S).log"
```

```rust
// clade-build/src/main.rs
fn project_dir() -> String {
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

## Algorithm
1. At script start, compute `PROJECT_DIR`.
2. Create `.trinity/logs/` if missing.
3. Tee build output to `LOG_FILE`.
4. Compile Swift sources with `swiftc -O`.
5. Copy binary into `trios.app/Contents/MacOS/trios` and generate `Info.plist`.
6. `clade-build` writes its own log to `.trinity/logs/clade-build_{variant}.log`.

## Tests
- `./build.sh` passes from a fresh shell without `TRIOS_ROOT`.
- `cargo test -p clade-build` passes.
- `cargo clippy -p clade-build --all-targets --all-features` is clean.
- `cargo test --workspace` still passes.

## Change flow
Spec-first. Land only after `./build.sh`, `cargo test -p clade-build`, and `cargo test --workspace` all succeed.
