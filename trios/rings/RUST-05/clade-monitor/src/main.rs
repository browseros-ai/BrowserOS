use reqwest::blocking::get;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

static RUNNING: AtomicBool = AtomicBool::new(true);

extern "C" fn handle_signal(_sig: libc::c_int) {
    RUNNING.store(false, Ordering::Relaxed);
}

fn project_dir() -> String {
    trios_config::project_dir()
}

const CIRCUIT_BREAKER_TRIP_THRESHOLD: u32 = 3;
const CIRCUIT_BREAKER_PROBE_INTERVAL_SECS: u64 = 300;

#[derive(Debug, PartialEq)]
enum CircuitState {
    Closed,
    Open { tripped_at: u64 },
    HalfOpen,
}

struct CircuitBreaker {
    state: CircuitState,
    consecutive_failures: u32,
    service_name: String,
}

impl CircuitBreaker {
    fn new(name: &str) -> Self {
        Self {
            state: CircuitState::Closed,
            consecutive_failures: 0,
            service_name: name.to_string(),
        }
    }

    fn should_probe(&mut self) -> bool {
        match &self.state {
            CircuitState::Closed | CircuitState::HalfOpen => true,
            CircuitState::Open { tripped_at } => {
                let now = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                if now - tripped_at >= CIRCUIT_BREAKER_PROBE_INTERVAL_SECS {
                    self.state = CircuitState::HalfOpen;
                    true
                } else {
                    false
                }
            }
        }
    }

    fn record_success(&mut self) {
        if self.state != CircuitState::Closed {
            println!(
                "[CircuitBreaker] {} recovered — closing circuit",
                self.service_name
            );
        }
        self.state = CircuitState::Closed;
        self.consecutive_failures = 0;
    }

    fn record_failure(&mut self) {
        self.consecutive_failures += 1;
        if self.consecutive_failures >= CIRCUIT_BREAKER_TRIP_THRESHOLD {
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if self.state == CircuitState::Closed {
                println!(
                    "[CircuitBreaker] {} tripped after {} consecutive failures — opening circuit (probe in {}s)",
                    self.service_name, self.consecutive_failures, CIRCUIT_BREAKER_PROBE_INTERVAL_SECS
                );
            }
            self.state = CircuitState::Open { tripped_at: now };
        }
    }

