# trios-mesh integration notes

## What was done

- Added `gHashTag/tri-net` as a git submodule at `trios/rings/RUST-13/trios-mesh`,
  pinned to `main` branch commit `6850649`.
- Wired `trios-mesh` into the trios Cargo workspace (`trios/Cargo.toml`).
- Patched the submodule to compile inside trios:
  - `build.rs`: fixed `metadata().modified().ok()` type error.
  - `Cargo.toml`: disabled broken auto-discovered binaries (`autobins = false`,
    removed `[[bin]]` and `[profile.release]`).
  - `src/lib.rs`: excluded stub `gen/rust/` modules until `t27c` produces valid
    Rust.
  - `src/wire.rs`: inlined the wire spec implementation instead of including the
    broken generated file.
- Saved the exact patch to
  `trios/.trinity/patches/trios-mesh-integration.patch`.
- Updated `trios/CLAUDE.md` with the mesh ring test command.

## Branch study summary

`tri-net` has ~25 remote branches. Key ones inspected:

- `main` — Rust mesh crate (`trios-mesh`), M1 crypto proven, M2-M5 `-sim`.
  Selected as source of truth.
- `feat/trios-chat-spec` — chat product spec and M2 convergence results.
- `feat/regen-final` — most active; contains iOS/macOS `TriNetVideo` and
  `TriNetMonitor` apps plus OTA RF demos. Not merged into `main`.

## Verification

- `cargo test -p trios-mesh` — 101 passed.
- `cargo clippy --all-targets --all-features` — clean.
- `./build.sh` — passed.
- `e2e/trios_e2e_flow.sh` — trios app running; BrowserOS Server was DOWN
  (pre-existing environment, not caused by this change).

## Open items

1. The submodule fixes live in a local `feat/trios-integration` branch. Push it
   to `gHashTag/tri-net` before sharing this branch or opening a PR.
2. L1 TRACEABILITY requires a GitHub issue with `Closes #N` before merge.
3. Re-enable `gen/rust/` modules once `t27c` generates valid Rust.
4. Re-implement `trios_meshd` and `smoke-m1` binaries against the cleaned-up
   API if they are needed.
