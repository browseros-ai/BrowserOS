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

/// Create GitHub issues for critical/warning findings that don't already have one.
/// Uses local `.trinity/state/auto_issues.json` to track created fingerprints.
fn create_issues(report: &AuditReport, dry_run: bool) -> usize {
    use reqwest::blocking::Client;

    println!("[Step 4/7] Creating GitHub issues for findings...");
    let token = std::env::var("GITHUB_TOKEN").unwrap_or_default();
    if token.is_empty() {
        println!("   ⚠️  GITHUB_TOKEN not set — skipping issue creation");
        return 0;
    }

    let client = Client::new();
    let repo = "trios"; // TODO: read from .trinity/state/github.json
    let mut created = 0;

    let state_path = format!("{}/.trinity/state/auto_issues.json", PROJECT_DIR);
    let mut known: Vec<String> = fs::read_to_string(&state_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let checks = [
        &report.security_check,
        &report.shell_safety_check,
        &report.error_handling_check,
        &report.concurrency_check,
        &report.retain_cycle_check,
    ];

    for check in &checks {
        for finding in &check.findings {
            if finding.severity != "critical" && finding.severity != "warning" {
                continue;
            }
            if known.contains(&finding.fingerprint) {
                continue;
            }

            let title = format!("[auto-audit] {} — {}", finding.category, finding.message);
            let body = format!(
                "**File:** `{}`\n**Line:** {}\n**Severity:** {}\n**Fingerprint:** `{}`\n\n_Automatically generated by clade-audit._",
                finding.file, finding.line, finding.severity, finding.fingerprint
            );

            if dry_run {
                println!("   [DRY-RUN] Would create issue: {}", title);
                known.push(finding.fingerprint.clone());
                created += 1;
                continue;
            }

            let url = format!("https://api.github.com/repos/gHashTag/{}/issues", repo);
            let payload = serde_json::json!({
                "title": title,
                "body": body,
                "labels": ["auto-audit"],
            });

            let response = client
                .post(&url)
                .header("Authorization", format!("Bearer {}", token))
                .header("Accept", "application/vnd.github.v3+json")
                .header("User-Agent", "clade-tablecloth")
                .json(&payload)
                .send();

            match response {
                Ok(resp) => {
                    if resp.status().is_success() {
                        println!("   ✅ Issue created: {}", title);
                        known.push(finding.fingerprint.clone());
                        created += 1;
                    } else {
                        println!("   ❌ Issue creation failed: {:?}", resp.status());
                        log_event("issue_create_fail", &format!("{} {:?}", title, resp.status()));
                    }
                }
                Err(e) => {
                    println!("   ❌ Network error creating issue: {}", e);
                    log_event("issue_network_fail", &e.to_string());
                }
            }
        }
    }

    let _ = fs::create_dir_all(format!("{}/.trinity/state", PROJECT_DIR));
    let _ = fs::write(&state_path, serde_json::to_string_pretty(&known).unwrap_or_default());

    println!("   Issues created: {}", created);
    created
}

/// Attempt auto-fix for error-handling findings (try! → try?).
/// Creates branch, applies regex replacement, runs typecheck subset.
fn attempt_fix(report: &AuditReport, dry_run: bool) -> (usize, usize) {
    use regex::Regex;
    println!("[Step 5/7] Attempting auto-fixes...");

    let mut attempted = 0;
    let mut passed = 0;

    if dry_run {
        println!("   [DRY-RUN] Would attempt fixes for {} findings", report.error_handling_check.findings.len());
        return (0, 0);
    }

    let try_bang = Regex::new(r"try!\s*\(").unwrap();
    let branch_prefix = format!("auto-fix/{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"));

    for finding in &report.error_handling_check.findings {
        if finding.message != "Bare try! — use try? or do-catch" {
            continue;
        }
        let file_path = format!("{}/{}", PROJECT_DIR, finding.file);
        let content = match fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        if !try_bang.is_match(&content) {
            continue;
        }

        attempted += 1;
        let branch = format!("{}-{}", branch_prefix, attempted);

        // Create branch
        let _ = Command::new("git")
            .args(["checkout", "-b", &branch])
            .current_dir(PROJECT_DIR)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        // Apply fix
        let fixed = try_bang.replace_all(&content, "try?(");
        if let Err(e) = fs::write(&file_path, fixed.as_ref()) {
            println!("   ❌ Failed to write fix: {}", e);
            let _ = Command::new("git")
                .args(["checkout", "-"])
                .current_dir(PROJECT_DIR)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            continue;
        }

        // Run subset build
        let build_ok = Command::new("swiftc")
            .args(["-typecheck", &file_path])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if !build_ok {
            println!("   ❌ Fix broke build on {} — discarding branch {}", finding.file, branch);
            let _ = Command::new("git")
                .args(["checkout", "-"])
                .current_dir(PROJECT_DIR)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = Command::new("git")
                .args(["branch", "-D", &branch])
                .current_dir(PROJECT_DIR)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            // Restore file
            let _ = Command::new("git")
                .args(["checkout", "--", &finding.file])
                .current_dir(PROJECT_DIR)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            continue;
        }

        // Commit and push
        let _ = Command::new("git")
            .args(["add", &finding.file])
            .current_dir(PROJECT_DIR)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("git")
            .args(["commit", "-m", &format!("auto-fix: {} → try? in {}", finding.message, finding.file)])
            .current_dir(PROJECT_DIR)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("git")
            .args(["push", "-u", "origin", &branch])
            .current_dir(PROJECT_DIR)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        println!("   ✅ Fix passed build: {} on branch {}", finding.file, branch);
        passed += 1;
    }

    println!("   Fixes attempted: {} | passed: {}", attempted, passed);
    (attempted, passed)
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
    let report = run_audit();

    // Step 3: Update awareness
    update_awareness(dry_run);

    // Step 4: Create issues
    let issues_created = if let Some(ref r) = report {
        create_issues(r, dry_run)
    } else {
        0
    };

    // Step 5: Attempt fixes
    let (fixes_attempted, fixes_passed) = if let Some(ref r) = report {
        attempt_fix(r, dry_run)
    } else {
        (0, 0)
    };

    // Step 6: Write report
    write_report(&budget, issues_created, fixes_attempted, fixes_passed, 0, dry_run);

    println!("\n📊 Summary: {} issues created, {} fixes attempted, {} fixes passed", issues_created, fixes_attempted, fixes_passed);
}

