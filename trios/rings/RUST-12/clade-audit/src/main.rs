use serde::{Deserialize, Serialize};
use std::process::{Command, Stdio};
use std::time::Instant;

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

    if json_mode {
        let report = serde_json::json!({
            "build_check": build,
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
