use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::process::{Command, Stdio};
use std::time::Instant;

const PROJECT_DIR: &str = "/Users/playra/BrowserOS-full/trios";

#[derive(Serialize, Deserialize, Debug)]
struct SafetyBudget {
    budget: f64,
    max_budget: f64,
    total_trials: u64,
    total_failures: u64,
    halted: bool,
}

#[derive(Serialize, Deserialize, Debug)]
struct AuditFinding {
    file: String,
    line: u32,
    severity: String,
    category: String,
    message: String,
    fingerprint: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct AuditReport {
    build_check: BuildCheck,
    security_check: CheckResult,
    shell_safety_check: CheckResult,
    error_handling_check: CheckResult,
    concurrency_check: CheckResult,
    todo_check: CheckResult,
    unused_code_check: CheckResult,
    retain_cycle_check: CheckResult,
}

#[derive(Serialize, Deserialize, Debug)]
struct BuildCheck {
    passed: bool,
    swift_ok: bool,
    rust_ok: bool,
    swift_errors: Vec<String>,
    rust_errors: Vec<String>,
    duration_ms: u128,
}

#[derive(Serialize, Deserialize, Debug)]
struct CheckResult {
    passed: bool,
    findings: Vec<AuditFinding>,
    scanned_files: usize,
    duration_ms: u128,
}

#[derive(Serialize, Deserialize, Debug)]
struct ImprovementReport {
    timestamp: String,
    budget_before: f64,
    budget_after: f64,
    findings_total: usize,
    issues_created: usize,
    fixes_attempted: usize,
    fixes_passed: usize,
    prs_created: usize,
    mode: String,
}

/// Load safety budget from `.trinity/state/safety_budget.json`.
fn load_budget() -> SafetyBudget {
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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dry_run = args.iter().any(|a| a == "--dry-run");

    println!("═══════════════════════════════════════════════════════════");
    println!("  CLADE-TABLECLOTH: Autonomous Self-Improvement Loop");
    println!("  Dry run: {}", dry_run);
    println!("═══════════════════════════════════════════════════════════\n");

    // Step 1: Load budget
    let budget = load_budget();
    println!("[Step 1/7] Budget: {}/{} | halted={}", budget.budget, budget.max_budget, budget.halted);

    if budget.halted || budget.budget <= 0.0 {
        println!("🔴 HALTED: budget depleted or manually halted — stopping loop");
        log_event("loop_halted_budget", &format!("budget={}", budget.budget));
        std::process::exit(0);
    }

    println!("\n🟢 Safety gates passed — continuing loop");
}

fn log_event(event: &str, details: &str) {
    let ts = Utc::now().to_rfc3339();
    let line = format!(
        r#"{{"timestamp":"{}","event":"{}","details":"{}"}}"#,
        ts, event, details
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
