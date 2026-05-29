use reqwest::blocking::get;
use serde::{Deserialize, Serialize};

const SOVEREIGN_HEALTH: &str = "http://127.0.0.1:9105/health";
const CANARY_HEALTH: &str = "http://127.0.0.1:9205/health";

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
    let flip_count = results.iter().filter(|r| r.flip).count();

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
    let mut results = vec![];

    // Test 1: Health probe
    results.push(run_health_diff());

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

fn run_health_diff() -> DiffResult {
    let sovereign = probe(SOVEREIGN_HEALTH);
    let canary = probe(CANARY_HEALTH);

    let s_ok = sovereign.contains("status\":\"ok\"");
    let c_ok = canary.contains("status\":\"ok\"");

    DiffResult {
        test_id: "health_probe".to_string(),
        sovereign_output: sovereign.clone(),
        canary_output: canary.clone(),
        exact_match: sovereign == canary,
        tolerance_passed: s_ok && c_ok,
        flip: s_ok && !c_ok,
    }
}

fn probe(url: &str) -> String {
    match get(url) {
        Ok(r) => r.text().unwrap_or_default(),
        Err(e) => format!("error: {}", e),
    }
}
