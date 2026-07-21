use chrono::Local;
use reqwest::blocking::get;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const MAX_RESPONSE_BYTES: usize = 1024 * 1024; // 1MB - matches clade-diff

/// Cap an HTTP response body so a misbehaving/oversized response can't blow up
/// the report (consistency with clade-diff's MAX_RESPONSE_BYTES guard).
fn cap_body(body: String) -> String {
    if body.len() > MAX_RESPONSE_BYTES {
        format!("error: response too large ({}KB)", body.len() / 1024)
    } else {
        body
    }
}

struct Variant {
    name: &'static str,
    mcp_port: &'static str,
    app_pattern: &'static str,
    log_process: &'static str,
}

fn main() {
    let variant_name = env::var("TRIOS_VARIANT").unwrap_or_else(|_| "prod".into());
    let v = resolve_variant(&variant_name);
    let log_dir = PathBuf::from(format!("{}/.trinity/e2e", trios_config::project_dir()));
    fs::create_dir_all(&log_dir).ok();

    let ts = Local::now().timestamp();
    let report_path = log_dir.join(format!("report_{}_{}.md", v.name, ts));
    let screenshot_path = log_dir.join(format!("screenshot_{}_{}.png", v.name, ts));

    let mut report = format!(
        "# TRIOS E2E Report {} - Variant: {}\n\n",
        Local::now().to_rfc2822(),
        v.name
    );
    let mut failures = 0u32;

    // 0. Swift logic unit tests (pre-flight, no running app required)
    if !run_swift_logic_tests(&mut report) {
        failures += 1;
    }

    // 1. Server Health
    let health_url = format!("http://127.0.0.1:{}/health", v.mcp_port);
    match get(&health_url) {
        Ok(resp) => {
            let body = cap_body(resp.text().unwrap_or_default());
            if body.contains("\"status\":\"ok\"") {
                report.push_str(&format!(
                    "- [OK] BrowserOS Server ({}): OK ({})\n",
                    v.name, body
                ));
            } else {
                report.push_str(&format!(
                    "- [FAIL] BrowserOS Server ({}): DOWN ({})\n",
                    v.name, body
                ));
                failures += 1;
            }
        }
        Err(e) => {
            report.push_str(&format!(
                "- [FAIL] BrowserOS Server ({}): DOWN ({})\n",
                v.name, e
            ));
            failures += 1;
        }
    }

    // 2. App Running
    let pid = Command::new("pgrep")
        .args(["-f", v.app_pattern])
        .stdout(Stdio::piped())
        .output()
        .ok()
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        });

    if let Some(ref p) = pid {
        report.push_str(&format!("- [OK] Trios App ({}): PID {}\n", v.name, p));
    } else {
        report.push_str(&format!("- [FAIL] Trios App ({}): NOT RUNNING\n", v.name));
        failures += 1;
    }

    // 3. Screenshot
    let _ = Command::new("screencapture")
        .args([
            "-x",
            screenshot_path
                .to_str()
                .unwrap_or("trios_screenshot.png"),
        ])
        .output();
    report.push_str(&format!(
        "- [IMG] Screenshot ({}): {}\n",
        v.name,
        screenshot_path.display()
    ));

    // 4. Log Errors (last 5 min)
    let log_output = Command::new("log")
        .args([
            "show",
            "--predicate",
            &format!("process == \"{}\"", v.log_process),
            "--last",
            "5m",
            "--style",
            "compact",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok();

    let errors: Vec<String> = log_output
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
        .lines()
        .filter(|l| {
            let lower = l.to_lowercase();
            lower.contains("timed out")
                || lower.contains("transporterror")
                || lower.contains("crash")
                || lower.contains("fatal")
                || lower.contains("error")
        })
        .take(5)
        .map(|s| s.to_string())
        .collect();

    if errors.is_empty() {
        report.push_str(&format!(
            "- [OK] No critical errors in last 5m ({})\n",
            v.name
        ));
    } else {
        report.push_str(&format!("- [U+26A0][U+FE0F] Recent Errors ({})\n", v.name));
        report.push_str("```\n");
        for e in errors {
            report.push_str(&e);
            report.push('\n');
        }
        report.push_str("```\n");
    }

    // 5. UI Anomaly Checklist
    report.push_str(&format!("\n## UI Anomaly Checklist ({})\n", v.name));
    report.push_str("- [ ] Title bar shows correct status (Online green dot, A2A blue dot)\n");
    report.push_str(
        "- [ ] Tab bar icons visible and not duplicated (Chat/Git/Terminal/Queen/Settings)\n",
    );
    report
        .push_str("- [ ] Chat input field visible at bottom with placeholder 'Ask anything...'\n");
    report.push_str("- [ ] No overlapping views, no black rectangles, no glitched rendering\n");
    report.push_str("- [ ] Glassmorphism blur visible behind panel content\n");
    report.push_str("- [ ] Messages scroll correctly without cutting off bubbles\n");
    report.push_str("- [ ] No duplicate headers or buttons outside tab bar\n");

    if let Err(e) = fs::write(&report_path, &report) {
        eprintln!("[e2e] Failed to write report: {}", e);
    }
    println!("{}", report_path.display());

    if failures > 0 {
        eprintln!("[e2e] {} check(s) failed", failures);
        std::process::exit(1);
    }
}

/// Compile and run the standalone Swift logic tests (ChatLogic). This is the
/// L7-compliant replacement for a shell test step - invoked from Rust, no .sh.
/// Returns true on pass; appends a line to the e2e report either way.
fn run_swift_logic_tests(report: &mut String) -> bool {
    let dir = trios_config::project_dir();
    let bin = "/tmp/trios_chat_logic_test";

    let compiled = Command::new("swiftc")
        .args([
            "tests/swift/chat_logic_test.swift",
            "BR-OUTPUT/ChatLogic.swift",
            "-o",
            bin,
        ])
        .current_dir(&dir)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();

    match compiled {
        Ok(out) if out.status.success() => match Command::new(bin).output() {
            Ok(run) if run.status.success() => {
                report.push_str("- [OK] Swift logic tests: passed\n");
                true
            }
            Ok(run) => {
                let tail = cap_body(String::from_utf8_lossy(&run.stdout).to_string());
                report.push_str(&format!(
                    "- [FAIL] Swift logic tests FAILED\n```\n{}\n```\n",
                    tail
                ));
                false
            }
            Err(e) => {
                report.push_str(&format!("- [FAIL] Swift logic tests: could not run ({})\n", e));
                false
            }
        },
        Ok(out) => {
            let tail = cap_body(String::from_utf8_lossy(&out.stderr).to_string());
            report.push_str(&format!(
                "- [FAIL] Swift logic tests: compile failed\n```\n{}\n```\n",
                tail
            ));
            false
        }
        Err(e) => {
            report.push_str(&format!(
                "- [FAIL] Swift logic tests: swiftc unavailable ({})\n",
                e
            ));
            false
        }
    }
}

fn resolve_variant(name: &str) -> Variant {
    if name == "staging" {
        Variant {
            name: "staging",
            mcp_port: "9205",
            app_pattern: "trios-staging.app/Contents/MacOS/trios",
            log_process: "trios-staging",
        }
    } else {
        Variant {
            name: "prod",
            mcp_port: "9105",
            app_pattern: "trios.app/Contents/MacOS/trios",
            log_process: "trios",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_body_passes_small() {
        assert_eq!(
            cap_body("{\"status\":\"ok\"}".to_string()),
            "{\"status\":\"ok\"}"
        );
    }

    #[test]
    fn cap_body_truncates_oversized() {
        let big = "x".repeat(MAX_RESPONSE_BYTES + 1);
        let out = cap_body(big);
        assert!(out.starts_with("error: response too large"));
    }

    #[test]
    fn resolve_variant_prod_defaults() {
        let v = resolve_variant("prod");
        assert_eq!(v.name, "prod");
        assert_eq!(v.mcp_port, "9105");
        assert!(v.app_pattern.contains("trios.app"));
        assert_eq!(v.log_process, "trios");
    }

    #[test]
    fn resolve_variant_staging_ports() {
        let v = resolve_variant("staging");
        assert_eq!(v.name, "staging");
        assert_eq!(v.mcp_port, "9205");
        assert!(v.app_pattern.contains("trios-staging"));
        assert_eq!(v.log_process, "trios-staging");
    }

    #[test]
    fn resolve_variant_unknown_is_prod() {
        let v = resolve_variant("dev");
        assert_eq!(v.name, "prod");
    }
}
