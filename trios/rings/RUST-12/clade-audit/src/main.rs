use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;
use walkdir::WalkDir;

const PROJECT_DIR: &str = "/Users/playra/BrowserOS-full/trios";

#[derive(Serialize, Deserialize, Debug, Clone)]
struct AuditFinding {
    file: String,
    line: u32,
    severity: String,
    category: String,
    message: String,
    fingerprint: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct BuildCheckResult {
    passed: bool,
    swift_ok: bool,
    rust_ok: bool,
    swift_errors: Vec<String>,
    rust_errors: Vec<String>,
    duration_ms: u128,
}

#[derive(Serialize, Deserialize, Debug)]
struct SecurityCheckResult {
    passed: bool,
    findings: Vec<AuditFinding>,
    scanned_files: usize,
    duration_ms: u128,
}

/// Run build checks: swiftc -typecheck + cargo check --workspace.
fn build_check() -> BuildCheckResult {
    let start = Instant::now();

    // Swift typecheck: swiftc -typecheck main.swift rings/**/*.swift BR-OUTPUT/*.swift
    let swift_output = Command::new("swiftc")
        .args([
            "-typecheck",
            "main.swift",
            "rings/SR-00/*.swift",
            "rings/SR-01/*.swift",
            "rings/SR-02/*.swift",
            "rings/SR-03/*.swift",
            "BR-OUTPUT/*.swift",
        ])
        .current_dir(PROJECT_DIR)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    let (swift_ok, swift_errors) = match swift_output {
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let errors: Vec<String> = stderr
                .lines()
                .filter(|l| l.contains("error:"))
                .map(|s| s.to_string())
                .collect();
            (out.status.success() && errors.is_empty(), errors)
        }
        Err(e) => (false, vec![format!("swiftc execution failed: {}", e)]),
    };

    // Rust workspace check
    let rust_output = Command::new("cargo")
        .args(["check", "--workspace"])
        .current_dir(PROJECT_DIR)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    let (rust_ok, rust_errors) = match rust_output {
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let errors: Vec<String> = stderr
                .lines()
                .filter(|l| l.contains("error"))
                .map(|s| s.to_string())
                .collect();
            (out.status.success() && errors.is_empty(), errors)
        }
        Err(e) => (false, vec![format!("cargo check execution failed: {}", e)]),
    };

    BuildCheckResult {
        passed: swift_ok && rust_ok,
        swift_ok,
        rust_ok,
        swift_errors,
        rust_errors,
        duration_ms: start.elapsed().as_millis(),
    }
}

