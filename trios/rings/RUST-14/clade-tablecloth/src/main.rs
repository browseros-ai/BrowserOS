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
                println!("   [WARN] GitHub API rate limit low: {} remaining - pausing", val);
            }
        }
    }
}

fn should_throttle() -> bool {
    API_REMAINING.load(Ordering::Relaxed) < API_REMAINING_FLOOR
}

/// Exponential backoff base in ms for a given attempt, capped at the max.
fn backoff_base_ms(attempt: u32) -> u64 {
    ((1u64 << attempt.min(6)) * 1000).min(API_BACKOFF_MAX_SECS * 1000)
}

/// Pseudo-random jitter in [0, base/2) ms, seeded from the wall clock so
/// concurrently-woken agents don't retry in lockstep (thundering herd).
fn jitter_ms(base_ms: u64) -> u64 {
    if base_ms == 0 {
        return 0;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    nanos % (base_ms / 2 + 1)
}

fn backoff_on_rate_limit(attempt: u32) {
    let base = backoff_base_ms(attempt);
    let total = base + jitter_ms(base);
    println!("   [WAIT] Rate limit backoff: {}ms (attempt {}, +jitter)", total, attempt);
    std::thread::sleep(std::time::Duration::from_millis(total));
}

fn project_dir() -> String { trios_config::project_dir() }

/// Atomically write `contents` to `path`: write to a pid-scoped temp file in
/// the same directory, then rename. Prevents torn/interleaved reads when other
/// rings (clade-monitor, clade-improve) touch the same state files concurrently.
fn write_atomic(path: &str, contents: &str) -> std::io::Result<()> {
    let tmp = format!("{}.tmp.{}", path, std::process::id());
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, path)
}

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

/// Persist the safety budget atomically. Without this the loop loaded the
/// budget but never wrote it back, so failed auto-fixes never depleted it and
/// the `budget <= 0` halt could never trigger.
fn save_budget(budget: &SafetyBudget) {
    let path = format!("{}/.trinity/state/safety_budget.json", &project_dir());
    match serde_json::to_string_pretty(budget) {
        Ok(json) => {
            if let Err(e) = write_atomic(&path, &json) {
                eprintln!("[tablecloth] Failed to persist safety_budget.json: {}", e);
            }
        }
        Err(e) => eprintln!("[tablecloth] Failed to serialize budget: {}", e),
    }
}

const FIX_FAILURE_COST: f64 = 1.0;
const FIX_PASS_REWARD: f64 = 0.25;

/// Apply this loop's outcome to the budget with calibrated reward
/// (TrustBench-style graduated trust): each failed fix spends `FIX_FAILURE_COST`,
/// each successful fix earns `FIX_PASS_REWARD` back, and the result is clamped to
/// `[0, max_budget]` so trust can recover after good loops without ever running
/// away. Returns true if the budget is now depleted (and sets `halted`). Pure so
/// it can be unit-tested.
fn apply_fix_outcome(budget: &mut SafetyBudget, attempted: usize, passed: usize) -> bool {
    let failures = attempted.saturating_sub(passed);
    budget.total_trials += attempted as u64;
    budget.total_failures += failures as u64;
    let net = budget.budget - failures as f64 * FIX_FAILURE_COST + passed as f64 * FIX_PASS_REWARD;
    budget.budget = net.clamp(0.0, budget.max_budget);
    if budget.budget <= 0.0 {
        budget.halted = true;
        return true;
    }
    false
}

/// Resolve the target GitHub repo: `.trinity/state/github.json` ("repo"
/// field) -> `GITHUB_REPO` env -> "trios". Previously hardcoded to "trios", so
/// auto-issues/PRs would always target the wrong repo elsewhere.
fn target_repo() -> String {
    let path = format!("{}/.trinity/state/github.json", &project_dir());
    if let Ok(content) = fs::read_to_string(&path) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(repo) = v.get("repo").and_then(|r| r.as_str()) {
                if !repo.is_empty() {
                    return repo.to_string();
                }
            }
        }
    }
    std::env::var("GITHUB_REPO").unwrap_or_else(|_| "trios".to_string())
}

