:name: mesh-panic-hardening
:description: Harden trios-mesh daemon panic surfaces by replacing expect/unwrap with Result propagation, elevating workspace clippy lint, and moving /tmp/mesh.drop under .trinity/run/.
:owner: claude
:status: sealed
:wave: 005

# Spec - trios-mesh panic hardening and runtime-state isolation

## Goal

Make `trios-mesh` production code clippy-clean for `expect_used`/`unwrap_used` at `deny` level, harden the `trios-meshd` binary startup so it returns errors instead of panicking, and move the mesh drop file from world-writable `/tmp` to a project-relative runtime directory.

## Background

Weak-spot audit after Wave 004 found 11 `expect`/`unwrap` warnings in `trios-mesh`, concentrated in crypto primitive calls and the unregistered `trios-meshd` binary. Research on real-world Rust outages (PanicFI, PanicCheck) shows `unwrap`/`expect` in daemon code are a dominant outage surface. trios Invariant Law L1 TRACEABILITY requires every change to be spec-first; L3 PURITY requires ASCII-only source/specs/agents/skills.

## Changes

### trios/Cargo.toml

- Elevated workspace clippy lints:
  - `unwrap_used = "deny"`
  - `expect_used = "deny"`

### rings/RUST-13/trios-mesh/src/lib.rs

- Extended test exemption: `#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]`.

### rings/RUST-13/trios-mesh/src/crypto.rs

- Added `MeshError::CryptoInternal` variant for "should never happen" crypto primitive failures.
- Added helpers:
  - `hkdf_expand_32` - fallible HKDF-Expand to 32 bytes.
  - `hkdf_from_prk` - fallible HKDF construction from a PRK.
  - `read_u32_be` / `read_u64_be` - safe big-endian byte readers.
- Converted public functions to return `Result`:
  - `combine_dh_shares`
  - `NoiseXX::complete_initiator` / `complete_responder`
  - `Session::from_shared`
  - `Session::ratchet`
  - `Handshake::complete`
  - `StaticKey::session_with`
- `Session::seal` uses `?` on ChaCha20-Poly1305 encrypt.
- `Session::open` uses safe byte readers instead of `try_into().expect()`.

### rings/RUST-13/trios-mesh/src/discovery.rs

- `Hello::compute_mac` returns `Result<[u8; 16], MeshError>`.
- `Hello::authenticated` returns `Result<Self, MeshError>`.
- `Hello::verify_mac` returns `false` on internal MAC failure instead of panicking.

### rings/RUST-13/trios-mesh/src/router.rs

- Test-only `VecTransport::take()` returns `Option` and recovers from mutex poison via `lock().unwrap_or_else(|p| p.into_inner())`; test callers use `.expect()`.

### rings/RUST-13/trios-mesh/src/bin/trios_meshd.rs

- Rewritten startup to return `Result` and exit 1 on failure.
- `parse_cfg` returns `Result<Cfg, String>` with line-numbered errors.
- Default mesh drop path moved from `/tmp/mesh.drop` to `.trinity/run/mesh.drop`, overridable via `TRIOS_MESH_DROP`.
- Mutex poison recovery in daemon hot-path locks.
- NOTE: `trios-meshd` binary is intentionally left unregistered in `Cargo.toml` because the current binary targets an older `trios-mesh` API (pre-`Delivery` enum, pre-struct `MeshRouter`). A future revival pass is needed before it can be registered and clippied as part of the workspace.

### rings/RUST-13/clade-meshd/src/main.rs

- `hello_handler` and `seed_peer_handler` handle `Hello::authenticated` and `StaticKey::session_with` errors and return HTTP 500 instead of panicking.

### ASCII cleanup

- Bulk cleaned touched files to ASCII-only: `crypto.rs`, `discovery.rs`, `router.rs`, `bin/trios_meshd.rs`.
- Added new mappings to `trios/.claude/skills/ascii-lint/SKILL.md`:
  - U+2190 leftwards arrow -> `<-`
  - U+21D4 left right double arrow -> `<=>`
  - U+00A7 section sign -> `section`

### trios/.claude/skills/panic-hardening/SKILL.md

- New reusable skill documenting the conversion patterns used in this wave.

## Verification

- `./build.sh` passes.
- `cargo test -p trios-mesh --all-features` passes: 101 tests.
- `cargo test -p clade-meshd --all-features` passes: 2 tests.
- `cargo clippy -p trios-mesh -p clade-meshd --all-targets --all-features` is clean.
- `grep -RIn '[^\x00-\x7F]'` on all changed files returns zero violations.

## Non-goals / Backlog

- `tmp-zero`: eliminate remaining `/tmp` usage in other rings (`clade-experience`, `clade-launchd`, `clade-audit`).
- `seal-automation`: add `clade-seal` ring that gates promotion on build/test/clippy/ASCII.
- `promotion-lock`: prevent concurrent `clade-promote` runs.
- `meshd-revival`: repair `trios_meshd.rs` API drift and register it as a workspace `[[bin]]`.

## Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| Cascading `Result` changes break call sites in other rings | Limited to `trios-mesh` and `clade-meshd`; tests cover both. |
| `CryptoInternal` masks real crypto failures | Logged as auth-equivalent; rate-limited by existing frame counters. |
| Mutex poison recovery hides bugs | Used only in daemon hot paths; tests still fail fast on poison. |
| `/tmp/mesh.drop` consumers break | `TRIOS_MESH_DROP` env override preserves backward compatibility. |

## Related

- `.claude/plans/trios-wave-005-mesh-panic-hardening.md`
- `.trinity/wave-loop-005.md`
- `trios/.claude/skills/panic-hardening/SKILL.md`
- `trios/.claude/skills/ascii-lint/SKILL.md`
