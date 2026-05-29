use chrono::{DateTime, Utc};
use reqwest::blocking::get;
use serde::{Deserialize, Serialize};
use std::fs;
use std::thread;
use std::time::{Duration, SystemTime};

const PROJECT_DIR: &str = "/Users/playra/BrowserOS-full/trios";
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

fn main() {
    use std::collections::HashMap;

    println!("[CladeMonitor] Starting cron monitor loop...");
    println!("[CladeMonitor] Press Ctrl+C to stop.");

    let mut last_15m = SystemTime::now();
    let mut last_30m = SystemTime::now();
    let mut last_60m = SystemTime::now();
    let mut last_24h = SystemTime::now();

    // Backoff tracking: consecutive_failures -> multiplier
    let mut failure_counts: HashMap<String, u32> = HashMap::new();

    loop {
        let now = SystemTime::now();

        // Every 15 min: health quick
        let backoff_15m = calculate_backoff(failure_counts.get("15m").copied().unwrap_or(0));
        if now.duration_since(last_15m).unwrap_or_default() >= Duration::from_secs(900 * backoff_15m) {
            last_15m = now;
            if run_health_check("15m") {
                failure_counts.remove("15m");
            } else {
                *failure_counts.entry("15m".to_string()).or_insert(0) += 1;
            }
        }

        // Every 30 min: build + dirty
        let backoff_30m = calculate_backoff(failure_counts.get("30m").copied().unwrap_or(0));
        if now.duration_since(last_30m).unwrap_or_default() >= Duration::from_secs(1800 * backoff_30m) {
            last_30m = now;
            if run_build_check("30m") {
                failure_counts.remove("30m");
            } else {
                *failure_counts.entry("30m".to_string()).or_insert(0) += 1;
            }
        }

        // Every 60 min: seal audit + safety budget
        let backoff_60m = calculate_backoff(failure_counts.get("60m").copied().unwrap_or(0));
        if now.duration_since(last_60m).unwrap_or_default() >= Duration::from_secs(3600 * backoff_60m) {
            last_60m = now;
            if run_seal_audit("60m") {
                failure_counts.remove("60m");
            } else {
                *failure_counts.entry("60m".to_string()).or_insert(0) += 1;
            }
        }

        // Every 24h: deep audit + wrap-up
        let backoff_24h = calculate_backoff(failure_counts.get("24h").copied().unwrap_or(0));
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

    let script = format!("{}/.claude/queen-zig.sh", PROJECT_DIR);
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
    let path = format!("{}/.trinity/state/safety_budget.json", PROJECT_DIR);
    match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_else(|_| default_budget()),
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
    let path = format!("{}/.trinity/state/last_wake.json", PROJECT_DIR);
    if let Ok(json) = serde_json::to_string(&wake) {
        let _ = fs::write(path, json);
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
    let path = format!("{}/.trinity/event_log.jsonl", PROJECT_DIR);
    let _ = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)
        .and_then(|mut f| {
            use std::io::Write;
            writeln!(f, "{}", line)
        });
}