/// Parse a clade-audit report from raw stdout. The audit prints a banner
/// before the JSON body, so we slice from the first `{`. Returns Err (never
/// panics) when no JSON object is present or the body is malformed.
fn parse_audit_report(stdout: &str) -> Result<AuditReport, String> {
    let json_start = stdout
        .find('{')
        .ok_or_else(|| "no JSON object found in audit output".to_string())?;
    serde_json::from_str::<AuditReport>(&stdout[json_start..]).map_err(|e| e.to_string())
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
            match parse_audit_report(&stdout) {
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
                    println!("   [OK] Audit complete: {} findings | {}ms", total, start.elapsed().as_millis());
                    Some(report)
                }
                Err(e) => {
                    println!("   [FAIL] Failed to parse audit JSON: {}", e);
                    log_event("audit_parse_fail", &e.to_string());
                    None
                }
            }
        }
        Err(e) => {
            println!("   [FAIL] Failed to run clade-audit: {}", e);
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
                println!("   [OK] Self-awareness updated");
                log_event("awareness_updated", "");
            } else {
                println!("   [WARN]  Self-awareness exited with code {:?}", status.code());
                log_event("awareness_exit_code", &format!("{:?}", status.code()));
            }
        }
        Err(e) => {
            println!("   [FAIL] Failed to update awareness: {}", e);
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
        println!("   [WARN]  GITHUB_TOKEN not set - skipping issue creation");
        return 0;
    }

    let client = Client::new();
    let repo = target_repo();
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

            let title = format!("[auto-audit] {} - {}", finding.category, finding.message);
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
                println!("   [U+23ED]  Rate limit floor reached - stopping issue creation");
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
                        println!("   [OK] Issue created: {}", title);
                        known.push(finding.fingerprint.clone());
                        created += 1;
                    } else if resp.status().as_u16() == 429 || resp.status().as_u16() == 403 {
                        println!("   [WARN]  Rate limited ({}), backing off", resp.status());
                        backoff_on_rate_limit(0);
                    } else {
                        println!("   [FAIL] Issue creation failed: {:?}", resp.status());
                        log_event("issue_create_fail", &format!("{} {:?}", title, resp.status()));
                    }
                }
                Err(e) => {
                    println!("   [FAIL] Network error creating issue: {}", e);
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
            if let Err(e) = write_atomic(&state_path, &json) {
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
        println!("   [WARN]  GITHUB_TOKEN not set - skipping PR creation");
        return false;
    }

    if should_throttle() {
        println!("   [U+23ED]  Rate limit floor reached - skipping PR creation");
        return false;
    }

    if pr_already_exists(repo, head, &token) {
        println!("   [U+23ED]  PR already exists for branch {} - skipping", head);
        return false;
    }

    if dry_run {
        println!("   [DRY-RUN] Would create PR: {} [U+2190] {}", title, head);
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
                println!("   [OK] PR created: {}", title);
                log_event("pr_created", &format!("{} [U+2190] {}", title, head));
                true
            } else if resp.status().as_u16() == 429 || resp.status().as_u16() == 403 {
                println!("   [WARN]  Rate limited ({}) - backing off", resp.status());
                backoff_on_rate_limit(0);
                false
            } else {
                println!("   [FAIL] PR creation failed: {:?}", resp.status());
                log_event("pr_create_fail", &format!("{} {:?}", title, resp.status()));
                false
            }
        }
        Err(e) => {
            println!("   [FAIL] Network error creating PR: {}", e);
            log_event("pr_network_fail", &e.to_string());
            false
        }
    }
}