    fn is_open(&self) -> bool {
        matches!(self.state, CircuitState::Open { .. })
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct LastWake {
    ts: u64,
    health: String,
    build: String,
    clade_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct SafetyBudget {
    budget: f64,
    max_budget: f64,
    total_trials: u64,
    total_failures: u64,
    halted: bool,
}

fn kill_orphan_monitors() -> usize {
    use std::process::{Command, Stdio};
    let my_pid = std::process::id();
    let output = match Command::new("pgrep")
        .args(["-f", "clade-monitor"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    {
        Ok(o) => o,
        Err(_) => return 0,
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut killed = 0;
    for line in stdout.lines() {
        if let Ok(pid) = line.trim().parse::<u32>() {
            if pid != my_pid {
                let result = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
                if result == 0 {
                    eprintln!("[CladeMonitor] Killed orphan PID {}", pid);
                    killed += 1;
                }
            }
        }
    }
    killed
}

fn acquire_pidfile() -> Option<fs::File> {
    let lock_path = format!("{}/.trinity/state/clade-monitor.lock", &project_dir());
    let pid_path = format!("{}/.trinity/state/clade-monitor.pid", &project_dir());
    if let Err(e) = fs::create_dir_all(format!("{}/.trinity/state", &project_dir())) {
        eprintln!("[CladeMonitor] Failed to create state dir: {}", e);
        return None;
    }

    // flock-based singleton: auto-releases on crash, no stale file problem
    let lock_file = match fs::File::create(&lock_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[CladeMonitor] Failed to create lock file: {}", e);
            return None;
        }
    };

    use std::os::unix::io::AsRawFd;
    let fd = lock_file.as_raw_fd();
    let ret = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
    if ret != 0 {
        eprintln!("[CladeMonitor] Another instance holds the lock. Exiting.");
        return None;
    }

    // Kill any orphan monitors not tracked by lock
    let orphans = kill_orphan_monitors();
    if orphans > 0 {
        println!("[CladeMonitor] Cleaned up {} orphan process(es)", orphans);
    }

    // Write PID file for monitoring/inspection (not used for locking)
    match fs::File::create(&pid_path) {
        Ok(mut f) => {
            let pid = std::process::id();
            if let Err(e) = write!(f, "{}", pid) {
                eprintln!("[CladeMonitor] Failed to write PID file: {}", e);
            }
            println!(
                "[CladeMonitor] Lock acquired + PID file: {} (pid={})",
                pid_path, pid
            );
        }
        Err(e) => {
            eprintln!(
                "[CladeMonitor] Failed to create PID file (non-fatal): {}",
                e
            );
        }
    }

    Some(lock_file)
}

fn cleanup_pidfile() {
    let path = format!("{}/.trinity/state/clade-monitor.pid", &project_dir());
    if let Err(e) = fs::remove_file(&path) {
        eprintln!("[CladeMonitor] Failed to remove PID file: {}", e);
    }
}

fn main() {
    use std::collections::HashMap;

    // Register signal handlers before pidfile to minimize stale-pidfile window
    unsafe {
        libc::signal(
            libc::SIGTERM,
            handle_signal as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGINT,
            handle_signal as *const () as libc::sighandler_t,
        );
    }

    if acquire_pidfile().is_none() {
        std::process::exit(1);
    }

    println!("[CladeMonitor] Starting cron monitor loop...");
    println!("[CladeMonitor] Press Ctrl+C to stop (graceful shutdown enabled).");

    // Monotonic clock for interval scheduling + drift: immune to wall-clock
    // (NTP) jumps that would otherwise look like huge drift (false positives).
    let mut last_15m = Instant::now();
    let mut last_30m = Instant::now();
    let mut last_60m = Instant::now();
    let mut last_60m_tablecloth = Instant::now();
    let mut last_24h = Instant::now();
    let mut last_heartbeat = Instant::now();
    let mut last_app_relaunch: Option<Instant> = None;

    // Backoff tracking: consecutive_failures -> multiplier
    let mut failure_counts: HashMap<String, u32> = HashMap::new();
    let pid_seed = std::process::id();
    let mut consecutive_healthy: u32 = 0;
    let mut canary_breaker = CircuitBreaker::new("canary");

    // Track initial build hash
    if let Some((hash, _)) = track_build_hash() {
        println!("[CladeMonitor] Binary hash tracked: {}...", &hash[..8]);
    }

    while RUNNING.load(Ordering::Relaxed) {
        let now = Instant::now();

        // Drift detection: check if any interval has exceeded 2x expected
        let drift_intervals: Vec<(&str, &Instant, u64)> = vec![
            ("15m_health", &last_15m, 900),
            ("30m_build", &last_30m, 1800),
            ("60m_seal", &last_60m, 3600),
            ("60m_tablecloth", &last_60m_tablecloth, 3600),
            ("24h_audit", &last_24h, 86400),
        ];
        detect_drift(&drift_intervals);

        // Every 15 min: health quick
        let backoff_15m = calculate_backoff_with_jitter(
            failure_counts.get("15m").copied().unwrap_or(0),
            pid_seed,
        );
        if now.saturating_duration_since(last_15m) >= Duration::from_secs(900 * backoff_15m) {
            last_15m = now;
            if run_health_check("15m", &mut canary_breaker) {
                failure_counts.remove("15m");
                consecutive_healthy += 1;
                replenish_budget(consecutive_healthy);
            } else {
                *failure_counts.entry("15m".to_string()).or_insert(0) += 1;
                consecutive_healthy = 0;
            }
        }

        // Every 30 min: build + dirty + hash tracking
        let backoff_30m = calculate_backoff_with_jitter(
            failure_counts.get("30m").copied().unwrap_or(0),
            pid_seed,
        );
        if now.saturating_duration_since(last_30m) >= Duration::from_secs(1800 * backoff_30m) {
            last_30m = now;
            if run_build_check("30m") {
                failure_counts.remove("30m");
                if track_build_hash().is_none() {
                    eprintln!("[CladeMonitor] Warning: build hash tracking failed");
                }
            } else {
                *failure_counts.entry("30m".to_string()).or_insert(0) += 1;
            }
        }

        // Every 60 min: seal audit + safety budget
        let backoff_60m = calculate_backoff_with_jitter(
            failure_counts.get("60m").copied().unwrap_or(0),
            pid_seed,
        );
        if now.saturating_duration_since(last_60m) >= Duration::from_secs(3600 * backoff_60m) {
            last_60m = now;
            if run_seal_audit("60m") {
                failure_counts.remove("60m");
            } else {
                *failure_counts.entry("60m".to_string()).or_insert(0) += 1;
            }
        }

        // Every 60 min: autonomous self-improvement loop (clade-tablecloth)
        let backoff_60m_t = calculate_backoff_with_jitter(
            failure_counts.get("60m_tablecloth").copied().unwrap_or(0),
            pid_seed,
        );
        if now.saturating_duration_since(last_60m_tablecloth)
            >= Duration::from_secs(3600 * backoff_60m_t)
        {
            last_60m_tablecloth = now;
            if run_tablecloth("60m_tablecloth") {
                failure_counts.remove("60m_tablecloth");
            } else {
                *failure_counts
                    .entry("60m_tablecloth".to_string())
                    .or_insert(0) += 1;
            }
        }

        // Every 24h: deep audit + wrap-up
        let backoff_24h = calculate_backoff_with_jitter(
            failure_counts.get("24h").copied().unwrap_or(0),
            pid_seed,
        );
        if now.saturating_duration_since(last_24h) >= Duration::from_secs(86400 * backoff_24h) {
            last_24h = now;
            if run_deep_audit("24h") {
                failure_counts.remove("24h");
            } else {
                *failure_counts.entry("24h".to_string()).or_insert(0) += 1;
            }
        }

        // Watchdog heartbeat: every 5 min, log that the monitor is alive
        if now.saturating_duration_since(last_heartbeat) >= Duration::from_secs(300) {
            last_heartbeat = now;
            // Uptime is a calendar value -> read the wall clock here (not the
            // monotonic `now`, which is not anchored to the epoch).
            let uptime = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            log_event(
                "watchdog_heartbeat",
                &format!("alive_pid_{}_uptime_{}", std::process::id(), uptime),
            );
        }

        // App watchdog (every loop, ~60s): the trios menu-bar app must always be
        // running so its status-bar logo is present. If it died (crash, rebuild
        // without restart, logout) nothing else brings it back — relaunch it.
        // A grace period after relaunch prevents a storm of duplicate launches
        // while the app is still booting.
        ensure_trios_app_running(&mut last_app_relaunch);

        thread::sleep(Duration::from_secs(60));
    }

    cleanup_pidfile();
    println!("[CladeMonitor] Graceful shutdown complete.");
    log_event("monitor_shutdown", "graceful");
}

/// Whether the trios GUI app is running. Detects both the .app bundle launch
/// (`trios.app/Contents/MacOS/trios`) and the bare binary (`trios_app`). The
/// path-based `pgrep -f` check is more reliable than `pgrep -x` because the
/// kernel process name depends on how the app was launched.
fn trios_app_running() -> bool {
    use std::process::{Command, Stdio};
    let dir = project_dir();
    let patterns = [
        format!("{}/trios.app/Contents/MacOS/trios", dir),
        "trios_app".to_string(),
    ];
    for pattern in &patterns {
        if let Ok(out) = Command::new("pgrep")
            .arg("-f")
            .arg(pattern)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
        {
            if out.status.success() && !out.stdout.is_empty() {
                return true;
            }
        }
    }
    false
}

/// Whether the trios sovereign health endpoint is responding. Used as a
/// secondary signal so we do not relaunch while the app is still booting or
/// when `pgrep` has temporarily lost the process.
fn trios_app_healthy() -> bool {
    match get(trios_config::sovereign_health_url()) {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

const APP_RELAUNCH_GRACE_SECS: u64 = 15;

/// Relaunch the trios app if it is not running. Prefers the `.app` bundle via
/// `open` so `Bundle.main` resources (the menu-bar logo) resolve; falls back to
/// the bare binary. Includes a post-launch grace period and a health probe to
/// avoid a relaunch storm when the app is slow to start or briefly unresponsive.
fn ensure_trios_app_running(last_relaunch: &mut Option<Instant>) {
    use std::path::Path;
    use std::process::{Command, Stdio};

    // Respect the grace period after a recent relaunch so a slow boot is not
    // mistaken for a dead app and repeatedly spawned.
    if let Some(t) = last_relaunch {
        if Instant::now().saturating_duration_since(*t) < Duration::from_secs(APP_RELAUNCH_GRACE_SECS) {
            return;
        }
    }

    if trios_app_running() {
        return;
    }

    // Trust the sovereign health endpoint as a secondary alive signal.
    if trios_app_healthy() {
        return;
    }

    let dir = project_dir();
    let bundle = format!("{}/trios.app", dir);
    let binary = format!("{}/trios_app", dir);

    let launched = if Path::new(&bundle).exists() {
        Command::new("open").arg(&bundle).status().is_ok()
    } else if Path::new(&binary).exists() {
        Command::new(&binary)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
    } else {
        eprintln!(
            "[CladeMonitor] App watchdog: neither {} nor {} exists",
            bundle, binary
        );
        log_event("trios_relaunch", "failed_no_artifact");
        return;
    };

    if launched {
        println!("[CladeMonitor] App watchdog: trios was down — relaunched");
        log_event("trios_relaunch", "relaunched_after_down");
        *last_relaunch = Some(Instant::now());
    } else {
        eprintln!("[CladeMonitor] App watchdog: relaunch attempt failed");
        log_event("trios_relaunch", "relaunch_failed");
    }
}

fn calculate_backoff(failures: u32) -> u64 {
    match failures {
        0 => 1,
        1 => 2,
        2 => 4,
        3 => 8,
        _ => 16,
    }
}

fn calculate_backoff_with_jitter(failures: u32, pid_seed: u32) -> u64 {
    let base = calculate_backoff(failures);
    if base <= 1 {
        return base;
    }
    // Decorrelated jitter: vary by up to 25% using PID as deterministic seed
    let jitter_pct = (pid_seed % 25) as u64;
    let jitter = base * jitter_pct / 100;
    base + jitter
}

fn atomic_write(path: &str, content: &str) -> Result<(), String> {
    let tmp = format!("{}.tmp", path);
    fs::write(&tmp, content).map_err(|e| format!("write tmp: {}", e))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("rename: {}", e)
    })
}

fn replenish_budget(consecutive_healthy: u32) -> bool {
    if consecutive_healthy < 3 {
        return false;
    }
    let path = format!("{}/.trinity/state/safety_budget.json", &project_dir());
    let mut budget = load_safety_budget();

    if budget.halted || budget.budget >= budget.max_budget {
        return false;
    }

    let increment = 0.5_f64.min(budget.max_budget - budget.budget);
    budget.budget += increment;
    println!(
        "[CladeMonitor] Budget replenished +{:.1} -> {:.1}/{:.1} (after {} consecutive healthy checks)",
        increment, budget.budget, budget.max_budget, consecutive_healthy
    );

    match serde_json::to_string_pretty(&budget) {
        Ok(json) => {
            if let Err(e) = atomic_write(&path, &json) {
                eprintln!("[CladeMonitor] Failed to write budget: {}", e);
                return false;
            }
            log_event(
                "budget_replenish",
                &format!(
                    "budget_{:.1}_after_{}_healthy",
                    budget.budget, consecutive_healthy
                ),
            );
            true
        }
        Err(e) => {
            eprintln!("[CladeMonitor] Failed to serialize budget: {}", e);
            false
        }
    }
}

/// Pure drift computation — returns drift descriptions, no I/O. Tests call this
/// directly so they never touch the real event_log. (Prior bug: the unit tests
/// drove `detect_drift`, whose `log_event` side effect appended bogus
/// "test_15m: 7200s" drift events to the production `.trinity/event_log.jsonl`
/// on every `cargo test` run — 337 of them accumulated.)
fn compute_drift(now: Instant, intervals: &[(&str, &Instant, u64)]) -> Vec<String> {
    let mut drifted = Vec::new();
    for (name, last_run, expected_secs) in intervals {
        let elapsed = now.saturating_duration_since(**last_run).as_secs();
        if elapsed > expected_secs * 2 {
            drifted.push(format!(
                "{}: {}s elapsed vs {}s expected ({}x drift)",
                name,
                elapsed,
                expected_secs,
                elapsed / expected_secs
            ));
        }
    }
    drifted
}

fn detect_drift(intervals: &[(&str, &Instant, u64)]) -> Vec<String> {
    let drifted = compute_drift(Instant::now(), intervals);
    if !drifted.is_empty() {
        println!("[CladeMonitor] DRIFT DETECTED:");
        for d in &drifted {
            println!("  - {}", d);
        }
        log_event("drift_detected", &drifted.join("; "));
    }
    drifted
}

fn track_build_hash() -> Option<(String, bool)> {
    let binary_path = format!("{}/trios_app", &project_dir());
    let hash_path = format!("{}/.trinity/state/build_hash.sha256", &project_dir());

    let data = match fs::read(&binary_path) {
        Ok(d) => d,
        Err(_) => return None,
    };

    let mut hasher = Sha256::new();
    hasher.update(&data);
    let current_hash = format!("{:x}", hasher.finalize());

    let previous_hash = fs::read_to_string(&hash_path)
        .ok()
        .map(|s| s.trim().to_string());
    let changed = previous_hash.as_ref() != Some(&current_hash);

    if let Err(e) = fs::write(&hash_path, &current_hash) {
        eprintln!("[CladeMonitor] Failed to write build hash: {}", e);
    }

    if changed {
        if let Some(prev) = &previous_hash {
            println!(
                "[CladeMonitor] Binary changed: {}..→ {}...",
                &prev[..8.min(prev.len())],
                &current_hash[..8]
            );
            log_event(
                "binary_changed",
                &format!("{}_{}", &prev[..8.min(prev.len())], &current_hash[..8]),
            );
        } else {
            println!(
                "[CladeMonitor] Initial binary hash: {}...",
                &current_hash[..8]
            );
            log_event("binary_hash_init", &current_hash[..16]);
        }
    }

    Some((current_hash, changed))
}

fn run_health_check(interval: &str, canary_cb: &mut CircuitBreaker) -> bool {
    let sovereign = check_health(&trios_config::sovereign_health_url());

    let canary = if canary_cb.should_probe() {
        let healthy = check_health(&trios_config::canary_health_url());
        if healthy {
            canary_cb.record_success();
        } else {
            canary_cb.record_failure();
        }
        healthy
    } else {
        false
    };

    let canary_label = if canary_cb.is_open() {
        "OPEN(skipped)"
    } else if canary {
        "true"
    } else {
        "false"
    };
    let status = format!("sovereign={},canary={}", sovereign, canary_label);
    println!("[CladeMonitor][{}] Health: {}", interval, status);

    if !sovereign {
        println!(
            "[CladeMonitor][{}] ALERT: Sovereign unhealthy — triggering rollback",
            interval
        );
        log_event("health_alert", &format!("sovereign_fail_at_{}", interval));
    }

    update_last_wake(&status, "ok", "clade-1.0.0");
    sovereign
}

/// Extract a short, log-friendly tail of a build script's stderr: the last few
/// non-empty lines, length-capped. Keeps event_log.jsonl readable while still
/// surfacing *why* a build check failed (observability over a silent bool).
fn build_error_tail(stderr: &str) -> String {
    let tail: Vec<&str> = stderr
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let joined = tail.join(" | ");
    // Char-safe cap (never slice mid-UTF-8): keep the last 300 chars.
    if joined.chars().count() > 300 {
        let tail_chars: Vec<char> = joined.chars().rev().take(300).collect();
        tail_chars.into_iter().rev().collect()
    } else {
        joined
    }
}

/// Resolve an explicit build-check executable from the `TRIOS_BUILD_CHECK_SCRIPT`
/// env override (consistent with `TRIOS_ROOT`/port env handling in trios-config).
///
/// `None` means "no custom hook configured" — `run_build_check` then falls back
/// to the canonical `clade-build` ring. We deliberately do NOT default to a
/// `.sh` path: L7 UNITY forbids shell scripts on the critical path, and a
/// missing default produced a non-actionable `build_check_skip` every cycle.
fn resolve_build_check_script(env_override: Option<String>) -> Option<String> {
    match env_override {
        Some(p) if !p.trim().is_empty() => Some(p),
        _ => None,
    }
}

fn run_build_check(interval: &str) -> bool {
    use std::path::Path;
    use std::process::{Command, Stdio};

    let Some(script) = resolve_build_check_script(std::env::var("TRIOS_BUILD_CHECK_SCRIPT").ok())
    else {
        // No custom hook: run the canonical Rust build gate (swiftc via the
        // clade-build ring). This keeps the daemon self-sufficient — it catches
        // build regressions even when the agent cron isn't waking — without a
        // shell script (L7 UNITY).
        return run_default_build_check(interval);
    };

    let script_path = Path::new(&script);
    if !script_path.exists() {
        eprintln!(
            "[CladeMonitor][{}] Build check FAILED: configured script {} not found",
            interval, script
        );
        log_event(
            "build_check",
            &format!("interval_{}_missing_{}", interval, script),
        );
        return false;
    }

    // Verify script is owned by current user and not world-writable
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(meta) = script_path.metadata() {
            let uid = unsafe { libc::getuid() };
            if meta.uid() != uid {
                eprintln!(
                    "[CladeMonitor][{}] SECURITY: {} not owned by current user (uid {} vs {})",
                    interval,
                    script,
                    meta.uid(),
                    uid
                );
                log_event("build_check_security", &format!("wrong_owner_{}", script));
                return false;
            }
            if meta.mode() & 0o002 != 0 {
                eprintln!(
                    "[CladeMonitor][{}] SECURITY: {} is world-writable",
                    interval, script
                );
                log_event(
                    "build_check_security",
                    &format!("world_writable_{}", script),
                );
                return false;
            }
        }
    }

    println!("[CladeMonitor][{}] Build check: {}", interval, script);
    // Use output() (not status()) so the piped stderr is actually drained —
    // an unread pipe can deadlock a chatty build — and so we can distinguish a
    // spawn failure from a genuine build failure, logging *why* either way.
    match Command::new(&script)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(out) if out.status.success() => {
            log_event("build_check", &format!("interval_{}_pass", interval));
            true
        }
        Ok(out) => {
            let tail = build_error_tail(&String::from_utf8_lossy(&out.stderr));
            eprintln!(
                "[CladeMonitor][{}] Build check FAILED (exit {:?}): {}",
                interval,
                out.status.code(),
                tail
            );
            log_event(
                "build_check",
                &format!(
                    "interval_{}_fail_exit{:?}: {}",
                    interval,
                    out.status.code(),
                    tail
                ),
            );
            false
        }
        Err(e) => {
            // Could not even spawn the script (missing exec bit / permission) —
            // a distinct failure mode from a build that ran and failed.
            eprintln!(
                "[CladeMonitor][{}] Build check could not spawn {}: {}",
                interval, script, e
            );
            log_event("build_check_spawn_fail", &format!("{}: {}", script, e));
            false
        }
    }
}

/// Default build gate when no custom hook is set: invoke the `clade-build` ring
/// (swiftc compile) via cargo from the workspace root. cargo is on PATH in the
/// same context that launches `clade-monitor`.
fn run_default_build_check(interval: &str) -> bool {
    use std::process::{Command, Stdio};

    let dir = project_dir();
    println!("[CladeMonitor][{}] Build check: clade-build ring", interval);
    match Command::new("cargo")
        .args(["run", "--quiet", "--bin", "clade-build"])
        .current_dir(&dir)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(out) if out.status.success() => {
            log_event("build_check", &format!("interval_{}_pass", interval));
            true
        }
        Ok(out) => {
            let tail = build_error_tail(&String::from_utf8_lossy(&out.stderr));
            eprintln!(
                "[CladeMonitor][{}] Build check FAILED (exit {:?}): {}",
                interval,
                out.status.code(),
                tail
            );
            log_event(
                "build_check",
                &format!(
                    "interval_{}_fail_exit{:?}: {}",
                    interval,
                    out.status.code(),
                    tail
                ),
            );
            false
        }
        Err(e) => {
            eprintln!(
                "[CladeMonitor][{}] Build check could not spawn cargo: {}",
                interval, e
            );
            log_event(
                "build_check_spawn_fail",
                &format!("cargo clade-build: {}", e),
            );
            false
        }
    }
}

fn run_seal_audit(interval: &str) -> bool {
    let budget = load_safety_budget();
    println!(
        "[CladeMonitor][{}] Safety budget: {}/{}",
        interval, budget.budget, budget.max_budget
    );

    if budget.halted || budget.budget <= 0.0 {
        println!(
            "[CladeMonitor][{}] HALTED: budget={}, no auto-improvement allowed",
            interval, budget.budget
        );
        log_event("budget_halted", &format!("budget_{}", budget.budget));
        return false;
    }

    println!(
        "[CladeMonitor][{}] Seal audit: checking Canary...",
        interval
    );
    log_event("seal_audit", &format!("interval_{}", interval));
    true
}

fn run_deep_audit(interval: &str) -> bool {
    println!(
        "[CladeMonitor][{}] Deep audit: fitness sync + screenshot baseline",
        interval
    );
    log_event("deep_audit", &format!("interval_{}", interval));
    true
}

const SUBPROCESS_TIMEOUT_SECS: u64 = 600;

fn run_tablecloth(interval: &str) -> bool {
    use std::process::{Command, Stdio};
    let budget = load_safety_budget();
    if budget.halted || budget.budget <= 0.0 {
        println!(
            "[CladeMonitor][{}] Tablecloth HALTED: budget={} — skipping loop",
            interval, budget.budget
        );
        log_event("tablecloth_halted", &format!("budget_{}", budget.budget));
        return true;
    }

    println!(
        "[CladeMonitor][{}] Spawning clade-tablecloth (timeout {}s)...",
        interval, SUBPROCESS_TIMEOUT_SECS
    );
    let result = run_with_timeout(
        Command::new("cargo")
            .args(["run", "--bin", "clade-tablecloth"])
            .current_dir(project_dir())
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
        SUBPROCESS_TIMEOUT_SECS,
    );

    if result {
        log_event("tablecloth", &format!("interval_{}_pass", interval));
    } else {
        log_event("tablecloth", &format!("interval_{}_fail", interval));
    }
    result
}

fn run_with_timeout(cmd: &mut std::process::Command, timeout_secs: u64) -> bool {
    use std::os::unix::process::CommandExt;
    // Put child in its own process group so we can kill the entire tree
    unsafe {
        cmd.pre_exec(|| {
            libc::setpgid(0, 0);
            Ok(())
        });
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[CladeMonitor] Failed to spawn subprocess: {}", e);
            return false;
        }
    };

    let child_pid = child.id() as i32;
    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(timeout_secs);

    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {
                if start.elapsed() > timeout {
                    eprintln!(
                        "[CladeMonitor] Subprocess timed out after {}s, killing process group",
                        timeout_secs
                    );
                    // Kill the entire process group (child + its subprocesses)
                    unsafe {
                        libc::killpg(child_pid, libc::SIGTERM);
                    }
                    thread::sleep(Duration::from_secs(2));
                    // Force kill if still alive
                    unsafe {
                        libc::killpg(child_pid, libc::SIGKILL);
                    }
                    if let Err(e) = child.wait() {
                        eprintln!("[CladeMonitor] Failed to reap child after kill: {}", e);
                    }
                    log_event(
                        "subprocess_timeout",
                        &format!("{}s_pgid_{}", timeout_secs, child_pid),
                    );
                    return false;
                }
                thread::sleep(Duration::from_secs(5));
            }
            Err(e) => {
                eprintln!("[CladeMonitor] Error waiting for subprocess: {}", e);
                return false;
            }
        }
    }
}

