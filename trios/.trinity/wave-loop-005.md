# Wave Loop 005 - trios-mesh daemon panic hardening and runtime-state isolation

## Sources

- Wave 5 weak-spot audit after Wave 004.
- Literature scan:
  - [PanicFI: An Infrastructure for Fixing Panic Bugs in Real-World Rust Programs](https://www.arxiv.org/pdf/2408.03262) (TOSEM 2025) - dataset of 102 real-world Rust panic bugs and 19 fix patterns; shows `unwrap`/`expect` are a dominant outage surface.
  - [Broadly Enabling KLEE to Effortlessly Find Unrecoverable Errors in Rust](https://people.cs.vt.edu/djwillia/papers/icse-seip24-paniccheck.pdf) (ICSE-SEIP 2024) - symbolic execution for panic detection; 61 real panics found and fixed.
  - [Beyond Memory Safety: An Empirical Study on Bugs and Fixes of Rust Programs](https://doi.org/10.1109/QRS62785.2024.00035) (QRS 2024) - 201 panic instances across safe Rust.
  - [AgentBound: Securing Execution Boundaries of AI Agents](https://www.lucadigrazia.com/papers/fse2026.pdf) (FSE 2026) - allowlist-based daemon/runtime isolation.
  - [Sandlock: Confining AI Agent Code with Unprivileged Linux Primitives](https://arxiv.org/html/2605.26298) (2026) - lightweight process confinement for daemons.
  - [A Hybrid Approach to Semi-automated Rust Verification](https://vtss.doc.ic.ac.uk/publications/Ayoun2025Hybrid.published.pdf) (PLDI 2025) - automated verification as a CI gate.

## Key research takeaways

1. `unwrap`/`expect` in daemon code cause real outages (PanicFI fixed 28 merged bugs). Network/config/socket failures must return errors, not panic.
2. Symbolic execution (PanicCheck) shows most panics come from invalid API inputs and `unwrap()` on unexpected values - exactly the pattern in `trios_meshd.rs` config parsing.
3. Secure daemon deployment requires project-relative writable paths, not world-writable `/tmp` (AgentBound/Sandlock).
4. Automated verification as a CI gate is practical for Rust (Creusot/Gillian-Rust), but Wave 5 stays at the clippy/Result-propagation layer.

## Decomposed plan (P0 -> P5)

### P0 - Harden trios-mesh library production panic surfaces
- [x] Add `MeshError::CryptoInternal` variant for "should never happen" crypto primitive failures.
- [x] Replace 9 `expect` calls in `crypto.rs` with `Result` propagation or infallible helpers that map failure to `MeshError::CryptoInternal`.
- [x] Cascade `Result` through `Handshake::complete`, `StaticKey::session_with`, `clade-meshd` handlers, and tests.
- [x] Add helpers `hkdf_expand_32` and `hkdf_from_prk` to centralize the 32-byte invariant.

### P1 - Harden discovery.rs MAC computation
- [x] Change `Hello::compute_mac` to return `Result<[u8; 16], MeshError>`.
- [x] Make `Hello::authenticated` return `Result<Self, MeshError>`.
- [x] Make `Hello::verify_mac` return `false` on internal MAC failure.

### P2 - Harden trios_meshd binary and runtime-state path
- [x] Convert `parse_cfg` to `Result<Cfg, String>` with line-numbered errors.
- [x] Convert `main` to print error and exit 1 on failure instead of panicking.
- [x] Use `std::sync::Mutex::lock().unwrap_or_else(|p| p.into_inner())` for poison recovery in daemon hot-path mutex locks.
- [x] Move `/tmp/mesh.drop` default to `.trinity/run/mesh.drop`, overridable via `TRIOS_MESH_DROP`.
- [ ] Register `trios-meshd` binary in `Cargo.toml` - **deferred**: binary uses an older `trios-mesh` API and needs a revival pass before it can compile under the current crate shape.

### P3 - Elevate workspace lint and add test exemptions
- [x] Change `expect_used = "warn"` to `"deny"` in `trios/Cargo.toml`.
- [x] Extend `trios-mesh/src/lib.rs` with `#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]`.
- [x] Ensure `cargo clippy -p trios-mesh -p clade-meshd --all-targets --all-features` is clean.

### P4 - Verify
- [x] Run `./build.sh`.
- [x] Run `cargo test -p trios-mesh --all-features` (101 tests) and `cargo test -p clade-meshd --all-features` (2 tests).
- [x] Run ASCII scan on all changed files; zero violations.

### P5 - Backlog
- [ ] `tmp-zero`: move remaining test-only `/tmp` usage in `clade-experience`, `clade-launchd`, `clade-audit` to project-relative test dirs or `tempfile` crate.
- [ ] `seal-automation`: add a `clade-seal` ring that runs build + test + clippy + ASCII scan and gates promotion.
- [ ] `promotion-lock`: prevent concurrent `clade-promote` runs.
- [ ] `meshd-revival`: repair `trios_meshd.rs` against current `trios-mesh` API and register it as `[[bin]]`.

## This iteration goal

Land P0-P4: make `trios-mesh` clippy-clean for `expect_used`/`unwrap_used` in production, harden the `trios_meshd` binary startup, move `/tmp/mesh.drop` under `.trinity/run/`, and elevate the workspace lint so new panic surfaces cannot be introduced. Keep changes ASCII-only and spec-first.

## Wave 005 Closeout Report

Status: LANDED in commit `HEAD` on branch `feat/zai-provider`.

### What shipped

- **P0 crypto panic hardening**: added `MeshError::CryptoInternal`, replaced 9 `expect` calls in `crypto.rs` with `Result` propagation via helpers `hkdf_expand_32`, `hkdf_from_prk`, `read_u32_be`, `read_u64_be`. Public signatures now return `Result`: `combine_dh_shares`, `NoiseXX::complete_*`, `Session::from_shared`, `Session::ratchet`, `Handshake::complete`, `StaticKey::session_with`.
- **P1 discovery MAC hardening**: `Hello::compute_mac` and `Hello::authenticated` return `Result`; `Hello::verify_mac` returns `false` on internal failure instead of panicking.
- **P2 meshd startup + state isolation**: rewrote `trios_meshd.rs` with `Result`-based startup, line-numbered config errors, mutex poison recovery, and default drop path `.trinity/run/mesh.drop` (`TRIOS_MESH_DROP` override).
- **P3 lint elevation**: workspace `expect_used`/`unwrap_used` set to `"deny"`; `trios-mesh` tests exempted via `#![cfg_attr(test, allow(...))]`.
- **Spec-first**: added `.trinity/specs/mesh-panic-hardening.md`.
- **Skills saved**: updated `trios/.claude/skills/ascii-lint/SKILL.md` (U+2190, U+21D4, U+00A7 mappings) and created `trios/.claude/skills/panic-hardening/SKILL.md`.
- **ASCII purity**: all changed Rust source, Cargo.toml descriptions, specs, and skills are ASCII-only.

### Verification

- `./build.sh` -> OK.
- `cargo test -p trios-mesh --all-features` -> 101 tests passed.
- `cargo test -p clade-meshd --all-features` -> 2 tests passed.
- `cargo clippy -p trios-mesh -p clade-meshd --all-targets --all-features` -> clean.
- ASCII scan on all changed files -> zero violations.

### Remaining backlog (not in scope)

- `tmp-zero`: remaining `/tmp` paths in other rings.
- `seal-automation`: CI gate for build/test/clippy/ASCII.
- `promotion-lock`: concurrent `clade-promote` guard.
- `meshd-revival`: register `trios-meshd` binary once API drift is repaired.

## [FUTURE OPTIONS - next Wave loop]

1) **`meshd-revival`** - repair `src/bin/trios_meshd.rs` against the current `trios-mesh` API (new `Delivery` enum, struct `MeshRouter`, exported `StaticKey::from_seed`), register it as `[[bin]]`, and add config parsing/host-sim e2e tests. Goal: make the mesh daemon a first-class workspace binary that can be started/stopped by `clade-monitor`.

2) **`tmp-zero`** - move remaining `/tmp` usage in `clade-experience`, `clade-launchd`, and `clade-audit` to project-relative dirs or `tempfile`, add a CI test that greps for `/tmp` in production source, and finalize the runtime-state policy. Goal: zero world-writable runtime paths across all trios rings.

3) **`seal-automation`** - implement a `clade-seal` Rust ring that runs `./build.sh` + `cargo test --workspace` + `cargo clippy --workspace --all-targets --all-features` + ASCII scan, then writes a signed seal file to `.trinity/state/seal.json` that `clade-promote` requires before advancing any branch. Goal: reproducible, automated Wave closeout gating.
