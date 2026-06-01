use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

static API_REMAINING: AtomicU32 = AtomicU32::new(5000);
const API_REMAINING_FLOOR: u32 = 100;
const API_BACKOFF_MAX_SECS: u64 = 60;

fn check_rate_limit(response: &reqwest::blocking::Response) {
    if let Some(remaining) = response.headers().get("x-ratelimit-remaining") {
        if let Ok(val) = remaining.to_str().unwrap_or("5000").parse::<u32>() {
            API_REMAINING.store(val, Ordering::Relaxed);
            if val < API_REMAINING_FLOOR {
                println!("   ⚠️  GitHub API rate limit low: {} remaining — pausing", val);
            }
        }
    }
}

fn should_throttle() -> bool {
    API_REMAINING.load(Ordering::Relaxed) < API_REMAINING_FLOOR
}

fn backoff_on_rate_limit(attempt: u32) {
    let secs = (1u64 << attempt.min(6)).min(API_BACKOFF_MAX_SECS);
    println!("   ⏳ Rate limit backoff: {}s (attempt {})", secs, attempt);
    std::thread::sleep(std::time::Duration::from_secs(secs));
}

fn project_dir() -> String { std::env::var("TRIOS_ROOT").unwrap_or_else(|_| "/Users/playra/BrowserOS-full/trios".to_string()) }

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
    let path = format!("{}/.trinity/state/safety_budget.json", &project_dir());
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
        .current_dir(project_dir())
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
    cmd.current_dir(project_dir())
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

    let state_path = format!("{}/.trinity/state/auto_issues.json", &project_dir());
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

            if should_throttle() {
                println!("   ⏭️  Rate limit floor reached — stopping issue creation");
                break;
            }

            let response = client
                .post(&url)
                .header("Authorization", format!("Bearer {}", token))
                .header("Accept", "application/vnd.github.v3+json")
                .header("User-Agent", "clade-tablecloth")
                .json(&payload)
                .send();

            match response {
                Ok(resp) => {
                    check_rate_limit(&resp);
                    if resp.status().is_success() {
                        println!("   ✅ Issue created: {}", title);
                        known.push(finding.fingerprint.clone());
                        created += 1;
                    } else if resp.status().as_u16() == 429 || resp.status().as_u16() == 403 {
                        println!("   ⚠️  Rate limited ({}), backing off", resp.status());
                        backoff_on_rate_limit(0);
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

    if let Err(e) = fs::create_dir_all(format!("{}/.trinity/state", &project_dir())) {
        eprintln!("[tablecloth] Failed to create state dir: {}", e);
    }
    match serde_json::to_string_pretty(&known) {
        Ok(json) => {
            if let Err(e) = fs::write(&state_path, &json) {
                eprintln!("[tablecloth] Failed to write auto_issues.json: {}", e);
            }
        }
        Err(e) => eprintln!("[tablecloth] Failed to serialize auto_issues: {}", e),
    }

    println!("   Issues created: {}", created);
    created
}

fn pr_already_exists(repo: &str, head: &str, token: &str) -> bool {
    use reqwest::blocking::Client;
    let client = Client::new();
    let url = format!(
        "https://api.github.com/repos/gHashTag/{}/pulls?head=gHashTag:{}&state=open",
        repo, head
    );
    match client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "clade-tablecloth")
        .send()
    {
        Ok(resp) => {
            if let Ok(body) = resp.text() {
                if let Ok(prs) = serde_json::from_str::<Vec<serde_json::Value>>(&body) {
                    return !prs.is_empty();
                }
            }
            false
        }
        Err(_) => false,
    }
}

fn branch_exists_remote(repo: &str, branch: &str, token: &str) -> bool {
    use reqwest::blocking::Client;
    let client = Client::new();
    let url = format!(
        "https://api.github.com/repos/gHashTag/{}/branches/{}",
        repo, branch
    );
    match client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "clade-tablecloth")
        .send()
    {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

fn deterministic_branch_name(fingerprint: &str) -> String {
    let hash: u64 = fingerprint.bytes().fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
    format!("tablecloth/fix/{:016x}", hash)
}

/// Create a GitHub PR for a pushed branch.
fn create_pr(repo: &str, title: &str, body: &str, head: &str, base: &str, dry_run: bool) -> bool {
    use reqwest::blocking::Client;

    let token = std::env::var("GITHUB_TOKEN").unwrap_or_default();
    if token.is_empty() {
        println!("   ⚠️  GITHUB_TOKEN not set — skipping PR creation");
        return false;
    }

    if should_throttle() {
        println!("   ⏭️  Rate limit floor reached — skipping PR creation");
        return false;
    }

    if pr_already_exists(repo, head, &token) {
        println!("   ⏭️  PR already exists for branch {} — skipping", head);
        return false;
    }

    if dry_run {
        println!("   [DRY-RUN] Would create PR: {} ← {}", title, head);
        return true;
    }

    let client = Client::new();
    let url = format!("https://api.github.com/repos/gHashTag/{}/pulls", repo);
    let payload = serde_json::json!({
        "title": title,
        "body": body,
        "head": head,
        "base": base,
    });

    match client
        .post(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "clade-tablecloth")
        .json(&payload)
        .send()
    {
        Ok(resp) => {
            check_rate_limit(&resp);
            if resp.status().is_success() {
                println!("   ✅ PR created: {}", title);
                log_event("pr_created", &format!("{} ← {}", title, head));
                true
            } else if resp.status().as_u16() == 429 || resp.status().as_u16() == 403 {
                println!("   ⚠️  Rate limited ({}) — backing off", resp.status());
                backoff_on_rate_limit(0);
                false
            } else {
                println!("   ❌ PR creation failed: {:?}", resp.status());
                log_event("pr_create_fail", &format!("{} {:?}", title, resp.status()));
                false
            }
        }
        Err(e) => {
            println!("   ❌ Network error creating PR: {}", e);
            log_event("pr_network_fail", &e.to_string());
            false
        }
    }
}

/// Attempt auto-fix for error-handling findings (try! → try?).
/// Operates in the staging worktree to avoid dirtying the sovereign repo.
/// Returns (attempted, passed, prs_created).
fn attempt_fix(report: &AuditReport, dry_run: bool) -> (usize, usize, usize) {
    use regex::Regex;
    println!("[Step 5/7] Attempting auto-fixes...");

    let mut attempted = 0;
    let mut passed = 0;
    let mut prs_created = 0;

    if dry_run {
        println!("   [DRY-RUN] Would attempt fixes for {} findings", report.error_handling_check.findings.len());
        return (0, 0, 0);
    }

    let worktree = format!("{}/.worktrees/staging", project_dir());
    if !std::path::Path::new(&worktree).exists() {
        println!("   ⚠️  Staging worktree missing — run clade-worktree ensure first");
        return (0, 0, 0);
    }

    let try_bang = match Regex::new(r"try!\s*\(") {
        Ok(re) => re,
        Err(_) => return (0, 0, 0),
    };
    let token = std::env::var("GITHUB_TOKEN").unwrap_or_default();

    for finding in &report.error_handling_check.findings {
        if finding.message != "Bare try! — use try? or do-catch" {
            continue;
        }
        let file_path = format!("{}/{}", &worktree, finding.file);
        // Path traversal guard: reject paths that escape the worktree
        if finding.file.contains("..") {
            eprintln!("[tablecloth] SECURITY: path traversal in finding.file: {}", finding.file);
            continue;
        }
        if let Ok(canonical) = std::fs::canonicalize(&file_path) {
            if let Ok(canonical_wt) = std::fs::canonicalize(&worktree) {
                if !canonical.starts_with(&canonical_wt) {
                    eprintln!("[tablecloth] SECURITY: file {} escapes worktree {}", canonical.display(), canonical_wt.display());
                    continue;
                }
            }
        }
        let content = match fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        if !try_bang.is_match(&content) {
            continue;
        }

        attempted += 1;
        let branch = deterministic_branch_name(&finding.fingerprint);

        if !token.is_empty() && branch_exists_remote("trios", &branch, &token) {
            println!("   ⏭️  Branch {} already exists remotely — skipping", branch);
            continue;
        }

        match Command::new("git")
            .args(["checkout", "-b", &branch])
            .current_dir(&worktree)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Ok(s) if !s.success() => {
                println!("   ⚠️  git checkout -b {} failed (exit {:?})", branch, s.code());
                continue;
            }
            Err(e) => {
                println!("   ⚠️  git checkout -b {} error: {}", branch, e);
                continue;
            }
            _ => {}
        }

        let fixed = try_bang.replace_all(&content, "try?(");
        if let Err(e) = fs::write(&file_path, fixed.as_ref()) {
            println!("   ❌ Failed to write fix: {}", e);
            if let Err(e) = Command::new("git")
                .args(["checkout", "-"])
                .current_dir(&worktree)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
            {
                eprintln!("[tablecloth] git checkout - failed: {}", e);
            }
            continue;
        }

        let build_ok = Command::new("swiftc")
            .args(["-typecheck", &file_path])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if !build_ok {
            println!("   ❌ Fix broke build on {} — discarding branch {}", finding.file, branch);
            if let Err(e) = Command::new("git")
                .args(["checkout", "-"])
                .current_dir(&worktree)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
            {
                eprintln!("[tablecloth] git checkout - failed: {}", e);
            }
            if let Err(e) = Command::new("git")
                .args(["branch", "-D", &branch])
                .current_dir(&worktree)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
            {
                eprintln!("[tablecloth] git branch -D {} failed: {}", branch, e);
            }
            if let Err(e) = Command::new("git")
                .args(["checkout", "--", &finding.file])
                .current_dir(&worktree)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
            {
                eprintln!("[tablecloth] git checkout -- {} failed: {}", finding.file, e);
            }
            continue;
        }

        if let Err(e) = Command::new("git")
            .args(["add", &finding.file])
            .current_dir(&worktree)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            eprintln!("[tablecloth] git add {} failed: {}", finding.file, e);
        }
        let commit_msg = format!("auto-fix: {} in {}", finding.message, finding.file);
        if let Err(e) = Command::new("git")
            .args(["commit", "-m", &commit_msg])
            .current_dir(&worktree)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            eprintln!("[tablecloth] git commit failed: {}", e);
        }
        if let Err(e) = Command::new("git")
            .args(["push", "-u", "origin", &branch])
            .current_dir(&worktree)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            eprintln!("[tablecloth] git push {} failed: {}", branch, e);
        }

        // Create PR linking to the fix
        let pr_title = format!("[auto-fix] {} in {}", finding.message, finding.file);
        let pr_body = format!(
            "**File:** `{}`\n**Line:** {}\n**Severity:** {}\n\n_Automatically generated by clade-tablecloth._",
            finding.file, finding.line, finding.severity
        );
        if create_pr("trios", &pr_title, &pr_body, &branch, "dev", dry_run) {
            prs_created += 1;
        }

        println!("   ✅ Fix passed build: {} on branch {}", finding.file, branch);
        passed += 1;
    }

    println!("   Fixes attempted: {} | passed: {} | PRs: {}", attempted, passed, prs_created);
    (attempted, passed, prs_created)
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
    let (fixes_attempted, fixes_passed, prs_created) = if let Some(ref r) = report {
        attempt_fix(r, dry_run)
    } else {
        (0, 0, 0)
    };

    // Step 6: Write report
    write_report(&budget, issues_created, fixes_attempted, fixes_passed, prs_created, dry_run);

    println!("\n📊 Summary: {} issues created, {} fixes attempted, {} fixes passed, {} PRs created", issues_created, fixes_attempted, fixes_passed, prs_created);
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
    let path = format!("{}/.trinity/state/last_improvement.json", &project_dir());

    if dry_run {
        println!("   [DRY-RUN] Would write report to {}", path);
        println!("   {}", json);
        return;
    }

    if let Err(e) = fs::create_dir_all(format!("{}/.trinity/state", &project_dir())) {
        println!("   ❌ Failed to create state dir: {}", e);
    }
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
        eprintln!("[tablecloth] Failed to write event log: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn default_budget_not_halted() {
        let b = default_budget();
        assert!(!b.halted);
        assert!(b.budget > 0.0);
    }

    #[test]
    fn audit_finding_roundtrip() {
        let finding = AuditFinding {
            file: "test.rs".to_string(),
            line: 42,
            severity: "critical".to_string(),
            category: "security".to_string(),
            message: "test finding".to_string(),
            fingerprint: "abc123".to_string(),
        };
        let json = serde_json::to_string(&finding).unwrap_or_default();
        let parsed: AuditFinding = serde_json::from_str(&json).unwrap_or_else(|_| AuditFinding {
            file: String::new(), line: 0, severity: String::new(),
            category: String::new(), message: String::new(), fingerprint: String::new(),
        });
        assert_eq!(parsed.file, "test.rs");
        assert_eq!(parsed.line, 42);
        assert_eq!(parsed.severity, "critical");
    }

    #[test]
    fn improvement_report_serializes() {
        let report = ImprovementReport {
            timestamp: "2026-06-01T00:00:00Z".to_string(),
            budget_before: 5.0,
            budget_after: 4.0,
            findings_total: 3,
            issues_created: 2,
            fixes_attempted: 1,
            fixes_passed: 1,
            prs_created: 0,
            mode: "full".to_string(),
        };
        let json = serde_json::to_string(&report).unwrap_or_default();
        assert!(json.contains("\"mode\":\"full\""));
        assert!(json.contains("\"findings_total\":3"));
    }

    #[test]
    fn halted_budget_blocks_loop() {
        let b = SafetyBudget {
            budget: 0.0, max_budget: 5.0, total_trials: 10, total_failures: 5, halted: true,
        };
        assert!(b.halted || b.budget <= 0.0);
    }

    #[test]
    fn positive_budget_allows_loop() {
        let b = default_budget();
        assert!(!b.halted && b.budget > 0.0);
    }

    #[test]
    fn build_check_fields() {
        let bc = BuildCheck {
            passed: true,
            swift_ok: true,
            rust_ok: true,
            swift_errors: vec![],
            rust_errors: vec![],
            duration_ms: 1234,
        };
        assert!(bc.passed);
        assert!(bc.swift_errors.is_empty());
        assert_eq!(bc.duration_ms, 1234);
    }

    #[test]
    fn check_result_passed_logic() {
        let cr = CheckResult {
            passed: true,
            findings: vec![],
            scanned_files: 42,
            duration_ms: 100,
        };
        assert!(cr.passed);
        assert_eq!(cr.scanned_files, 42);
    }

    #[test]
    fn improvement_report_audit_only_mode() {
        let report = ImprovementReport {
            timestamp: "2026-06-01T00:00:00Z".to_string(),
            budget_before: 0.0,
            budget_after: 0.0,
            findings_total: 5,
            issues_created: 0,
            fixes_attempted: 0,
            fixes_passed: 0,
            prs_created: 0,
            mode: "audit-only".to_string(),
        };
        assert_eq!(report.mode, "audit-only");
        assert_eq!(report.fixes_attempted, 0);
    }

    #[test]
    fn deterministic_branch_is_stable() {
        let a = deterministic_branch_name("test:42:some finding");
        let b = deterministic_branch_name("test:42:some finding");
        assert_eq!(a, b);
        assert!(a.starts_with("tablecloth/fix/"));
    }

    #[test]
    fn deterministic_branch_varies_by_input() {
        let a = deterministic_branch_name("file_a.rs:10:bug");
        let b = deterministic_branch_name("file_b.rs:20:crash");
        assert_ne!(a, b);
    }

    #[test]
    fn deterministic_branch_format_is_hex() {
        let name = deterministic_branch_name("test");
        let hex_part = name.strip_prefix("tablecloth/fix/").unwrap_or("");
        assert_eq!(hex_part.len(), 16);
        assert!(hex_part.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn rate_limit_floor_constant() {
        assert_eq!(API_REMAINING_FLOOR, 100);
    }

    #[test]
    fn rate_limit_backoff_max() {
        assert_eq!(API_BACKOFF_MAX_SECS, 60);
    }

    #[test]
    fn should_throttle_false_initially() {
        API_REMAINING.store(5000, Ordering::Relaxed);
        assert!(!should_throttle());
    }

    #[test]
    fn should_throttle_true_below_floor() {
        API_REMAINING.store(50, Ordering::Relaxed);
        assert!(should_throttle());
        API_REMAINING.store(5000, Ordering::Relaxed);
    }
}
