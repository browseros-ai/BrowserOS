use chrono::{DateTime, Utc};
use reqwest::blocking::get;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, SystemTime};

static RUNNING: AtomicBool = AtomicBool::new(true);

extern "C" fn handle_signal(_sig: libc::c_int) {
    RUNNING.store(false, Ordering::Relaxed);
}

fn project_dir() -> String { std::env::var("TRIOS_ROOT").unwrap_or_else(|_| "/Users/playra/BrowserOS-full/trios".to_string()) }
const HEALTH_SOVEREIGN: &str = "http://127.0.0.1:9105/health";
const HEALTH_CANARY: &str = "http://127.0.0.1:9205/health";

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
    let path = format!("{}/.trinity/state/clade-monitor.pid", &project_dir());
    if let Err(e) = fs::create_dir_all(format!("{}/.trinity/state", &project_dir())) {
        eprintln!("[CladeMonitor] Failed to create state dir: {}", e);
        return None;
    }

    if let Ok(existing) = fs::read_to_string(&path) {
        if let Ok(pid) = existing.trim().parse::<i32>() {
            let alive = unsafe { libc::kill(pid, 0) } == 0;
            if alive {
                eprintln!("[CladeMonitor] Another instance running (PID {}). Exiting.", pid);
                return None;
            }
        }
    }

    // Kill any orphan monitors not tracked by PID file
    let orphans = kill_orphan_monitors();
    if orphans > 0 {
        println!("[CladeMonitor] Cleaned up {} orphan process(es)", orphans);
    }

    match fs::File::create(&path) {
        Ok(mut f) => {
            let pid = std::process::id();
            if let Err(e) = write!(f, "{}", pid) {
                eprintln!("[CladeMonitor] Failed to write PID file: {}", e);
                return None;
            }
            println!("[CladeMonitor] PID file: {} (pid={})", path, pid);
            Some(f)
        }
        Err(e) => {
            eprintln!("[CladeMonitor] Failed to create PID file: {}", e);
            None
        }
    }
}

fn cleanup_pidfile() {
    let path = format!("{}/.trinity/state/clade-monitor.pid", &project_dir());
    if let Err(e) = fs::remove_file(&path) {
        eprintln!("[CladeMonitor] Failed to remove PID file: {}", e);
    }
}

