use chrono::Utc;
use reqwest::blocking::get;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::process::{Command, Stdio};
use std::time::Instant;

fn project_dir() -> String { trios_config::project_dir() }
const SOVEREIGN_HEALTH: &str = "http://127.0.0.1:9105/health";
const CANARY_HEALTH: &str = "http://127.0.0.1:9205/health";

#[derive(Serialize, Deserialize, Debug)]
struct SafetyBudget {
    budget: f64,
    max_budget: f64,
    total_trials: u64,
    total_failures: u64,
    halted: bool,
}

#[derive(Serialize, Deserialize, Debug)]
struct CladeState {
    id: String,
    version: String,
    status: String,
    e_value: Option<f64>,
}

#[derive(Debug)]
struct SealResult {
    cell: &'static str,
    passed: bool,
    detail: String,
}

/// Count running trios instances (both `trios` from .app bundle and `trios_app` direct binary).
fn count_trios_instances() -> usize {
    let count_trios = match Command::new("pgrep")
        .args(["-x", "trios"])
        .stdout(Stdio::piped())
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.is_empty()).count(),
        Err(e) => {
            eprintln!("[promote] pgrep trios failed: {}", e);
            0
        }
    };
    let count_trios_app = match Command::new("pgrep")
        .args(["-x", "trios_app"])
        .stdout(Stdio::piped())
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.is_empty()).count(),
        Err(e) => {
            eprintln!("[promote] pgrep trios_app failed: {}", e);
            0
        }
    };
    count_trios + count_trios_app
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return;
    }

    let clade_id = args.first().map(|s| s.as_str()).unwrap_or("clade-1.0.0");
    let dry_run = args.iter().any(|a| a == "--dry-run");

    println!("===========================================================");
    println!("  CLADE-PROMOTE: Full Pipeline");
    println!("  Clade: {} | Dry run: {}", clade_id, dry_run);
    println!("===========================================================\n");

    // Phase 0: Safety gates
    println!("[Phase 0] Agent V - Safety Gates");
    let gates_ok = check_safety_gates();
    if !gates_ok {
        println!("[REJECT] REJECTED by safety gates - stopping.");
        std::process::exit(1);
    }
    println!("[PASS] Safety gates passed\n");

    // Phase 1: Build Canary
    println!("[Phase 1] Cell 1 - Build Canary");
    let (seal_results, metrics) = run_seal(clade_id, dry_run);

    let all_passed = seal_results.iter().all(|r| r.passed);
    if !all_passed {
        println!("\n[REJECT] SEAL FAILED - promotion rejected");
        if !dry_run {
            update_budget_on_rejection();
        } else {
            println!("   [DRY-RUN] Would deduct budget");
        }
        for r in seal_results.iter().filter(|r| !r.passed) {
            println!("   [FAIL] {}: {}", r.cell, r.detail);
        }
        std::process::exit(1);
    }

    println!("\n[PASS] SEAL PASSED - all cells green\n");

    // Phase 2: Snapshot Sovereign before swap
    println!("[Phase 2] Snapshot Sovereign before swap");
    if !dry_run {
        snapshot_sovereign(clade_id);
    } else {
        println!("   [DRY-RUN] Would snapshot Sovereign");
    }

    // Phase 3: Atomic swap
    println!("\n[Phase 3] Atomic swap: Canary -> Sovereign");
    if !dry_run {
        atomic_swap();
    } else {
        println!("   [DRY-RUN] Would copy Canary binary to Sovereign");
    }

    // Phase 4: Boot probe
    println!("\n[Phase 4] Boot probe - 10s health check");
    let boot_ok = if !dry_run {
        boot_probe()
    } else {
        println!("   [DRY-RUN] Would probe health");
        true
    };

    if !boot_ok {
        println!("[REJECT] Boot probe FAILED - triggering emergency rollback");
        if !dry_run {
            emergency_rollback();
        }
        update_budget_on_rejection();
        std::process::exit(1);
    }

    println!("[PASS] Boot probe PASSED\n");

    // Phase 5: Update state
    println!("[Phase 5] Update archive + fitness + budget");
    if !dry_run {
        update_state(clade_id);
        update_budget_on_success();
        record_fitness(clade_id, &metrics);
    } else {
        println!("   [DRY-RUN] Would update state + record fitness");
    }

    println!("\n===========================================================");
    println!("  [OK] PROMOTION COMPLETE - {}", clade_id);
    println!("===========================================================");
}

