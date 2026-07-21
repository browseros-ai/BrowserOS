# Wave 005 Plan - trios-mesh daemon panic hardening and runtime-state isolation

## Sources and research

- Weak-spot audit of the current `trios/` tree after Wave 004.
- Scientific literature:
  - [PanicFI: An Infrastructure for Fixing Panic Bugs in Real-World Rust Programs](https://www.arxiv.org/pdf/2408.03262) (TOSEM 2025) - dataset of 102 real-world Rust panic bugs and 19 fix patterns; shows `unwrap`/`expect` are a dominant outage surface.
  - [Broadly Enabling KLEE to Effortlessly Find Unrecoverable Errors in Rust](https://people.cs.vt.edu/djwillia/papers/icse-seip24-paniccheck.pdf) (ICSE-SEIP 2024) - symbolic execution for panic detection; 61 real panics found and fixed.
  - [Beyond Memory Safety: An Empirical Study on Bugs and Fixes of Rust Programs](https://doi.org/10.1109/QRS62785.2024.00035) (QRS 2024) - 201 panic instances across safe Rust.
  - [AgentBound: Securing Execution Boundaries of AI Agents](https://www.lucadigrazia.com/papers/fse2026.pdf) (FSE 2026) - allowlist-based daemon/runtime isolation.
  - [Sandlock: Confining AI Agent Code with Unprivileged Linux Primitives](https://arxiv.org/html/2605.26298) (2026) - lightweight process confinement for daemons.
  - [A Hybrid Approach to Semi-automated Rust Verification](https://vtss.doc.ic.ac.uk/publications/Ayoun2025Hybrid.published.pdf) (PLDI 2025) - automated verification as a CI gate.

## Research takeaways

1. `unwrap`/`expect` in daemon code cause real outages (PanicFI fixed 28 merged bugs). Network/config/socket failures must return errors, not panic.
2. Symbolic execution (PanicCheck) shows most panics come from invalid API inputs and `unwrap()` on unexpected values - exactly the pattern in `trios_meshd.rs` config parsing.
3. Secure daemon deployment requires project-relative writable paths, not world-writable `/tmp` (AgentBound/Sandlock).
4. Automated verification as a CI gate is practical for Rust (Creusot/Gillian-Rust), but Wave 5 stays at the clippy/Result-propagation layer.

## Current weak spots

- 11 clippy `expect`/`unwrap` warnings, all in `trios-mesh`:
  - `crypto.rs` - 9 infallible-looking HKDF/ChaCha `expect` calls in production code.
  - `discovery.rs:105` - `ChaCha20-Poly1305` MAC `expect` in production.
  - `router.rs:483` - `expect` inside `#[cfg(test)]` VecTransport.
- `trios_meshd.rs` (unregistered binary) panics on bad config, missing file, bind failure, and parses `/tmp/mesh.drop`.
- `trios-mesh` binary is not registered in `Cargo.toml` (`autobins = false`, no `[[bin]]`), so it is not covered by `cargo clippy --workspace`.
- Workspace lint `expect_used = "warn"` is too weak; it does not prevent new `expect` calls from landing.

## Decomposed plan (P0 -> P5)

### P0 - Harden trios-mesh library production panic surfaces
- Add `MeshError::CryptoInternal` variant for "should never happen" crypto primitive failures.
- Replace 9 `expect` calls in `crypto.rs` with `Result` propagation or infallible helpers that map failure to `MeshError::CryptoInternal`.
  - `combine_dh_shares` -> return `Result<[u8; 32], MeshError>`.
  - `Session::from_shared` -> return `Result<Session, MeshError>`.
  - `Session::ratchet` -> bubble error via `Result<(), MeshError>`.
  - `Session::seal` -> use `?` on ChaCha encrypt.
  - `Session::open` -> replace `try_into().expect()` with safe byte extraction.
  - `derive_cipher` -> return `Result<ChaCha20Poly1305, MeshError>`.
- Cascade `Result` through `Handshake::complete`, `StaticKey::session_with`, `Node::add_session`, `MeshRouter::add_link`, `clade-meshd` handlers, and tests.
- Add helper `hkdf_expand_32` and `hkdf_from_prk_32` to centralize the 32-byte invariant.

### P1 - Harden discovery.rs MAC computation
- Change `Hello::compute_mac` to return `Result<[u8; 16], MeshError>`.
- Propagate through `Hello::authenticated`, `Hello::to_bytes` (no Result needed if authenticated precomputes), and `Hello::verify_mac`.
- Make `Hello::authenticated` return `Result<Self, MeshError>`.

### P2 - Harden trios_meshd binary and register it in Cargo.toml
- Register `src/bin/trios_meshd.rs` as `[[bin]]` with name `trios-meshd` so clippy/build covers it.
- Convert `parse_cfg` to `Result<Cfg, String>` with line-numbered errors.
- Convert `main` to print error and exit 1 on failure instead of panicking.
- Use `std::sync::Mutex::lock().unwrap_or_else(|p| p.into_inner())` for poison recovery in all daemon hot-path mutex locks.
- Move `/tmp/mesh.drop` default to `.trinity/run/mesh.drop`, overridable via `TRIOS_MESH_DROP`.
- Add `trios-config` dependency to the binary target or read `TRIOS_ROOT` directly.

### P3 - Elevate workspace lint and add test exemptions
- Change `expect_used = "warn"` to `"deny"` in `trios/Cargo.toml`.
- Extend `trios-mesh/src/lib.rs` with `#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]`.
- Ensure `cargo clippy --all-targets --all-features` is clean.

### P4 - Add tests and verify
- Add `trios_meshd` config parsing unit tests (valid config, invalid id, missing file, bad listen address).
- Run `./build.sh`, `cargo test --workspace`, `cargo clippy --all-targets --all-features`.
- Run ASCII scan on all changed files.

### P5 - Backlog
- tmp-zero: move remaining test-only `/tmp` usage in `clade-experience`, `clade-launchd`, `clade-audit` to project-relative test dirs or `tempfile` crate.
- seal-automation: add a `clade-seal` ring that runs build + test + clippy + ASCII scan and gates promotion.
- promotion-lock: prevent concurrent `clade-promote` runs.

## This iteration goal

Land P0-P4: make `trios-mesh` clippy-clean for `expect_used`/`unwrap_used` in production, harden the `trios_meshd` binary startup, move `/tmp/mesh.drop` under `.trinity/run/`, and elevate the workspace lint so new panic surfaces cannot be introduced. Keep changes ASCII-only and spec-first.

## [FUTURE OPTIONS]

1. `tmp-zero` - eliminate all remaining `/tmp` usage in tests and tools; add `tempfile` crate policy.
2. `seal-automation` - create `clade-seal` ring that runs build/test/clippy/ASCII gate and only allows promotion when all gates pass.
3. `meshd-e2e` - add a host-sim e2e test that starts `trios-meshd` on 127.0.0.1 ports, injects `/tmp/mesh.drop`/`TRIOS_MESH_DROP`, and verifies reroute in <5s.
