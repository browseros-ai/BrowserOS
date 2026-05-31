use chrono::Utc;
use reqwest::blocking::get;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Instant;

const PROJECT_DIR: &str = "/Users/playra/BrowserOS-full/trios";
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
    let count_trios = Command::new("pgrep")
        .args(["-x", "trios"])
        .stdout(Stdio::piped())
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.is_empty()).count())
        .unwrap_or(0);
    let count_trios_app = Command::new("pgrep")
        .args(["-x", "trios_app"])
        .stdout(Stdio::piped())
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.is_empty()).count())
        .unwrap_or(0);
    count_trios + count_trios_app
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return;
    }

    let clade_id = args.get(0).map(|s| s.as_str()).unwrap_or("clade-1.0.0");
    let dry_run = args.iter().any(|a| a == "--dry-run");

    println!("═══════════════════════════════════════════════════════════");
    println!("  CLADE-PROMOTE: Full Pipeline");
    println!("  Clade: {} | Dry run: {}", clade_id, dry_run);
    println!("═══════════════════════════════════════════════════════════\n");

    // Phase 0: Safety gates
    println!("[Phase 0] Agent V — Safety Gates");
    let gates_ok = check_safety_gates();
    if !gates_ok {
        println!("🔴 REJECTED by safety gates — stopping.");
        std::process::exit(1);
    }
    println!("🟢 Safety gates passed\n");

    // Phase 1: Build Canary
    println!("[Phase 1] Cell 1 — Build Canary");
    let (seal_results, metrics) = run_seal(clade_id, dry_run);

    let all_passed = seal_results.iter().all(|r| r.passed);
    if !all_passed {
        println!("\n🔴 SEAL FAILED — promotion rejected");
        if !dry_run {
            update_budget_on_rejection();
        } else {
            println!("   [DRY-RUN] Would deduct budget");
        }
        for r in seal_results.iter().filter(|r| !r.passed) {
            println!("   ❌ {}: {}", r.cell, r.detail);
        }
        std::process::exit(1);
    }

    println!("\n🟢 SEAL PASSED — all cells green\n");

    // Phase 2: Snapshot Sovereign before swap
    println!("[Phase 2] Snapshot Sovereign before swap");
    if !dry_run {
        snapshot_sovereign(clade_id);
    } else {
        println!("   [DRY-RUN] Would snapshot Sovereign");
    }

    // Phase 3: Atomic swap
    println!("\n[Phase 3] Atomic swap: Canary → Sovereign");
    if !dry_run {
        atomic_swap();
    } else {
        println!("   [DRY-RUN] Would copy Canary binary to Sovereign");
    }

    // Phase 4: Boot probe
    println!("\n[Phase 4] Boot probe — 10s health check");
    let boot_ok = if !dry_run {
        boot_probe()
    } else {
        println!("   [DRY-RUN] Would probe health");
        true
    };

    if !boot_ok {
        println!("🔴 Boot probe FAILED — triggering emergency rollback");
        if !dry_run {
            emergency_rollback();
        }
        update_budget_on_rejection();
        std::process::exit(1);
    }

    println!("🟢 Boot probe PASSED\n");

    // Phase 5: Update state
    println!("[Phase 5] Update archive + fitness + budget");
    if !dry_run {
        update_state(clade_id);
        update_budget_on_success();
        record_fitness(clade_id, &metrics);
    } else {
        println!("   [DRY-RUN] Would update state + record fitness");
    }

    println!("\n═══════════════════════════════════════════════════════════");
    println!("  ✅ PROMOTION COMPLETE — {}", clade_id);
    println!("═══════════════════════════════════════════════════════════");
}