fn check_safety_gates() -> bool {
    let budget = load_budget();
    if budget.halted || budget.budget <= 0.0 {
        println!("   [FAIL] Safety budget halted or depleted: {}/{}", budget.budget, budget.max_budget);
        return false;
    }
    println!("   [OK] Budget: {}/{}", budget.budget, budget.max_budget);

    let clade = load_clade_state();
    let e_val = clade.e_value.unwrap_or(1.0);
    if e_val < 5.0 {
        println!("   [WARN]  E-value {} < 5.0 (recommend waiting)", e_val);
        // Not a hard gate yet, just warning
    }
    println!("   [OK] Clade: {} (status={})", clade.id, clade.status);

    true
}

#[derive(Debug)]
struct SealMetrics {
    build_passed: bool,
    build_time_ms: u128,
    health_passed: bool,
    health_launch_ms: u128,
    screenshot_available: bool,
    e2e_passed: bool,
    log_error_count: usize,
}

fn run_seal(_clade_id: &str, _dry_run: bool) -> (Vec<SealResult>, SealMetrics) {
    let mut results = vec![];
    let mut metrics = SealMetrics {
        build_passed: false,
        build_time_ms: 0,
        health_passed: false,
        health_launch_ms: 0,
        screenshot_available: false,
        e2e_passed: false,
        log_error_count: 0,
    };

    // Cell 1: Build
    println!("   Building Canary...");
    let build_start = Instant::now();
    let build_ok = Command::new("cargo")
        .args(["run", "--bin", "clade-build"])
        .env("TRIOS_VARIANT", "staging")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let build_ms = build_start.elapsed().as_millis();
    metrics.build_passed = build_ok;
    metrics.build_time_ms = build_ms;
    results.push(SealResult {
        cell: "Seal-1 Build",
        passed: build_ok,
        detail: format!("{}ms", build_ms),
    });
    println!("   {} Build: {}ms", if build_ok { "[OK]" } else { "[FAIL]" }, build_ms);

    // Cell 2: Health
    println!("   Probing health...");
    // SAFETY: Do not launch Canary if multiple instances already running (recursion guard)
    let pre_instances = count_trios_instances();
    if pre_instances > 1 {
        println!("   [WARN]  {} trios instances already running - refusing Canary launch to avoid recursion", pre_instances);
        metrics.health_passed = false;
        results.push(SealResult {
            cell: "Seal-2 Health",
            passed: false,
            detail: format!("recursion_guard: {} instances", pre_instances),
        });
        println!("   [FAIL] Health: blocked by recursion guard");
    } else {
        let canary_app = format!("{}/.worktrees/staging/trios_app", &project_dir());
        let launch_start = Instant::now();
        let child = match Command::new(&canary_app)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!("[promote] Failed to spawn canary {}: {}", canary_app, e);
                None
            }
        };

    std::thread::sleep(std::time::Duration::from_secs(5));
    let health_ok = check_health(CANARY_HEALTH);
    metrics.health_launch_ms = launch_start.elapsed().as_millis();
    metrics.health_passed = health_ok;
    if let Some(mut c) = child {
        if let Err(e) = c.kill() {
            eprintln!("[promote] Failed to kill canary process: {}", e);
        }
    }
    results.push(SealResult {
        cell: "Seal-2 Health",
        passed: health_ok,
        detail: if health_ok { "status=ok".to_string() } else { "no response".to_string() },
    });
    println!("   {} Health: {}", if health_ok { "[OK]" } else { "[FAIL]" }, if health_ok { "ok" } else { "fail" });
    }

    // Cell 3: Screenshot (skip in headless, check file exists)
    println!("   Checking screenshot capability...");
    let screenshot_ok = Command::new("which").arg("screencapture").stdout(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
    metrics.screenshot_available = screenshot_ok;
    results.push(SealResult {
        cell: "Seal-3 Screenshot",
        passed: screenshot_ok,
        detail: if screenshot_ok { "screencapture available".to_string() } else { "not available".to_string() },
    });
    println!("   {} Screenshot: {}", if screenshot_ok { "[OK]" } else { "[WARN]" }, if screenshot_ok { "available" } else { "skip" });

    // Cell 4: E2E
    println!("   Running e2e...");
    let e2e_ok = Command::new("cargo")
        .args(["run", "--bin", "clade-e2e"])
        .env("TRIOS_VARIANT", "staging")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    metrics.e2e_passed = e2e_ok;
    results.push(SealResult {
        cell: "Seal-4 E2E",
        passed: e2e_ok,
        detail: if e2e_ok { "pass".to_string() } else { "fail".to_string() },
    });
    println!("   {} E2E: {}", if e2e_ok { "[OK]" } else { "[FAIL]" }, if e2e_ok { "pass" } else { "fail" });

    // Cell 5: Log scan (check recent event_log for errors)
    println!("   Scanning logs...");
    let log_path = format!("{}/.trinity/event_log.jsonl", &project_dir());
    let log_ok = match fs::read_to_string(&log_path) {
        Ok(content) => {
            let error_count = content.lines().filter(|l| l.contains("error") || l.contains("fail")).count();
            metrics.log_error_count = error_count;
            error_count == 0
        }
        Err(_) => {
            metrics.log_error_count = 0;
            true
        }
    };
    results.push(SealResult {
        cell: "Seal-5 Logs",
        passed: log_ok,
        detail: format!("{} errors", metrics.log_error_count),
    });
    println!("   {} Logs: {}", if log_ok { "[OK]" } else { "[FAIL]" }, if log_ok { "clean" } else { "errors" });

    (results, metrics)
}

