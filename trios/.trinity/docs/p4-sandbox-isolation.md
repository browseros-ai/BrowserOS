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
- **P4.2b — shadow wiring (NEXT)**: in `pipeline.rs`, after the authoritative build,
  if `sandbox_exec_available()`, also run it under `sandbox-exec` (observe-only), log
  `shadow_verdict` to event_log. The authoritative result still decides everything.
  REQUIRES validation: run the full `clade-e2e` suite and confirm verdicts converge to
  `Match` (tune the allowlist for any `TooTight`) before this lands enabled by default.
- **P4.3 — enforce (opt-in)**: gate on `TRIOS_SANDBOX=enforce`; run the build ONLY
  under `sandbox-exec`. Fail closed if `sandbox-exec` is missing. Default stays off.
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