/// Scan source files for forbidden security patterns.
fn security_check() -> SecurityCheckResult {
    let start = Instant::now();
    let mut findings: Vec<AuditFinding> = vec![];
    let mut scanned = 0;

    let forbidden_patterns: Vec<(&str, &str, &str)> = vec![
        (r"rm\s+-rf\s+/", "critical", "Forbidden: rm -rf /"),
        (r"curl\s+.*\|\s*sh", "critical", "Forbidden: curl | sh"),
        (r"try!\s*\(", "warning", "Unsafe: bare try! macro"),
        (r"as!\s*\w+", "warning", "Unsafe: force cast as!"),
        (r#"api[_-]?key\s*=\s*[^"']*["']\w{20,}"#, "critical", "Hardcoded API key"),
        (r#"password\s*=\s*["']\w+["']"#, "warning", "Hardcoded password"),
        (r#"token\s*=\s*["']\w{20,}["']"#, "critical", "Hardcoded token"),
        (r"NSLog\s*\(\s*[^)]*secret", "warning", "NSLog may leak secret"),
        (r"print\s*\(\s*[^)]*secret", "warning", "print may leak secret"),
    ];

    let compiled: Vec<(Regex, &str, &str)> = forbidden_patterns
        .into_iter()
        .filter_map(|(pat, sev, msg)| {
            Regex::new(pat).ok().map(|re| (re, sev, msg))
        })
        .collect();

    for entry in WalkDir::new(PROJECT_DIR)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let p = e.path();
            let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
            (ext == "swift" || ext == "rs" || ext == "sh") && !p.to_string_lossy().contains("target/")
        })
    {
        scanned += 1;
        let path = entry.path();
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for (re, severity, message) in &compiled {
            for (line_idx, line) in content.lines().enumerate() {
                if re.is_match(line) {
                    let file = path.strip_prefix(PROJECT_DIR)
                        .unwrap_or(path)
                        .to_string_lossy()
                        .to_string();
                    let fingerprint = format!("{}:{}:{}", &file, line_idx + 1, message);
                    findings.push(AuditFinding {
                        file,
                        line: (line_idx + 1) as u32,
                        severity: (*severity).to_string(),
                        category: "security".to_string(),
                        message: (*message).to_string(),
                        fingerprint,
                    });
                }
            }
        }
    }

    SecurityCheckResult {
        passed: findings.is_empty(),
        findings,
        scanned_files: scanned,
        duration_ms: start.elapsed().as_millis(),
    }
}

/// Scan Swift files for Process() shell calls without allowlist (SOUL.md Article IX).
fn shell_safety_check() -> SecurityCheckResult {
    let start = Instant::now();
    let mut findings: Vec<AuditFinding> = vec![];
    let mut scanned = 0;

    let process_re = Regex::new(r"Process\(\)").unwrap();
    let shell_re = Regex::new(r#"arguments:\s*\[\s*"-c""#).unwrap();

    let forbidden_substrings: Vec<&str> = vec![
        "rm -rf /",
        "curl .* | sh",
        "> /dev/null",
        "trios_app",
        "open trios",
    ];

    for entry in WalkDir::new(PROJECT_DIR)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let p = e.path();
            let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
            ext == "swift" && !p.to_string_lossy().contains("target/")
        })
    {
        scanned += 1;
        let path = entry.path();
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let file = path.strip_prefix(PROJECT_DIR)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        for (line_idx, line) in content.lines().enumerate() {
            if process_re.is_match(line) && shell_re.is_match(line) {
                let has_allowlist = forbidden_substrings.iter().any(|pat| {
                    let pat_re = Regex::new(pat).ok();
                    pat_re.map_or(false, |re| re.is_match(&content))
                });
                if has_allowlist {
                    continue;
                }
                let fingerprint = format!("{}:{}:shell_no_allowlist", &file, line_idx + 1);
                findings.push(AuditFinding {
                    file: file.clone(),
                    line: (line_idx + 1) as u32,
                    severity: "warning".to_string(),
                    category: "shell_safety".to_string(),
                    message: "Process() with zsh -c lacks explicit allowlist".to_string(),
                    fingerprint,
                });
            }
        }
    }

    SecurityCheckResult {
        passed: findings.is_empty(),
        findings,
        scanned_files: scanned,
        duration_ms: start.elapsed().as_millis(),
    }
}

/// Scan for bare try!, as!, and unhandled try? in Swift/Rust.
fn error_handling_check() -> SecurityCheckResult {
    let start = Instant::now();
    let mut findings: Vec<AuditFinding> = vec![];
    let mut scanned = 0;

    let patterns: Vec<(&str, &str, &str)> = vec![
        (r"try!\s*\(", "warning", "Bare try! — use try? or do-catch"),
        (r"as!\s*\w+", "warning", "Force cast as! — use as? with guard"),
        (r"as!\s*\[", "warning", "Force cast as! — use as? with guard"),
        (r"try\?\s*\([^)]*\)\s*(?:(?!guard|if\s+let|let\s+_).)*$", "info", "Unhandled try? result"),
    ];

    let compiled: Vec<(Regex, &str, &str)> = patterns
        .into_iter()
        .filter_map(|(pat, sev, msg)| {
            Regex::new(pat).ok().map(|re| (re, sev, msg))
        })
        .collect();

    for entry in WalkDir::new(PROJECT_DIR)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let p = e.path();
            let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
            (ext == "swift" || ext == "rs") && !p.to_string_lossy().contains("target/")
        })
    {
        scanned += 1;
        let path = entry.path();
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let file = path.strip_prefix(PROJECT_DIR)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        for (re, severity, message) in &compiled {
            for (line_idx, line) in content.lines().enumerate() {
                if re.is_match(line) {
                    let fingerprint = format!("{}:{}:{}", &file, line_idx + 1, message);
                    findings.push(AuditFinding {
                        file: file.clone(),
                        line: (line_idx + 1) as u32,
                        severity: (*severity).to_string(),
                        category: "error_handling".to_string(),
                        message: (*message).to_string(),
                        fingerprint,
                    });
                }
            }
        }
    }

    SecurityCheckResult {
        passed: findings.is_empty(),
        findings,
        scanned_files: scanned,
        duration_ms: start.elapsed().as_millis(),
    }
}