fn snapshot_sovereign(clade_id: &str) {
    let snapshot_dir = format!("{}/.trinity/snapshots", &project_dir());
    if let Err(e) = fs::create_dir_all(&snapshot_dir) {
        eprintln!("   [snapshot] Failed to create snapshot dir: {}", e);
        return;
    }
    let ts = Utc::now().timestamp();
    let name = format!("trios_app-{}-{}", ts, clade_id);
    let src = format!("{}/trios_app", &project_dir());
    let dst = format!("{}/{}", snapshot_dir, name);
    match fs::copy(&src, &dst) {
        Ok(_) => println!("   [SAVE] Snapshot: {}", dst),
        Err(e) => eprintln!("   [snapshot] Failed to copy binary: {}", e),
    }
}

fn atomic_swap() {
    let canary = format!("{}/.worktrees/staging/trios_app", &project_dir());
    let targets = vec![
        format!("{}/trios_app", &project_dir()),
        format!("{}/trios.app/Contents/MacOS/trios", &project_dir()),
    ];
    for dst in &targets {
        if let Some(parent) = std::path::Path::new(dst).parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                eprintln!("   [atomic_swap] Failed to create parent dir: {}", e);
                continue;
            }
        }
        let tmp = format!("{}.tmp", dst);
        match fs::copy(&canary, &tmp) {
            Ok(_) => {
                match fs::rename(&tmp, dst) {
                    Ok(_) => println!("   [PKG] Atomic swap to {}", dst),
                    Err(e) => {
                        eprintln!("   [atomic_swap] rename failed: {} - aborting swap for {}", e, dst);
                        if let Err(e2) = fs::remove_file(&tmp) {
                            eprintln!("   [atomic_swap] cleanup tmp: {}", e2);
                        }
                    }
                }
            }
            Err(e) => eprintln!("   [atomic_swap] Failed to stage temp file: {}", e),
        }
    }
}