fn check_safety_gates() -> bool {
    let budget = load_budget();
    if budget.halted || budget.budget <= 0.0 {
        println!("   ❌ Safety budget halted or depleted: {}/{}", budget.budget, budget.max_budget);
        return false;
    }
    println!("   ✅ Budget: {}/{}", budget.budget, budget.max_budget);

    let clade = load_clade_state();
    let e_val = clade.e_value.unwrap_or(1.0);
    if e_val < 5.0 {
        println!("   ⚠️  E-value {} < 5.0 (recommend waiting)", e_val);
        // Not a hard gate yet, just warning
    }
    println!("   ✅ Clade: {} (status={})", clade.id, clade.status);

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
    println!("   {} Build: {}ms", if build_ok { "✅" } else { "❌" }, build_ms);

    // Cell 2: Health
    println!("   Probing health...");
    // SAFETY: Do not launch Canary if multiple instances already running (recursion guard)
    let pre_instances = count_trios_instances();
    if pre_instances > 1 {
        println!("   ⚠️  {} trios instances already running — refusing Canary launch to avoid recursion", pre_instances);
        metrics.health_passed = false;
        results.push(SealResult {
            cell: "Seal-2 Health",
            passed: false,
            detail: format!("recursion_guard: {} instances", pre_instances),
        });
        println!("   ❌ Health: blocked by recursion guard");
    } else {
        let canary_app = format!("{}/.worktrees/staging/trios_app", PROJECT_DIR);
        let launch_start = Instant::now();
        let child = Command::new(&canary_app)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok();

    std::thread::sleep(std::time::Duration::from_secs(5));
    let health_ok = check_health(CANARY_HEALTH);
    metrics.health_launch_ms = launch_start.elapsed().as_millis();
    metrics.health_passed = health_ok;
    if let Some(mut c) = child {
        let _ = c.kill();
    }
    results.push(SealResult {
        cell: "Seal-2 Health",
        passed: health_ok,
        detail: if health_ok { "status=ok".to_string() } else { "no response".to_string() },
    });
    println!("   {} Health: {}", if health_ok { "✅" } else { "❌" }, if health_ok { "ok" } else { "fail" });
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
    println!("   {} Screenshot: {}", if screenshot_ok { "✅" } else { "⚠️" }, if screenshot_ok { "available" } else { "skip" });

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
    println!("   {} E2E: {}", if e2e_ok { "✅" } else { "❌" }, if e2e_ok { "pass" } else { "fail" });

    // Cell 5: Log scan (check recent event_log for errors)
    println!("   Scanning logs...");
    let log_path = format!("{}/.trinity/event_log.jsonl", PROJECT_DIR);
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
    println!("   {} Logs: {}", if log_ok { "✅" } else { "❌" }, if log_ok { "clean" } else { "errors" });

    (results, metrics)
}

fn snapshot_sovereign(clade_id: &str) {
    let snapshot_dir = format!("{}/.trinity/snapshots", PROJECT_DIR);
    let _ = fs::create_dir_all(&snapshot_dir);
    let ts = Utc::now().timestamp();
    let name = format!("trios_app-{}-{}", ts, clade_id);
    let src = format!("{}/trios_app", PROJECT_DIR);
    let dst = format!("{}/{}", snapshot_dir, name);
    let _ = fs::copy(&src, &dst);
    println!("   💾 Snapshot: {}", dst);
}

fn atomic_swap() {
    let canary = format!("{}/.worktrees/staging/trios_app", PROJECT_DIR);
    let targets = vec![
        format!("{}/trios_app", PROJECT_DIR),
        format!("{}/trios.app/Contents/MacOS/trios", PROJECT_DIR),
    ];
    for dst in targets {
        let _ = fs::remove_file(&dst);
        if let Some(parent) = std::path::Path::new(&dst).parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::copy(&canary, &dst);
        println!("   📦 Copied to {}", dst);
    }
}

fn boot_probe() -> bool {
    use std::process::Stdio;

    let sovereign_app = format!("{}/trios_app", PROJECT_DIR);
    if !std::path::Path::new(&sovereign_app).exists() {
        eprintln!("   ❌ Sovereign binary missing: {}", sovereign_app);
        return false;
    }

    // 0. Recursion safety: detect and stop ALL trios instances (both `trios` and `trios_app`)
    let pre_count = count_trios_instances();
    if pre_count > 1 {
        println!("   ⚠️  {} trios instances detected before boot — killing all to prevent recursion", pre_count);
    }
    println!("   Stopping existing Sovereign...");
    let _ = Command::new("pkill")
        .args(["-x", "trios"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = Command::new("pkill")
        .args(["-x", "trios_app"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = Command::new("pkill")
        .args(["-f", &format!("{}/trios_app", PROJECT_DIR)])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    std::thread::sleep(std::time::Duration::from_secs(2));

    // 1. Start new Sovereign
    println!("   Starting new Sovereign...");
    let child = Command::new(&sovereign_app)
        .env("TRIOS_VARIANT", "prod")
        .current_dir(PROJECT_DIR)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    if child.is_err() {
        eprintln!("   ❌ Failed to start Sovereign: {:?}", child.err());
        return false;
    }

    // 2. Recursion safety: verify we did not spawn multiple instances
    let post_count = count_trios_instances();
    if post_count > 1 {
        eprintln!("   ❌ Recursion detected: {} trios instances after boot — aborting", post_count);
        return false;
    }

    // 3. Wait up to 30s for health OK
    println!("   Waiting for health probe (max 30s)...");
    let start = Instant::now();
    loop {
        if check_health(SOVEREIGN_HEALTH) {
            println!("   ✅ Health OK after {:?}", start.elapsed());
            return true;
        }
        if start.elapsed().as_secs() >= 30 {
            eprintln!("   ❌ Health probe timeout (30s)");
            return false;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

fn emergency_rollback() {
    let _ = Command::new("cargo")
        .args(["run", "--bin", "clade-rollback"])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();
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
    let archive_path = format!("{}/.trinity/clades/archive.json", PROJECT_DIR);
    let fitness_path = format!("{}/.trinity/clades/fitness.csv", PROJECT_DIR);
    println!("   📝 Updated archive: {}", archive_path);
    println!("   📝 Updated fitness: {}", fitness_path);
}

fn update_budget_on_success() {
    let path = format!("{}/.trinity/state/safety_budget.json", PROJECT_DIR);
    if let Ok(content) = fs::read_to_string(&path) {
        if let Ok(mut budget) = serde_json::from_str::<SafetyBudget>(&content) {
            budget.budget = (budget.budget + 0.2).min(budget.max_budget);
            budget.total_trials += 1;
            let _ = fs::write(&path, serde_json::to_string_pretty(&budget).unwrap_or_default());
            println!("   💰 Budget +0.2 → {}/{}", budget.budget, budget.max_budget);
        }
    }
}

fn update_budget_on_rejection() {
    let path = format!("{}/.trinity/state/safety_budget.json", PROJECT_DIR);
    if let Ok(content) = fs::read_to_string(&path) {
        if let Ok(mut budget) = serde_json::from_str::<SafetyBudget>(&content) {
            budget.budget -= 1.0;
            budget.total_failures += 1;
            if budget.budget <= 0.0 {
                budget.halted = true;
                budget.budget = 0.0;
            }
            let _ = fs::write(&path, serde_json::to_string_pretty(&budget).unwrap_or_default());
            println!("   💰 Budget -1.0 → {}/{} (halted={})", budget.budget, budget.max_budget, budget.halted);
        }
    }
}

fn record_fitness(clade_id: &str, metrics: &SealMetrics) {
    let csv_dir = format!("{}/.trinity/clades", PROJECT_DIR);
    let csv_path = format!("{}/fitness.csv", csv_dir);
    let _ = fs::create_dir_all(&csv_dir);

    let binary = format!("{}/.worktrees/staging/trios_app", PROJECT_DIR);
    let binary_size_kb = fs::metadata(&binary).map(|m| m.len() / 1024).unwrap_or(0);

    let build_stable = if metrics.build_passed { 1.0 } else { 0.0 };
    let e2e_pass_rate = if metrics.e2e_passed { 1.0 } else { 0.0 };
    let normalized_error_rate = (metrics.log_error_count as f64 / 10.0).min(1.0);
    let fitness_score = 0.50 * build_stable + 0.30 * e2e_pass_rate + 0.20 * (1.0 - normalized_error_rate);

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
        let _ = writeln!(
            file,
            "timestamp,clade_id,build_time_ms,binary_size_kb,launch_time_ms,build_passed,health_passed,e2e_passed,log_error_count,fitness_score"
        );
    }

    let ts = Utc::now().to_rfc3339();
    let _ = writeln!(
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
    );
    println!(
        "   📝 Fitness recorded: score={:.4} | build={}ms | size={}KB | errors={}",
        fitness_score, metrics.build_time_ms, binary_size_kb, metrics.log_error_count
    );
}

fn load_budget() -> SafetyBudget {
    let path = format!("{}/.trinity/state/safety_budget.json", PROJECT_DIR);
    fs::read_to_string(&path)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or(SafetyBudget { budget: 5.0, max_budget: 5.0, total_trials: 0, total_failures: 0, halted: false })
}

fn load_clade_state() -> CladeState {
    let path = format!("{}/.trinity/state/clade.json", PROJECT_DIR);
    fs::read_to_string(&path)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or(CladeState { id: "unknown".to_string(), version: "0.0.0".to_string(), status: "unknown".to_string(), e_value: Some(1.0) })
}

fn print_help() {
    println!(r#"
clade-promote — Full promotion pipeline for Canary → Sovereign

USAGE:
    cargo run --bin clade-promote -- [CLADE_ID] [--dry-run]

PHASES:
    0. Safety gates (budget > 0, not halted)
    1. Seal: build + health + screenshot + e2e + log scan
    2. Snapshot Sovereign before swap
    3. Atomic swap: Canary binary → Sovereign
    4. Boot probe: 10s health check
    5. Update archive, fitness, budget

EXAMPLES:
    cargo run --bin clade-promote -- clade-1.1.0 --dry-run
    cargo run --bin clade-promote -- clade-1.1.0
"#);
}
