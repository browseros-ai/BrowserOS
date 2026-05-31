use chrono::Local;
use reqwest::blocking::get;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Variant {
    name: &'static str,
    mcp_port: &'static str,
    app_pattern: &'static str,
    log_process: &'static str,
}

fn main() {
    let variant_name = env::var("TRIOS_VARIANT").unwrap_or_else(|_| "prod".into());
    let v = resolve_variant(&variant_name);
    let log_dir = PathBuf::from("/tmp/trios_e2e");
    fs::create_dir_all(&log_dir).ok();

    let ts = Local::now().timestamp();
    let report_path = log_dir.join(format!("report_{}_{}.md", v.name, ts));
    let screenshot_path = log_dir.join(format!("screenshot_{}_{}.png", v.name, ts));

    let mut report = format!("# TRIOS E2E Report {} — Variant: {}\n\n", Local::now().to_rfc2822(), v.name);

    // 1. Server Health
    let health_url = format!("http://127.0.0.1:{}/health", v.mcp_port);
    match get(&health_url) {
        Ok(resp) => {
            let body = resp.text().unwrap_or_default();
            if body.contains("\"status\":\"ok\"") {
                report.push_str(&format!("- ✅ BrowserOS Server ({}): OK ({})\n", v.name, body));
            } else {
                report.push_str(&format!("- ❌ BrowserOS Server ({}): DOWN ({})\n", v.name, body));
            }
        }
        Err(e) => {
            report.push_str(&format!("- ❌ BrowserOS Server ({}): DOWN ({})\n", v.name, e));
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
            if s.is_empty() { None } else { Some(s) }
        });

    if let Some(ref p) = pid {
        report.push_str(&format!("- ✅ Trios App ({}): PID {}\n", v.name, p));
    } else {
        report.push_str(&format!("- ❌ Trios App ({}): NOT RUNNING\n", v.name));
    }

    // 3. Screenshot
    let _ = Command::new("screencapture")
        .args(["-x", screenshot_path.to_str().unwrap_or("/tmp/trios_screenshot.png")])
        .output();
    report.push_str(&format!(
        "- 📸 Screenshot ({}): {}\n",
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
        report.push_str(&format!("- ✅ No critical errors in last 5m ({})\n", v.name));
    } else {
        report.push_str(&format!("- ⚠️ Recent Errors ({})\n", v.name));
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
    report.push_str("- [ ] Tab bar icons visible and not duplicated (Chat/Git/Terminal/Queen/Settings)\n");
    report.push_str("- [ ] Chat input field visible at bottom with placeholder 'Ask anything...'\n");
    report.push_str("- [ ] No overlapping views, no black rectangles, no glitched rendering\n");
    report.push_str("- [ ] Glassmorphism blur visible behind panel content\n");
    report.push_str("- [ ] Messages scroll correctly without cutting off bubbles\n");
    report.push_str("- [ ] No duplicate headers or buttons outside tab bar\n");

    if let Err(e) = fs::write(&report_path, &report) {
        eprintln!("[e2e] Failed to write report: {}", e);
    }
    println!("{}", report_path.display());
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