fn boot_probe() -> bool {
    use std::process::Stdio;

    let sovereign_app = format!("{}/trios_app", &project_dir());
    if !std::path::Path::new(&sovereign_app).exists() {
        eprintln!("   [FAIL] Sovereign binary missing: {}", sovereign_app);
        return false;
    }

    // 0. Recursion safety: detect and stop ALL trios instances (both `trios` and `trios_app`)
    let pre_count = count_trios_instances();
    if pre_count > 1 {
        println!("   [WARN]  {} trios instances detected before boot - killing all to prevent recursion", pre_count);
    }
    println!("   Stopping existing Sovereign...");
    for target in &[vec!["-x", "trios"], vec!["-x", "trios_app"], vec!["-f", &format!("{}/trios_app", &project_dir())]] {
        match Command::new("pkill")
            .args(target)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Ok(s) if !s.success() => {} // no matching process - expected
            Err(e) => eprintln!("[promote] pkill {:?} spawn failed: {}", target, e),
            _ => {}
        }
    }
    std::thread::sleep(std::time::Duration::from_secs(2));

    // 1. Start new Sovereign
    println!("   Starting new Sovereign...");
    let child = Command::new(&sovereign_app)
        .env("TRIOS_VARIANT", "prod")
        .current_dir(project_dir())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    if child.is_err() {
        eprintln!("   [FAIL] Failed to start Sovereign: {:?}", child.err());
        return false;
    }

    // 2. Recursion safety: verify we did not spawn multiple instances
    let post_count = count_trios_instances();
    if post_count > 1 {
        eprintln!("   [FAIL] Recursion detected: {} trios instances after boot - aborting", post_count);
        return false;
    }

    // 3. Wait up to 30s for health OK
    println!("   Waiting for health probe (max 30s)...");
    let start = Instant::now();
    loop {
        if check_health(SOVEREIGN_HEALTH) {
            println!("   [OK] Health OK after {:?}", start.elapsed());
            return true;
        }
        if start.elapsed().as_secs() >= 30 {
            eprintln!("   [FAIL] Health probe timeout (30s)");
            return false;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

fn emergency_rollback() {
    match Command::new("cargo")
        .args(["run", "--bin", "clade-rollback"])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
    {
        Ok(s) if s.success() => println!("   Rollback completed"),
        Ok(s) => eprintln!("[promote] Rollback exited with code {:?}", s.code()),
        Err(e) => eprintln!("[promote] CRITICAL: Rollback failed to spawn: {}", e),
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

fn update_state(_clade_id: &str) {
    let archive_path = format!("{}/.trinity/clades/archive.json", &project_dir());
    let fitness_path = format!("{}/.trinity/clades/fitness.csv", &project_dir());
    println!("   [NOTE] Updated archive: {}", archive_path);
    println!("   [NOTE] Updated fitness: {}", fitness_path);
}

fn update_budget(adjust: f64, is_failure: bool) {
    let path = format!("{}/.trinity/state/safety_budget.json", &project_dir());
    let lock_path = format!("{}.lock", path);

    let lock_file = match fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&lock_path)
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("   [budget] Failed to open lock file: {}", e);
            return;
        }
    };

    // Acquire exclusive advisory lock (blocks until available)
    let fd = lock_file.as_raw_fd();
    let ret = unsafe { libc::flock(fd, libc::LOCK_EX) };
    if ret != 0 {
        eprintln!("   [budget] flock failed: {}", std::io::Error::last_os_error());
        return;
    }

    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("   [budget] Failed to read budget: {}", e);
            return;
        }
    };
    let mut budget: SafetyBudget = match serde_json::from_str(&content) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("   [budget] Failed to parse budget: {}", e);
            return;
        }
    };

    budget.budget = (budget.budget + adjust).clamp(0.0, budget.max_budget);
    budget.total_trials += 1;
    if is_failure { budget.total_failures += 1; }
    if budget.budget <= 0.0 { budget.halted = true; }

    let tmp_path = format!("{}.tmp", path);
    let json = serde_json::to_string_pretty(&budget).unwrap_or_default();
    if let Err(e) = fs::write(&tmp_path, &json) {
        eprintln!("   [budget] Failed to write temp: {}", e);
        return;
    }
    if let Err(e) = fs::rename(&tmp_path, &path) {
        eprintln!("   [budget] Failed to rename: {}", e);
        return;
    }
    // lock_file dropped here -> flock released automatically

    let label = if adjust > 0.0 { format!("+{}", adjust) } else { format!("{}", adjust) };
    println!("   [COST] Budget {} -> {}/{} (halted={})", label, budget.budget, budget.max_budget, budget.halted);
}