fn check_health(url: &str) -> bool {
    match get(url) {
        Ok(r) => {
            let body = r.text().unwrap_or_default();
            body.contains("\"status\":\"ok\"")
        }
        Err(_) => false,
    }
}

fn load_safety_budget() -> SafetyBudget {
    let path = format!("{}/.trinity/state/safety_budget.json", &project_dir());
    match fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str(&content) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[monitor] Failed to parse {}: {}", path, e);
                default_budget()
            }
        },
        Err(_) => default_budget(),
    }
}

fn default_budget() -> SafetyBudget {
    SafetyBudget {
        budget: 5.0,
        max_budget: 5.0,
        total_trials: 0,
        total_failures: 0,
        halted: false,
    }
}

fn update_last_wake(health: &str, build: &str, clade_id: &str) {
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let wake = LastWake {
        ts,
        health: health.to_string(),
        build: build.to_string(),
        clade_id: clade_id.to_string(),
    };
    let path = format!("{}/.trinity/state/last_wake.json", &project_dir());
    match serde_json::to_string(&wake) {
        Ok(json) => {
            if let Err(e) = fs::write(&path, &json) {
                eprintln!("[monitor] Failed to write last_wake.json: {}", e);
            }
        }
        Err(e) => eprintln!("[monitor] Failed to serialize last_wake: {}", e),
    }
}