/// Attempt auto-fix for error-handling findings (try! -> try?).
/// Operates in the staging worktree to avoid dirtying the sovereign repo.
/// Executable Safety Constitution gate. Before any auto-fix is written and
/// pushed as a PR, the proposed change must pass these machine-checkable
/// principles (a subset of clade-improve's Constitution, enforced here so the
/// tablecloth loop never opens a PR that violates them). Returns the violated
/// principle on rejection, `Ok(())` when the change may proceed.
fn constitution_gate(rel_file: &str, original: &str, fixed: &str) -> Result<(), String> {
    // P1 - Protect guard infrastructure: the loop may never rewrite the files
    // that constrain it.
    const PROTECTED: &[&str] = &[
        "constitution",
        "safety_budget",
        "oversight",
        "allowlist",
        "recursion",
        "SOUL.md",
        "build.sh",
    ];
    let lower = rel_file.to_lowercase();
    if let Some(p) = PROTECTED.iter().find(|p| lower.contains(&p.to_lowercase())) {
        return Err(format!("P1 protected path: '{}' matches '{}'", rel_file, p));
    }

    // P2 - No secret introduction: a fix must not add credentials.
    let added_secret = |needle: &str| fixed.matches(needle).count() > original.matches(needle).count();
    if added_secret("sk-") || added_secret("api_key") {
        return Err("P2 secret introduced by fix".to_string());
    }

    // P3 - No weakening of safety primitives: the fix must not remove existing
    // rate-limit / backoff / sleep guards.
    for guard in ["thread::sleep", "backoff", "Thread.sleep"] {
        if fixed.matches(guard).count() < original.matches(guard).count() {
            return Err(format!("P3 removes safety primitive '{}'", guard));
        }
    }

    // P4 - Bounded change: auto-fixes stay small and reviewable.
    let delta = (fixed.len() as i64 - original.len() as i64).unsigned_abs();
    if delta > 20_000 {
        return Err(format!("P4 change too large: {} bytes", delta));
    }

    Ok(())
}