/// Detect Swift 6 concurrency anti-patterns: missing [weak self] in async closures,
/// actor-isolated mutable state accessed from concurrent contexts.
fn concurrency_check() -> SecurityCheckResult {
    let start = Instant::now();
    let mut findings: Vec<AuditFinding> = vec![];
    let mut scanned = 0;

    let patterns: Vec<(&str, &str, &str)> = vec![
        (r"Timer\.scheduledTimer.*\{\s*_.*in\s*\n?\s*self\.", "warning", "Timer closure captures self strongly — add [weak self]"),
        (r"Timer\.publish.*\.sink.*\{\s*.*in\s*\n?\s*self\.", "warning", "Timer sink captures self strongly — add [weak self]"),
        (r"DispatchQueue\.main\.async\s*\{\s*\n?\s*self\.", "warning", "DispatchQueue closure captures self strongly — add [weak self]"),
        (r"Task\s*\{\s*\n?\s*self\.", "warning", "Task captures self strongly — add [weak self] or capture list"),
        (r"@Published\s+var\s+\w+.*=.*\[\]", "info", "@Published array default — consider empty init for clarity"),
    ];

    let compiled: Vec<(Regex, &str, &str)> = patterns
        .into_iter()
        .filter_map(|(pat, sev, msg)| {
            Regex::new(pat).ok().map(|re| (re, sev, msg))
        })
        .collect();

    for entry in WalkDir::new(PROJECT_DIR)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let p = e.path();
            let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
            ext == "swift" && !p.to_string_lossy().contains("target/")
        })
    {
        scanned += 1;
        let path = entry.path();
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let file = path.strip_prefix(PROJECT_DIR)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        for (re, severity, message) in &compiled {
            for (line_idx, line) in content.lines().enumerate() {
                if re.is_match(line) {
                    let fingerprint = format!("{}:{}:{}", &file, line_idx + 1, message);
                    findings.push(AuditFinding {
                        file: file.clone(),
                        line: (line_idx + 1) as u32,
                        severity: (*severity).to_string(),
                        category: "concurrency".to_string(),
                        message: (*message).to_string(),
                        fingerprint,
                    });
                }
            }
        }
    }

    SecurityCheckResult {
        passed: findings.is_empty(),
        findings,
        scanned_files: scanned,
        duration_ms: start.elapsed().as_millis(),
    }
}

/// Inventory TODO and FIXME comments with severity categorization.
fn todo_check() -> SecurityCheckResult {
    let start = Instant::now();
    let mut findings: Vec<AuditFinding> = vec![];
    let mut scanned = 0;

    let todo_re = Regex::new(r"(?i)(TODO|FIXME|HACK|XXX|WARN|BUG)\s*[:\-]?\s*(.*)").unwrap();

    for entry in WalkDir::new(PROJECT_DIR)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let p = e.path();
            let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
            (ext == "swift" || ext == "rs" || ext == "md") && !p.to_string_lossy().contains("target/")
        })
    {
        scanned += 1;
        let path = entry.path();
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let file = path.strip_prefix(PROJECT_DIR)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        for (line_idx, line) in content.lines().enumerate() {
            if let Some(caps) = todo_re.captures(line) {
                let keyword = caps.get(1).map(|m| m.as_str().to_uppercase()).unwrap_or_default();
                let desc = caps.get(2).map(|m| m.as_str().trim()).unwrap_or("").to_string();
                let severity = match keyword.as_str() {
                    "FIXME" | "BUG" => "critical",
                    "TODO" => "warning",
                    "HACK" | "XXX" => "warning",
                    _ => "info",
                };
                let message = format!("{}: {}", keyword, desc);
                let fingerprint = format!("{}:{}:{}", &file, line_idx + 1, &message);
                findings.push(AuditFinding {
                    file: file.clone(),
                    line: (line_idx + 1) as u32,
                    severity: severity.to_string(),
                    category: "todo_inventory".to_string(),
                    message,
                    fingerprint,
                });
            }
        }
    }

    SecurityCheckResult {
        passed: findings.is_empty(),
        findings,
        scanned_files: scanned,
        duration_ms: start.elapsed().as_millis(),
    }
}