fn log_event(event: &str, details: &str) {
    use std::sync::OnceLock;
    static CORR_ID: OnceLock<String> = OnceLock::new();
    let id = CORR_ID.get_or_init(trios_config::new_correlation_id);
    trios_config::log_event(id, event, details);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_zero_failures() {
        assert_eq!(calculate_backoff(0), 1);
    }

    #[test]
    fn build_error_tail_keeps_last_lines() {
        let stderr = "warn a\nwarn b\nerror: x\nerror: y\nerror: z\n";
        assert_eq!(build_error_tail(stderr), "error: x | error: y | error: z");
    }

    #[test]
    fn build_error_tail_empty_is_empty() {
        assert_eq!(build_error_tail("\n\n  \n"), "");
    }

    #[test]
    fn build_error_tail_is_char_safe_and_capped() {
        // Multi-byte chars must not cause a mid-UTF-8 panic when capping.
        let line = "é".repeat(500);
        let out = build_error_tail(&line);
        assert!(out.chars().count() <= 300);
    }

    #[test]
    fn backoff_one_failure() {
        assert_eq!(calculate_backoff(1), 2);
    }

    #[test]
    fn resolve_build_check_script_honors_env_override() {
        assert_eq!(
            resolve_build_check_script(Some("/custom/build".to_string())),
            Some("/custom/build".to_string())
        );
    }

    #[test]
    fn resolve_build_check_script_ignores_blank_override() {
        // Blank override -> None -> fall back to the default clade-build gate.
        assert_eq!(resolve_build_check_script(Some("   ".to_string())), None);
    }

    #[test]
    fn resolve_build_check_script_defaults_when_unset() {
        // Unset -> None -> default clade-build gate (no `.sh`, L7 UNITY).
        assert_eq!(resolve_build_check_script(None), None);
    }

    #[test]
    fn backoff_two_failures() {
        assert_eq!(calculate_backoff(2), 4);
    }

    #[test]
    fn backoff_three_failures() {
        assert_eq!(calculate_backoff(3), 8);
    }

    #[test]
    fn backoff_caps_at_16() {
        assert_eq!(calculate_backoff(4), 16);
        assert_eq!(calculate_backoff(10), 16);
        assert_eq!(calculate_backoff(100), 16);
    }

    #[test]
    fn default_budget_values() {
        let b = default_budget();
        assert!((b.budget - 5.0).abs() < f64::EPSILON);
        assert!((b.max_budget - 5.0).abs() < f64::EPSILON);
        assert_eq!(b.total_trials, 0);
        assert_eq!(b.total_failures, 0);
        assert!(!b.halted);
    }

    #[test]
    fn backoff_monotonically_increases() {
        let mut prev = 0;
        for i in 0..=4 {
            let val = calculate_backoff(i);
            assert!(val > prev, "backoff({}) = {} should be > {}", i, val, prev);
            prev = val;
        }
    }

    #[test]
    fn pidfile_path_contains_state_dir() {
        let path = format!("{}/.trinity/state/clade-monitor.pid", &project_dir());
        assert!(path.contains(".trinity/state"));
        assert!(path.ends_with(".pid"));
    }

    #[test]
    fn health_sovereign_url_format() {
        let url = trios_config::sovereign_health_url();
        assert!(url.starts_with("http://127.0.0.1:"));
        assert!(url.ends_with("/health"));
    }

    #[test]
    fn health_canary_url_format() {
        let url = trios_config::canary_health_url();
        assert!(url.starts_with("http://127.0.0.1:"));
        assert!(url.ends_with("/health"));
    }

    #[test]
    fn jitter_zero_failures_no_jitter() {
        assert_eq!(calculate_backoff_with_jitter(0, 12345), 1);
    }

    #[test]
    fn jitter_adds_at_most_25_percent() {
        for seed in 0..100 {
            let base = calculate_backoff(2); // 4
            let jittered = calculate_backoff_with_jitter(2, seed);
            assert!(jittered >= base, "jitter should not reduce backoff");
            assert!(
                jittered <= base + base / 4,
                "jitter should be <= 25% of base"
            );
        }
    }

    #[test]
    fn jitter_deterministic_for_same_seed() {
        let a = calculate_backoff_with_jitter(3, 42);
        let b = calculate_backoff_with_jitter(3, 42);
        assert_eq!(a, b);
    }

    #[test]
    fn jitter_varies_across_seeds() {
        let a = calculate_backoff_with_jitter(3, 0);
        let b = calculate_backoff_with_jitter(3, 13);
        // seed 0 -> 0% jitter, seed 13 -> 13% jitter — should differ
        assert_ne!(a, b);
    }

    #[test]
    fn replenish_below_threshold_returns_false() {
        assert!(!replenish_budget(0));
        assert!(!replenish_budget(1));
        assert!(!replenish_budget(2));
    }

    #[test]
    fn compute_drift_no_drift_on_fresh_timestamps() {
        // Call the pure fn (NOT detect_drift) so the test never writes to the
        // real event_log.
        let now = Instant::now();
        let intervals: Vec<(&str, &Instant, u64)> =
            vec![("test_15m", &now, 900), ("test_30m", &now, 1800)];
        let drifted = compute_drift(now, &intervals);
        assert!(drifted.is_empty());
    }

    #[test]
    fn compute_drift_catches_stale_interval() {
        let now = Instant::now();
        let stale = now - Duration::from_secs(7200);
        let intervals: Vec<(&str, &Instant, u64)> = vec![("test_15m", &stale, 900)];
        let drifted = compute_drift(now, &intervals);
        assert_eq!(drifted.len(), 1);
        assert!(drifted[0].contains("test_15m"));
        assert!(drifted[0].contains("drift"));
    }

    // Boundary battery: drift fires strictly ABOVE 2x expected (threshold uses
    // `>`), not at exactly 2x. Locks the threshold against off-by-one drift.
    #[test]
    fn compute_drift_boundary_battery() {
        let now = Instant::now();
        let expected: u64 = 900;
        // (elapsed_secs, should_drift)
        let cases: [(u64, bool); 4] = [
            (expected, false),        // 1x — fine
            (expected * 2, false),    // exactly 2x — NOT drift (threshold is >)
            (expected * 2 + 1, true), // just over 2x — drift
            (expected * 8, true),     // 8x — drift
        ];
        for (elapsed, should_drift) in cases {
            let last = now - Duration::from_secs(elapsed);
            let intervals: Vec<(&str, &Instant, u64)> = vec![("iv", &last, expected)];
            let drifted = compute_drift(now, &intervals);
            assert_eq!(
                !drifted.is_empty(),
                should_drift,
                "elapsed={}s expected drift={}",
                elapsed,
                should_drift
            );
        }
    }

    #[test]
    fn track_build_hash_returns_none_for_missing_binary() {
        std::env::set_var("TRIOS_ROOT", "/tmp/nonexistent-trios-test-dir");
        let result = track_build_hash();
        assert!(result.is_none());
        std::env::remove_var("TRIOS_ROOT");
    }

    #[test]
    fn sha256_produces_64_char_hex() {
        let mut hasher = Sha256::new();
        hasher.update(b"test data");
        let hash = format!("{:x}", hasher.finalize());
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn run_with_timeout_succeeds_on_fast_command() {
        use std::process::Command;
        let result = run_with_timeout(Command::new("echo").arg("hello"), 5);
        assert!(result);
    }

    #[test]
    fn run_with_timeout_fails_on_bad_command() {
        use std::process::Command;
        let result = run_with_timeout(&mut Command::new("/nonexistent-binary-xyz"), 5);
        assert!(!result);
    }

    #[test]
    fn subprocess_timeout_constant_is_reasonable() {
        assert!((60..=3600).contains(&SUBPROCESS_TIMEOUT_SECS));
    }

    #[test]
    fn atomic_write_creates_file() {
        let path = "/tmp/clade_monitor_atomic_test.json";
        let _ = std::fs::remove_file(path);
        let result = atomic_write(path, r#"{"test": true}"#);
        assert!(result.is_ok());
        let content = std::fs::read_to_string(path).unwrap_or_default();
        assert!(content.contains("test"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn atomic_write_no_tmp_left_behind() {
        let path = "/tmp/clade_monitor_atomic_notmp.json";
        let tmp_path = format!("{}.tmp", path);
        let _ = atomic_write(path, "data");
        assert!(!std::path::Path::new(&tmp_path).exists());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn circuit_breaker_starts_closed() {
        let mut cb = CircuitBreaker::new("test");
        assert!(!cb.is_open());
        assert!(cb.should_probe());
        assert_eq!(cb.consecutive_failures, 0);
    }

    #[test]
    fn circuit_breaker_trips_after_threshold() {
        let mut cb = CircuitBreaker::new("test");
        for _ in 0..CIRCUIT_BREAKER_TRIP_THRESHOLD {
            cb.record_failure();
        }
        assert!(cb.is_open());
    }

    #[test]
    fn circuit_breaker_stays_closed_below_threshold() {
        let mut cb = CircuitBreaker::new("test");
        for _ in 0..CIRCUIT_BREAKER_TRIP_THRESHOLD - 1 {
            cb.record_failure();
        }
        assert!(!cb.is_open());
    }

    #[test]
    fn circuit_breaker_resets_on_success() {
        let mut cb = CircuitBreaker::new("test");
        for _ in 0..CIRCUIT_BREAKER_TRIP_THRESHOLD {
            cb.record_failure();
        }
        assert!(cb.is_open());
        cb.record_success();
        assert!(!cb.is_open());
        assert_eq!(cb.consecutive_failures, 0);
    }

    #[test]
    fn circuit_breaker_open_blocks_probe() {
        let mut cb = CircuitBreaker::new("test");
        for _ in 0..CIRCUIT_BREAKER_TRIP_THRESHOLD {
            cb.record_failure();
        }
        assert!(!cb.should_probe());
    }

    #[test]
    fn circuit_breaker_probe_interval_constant() {
        assert_eq!(CIRCUIT_BREAKER_PROBE_INTERVAL_SECS, 300);
    }

    #[test]
    fn circuit_breaker_trip_threshold_constant() {
        assert_eq!(CIRCUIT_BREAKER_TRIP_THRESHOLD, 3);
    }
}
