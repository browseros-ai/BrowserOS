# P4 — Real OS Isolation for the Self-Improvement Sandbox

**Status:** component built + unit-tested, NOT yet enforced. Tracking the TODO at
`rings/RUST-04/clade-improve/src/sandbox.rs`.

## Problem
`SandboxedDev` copies a secret-redacted source tree to `/tmp/clade-dev/<ticket>` and
runs the variant's build/test there with **no OS-level isolation**. A malicious or
buggy self-improvement variant can read `~/.ssh`, exfiltrate over the network, or
write outside its dev root. The secret-filter (`copy_tree_filtered`) is best-effort
content redaction, not a containment boundary.

## Approach (research-grounded)
macOS **Seatbelt** via `sandbox-exec -f <profile.sb> <program>`:
- Deny-by-default; allowlist only what toolchains need.
- Profile is **irreversible** and **inherited by every child** once applied — ideal
  for untrusted code (the variant cannot drop it).
- Explicitly deny credential stores (`~/.ssh`, Keychains) at the kernel level.
- Network restricted to localhost (Seatbelt's net rules are coarse; a localhost
  proxy is the heavier option if egress control must be exact).

### Caveats (why we don't enforce blind)
1. **Silent failures** — Seatbelt drops disallowed operations without an error, so an
   over-tight profile makes builds fail mysteriously. Must be validated against real
   `swiftc`/`cargo` runs.
2. **`sandbox-exec` is deprecated** (still functional on macOS 14+, emits a warning).
   No supported replacement API exists yet (Apple containerization issue #737). Keep
   it behind a flag so we can swap the mechanism later.

## Decomposed plan
- **P4.1 — profile generator (DONE)**: `generate_seatbelt_profile(dev_root, home)` +
  `sandbox_exec_argv(profile, program, args)`. Pure, 5 unit tests. Unwired.
- **P4.2a — shadow helpers (DONE)**: `write_seatbelt_profile(dev_root, home)` (writes
  `<dev_root>/.clade-sandbox.sb`), `sandbox_exec_available()` (fail-safe no-op probe),
  and `shadow_verdict(real_ok, sandboxed_ok) -> {Match, TooTight, Inconsistent}`. Pure
  + IO, 5 unit tests. Still unwired.
- **P4.2b — shadow wiring (DONE, default-off)**: `pipeline.rs::shadow_check_build`
  runs after the authoritative build; gated on `TRIOS_SANDBOX=shadow` (pure
  `shadow_mode_enabled`, unit-tested OFF by default). Observe-only: re-runs the build
  under `sandbox-exec`, logs `ShadowVerdict` via tracing. NEVER touches `results`.
  Smoke test of the profile (`sandbox-exec -f <profile> ...`): basic exec + `cargo
  --version` pass (exit 0); reading `~/.ssh` is blocked (exit 134 / SIGABRT — the
  sandbox kills on violation rather than returning EPERM). Implication: a build that
  incidentally touches a denied path is hard-killed, so the allowlist MUST be tuned in
  shadow mode (collect `TooTight` verdicts) before P4.3.
- **P4.2c — profile tuning (IN PROGRESS)**: validated against a controlled cargo
  harness (`/tmp` throwaway crate, real `cargo build` vs `sandbox-exec -f <profile>`):
  - `(allow default)` profile builds fine -> `sandbox-exec` + `cargo build` are
    compatible; failures are profile-restriction, not a wrapper incompatibility.
  - Security denies CONFIRMED: `~/.ssh`, `~/.cargo` (when not allowlisted), Keychains
    all blocked (SIGABRT/134) — the OS kills on violation, not EPERM.
  - Zero-dep builds reach `Match`; dependency builds were instantly `TooTight` because
    cargo/rustc live under `~/.cargo/bin` and read `~/.cargo/registry` + `~/.rustup`.
  - **MATCH ACHIEVED.** A real dependency build (`libc`) now compiles + finishes
    under the deny-default profile (exit 0) while `~/.ssh`/Keychains stay denied.
    The complete recipe, found by bisection (rustc startup was the blocker):
    1. **Ancestor-dir literals** `(literal "/")`, `(literal "/Users")`, `(literal
       "$HOME")` — opening `~/.cargo`/`~/.rustup` requires read access to each
       ancestor directory ENTRY (not contents). Missing -> rustc SIGABRT at startup.
    2. **`/private/etc`** — the rustup `rustc` shim links OpenSSL and reads
       `/private/etc/ssl/openssl.cnf` ("Auto configuration failed" otherwise).
    3. **`/private/tmp`** read (dev roots live under `/tmp` -> `/private/tmp`).
    4. **cwd = dev root** — rustc calls `getcwd()`; the wrapped command must
       `current_dir(&dev.root)` (done in `pipeline.rs::shadow_check_build`).
  - Validated against the EXACT generator output, not a hand-written profile.
  - Minor accepted leak: the literals expose directory *listings* of `/`, `/Users`,
    `$HOME` (names only, never contents; credentials still denied).
- **P4.2d — verdicts to event_log (DONE)**: `shadow_check_build` logs a
  `sandbox_shadow_verdict` event (details `<tag>_real<bool>_sandboxed<bool>`) via
  `trios-config::log_event`. `clade-dashboard` counts `sandbox_too_tight` so profile
  drift is visible in the metrics/ADLC feedback loop.
- **P4.3 — enforce (DONE, opt-in)**: `sandbox_mode(env) -> {Off, Shadow, Enforce}`
  (pure, unit-tested, default `Off`). `pipeline.rs::build_command` wraps the variant's
  `cargo test`/`cargo build` in `sandbox-exec -f <profile>` (cwd = dev root) when
  `TRIOS_SANDBOX=enforce`. FAIL-CLOSED: if `sandbox-exec`/`HOME`/profile is
  unavailable, `build_command` returns `None` and the step is recorded as FAILED.
  Validated: the enforce-mode invocation (generated profile + cwd=dev root) compiles
  a real dependency build (exit 0). Default stays `Off`; `swiftc` build still runs bare
  (follow-up). **Security win: untrusted self-improvement builds are now isolatable.**
  - **Validation coverage**: `libc` (build script) and a `serde`/`serde_derive`
    crate (proc-macro dylib chain: syn/quote/proc-macro2, 12 crates) both compile
    under the deny-default profile, exit 0 — the two hardest general mechanisms work.
    STILL UNTESTED: crates that link system libraries via build scripts (e.g.
    `openssl-sys`, `*-sys` with pkg-config) — the full trios workspace may surface
    new `TooTight` paths there; shadow mode will catch them before enforce relies on it.
- **P4.4 — network proxy (optional)**: localhost proxy outside the sandbox if exact
  egress allowlisting (not just localhost) is required.

## Integration point
`rings/RUST-04/clade-improve/src/pipeline.rs` — the stage that runs the variant's
`cargo`/`swiftc`. Replace `Command::new(program)` with, under shadow/enforce,
`Command::new("sandbox-exec").args(sandbox_exec_argv(&profile, program, &args))`.

## Verification gates per stage
`cargo test -p clade-improve`, `cargo clippy --workspace --all-targets` (0 warnings),
and — critically for P4.2/P4.3 — `cargo run --bin clade-e2e` must stay green under the
profile before advancing. SOUL Art. II: no stage lands without its build/test gate.

Sources: Apple Seatbelt / `sandbox-exec`; gemini-cli macOS seatbelt profiles;
agent-seatbelt-sandbox (data-egress blocking); Apple containerization issue #737.
