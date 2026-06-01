use reqwest::blocking::get;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct DiffResult {
    test_id: String,
    sovereign_output: String,
    canary_output: String,
    exact_match: bool,
    tolerance_passed: bool,
    flip: bool,
}

fn main() {
    println!("═══════════════════════════════════════════════════════════");
    println!("  CLADE-DIFF: Differential Testing — Sovereign vs Canary");
    println!("═══════════════════════════════════════════════════════════\n");

    let results = run_differential_suite();
    let flip_count = count_flips(&results);

    println!("\n📊 Results: {}/{} passed, {} flips",
        results.len() - flip_count, results.len(), flip_count);

    if flip_count > 0 {
        println!("\n🔴 REJECTED: {} regression(s) detected", flip_count);
        for r in results.iter().filter(|r| r.flip) {
            println!("   ❌ {}: Sovereign passed, Canary failed", r.test_id);
        }
        std::process::exit(1);
    }

    println!("\n🟢 APPROVED: no regressions detected");
}

fn run_differential_suite() -> Vec<DiffResult> {
    let mut results = vec![run_health_diff()];

    // Test 2: Static response comparison (placeholder for real API tests)
    results.push(DiffResult {
        test_id: "static_api".to_string(),
        sovereign_output: "{}".to_string(),
        canary_output: "{}".to_string(),
        exact_match: true,
        tolerance_passed: true,
        flip: false,
    });

    results
}

fn compute_diff(test_id: &str, sovereign: &str, canary: &str) -> DiffResult {
    let s_ok = sovereign.contains("status\":\"ok\"");
    let c_ok = canary.contains("status\":\"ok\"");

    DiffResult {
        test_id: test_id.to_string(),
        sovereign_output: sovereign.to_string(),
        canary_output: canary.to_string(),
        exact_match: sovereign == canary,
        tolerance_passed: s_ok && c_ok,
        flip: s_ok && !c_ok,
    }
}

fn run_health_diff() -> DiffResult {
    let sovereign = probe(&trios_config::sovereign_health_url());
    let canary = probe(&trios_config::canary_health_url());
    compute_diff("health_probe", &sovereign, &canary)
}

fn count_flips(results: &[DiffResult]) -> usize {
    results.iter().filter(|r| r.flip).count()
}

const MAX_RESPONSE_BYTES: usize = 1024 * 1024; // 1MB

fn probe(url: &str) -> String {
    match get(url) {
        Ok(r) => {
            let body = r.text().unwrap_or_default();
            if body.len() > MAX_RESPONSE_BYTES {
                format!("error: response too large ({}KB)", body.len() / 1024)
            } else {
                body
            }
        }
        Err(e) => format!("error: {}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_healthy_no_flip() {
        let r = compute_diff("t1", r#"{"status":"ok"}"#, r#"{"status":"ok"}"#);
        assert!(!r.flip);
        assert!(r.exact_match);
        assert!(r.tolerance_passed);
    }

    #[test]
    fn sovereign_ok_canary_fail_is_flip() {
        let r = compute_diff("t2", r#"{"status":"ok"}"#, r#"{"status":"error"}"#);
        assert!(r.flip);
        assert!(!r.tolerance_passed);
    }

    #[test]
    fn both_down_no_flip() {
        let r = compute_diff("t3", "error: connection refused", "error: connection refused");
        assert!(!r.flip);
        assert!(r.exact_match);
    }

    #[test]
    fn canary_ok_sovereign_down_no_flip() {
        let r = compute_diff("t4", "error: timeout", r#"{"status":"ok"}"#);
        assert!(!r.flip);
    }

    #[test]
    fn count_flips_mixed() {
        let results = vec![
            compute_diff("a", r#"{"status":"ok"}"#, r#"{"status":"ok"}"#),
            compute_diff("b", r#"{"status":"ok"}"#, "error"),
            compute_diff("c", r#"{"status":"ok"}"#, "down"),
        ];
        assert_eq!(count_flips(&results), 2);
    }

    #[test]
    fn count_flips_none() {
        let results = vec![
            compute_diff("a", r#"{"status":"ok"}"#, r#"{"status":"ok"}"#),
        ];
        assert_eq!(count_flips(&results), 0);
    }

    #[test]
    fn count_flips_empty() {
        let results: Vec<DiffResult> = vec![];
        assert_eq!(count_flips(&results), 0);
    }

    #[test]
    fn max_response_bytes_is_1mb() {
        assert_eq!(MAX_RESPONSE_BYTES, 1024 * 1024);
    }

    #[test]
    fn probe_unreachable_returns_error() {
        let result = probe("http://127.0.0.1:1/nonexistent");
        assert!(result.starts_with("error:"));
    }
}