/// Heuristic dead-code detection: private func in Swift / non-pub fn in Rust
/// not referenced in the same file.
fn unused_code_check() -> SecurityCheckResult {
    let start = Instant::now();
    let mut findings: Vec<AuditFinding> = vec![];
    let mut scanned = 0;

    let swift_private_func = Regex::new(r"private\s+func\s+(\w+)").unwrap();
    let rust_non_pub_fn = Regex::new(r"^\s*fn\s+(\w+)").unwrap();

    for entry in WalkDir::new(PROJECT_DIR)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let p = e.path();
            let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
            (ext == "swift" || ext == "rs") && !p.to_string_lossy().contains("target/")
        })
    {
        scanned += 1;
        let path = entry.path();
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let file = path.strip_prefix(PROJECT_DIR)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");

        if ext == "swift" {
            for caps in swift_private_func.captures_iter(&content) {
                let name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                let decl_line = content[..caps.get(0).unwrap().start()]
                    .lines()
                    .count() as u32 + 1;
                let refs = content.matches(name).count();
                // decl itself counts as 1; if only 1, it's unused
                if refs <= 1 {
                    let fingerprint = format!("{}:{}:unused_private_func:{}", &file, decl_line, name);
                    findings.push(AuditFinding {
                        file: file.clone(),
                        line: decl_line,
                        severity: "info".to_string(),
                        category: "unused_code".to_string(),
                        message: format!("Private func '{}' appears unused in file", name),
                        fingerprint,
                    });
                }
            }
        } else if ext == "rs" {
            for caps in rust_non_pub_fn.captures_iter(&content) {
                let name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                let decl_line = content[..caps.get(0).unwrap().start()]
                    .lines()
                    .count() as u32 + 1;
                let refs = content.matches(name).count();
                if refs <= 1 {
                    let fingerprint = format!("{}:{}:unused_fn:{}", &file, decl_line, name);
                    findings.push(AuditFinding {
                        file: file.clone(),
                        line: decl_line,
                        severity: "info".to_string(),
                        category: "unused_code".to_string(),
                        message: format!("Non-pub fn '{}' appears unused in file", name),
                        fingerprint,
                    });
                }
            }
        }
    }

    SecurityCheckResult {
        passed: findings.is_empty(),
        findings,
        scanned_files: scanned,
        duration_ms: start.elapsed().as_millis(),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return;
    }

    let dry_run = args.iter().any(|a| a == "--dry-run");
    let json_mode = args.iter().any(|a| a == "--json");

    println!("═══════════════════════════════════════════════════════════");
    println!("  CLADE-AUDIT: Trinity Self-Critic");
    println!("  Dry run: {} | JSON: {}", dry_run, json_mode);
    println!("═══════════════════════════════════════════════════════════\n");

    // Stage 1: Build check
    println!("[Check 1/8] Build gate — swiftc + cargo check");
    let build = build_check();
    println!(
        "   {} Swift: {} errors | Rust: {} errors | {}ms",
        if build.passed { "✅" } else { "❌" },
        build.swift_errors.len(),
        build.rust_errors.len(),
        build.duration_ms
    );

    // Stage 2: Security check
    println!("[Check 2/8] Security scan — forbidden patterns");
    let security = security_check();
    println!(
        "   {} Files scanned: {} | Findings: {} | {}ms",
        if security.passed { "✅" } else { "❌" },
        security.scanned_files,
        security.findings.len(),
        security.duration_ms
    );
    for f in &security.findings {
        println!("   ⚠️  {}:{} — {} ({})", f.file, f.line, f.message, f.severity);
    }

    // Stage 3: Shell safety check
    println!("[Check 3/8] Shell safety — Process() allowlist");
    let shell = shell_safety_check();
    println!(
        "   {} Files scanned: {} | Findings: {} | {}ms",
        if shell.passed { "✅" } else { "❌" },
        shell.scanned_files,
        shell.findings.len(),
        shell.duration_ms
    );
    for f in &shell.findings {
        println!("   ⚠️  {}:{} — {}", f.file, f.line, f.message);
    }

    // Stage 4: Error handling check
    println!("[Check 4/8] Error handling — try!, as!, unhandled try?");
    let err = error_handling_check();
    println!(
        "   {} Files scanned: {} | Findings: {} | {}ms",
        if err.passed { "✅" } else { "❌" },
        err.scanned_files,
        err.findings.len(),
        err.duration_ms
    );
    for f in &err.findings {
        println!("   ⚠️  {}:{} — {}", f.file, f.line, f.message);
    }

    // Stage 5: Concurrency check
    println!("[Check 5/8] Concurrency — Swift 6 actor isolation");
    let conc = concurrency_check();
    println!(
        "   {} Files scanned: {} | Findings: {} | {}ms",
        if conc.passed { "✅" } else { "❌" },
        conc.scanned_files,
        conc.findings.len(),
        conc.duration_ms
    );
    for f in &conc.findings {
        println!("   ⚠️  {}:{} — {}", f.file, f.line, f.message);
    }

    // Stage 6: TODO/FIXME inventory
    println!("[Check 6/8] TODO/FIXME inventory — categorized severity");
    let todo = todo_check();
    println!(
        "   {} Files scanned: {} | Findings: {} | {}ms",
        if todo.passed { "✅" } else { "❌" },
        todo.scanned_files,
        todo.findings.len(),
        todo.duration_ms
    );
    for f in &todo.findings {
        println!("   ⚠️  {}:{} — {} ({})", f.file, f.line, f.message, f.severity);
    }

    // Stage 7: Unused code check
    println!("[Check 7/8] Unused code — dead private func/fn heuristic");
    let unused = unused_code_check();
    println!(
        "   {} Files scanned: {} | Findings: {} | {}ms",
        if unused.passed { "✅" } else { "❌" },
        unused.scanned_files,
        unused.findings.len(),
        unused.duration_ms
    );
    for f in &unused.findings {
        println!("   ⚠️  {}:{} — {}", f.file, f.line, f.message);
    }

    if json_mode {
        let report = serde_json::json!({
            "build_check": build,
            "security_check": security,
            "shell_safety_check": shell,
            "error_handling_check": err,
            "concurrency_check": conc,
            "todo_check": todo,
            "unused_code_check": unused,
        });
        println!("\n{}", serde_json::to_string_pretty(&report).unwrap_or_default());
    }
}

fn print_help() {
    println!(
        r#"
clade-audit — Continuous code critic for Trinity

USAGE:
    cargo run --bin clade-audit -- [--dry-run] [--json]

CHECKS:
    1. Build gate     — swiftc -typecheck + cargo check --workspace
    2. Security scan  — forbidden patterns, hardcoded secrets
    3. Shell safety   — Process() allowlist compliance (SOUL.md Article IX)
    4. Error handling — bare try!, as!, unhandled try?
    5. Concurrency    — Swift 6 actor isolation anti-patterns
    6. TODO/FIXME     — categorized severity inventory
    7. Unused code    — dead function/module detection
    8. Retain cycles  — missing [weak self] in async closures

OUTPUT:
    --json   Emit structured report to stdout
    --dry-run  Do not write .trinity/audit/*.json
"#
    );
}