/// Write `.trinity/state/last_improvement.json` with loop summary.
fn write_report(
    budget: &SafetyBudget,
    issues_created: usize,
    fixes_attempted: usize,
    fixes_passed: usize,
    prs_created: usize,
    dry_run: bool,
) {
    println!("[Step 6/7] Writing improvement report...");
    let mode = if budget.halted || budget.budget <= 0.0 {
        "audit-only"
    } else {
        "full"
    };

    let report = ImprovementReport {
        timestamp: Utc::now().to_rfc3339(),
        budget_before: budget.budget,
        budget_after: budget.budget,
        findings_total: issues_created,
        issues_created,
        fixes_attempted,
        fixes_passed,
        prs_created,
        mode: mode.to_string(),
    };

    let json = serde_json::to_string_pretty(&report).unwrap_or_default();
    let path = format!("{}/.trinity/state/last_improvement.json", PROJECT_DIR);

    if dry_run {
        println!("   [DRY-RUN] Would write report to {}", path);
        println!("   {}", json);
        return;
    }

    let _ = fs::create_dir_all(format!("{}/.trinity/state", PROJECT_DIR));
    match fs::write(&path, &json) {
        Ok(_) => println!("   ✅ Report written: {}", path),
        Err(e) => {
            println!("   ❌ Failed to write report: {}", e);
            log_event("report_write_fail", &e.to_string());
        }
    }
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
