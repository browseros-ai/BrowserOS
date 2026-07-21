# Wave Loop 004 - trios portable root resolution and mesh daemon fault tolerance

## Sources
- Wave 4 weak-spot audit of all Rust rings and BR-OUTPUT/*.swift.
- Literature scan:
  - [Hopter: a Safe, Robust, and Responsive Embedded Operating System](https://www.yecl.org/publications/ma2025mobisys.pdf) (MobiSys 2025) - panic recovery via stack unwinding and task restart.
  - [PanicFI: An Infrastructure for Fixing Panic Bugs in Real-World Rust Programs](https://www.arxiv.org/pdf/2408.03262) (2024) - dataset of Rust panic bugs and automated repair.
  - [Static Analysis of Rust Error Propagation](https://essay.utwente.nl/fileshare/file/100758/kas_BA_EEMCS.pdf) (2024) - error-propagation graphs and `unwrap`/`expect` interrupt flow.
  - [Cargo `build.build-dir` templating](https://github.com/rust-lang/cargo/pull/15236) (Rust 1.87.0) - portable project-root-relative build directories.
  - [Securely deploying AI agents - Claude Code Docs](https://code.claude.com/docs/en/agent-sdk/secure-deployment) - tmpfs `/tmp`, read-only root, capability dropping.
  - [Anthropic sandbox-runtime](https://github.com/anthropic-experimental/sandbox-runtime) - OS-level sandbox with allow-only writable paths.
  - [AgentBound: Securing Execution Boundaries of AI Agents](https://www.lucadigrazia.com/papers/fse2026.pdf) (FSE 2026) - capability manifests and least-privilege sandboxing.
  - [MCPGuard-Dynamic](https://github.com/facebook/mcpguard-dynamic) (Meta 2026) - eBPF syscall sandbox for MCP servers.

## Key research takeaways
1. Hardcoded absolute paths are a deployment blocker and leak developer identity; environment-only root resolution or executable-relative derivation is the portable pattern (Cargo `build-dir` templating uses `{workspace-root}` exactly this way).
2. `unwrap`/`expect` in daemon/network code are the dominant panic surface; PanicFI shows they cause real-world outages, and Hopter shows recoverable panic handling is feasible.
3. `/tmp` for runtime state is unsafe on multi-user machines and breaks CI reproducibility; secure deployment guides recommend project-relative scratch dirs or `tmpfs` with `noexec,nosuid,size=...`.
4. Capability manifests + sandbox enforcement (AgentBound, MCPGuard-Dynamic) justify continuing the shell-free/allowlist work into runtime isolation, but Wave 4 stays at the path/hardening layer to keep the scope landable.

## Decomposed plan (P0 -> P5)

### P0 - Remove hardcoded `/Users/playra/...` TRIOS_ROOT fallbacks
- [x] `rings/RUST-03/clade-rollback/src/main.rs:7` - replace fallback with `current_dir()` or env-only fail.
- [x] `rings/RUST-04/clade-improve/src/variant.rs:33` - replace fallback with `current_dir()` or env-only fail.
- [x] `rings/RUST-06/clade-dashboard/src/main.rs:5` - replace fallback with `current_dir()` or env-only fail.
- [x] `rings/RUST-07/clade-experience/src/main.rs:5` - replace fallback with `current_dir()` or env-only fail.
- [x] `rings/RUST-08/clade-promote/src/main.rs:10` - replace fallback with `current_dir()` or env-only fail.
- [x] `rings/RUST-09/clade-launchd/src/main.rs:4` - replace fallback with `current_dir()` or env-only fail.
- [x] `rings/RUST-10/clade-worktree/src/main.rs:4` - replace const default with `current_dir()` or env-only fail.
- [x] `rings/RUST-12/clade-audit/src/main.rs:8` - replace fallback with `current_dir()` or env-only fail.
- [x] `rings/RUST-14/clade-tablecloth/src/main.rs:52` - replace fallback with `current_dir()` or env-only fail.
- [x] `BR-OUTPUT/ProjectPaths.swift:17` - replace fallback with `current_dir()` / bundle path resolution.
- [x] Centralize root resolution helper in `rings/RUST-00/trios-config/src/lib.rs` and make all rings use it.

### P1 - Harden RUST-13 mesh production panic surfaces
- [ ] `trios-mesh/src/bin/trios_meshd.rs` - replace config/lock/socket `expect`/`unwrap` with `Result` and graceful exit/error logging.
- [ ] `trios-mesh/src/crypto.rs` - replace infallible HKDF/ChaCha `expect` with `Result`/`?` propagation or explicit error enums.
- [ ] `trios-mesh/src/discovery.rs:105` - replace MAC `expect` with fallible Result.
- [ ] Add workspace lint exemption `#![cfg_attr(test, allow(clippy::unwrap_used))]` if needed, but keep production `expect_used = deny`.

### P2 - Move runtime `/tmp` state into TRIOS_ROOT
- [x] `clade-e2e` - move log/screenshot dir from `/tmp/trios_e2e` and `/tmp/trios_screenshot.png` to `.trinity/e2e/`.
- [x] `clade-improve` - move `/tmp/clade-rollback` and `/tmp/clade-dev` to `.trinity/rollback/` and `.trinity/dev/`.
- [ ] `trios-mesh/src/bin/trios_meshd.rs` - move `/tmp/mesh.drop` to `.trinity/run/mesh.drop`.
- [x] Ensure directories are created with restricted permissions (`0o700`) where applicable.

### P3 - Gitignore runtime artifacts
- [x] Update `trios/.gitignore` to ignore `.trinity/logs/`, `.trinity/e2e/`, `.trinity/dev/`, `.trinity/rollback/`, `.trinity/run/`, `.trinity/claims/`, `.trinity/events/`, `.trinity/queue/`, `.trinity/*.pid`, `.trinity/wave-loop-*.md`.
- [x] Untrack `.trinity/events/akashic-log.jsonl` if already tracked.

### P4/P5 - Backlog
- [ ] Add CI non-ASCII gate and `registry.json` sync validation.
- [ ] Promotion lock between `clade-promote` and `clade-monitor`.
- [ ] Mesh UI integration and e2e seal when BrowserOS Agent Server is up.

## This iteration goal
Land focused P0/P1 items plus a P2 starter (e2e /tmp migration) without touching the pending mesh UI files. Keep all changes ASCII-only and spec-first.

## Wave 004 Closeout Report

Status: LANDED in commit `HEAD` on branch `feat/zai-provider`.

### What shipped
- P0 hardcoded-path removal: every Rust ring and Swift `ProjectPaths` now resolves TRIOS_ROOT via `trios-config::project_dir()` (env override -> current_dir -> clear fatal error). No `/Users/playra/...` fallbacks remain.
- P2 runtime-state migration: `clade-e2e` logs/screenshots moved to `.trinity/e2e/`; `clade-improve` rollback and dev sandboxes moved to `.trinity/rollback/` and `.trinity/dev/` with `0o700` creation.
- P3 gitignore: `.trinity/logs/`, `e2e/`, `dev/`, `rollback/`, `run/`, `events/`, `claims/`, `queue/`, `*.pid` are now ignored; `.trinity/events/akashic-log.jsonl` was untracked.
- Spec-first: added `.trinity/specs/portable-root-resolution.md`.
- Skills saved: updated `.claude/skills/ascii-lint/SKILL.md` and created `.claude/skills/portable-paths/SKILL.md`.
- ASCII purity: all changed Rust, Swift, Cargo.toml descriptions, specs, and skills are ASCII-only.

### Verification
- `./build.sh` -> OK.
- `cargo test --workspace` -> all 332 tests passed (9 trios-config, 15 clade-audit, 7 clade-build, 5 clade-e2e, 12 clade-dashboard, 9 clade-experience, 32 clade-improve lib, 8 clade-improve bin, 13 clade-launchd, 2 clade-meshd, 38 clade-monitor, 14 clade-promote, 12 clade-rollback, 39 clade-tablecloth, 7 clade-worktree, 101 trios-mesh).
- `cargo clippy --all-targets --all-features` -> clean except for 11 pre-existing `trios-mesh` `expect`/`unwrap` warnings retained as P1 backlog.
- ASCII scan on all changed files -> zero violations.

### Remaining backlog (not in scope)
- P1 `trios-mesh` production panic hardening (crypto.rs, discovery.rs, trios_meshd.rs `expect`/`unwrap` propagation).
- P2 finish `/tmp/mesh.drop` migration to `.trinity/run/mesh.drop`.
- P4/P5 CI ASCII gate, registry.json sync, promotion lock, mesh UI e2e seal.

## [FUTURE OPTIONS - next Wave loop]
1) `mesh-panic-hardening` - convert trios-mesh production `expect`/`unwrap` to `Result`/`?`, add error enums, and write meshd unit tests. Goal: eliminate the 11 clippy warnings and make the daemon crash-proof on bad config/lock/socket/cipher inputs.
2) `tmp-zero` - move `/tmp/mesh.drop` and any remaining `/tmp/clade-*` paths under `.trinity/run/` with automatic cleanup policy and rollout verification. Goal: zero runtime state outside TRIOS_ROOT.
3) `seal-automation` - add a CI ASCII-only gate, `.gitignore` drift test, and a `clade-seal` ring that runs `./build.sh` + `cargo test --workspace` + `cargo clippy` and only allows promotion when all gates pass. Goal: make Wave closeout reproducible in CI.