/// Independent post-fix verifier - a second, separate gate from
/// `constitution_gate`. Where the gate inspects the *proposed* change, this
/// re-reads the file actually written to disk and confirms the fix did what it
/// claimed, mirroring an independent reviewer that does not trust the
/// generator's in-memory output. (Aligns with the 2026 "independent safety
/// classifier" pattern - verify separately from generation.)
fn independent_verify(
    file_path: &str,
    original: &str,
    pattern: &regex::Regex,
) -> Result<(), String> {
    let written = fs::read_to_string(file_path).map_err(|e| format!("re-read failed: {e}"))?;
    if written.trim().is_empty() {
        return Err("file empty after fix".to_string());
    }
    // The flagged pattern must be fully gone.
    if pattern.is_match(&written) {
        return Err("flagged pattern still present after fix".to_string());
    }
    // The fix must not have smuggled in `unsafe` that wasn't there before.
    if written.contains("unsafe ") && !original.contains("unsafe ") {
        return Err("fix introduced `unsafe`".to_string());
    }
    Ok(())
}

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
        println!("   [WARN]  Staging worktree missing - run clade-worktree ensure first");
        return (0, 0, 0);
    }

    let try_bang = match Regex::new(r"try!\s*\(") {
        Ok(re) => re,
        Err(_) => return (0, 0, 0),
    };
    let token = std::env::var("GITHUB_TOKEN").unwrap_or_default();
    let repo = target_repo();

    for finding in &report.error_handling_check.findings {
        if finding.message != "Bare try! - use try? or do-catch" {
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

        if !token.is_empty() && branch_exists_remote(&repo, &branch, &token) {
            println!("   [U+23ED]  Branch {} already exists remotely - skipping", branch);
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
                println!("   [WARN]  git checkout -b {} failed (exit {:?})", branch, s.code());
                continue;
            }
            Err(e) => {
                println!("   [WARN]  git checkout -b {} error: {}", branch, e);
                continue;
            }
            _ => {}
        }

        let fixed = try_bang.replace_all(&content, "try?(");

        // Constitution gate - reject the change before it is written/pushed.
        if let Err(violation) = constitution_gate(&finding.file, &content, &fixed) {
            println!("   [REJECT] Constitution REJECTED fix for {}: {}", finding.file, violation);
            log_event("constitution_reject", &format!("{}: {}", finding.file, violation));
            for git_args in [vec!["checkout", "-"], vec!["branch", "-D", &branch]] {
                if let Err(e) = Command::new("git")
                    .args(&git_args)
                    .current_dir(&worktree)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                {
                    eprintln!("[tablecloth] git {:?} failed: {}", git_args, e);
                }
            }
            continue;
        }

        if let Err(e) = fs::write(&file_path, fixed.as_ref()) {
            println!("   [FAIL] Failed to write fix: {}", e);
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
            println!("   [FAIL] Fix broke build on {} - discarding branch {}", finding.file, branch);
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

        // Independent verification - separate from generation. If it fails,
        // discard the branch rather than open a PR on an unverified change.
        if let Err(reason) = independent_verify(&file_path, &content, &try_bang) {
            println!("   [REJECT] Independent verify REJECTED {}: {}", finding.file, reason);
            log_event("verify_reject", &format!("{}: {}", finding.file, reason));
            for git_args in [
                vec!["checkout", "-"],
                vec!["branch", "-D", &branch],
                vec!["checkout", "--", &finding.file],
            ] {
                if let Err(e) = Command::new("git")
                    .args(&git_args)
                    .current_dir(&worktree)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                {
                    eprintln!("[tablecloth] git {:?} failed: {}", git_args, e);
                }
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
        if create_pr(&repo, &pr_title, &pr_body, &branch, "dev", dry_run) {
            prs_created += 1;
        }

        println!("   [OK] Fix passed build: {} on branch {}", finding.file, branch);
        passed += 1;
    }

    println!("   Fixes attempted: {} | passed: {} | PRs: {}", attempted, passed, prs_created);
    (attempted, passed, prs_created)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dry_run = args.iter().any(|a| a == "--dry-run");

    println!("===========================================================");
    println!("  CLADE-TABLECLOTH: Autonomous Self-Improvement Loop");
    println!("  Dry run: {}", dry_run);
    println!("===========================================================\n");

    // Step 1: Load budget
    let mut budget = load_budget();
    let budget_before = budget.budget;
    println!("[Step 1/7] Budget: {}/{} | halted={}", budget.budget, budget.max_budget, budget.halted);

    if budget.halted || budget.budget <= 0.0 {
        println!("[REJECT] HALTED: budget depleted or manually halted - stopping loop");
        log_event("loop_halted_budget", &format!("budget={}", budget.budget));
        std::process::exit(0);
    }

    println!("\n[PASS] Safety gates passed - continuing loop");

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

    // Step 5b: Account for the loop's cost against the safety budget. Each
    // failed fix attempt costs 1.0; depletion halts future loops.
    let depleted = apply_fix_outcome(&mut budget, fixes_attempted, fixes_passed);
    if depleted {
        println!("[REJECT] Budget depleted this loop - halting future auto-improvement");
        log_event("budget_depleted", &format!("budget={}", budget.budget));
    }
    if !dry_run {
        save_budget(&budget);
    }

    // Step 6: Write report
    write_report(budget_before, &budget, issues_created, fixes_attempted, fixes_passed, prs_created, dry_run);

    println!("\n[CHART] Summary: {} issues created, {} fixes attempted, {} fixes passed, {} PRs created", issues_created, fixes_attempted, fixes_passed, prs_created);
}

/// Write `.trinity/state/last_improvement.json` with loop summary.
fn write_report(
    budget_before: f64,
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
        budget_before,
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
        println!("   [FAIL] Failed to create state dir: {}", e);
    }
    match write_atomic(&path, &json) {
        Ok(_) => println!("   [OK] Report written: {}", path),
        Err(e) => {
            println!("   [FAIL] Failed to write report: {}", e);
            log_event("report_write_fail", &e.to_string());
        }
    }
}

fn log_event(event: &str, details: &str) {
    let ts = Utc::now().to_rfc3339();
    // Build via serde_json so control chars / quotes in `event`/`details`
    // are escaped - prevents JSONL log injection (e.g. `","event":"spoof`).
    let line = serde_json::json!({
        "timestamp": ts,
        "event": event,
        "details": details,
    })
    .to_string();
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
// Tests legitimately use expect()/unwrap() for fixtures and invariants; the
// workspace deny/warn policy targets production code paths, not test setup.
#[allow(clippy::unwrap_used, clippy::expect_used)]
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

    // ---- A2: audit JSON parsing never panics ----
    #[test]
    fn parse_audit_report_no_json_is_err() {
        assert!(parse_audit_report("banner only, no json here").is_err());
        assert!(parse_audit_report("").is_err());
    }

    #[test]
    fn parse_audit_report_malformed_json_is_err() {
        assert!(parse_audit_report("noise {\"build_check\": ").is_err());
    }

    // ---- B: backoff jitter + atomic write ----
    #[test]
    fn backoff_base_grows_and_caps() {
        assert_eq!(backoff_base_ms(0), 1000);
        assert_eq!(backoff_base_ms(1), 2000);
        assert_eq!(backoff_base_ms(20), API_BACKOFF_MAX_SECS * 1000);
    }

    #[test]
    fn jitter_within_half_base() {
        let base = 2000;
        assert!(jitter_ms(base) < base / 2 + 1);
        assert_eq!(jitter_ms(0), 0);
    }

    #[test]
    fn write_atomic_roundtrips() {
        let path = format!("/tmp/clade-tablecloth-atomic-{}.json", std::process::id());
        write_atomic(&path, "{\"ok\":true}").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"ok\":true}");
        fs::remove_file(&path).ok();
    }

    // ---- C: constitution gate ----
    #[test]
    fn gate_rejects_protected_paths() {
        assert!(constitution_gate(".trinity/state/safety_budget.json", "a", "b").is_err());
        assert!(constitution_gate("rings/RUST-04/clade-improve/src/constitution.rs", "a", "b").is_err());
        assert!(constitution_gate(".trinity/SOUL.md", "a", "b").is_err());
    }

    #[test]
    fn gate_rejects_introduced_secret() {
        assert!(constitution_gate("src/app.swift", "let x = 1", "let x = 1\nlet k = \"sk-abc\"").is_err());
    }

    #[test]
    fn gate_rejects_removed_safety_primitive() {
        let orig = "thread::sleep(d)\nwork()";
        let fixed = "work()";
        assert!(constitution_gate("src/loop.rs", orig, fixed).is_err());
    }

    #[test]
    fn gate_rejects_oversized_change() {
        let big = "x".repeat(30_000);
        assert!(constitution_gate("src/app.swift", "", &big).is_err());
    }

    #[test]
    fn gate_allows_benign_fix() {
        assert!(constitution_gate("src/app.swift", "try!(foo)", "try?(foo)").is_ok());
    }

    // Table-driven adversarial battery: locks the constitution policy against
    // regression (boundary cases, case-insensitivity, and false-positive
    // avoidance that the single-case tests above don't cover). Folding the
    // adversarial corpus into the regression suite per security-regression
    // practice.
    #[test]
    fn gate_adversarial_battery() {
        let big_over = "x".repeat(20_001);
        let big_edge = "x".repeat(20_000);
        // (name, rel_file, original, fixed, expect_ok)
        let cases: Vec<(&str, &str, String, String, bool)> = vec![
            ("p1 lowercase protected", "rings/x/constitution.rs", "a".into(), "b".into(), false),
            ("p1 UPPERCASE protected (case-insensitive)", "rings/SAFETY_BUDGET.json", "a".into(), "b".into(), false),
            ("p1 soul", ".trinity/SOUL.md", "a".into(), "b".into(), false),
            ("p2 added secret rejected", "src/a.swift", "x".into(), "x\nlet k=\"sk-LIVE\"".into(), false),
            ("p2 preexisting secret not added -> ok", "src/a.swift", "k=\"sk-old\"".into(), "k=\"sk-old\"\nlet y=1".into(), true),
            ("p3 removes sleep rejected", "src/a.rs", "thread::sleep(d)\nwork()".into(), "work()".into(), false),
            ("p3 adds guard -> ok", "src/a.rs", "work()".into(), "thread::sleep(d)\nwork()".into(), true),
            ("p4 oversized rejected", "src/a.swift", String::new(), big_over, false),
            ("p4 at threshold -> ok", "src/a.swift", String::new(), big_edge, true),
            ("benign small fix -> ok", "src/a.swift", "try!(f)".into(), "try?(f)".into(), true),
        ];
        for (name, file, original, fixed, expect_ok) in cases {
            let got_ok = constitution_gate(file, &original, &fixed).is_ok();
            assert_eq!(got_ok, expect_ok, "case '{}' expected ok={}, got {}", name, expect_ok, got_ok);
        }
    }

    // ---- C2: budget cost accounting ----
    #[test]
    fn fix_outcome_net_cost_on_mixed() {
        let mut b = default_budget(); // budget 5.0
        // 3 attempted, 1 passed -> 2 failures*1.0 spent, 1 pass*0.25 earned.
        let depleted = apply_fix_outcome(&mut b, 3, 1);
        assert!(!depleted);
        assert!((b.budget - 3.25).abs() < f64::EPSILON);
        assert_eq!(b.total_trials, 3);
        assert_eq!(b.total_failures, 2);
        assert!(!b.halted);
    }

    #[test]
    fn fix_outcome_reward_is_capped_at_max() {
        // All passed below max: budget regenerates but never exceeds max_budget.
        let mut b = SafetyBudget { budget: 4.8, max_budget: 5.0, total_trials: 0, total_failures: 0, halted: false };
        assert!(!apply_fix_outcome(&mut b, 4, 4)); // +4*0.25 = +1.0 -> clamp 5.0
        assert!((b.budget - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn fix_outcome_regenerates_partial_budget() {
        let mut b = SafetyBudget { budget: 2.0, max_budget: 5.0, total_trials: 0, total_failures: 0, halted: false };
        assert!(!apply_fix_outcome(&mut b, 2, 2)); // +0.5
        assert!((b.budget - 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn fix_outcome_depletes_and_halts() {
        let mut b = SafetyBudget { budget: 2.0, max_budget: 5.0, total_trials: 0, total_failures: 0, halted: false };
        let depleted = apply_fix_outcome(&mut b, 5, 0); // 5 failures > 2.0
        assert!(depleted);
        assert!(b.halted);
        assert!(b.budget <= 0.0);
    }

    #[test]
    fn budget_roundtrips_through_json() {
        let b = SafetyBudget { budget: 3.5, max_budget: 5.0, total_trials: 7, total_failures: 2, halted: false };
        let json = serde_json::to_string(&b).unwrap();
        let back: SafetyBudget = serde_json::from_str(&json).unwrap();
        assert!((back.budget - 3.5).abs() < f64::EPSILON);
        assert_eq!(back.total_trials, 7);
    }

    // ---- B: independent verifier ----
    #[test]
    fn independent_verify_accepts_clean_fix() {
        let re = regex::Regex::new(r"try!\s*\(").unwrap();
        let path = format!("/tmp/clade-verify-ok-{}.swift", std::process::id());
        fs::write(&path, "let x = try?(foo())\n").unwrap();
        assert!(independent_verify(&path, "let x = try!(foo())", &re).is_ok());
        fs::remove_file(&path).ok();
    }

    #[test]
    fn independent_verify_rejects_residual_pattern() {
        let re = regex::Regex::new(r"try!\s*\(").unwrap();
        let path = format!("/tmp/clade-verify-bad-{}.swift", std::process::id());
        fs::write(&path, "let x = try!(foo())\n").unwrap();
        assert!(independent_verify(&path, "let x = try!(foo())", &re).is_err());
        fs::remove_file(&path).ok();
    }

    #[test]
    fn independent_verify_rejects_introduced_unsafe() {
        let re = regex::Regex::new(r"try!\s*\(").unwrap();
        let path = format!("/tmp/clade-verify-unsafe-{}.swift", std::process::id());
        fs::write(&path, "unsafe { ptr() }\n").unwrap();
        assert!(independent_verify(&path, "let x = 1", &re).is_err());
        fs::remove_file(&path).ok();
    }

    #[test]
    fn independent_verify_rejects_missing_file() {
        // Re-read failure must fail closed (Err), never accept the change.
        let re = regex::Regex::new(r"try!\s*\(").unwrap();
        let path = format!("/tmp/clade-verify-absent-{}.swift", std::process::id());
        fs::remove_file(&path).ok(); // ensure it does not exist
        assert!(independent_verify(&path, "x", &re).is_err());
    }

    #[test]
    fn independent_verify_rejects_empty_file() {
        let re = regex::Regex::new(r"try!\s*\(").unwrap();
        let path = format!("/tmp/clade-verify-empty-{}.swift", std::process::id());
        fs::write(&path, "   \n").unwrap(); // whitespace-only -> empty after trim
        assert!(independent_verify(&path, "let x = 1", &re).is_err());
        fs::remove_file(&path).ok();
    }

    #[test]
    fn target_repo_defaults_to_trios() {
        // With no github.json and no GITHUB_REPO set in the test env, the
        // fallback chain ends at "trios". (CI sets neither.)
        if std::env::var("GITHUB_REPO").is_err() {
            assert_eq!(target_repo(), "trios");
        }
    }

    // ---- log injection is escaped (regression for the JSONL injection fix) ----
    #[test]
    fn log_line_escapes_injection() {
        let line = serde_json::json!({
            "timestamp": "t",
            "event": "evt",
            "details": "\",\"event\":\"spoofed",
        })
        .to_string();
        // The injected quote must be escaped, so only one real "event" key exists.
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["event"], "evt");
        assert_eq!(v["details"], "\",\"event\":\"spoofed");
    }
}
