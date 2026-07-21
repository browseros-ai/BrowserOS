---
name: recursion-guard
domain: Kernel
agent: K
priority: P0
status: draft
---

# Spec: RecursionGuard - Prevent Recursive Self-Launch of trios

## Purpose

Ensure that launching trios - whether via `open trios.app` or the bare `./trios_app` binary - never creates more than one user-facing instance. If another instance is already running, the new launch must activate the existing instance and exit cleanly.

## Invariants

### INV-1: Single User Instance
At most one trios process may present a menu-bar status item and side panel to the user.

### INV-2: Convergence of Launch Paths
Both `.app` bundle launches (`open trios.app`) and bare binary launches (`./trios_app`) must converge to a single running instance.

### INV-3: Existing Instance Activation
When a duplicate launch is detected, the existing instance must be brought to the foreground rather than killed or hidden.

### INV-4: Crash Safety
The locking mechanism must automatically release if the owning process crashes or is killed, so a subsequent manual relaunch succeeds without manual cleanup.

### INV-5: Health-Aware Watchdog
The `clade-monitor` app watchdog must not relaunch trios while a healthy Sovereign `/health` endpoint is already responding, even if `pgrep` temporarily loses the process.

## Interface

### RecursionGuard (Swift)

- `static let shared` - singleton.
- `func ensureSingleInstance() -> Bool` - returns `true` if this process should proceed; returns `false` and activates the existing instance otherwise.
- `func cleanup()` - removes PID file and releases lock on graceful exit.

### Detection Methods (priority order)

1. POSIX advisory file lock on `/tmp/trios_singleton.lock`.
2. `NSRunningApplication.runningApplications(withBundleIdentifier: "com.browseros.trios")`.
3. PID file `/tmp/trios_singleton.pid` with process validation by `comm` + command-line args.

### Main Entry Point

Singleton enforcement must run **before** `NSApplication.shared.run()` so the UI never initializes for a duplicate launch.

### clade-monitor Watchdog

- Detect trios by `pgrep -f` on the bundle path and bare binary name.
- Trust Sovereign `/health` as secondary alive signal.
- Observe a post-relaunch grace period (default 15s) to avoid relaunch storms.

## Tests

### T-1: Double .app Launch
Run `open trios.app` twice within 5 seconds. Exactly one `trios.app/Contents/MacOS/trios` process must remain.

### T-2: Bare Binary + .app
Launch `./trios_app`, then run `open trios.app`. The second launch must not spawn a new UI process.

### T-3: Crash Recovery
Kill the trios process with `kill -9`. Wait 2 seconds. Run `open trios.app` again. A new instance must start successfully.

### T-4: clade-monitor Single Relaunch
Kill trios while clade-monitor is running. Wait 60 seconds. Verify exactly one trios process is relaunched and no duplicates appear within the next 75 seconds.

### T-5: Build Pass
`./build.sh` must compile all Swift sources without errors.

### T-6: Rust Tests
`cargo test --workspace` and `cargo clippy --all-targets --all-features` must pass.

## Constraints

- No hardcoded absolute paths inside the Swift guard; use `ProjectPaths` or standard temp paths.
- ASCII-only source; English identifiers and comments.
- No new shell scripts (L7 UNITY).

## Change Flow

Any change to this spec or `BR-OUTPUT/RecursionGuard.swift` must pass:

1. Spec update (this file).
2. t27-creator implementation.
3. t27-verifier L1-L7 verdict.
4. `/t27-tri-pipeline seal`.
5. Land with `Closes #T27-EPIC-001`.
6. `/t27-experience-save`.