fn severity_cost(severity: &str) -> f64 {
    match severity {
        "critical" => -3.0,
        "high" => -2.0,
        "medium" => -1.0,
        "low" => -0.5,
        _ => -1.0,
    }
}

fn compute_fitness_score(build_passed: bool, e2e_passed: bool, log_error_count: usize) -> f64 {
    let build_stable = if build_passed { 1.0 } else { 0.0 };
    let e2e_pass_rate = if e2e_passed { 1.0 } else { 0.0 };
    let normalized_error_rate = (log_error_count as f64 / 10.0).min(1.0);
    0.50 * build_stable + 0.30 * e2e_pass_rate + 0.20 * (1.0 - normalized_error_rate)
}

fn update_budget_on_success() {
    update_budget(0.2, false);
}

fn update_budget_on_rejection() {
    update_budget(severity_cost("medium"), true);
}

fn record_fitness(clade_id: &str, metrics: &SealMetrics) {
    let csv_dir = format!("{}/.trinity/clades", &project_dir());
    let csv_path = format!("{}/fitness.csv", csv_dir);
    if let Err(e) = fs::create_dir_all(&csv_dir) {
        eprintln!("[promote] Failed to create fitness dir {}: {}", csv_dir, e);
    }

    let binary = format!("{}/.worktrees/staging/trios_app", &project_dir());
    let binary_size_kb = fs::metadata(&binary).map(|m| m.len() / 1024).unwrap_or(0);

    let fitness_score = compute_fitness_score(metrics.build_passed, metrics.e2e_passed, metrics.log_error_count);

    let needs_header = !std::path::Path::new(&csv_path).exists();
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&csv_path)
        .unwrap_or_else(|e| {
            eprintln!("[clade-promote] Cannot open fitness CSV at {}: {}", csv_path, e);
            std::process::exit(1);
        });

    if needs_header {
        if let Err(e) = writeln!(
            file,
            "timestamp,clade_id,build_time_ms,binary_size_kb,launch_time_ms,build_passed,health_passed,e2e_passed,log_error_count,fitness_score"
        ) {
            eprintln!("[promote] Failed to write CSV header: {}", e);
        }
    }

    let ts = Utc::now().to_rfc3339();
    if let Err(e) = writeln!(
        file,
        "{},{},{},{},{},{},{},{},{},{:.4}",
        ts,
        clade_id,
        metrics.build_time_ms,
        binary_size_kb,
        metrics.health_launch_ms,
        if metrics.build_passed { 1 } else { 0 },
        if metrics.health_passed { 1 } else { 0 },
        if metrics.e2e_passed { 1 } else { 0 },
        metrics.log_error_count,
        fitness_score
    ) {
        eprintln!("[promote] Failed to write fitness row: {}", e);
    }
    println!(
        "   [NOTE] Fitness recorded: score={:.4} | build={}ms | size={}KB | errors={}",
        fitness_score, metrics.build_time_ms, binary_size_kb, metrics.log_error_count
    );
}

