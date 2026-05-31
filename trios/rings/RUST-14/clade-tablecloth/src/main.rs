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

/// Run clade-audit --json and parse the structured report.
fn run_audit() -> Option<AuditReport> {
    println!("[Step 2/7] Running clade-audit...");
    let start = Instant::now();
    let output = Command::new("cargo")
        .args(["run", "--bin", "clade-audit", "--", "--json"])
        .current_dir(PROJECT_DIR)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            // Extract JSON from stdout (audit prints banner then JSON)
            let json_start = stdout.find('{').unwrap_or(0);
            let json_str = &stdout[json_start..];
            match serde_json::from_str::<AuditReport>(json_str) {
                Ok(report) => {
                    let total: usize = [
                        &report.security_check,
                        &report.shell_safety_check,
                        &report.error_handling_check,
                        &report.concurrency_check,
                        &report.todo_check,
                        &report.unused_code_check,
                        &report.retain_cycle_check,
                    ].iter().map(|c| c.findings.len()).sum();
                    println!("   ✅ Audit complete: {} findings | {}ms", total, start.elapsed().as_millis());
                    Some(report)
                }
                Err(e) => {
                    println!("   ❌ Failed to parse audit JSON: {}", e);
                    log_event("audit_parse_fail", &e.to_string());
                    None
                }
            }
        }
        Err(e) => {
            println!("   ❌ Failed to run clade-audit: {}", e);
            log_event("audit_run_fail", &e.to_string());
            None
        }
    }
}

/// Update `.trinity/self-awareness.json` via clade-audit generate-awareness.
fn update_awareness(dry_run: bool) {
    println!("[Step 3/7] Updating self-awareness...");
    let mut cmd = Command::new("cargo");
    cmd.args(["run", "--bin", "clade-audit", "--", "generate-awareness"]);
    if dry_run {
        cmd.arg("--dry-run");
    }
    cmd.current_dir(PROJECT_DIR)
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    match cmd.status() {
        Ok(status) => {
            if status.success() {
                println!("   ✅ Self-awareness updated");
                log_event("awareness_updated", "");
            } else {
                println!("   ⚠️  Self-awareness exited with code {:?}", status.code());
                log_event("awareness_exit_code", &format!("{:?}", status.code()));
            }
        }
        Err(e) => {
            println!("   ❌ Failed to update awareness: {}", e);
            log_event("awareness_fail", &e.to_string());
        }
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

    // Step 2: Run audit
    let _report = run_audit();

    // Step 3: Update awareness
    update_awareness(dry_run);
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