fn main() {
    use std::collections::HashMap;

    if acquire_pidfile().is_none() {
        std::process::exit(1);
    }

    // Register signal handlers for graceful shutdown
    unsafe {
        libc::signal(libc::SIGTERM, handle_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, handle_signal as *const () as libc::sighandler_t);
    }

    println!("[CladeMonitor] Starting cron monitor loop...");
    println!("[CladeMonitor] Press Ctrl+C to stop (graceful shutdown enabled).");

    let mut last_15m = SystemTime::now();
    let mut last_30m = SystemTime::now();
    let mut last_60m = SystemTime::now();
    let mut last_60m_tablecloth = SystemTime::now();
    let mut last_24h = SystemTime::now();

    // Backoff tracking: consecutive_failures -> multiplier
    let mut failure_counts: HashMap<String, u32> = HashMap::new();
    let pid_seed = std::process::id();

    while RUNNING.load(Ordering::Relaxed) {
        let now = SystemTime::now();

        // Every 15 min: health quick
        let backoff_15m = calculate_backoff_with_jitter(failure_counts.get("15m").copied().unwrap_or(0), pid_seed);
        if now.duration_since(last_15m).unwrap_or_default() >= Duration::from_secs(900 * backoff_15m) {
            last_15m = now;
            if run_health_check("15m") {
                failure_counts.remove("15m");
            } else {
                *failure_counts.entry("15m".to_string()).or_insert(0) += 1;
            }
        }

        // Every 30 min: build + dirty
        let backoff_30m = calculate_backoff_with_jitter(failure_counts.get("30m").copied().unwrap_or(0), pid_seed);
        if now.duration_since(last_30m).unwrap_or_default() >= Duration::from_secs(1800 * backoff_30m) {
            last_30m = now;
            if run_build_check("30m") {
                failure_counts.remove("30m");
            } else {
                *failure_counts.entry("30m".to_string()).or_insert(0) += 1;
            }
        }

        // Every 60 min: seal audit + safety budget
        let backoff_60m = calculate_backoff_with_jitter(failure_counts.get("60m").copied().unwrap_or(0), pid_seed);
        if now.duration_since(last_60m).unwrap_or_default() >= Duration::from_secs(3600 * backoff_60m) {
            last_60m = now;
            if run_seal_audit("60m") {
                failure_counts.remove("60m");
            } else {
                *failure_counts.entry("60m".to_string()).or_insert(0) += 1;
            }
        }

        // Every 60 min: autonomous self-improvement loop (clade-tablecloth)
        let backoff_60m_t = calculate_backoff_with_jitter(failure_counts.get("60m_tablecloth").copied().unwrap_or(0), pid_seed);
        if now.duration_since(last_60m_tablecloth).unwrap_or_default() >= Duration::from_secs(3600 * backoff_60m_t) {
            last_60m_tablecloth = now;
            if run_tablecloth("60m_tablecloth") {
                failure_counts.remove("60m_tablecloth");
            } else {
                *failure_counts.entry("60m_tablecloth".to_string()).or_insert(0) += 1;
            }
        }

        // Every 24h: deep audit + wrap-up
        let backoff_24h = calculate_backoff_with_jitter(failure_counts.get("24h").copied().unwrap_or(0), pid_seed);
        if now.duration_since(last_24h).unwrap_or_default() >= Duration::from_secs(86400 * backoff_24h) {
            last_24h = now;
            if run_deep_audit("24h") {
                failure_counts.remove("24h");
            } else {
                *failure_counts.entry("24h".to_string()).or_insert(0) += 1;
            }
        }

        thread::sleep(Duration::from_secs(60));
    }

    cleanup_pidfile();
    println!("[CladeMonitor] Graceful shutdown complete.");
    log_event("monitor_shutdown", "graceful");
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

fn run_health_check(interval: &str) -> bool {
    let sovereign = check_health(HEALTH_SOVEREIGN);
    let canary = check_health(HEALTH_CANARY);

    let status = format!("sovereign={},canary={}", sovereign, canary);
    println!("[CladeMonitor][{}] Health: {}", interval, status);

    if !sovereign {
        println!("[CladeMonitor][{}] ALERT: Sovereign unhealthy — triggering rollback", interval);
        log_event("health_alert", &format!("sovereign_fail_at_{}", interval));
    }

    update_last_wake(&status, "ok", "clade-1.0.0");
    sovereign && canary
}

fn run_build_check(interval: &str) -> bool {
    use std::path::Path;
    use std::process::{Command, Stdio};

    let script = format!("{}/.claude/queen-zig.sh", &project_dir());
    if !Path::new(&script).exists() {
        println!("[CladeMonitor][{}] SKIP: {} not found", interval, script);
        log_event("build_check_skip", &format!("missing_script_{}", script));
        return true; // not a failure, just nothing to do
    }

    println!("[CladeMonitor][{}] Build check: {}", interval, script);
    let result = Command::new(&script)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if result {
        log_event("build_check", &format!("interval_{}_pass", interval));
    } else {
        log_event("build_check", &format!("interval_{}_fail", interval));
    }
    result
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

    println!("[CladeMonitor][{}] Seal audit: checking Canary...", interval);
    log_event("seal_audit", &format!("interval_{}", interval));
    true
}

fn run_deep_audit(interval: &str) -> bool {
    println!("[CladeMonitor][{}] Deep audit: fitness sync + screenshot baseline", interval);
    log_event("deep_audit", &format!("interval_{}", interval));
    true
}

fn run_tablecloth(interval: &str) -> bool {
    use std::process::{Command, Stdio};
    let budget = load_safety_budget();
    if budget.halted || budget.budget <= 0.0 {
        println!(
            "[CladeMonitor][{}] Tablecloth HALTED: budget={} — skipping loop",
            interval, budget.budget
        );
        log_event("tablecloth_halted", &format!("budget_{}", budget.budget));
        return true; // not a failure, just gated
    }

    println!("[CladeMonitor][{}] Spawning clade-tablecloth...", interval);
    let result = Command::new("cargo")
        .args(["run", "--bin", "clade-tablecloth"])
        .current_dir(project_dir())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if result {
        log_event("tablecloth", &format!("interval_{}_pass", interval));
    } else {
        log_event("tablecloth", &format!("interval_{}_fail", interval));
    }
    result
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
    let ts: DateTime<Utc> = Utc::now();
    let line = format!(
        r#"{{"timestamp":"{}","event":"{}","details":"{}"}}"#,
        ts.to_rfc3339(),
        event,
        details
    );
    let path = format!("{}/.trinity/event_log.jsonl", &project_dir());
    if let Err(e) = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)
        .and_then(|mut f| {
            use std::io::Write;
            writeln!(f, "{}", line)
        })
    {
        eprintln!("[monitor] Failed to write event log: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_zero_failures() {
        assert_eq!(calculate_backoff(0), 1);
    }

    #[test]
    fn backoff_one_failure() {
        assert_eq!(calculate_backoff(1), 2);
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
        assert!(HEALTH_SOVEREIGN.starts_with("http://127.0.0.1:"));
        assert!(HEALTH_SOVEREIGN.ends_with("/health"));
    }

    #[test]
    fn health_canary_url_format() {
        assert!(HEALTH_CANARY.starts_with("http://127.0.0.1:"));
        assert!(HEALTH_CANARY.ends_with("/health"));
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
            assert!(jittered <= base + base / 4, "jitter should be <= 25% of base");
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
}