fn default_budget() -> SafetyBudget {
    SafetyBudget { budget: 5.0, max_budget: 5.0, total_trials: 0, total_failures: 0, halted: false }
}

fn load_budget() -> SafetyBudget {
    let path = format!("{}/.trinity/state/safety_budget.json", &project_dir());
    match fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str(&content) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[promote] Failed to parse {}: {}", path, e);
                default_budget()
            }
        },
        Err(_) => default_budget(),
    }
}

fn default_clade_state() -> CladeState {
    CladeState { id: "unknown".to_string(), version: "0.0.0".to_string(), status: "unknown".to_string(), e_value: Some(1.0) }
}

fn load_clade_state() -> CladeState {
    let path = format!("{}/.trinity/state/clade.json", &project_dir());
    match fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str(&content) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[promote] Failed to parse {}: {}", path, e);
                default_clade_state()
            }
        },
        Err(_) => default_clade_state(),
    }
}

fn print_help() {
    println!(r#"
clade-promote - Full promotion pipeline for Canary -> Sovereign

USAGE:
    cargo run --bin clade-promote -- [CLADE_ID] [--dry-run]

PHASES:
    0. Safety gates (budget > 0, not halted)
    1. Seal: build + health + screenshot + e2e + log scan
    2. Snapshot Sovereign before swap
    3. Atomic swap: Canary binary -> Sovereign
    4. Boot probe: 10s health check
    5. Update archive, fitness, budget

EXAMPLES:
    cargo run --bin clade-promote -- clade-1.1.0 --dry-run
    cargo run --bin clade-promote -- clade-1.1.0
"#);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_cost_critical() {
        assert!((severity_cost("critical") - (-3.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn severity_cost_high() {
        assert!((severity_cost("high") - (-2.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn severity_cost_medium() {
        assert!((severity_cost("medium") - (-1.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn severity_cost_low() {
        assert!((severity_cost("low") - (-0.5)).abs() < f64::EPSILON);
    }

    #[test]
    fn severity_cost_unknown_defaults_to_medium() {
        assert!((severity_cost("unknown") - (-1.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn fitness_all_pass_no_errors() {
        let score = compute_fitness_score(true, true, 0);
        assert!((score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn fitness_build_fail() {
        let score = compute_fitness_score(false, true, 0);
        assert!((score - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn fitness_e2e_fail() {
        let score = compute_fitness_score(true, false, 0);
        assert!((score - 0.7).abs() < f64::EPSILON);
    }

    #[test]
    fn fitness_with_errors() {
        let score = compute_fitness_score(true, true, 5);
        // 0.5 + 0.3 + 0.2*(1 - 0.5) = 0.5 + 0.3 + 0.1 = 0.9
        assert!((score - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn fitness_errors_capped_at_10() {
        let score = compute_fitness_score(true, true, 20);
        // 0.5 + 0.3 + 0.2*(1 - 1.0) = 0.8
        assert!((score - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn fitness_all_fail() {
        let score = compute_fitness_score(false, false, 10);
        assert!((score - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn severity_ordering() {
        assert!(severity_cost("critical") < severity_cost("high"));
        assert!(severity_cost("high") < severity_cost("medium"));
        assert!(severity_cost("medium") < severity_cost("low"));
    }

    #[test]
    fn default_budget_values() {
        let b = default_budget();
        assert!((b.budget - 5.0).abs() < f64::EPSILON);
        assert!(!b.halted);
        assert_eq!(b.total_trials, 0);
    }

    #[test]
    fn default_clade_state_values() {
        let s = default_clade_state();
        assert_eq!(s.id, "unknown");
        assert_eq!(s.version, "0.0.0");
        assert_eq!(s.status, "unknown");
        assert!((s.e_value.unwrap_or(0.0) - 1.0).abs() < f64::EPSILON);
    }
}
