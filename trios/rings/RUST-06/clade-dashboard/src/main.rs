use chrono::Utc;
use serde::{Deserialize, Serialize};
use warp::Filter;

fn project_dir() -> String { std::env::var("TRIOS_ROOT").unwrap_or_else(|_| "/Users/playra/BrowserOS-full/trios".to_string()) }
const DEFAULT_PORT: u16 = 9405;

fn port() -> u16 {
    std::env::var("TRIOS_PORT_DASHBOARD")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct HealthStatus {
    sovereign: bool,
    canary: bool,
    timestamp: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct SafetyBudget {
    budget: f64,
    max_budget: f64,
    total_trials: u64,
    total_failures: u64,
    halted: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct HealthScore {
    score: f64,
    components: Vec<HealthComponent>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct HealthComponent {
    name: String,
    weight: f64,
    healthy: bool,
}

fn compute_health_score(sovereign: bool, canary: bool, budget: &SafetyBudget, snapshot_count: usize) -> HealthScore {
    let components = vec![
        HealthComponent { name: "sovereign".to_string(), weight: 0.4, healthy: sovereign },
        HealthComponent { name: "canary".to_string(), weight: 0.2, healthy: canary },
        HealthComponent { name: "budget_ok".to_string(), weight: 0.2, healthy: !budget.halted && budget.budget > 0.0 },
        HealthComponent { name: "snapshots".to_string(), weight: 0.1, healthy: snapshot_count > 0 },
        HealthComponent { name: "budget_margin".to_string(), weight: 0.1, healthy: budget.budget > 1.0 },
    ];
    let score = components.iter()
        .map(|c| if c.healthy { c.weight } else { 0.0 })
        .sum();
    HealthScore { score, components }
}

#[derive(Serialize, Deserialize, Debug)]
struct DashboardData {
    health: HealthStatus,
    health_score: HealthScore,
    budget: SafetyBudget,
    clade_id: String,
    snapshots: Vec<String>,
    recent_events: Vec<String>,
}

#[tokio::main]
async fn main() {
    let p = port();
    println!("[CladeDashboard] Starting on http://127.0.0.1:{}", p);
    println!("[CladeDashboard] Press Ctrl+C to stop");

    let api = warp::path("api")
        .and(warp::path("status"))
        .and_then(|| async {
            let data = gather_status().await;
            Ok::<_, warp::Rejection>(warp::reply::json(&data))
        });

    let health = warp::path("health")
        .map(|| warp::reply::json(&serde_json::json!({"status": "ok"})));

    let routes = api.or(health);

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], p));
    match std::net::TcpListener::bind(addr) {
        Ok(listener) => {
            match tokio::net::TcpListener::from_std(listener) {
                Ok(tokio_listener) => {
                    println!("[CladeDashboard] Port {} bound, starting server...", p);
                    warp::serve(routes).run_incoming(
                        tokio_stream::wrappers::TcpListenerStream::new(tokio_listener),
                    ).await;
                }
                Err(e) => {
                    eprintln!("[CladeDashboard] Failed to convert listener: {} — exiting", e);
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("[CladeDashboard] Cannot bind to port {}: {} — exiting cleanly", p, e);
            std::process::exit(1);
        }
    }
}

async fn gather_status() -> DashboardData {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    let (sovereign, canary, budget, clade_id, snapshots, recent_events) = tokio::join!(
        check_health(&client, "http://127.0.0.1:9105/health"),
        check_health(&client, "http://127.0.0.1:9205/health"),
        load_safety_budget(),
        load_clade_id(),
        list_snapshots(),
        load_recent_events(10),
    );

    let health_score = compute_health_score(sovereign, canary, &budget, snapshots.len());

    DashboardData {
        health: HealthStatus {
            sovereign,
            canary,
            timestamp: Utc::now().to_rfc3339(),
        },
        health_score,
        budget,
        clade_id,
        snapshots,
        recent_events,
    }
}

async fn check_health(client: &reqwest::Client, url: &str) -> bool {
    match client.get(url).send().await {
        Ok(r) => {
            let body = r.text().await.unwrap_or_default();
            body.contains("\"status\":\"ok\"")
        }
        Err(_) => false,
    }
}

async fn load_safety_budget() -> SafetyBudget {
    let path = format!("{}/.trinity/state/safety_budget.json", &project_dir());
    match tokio::fs::read_to_string(&path).await {
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

async fn load_clade_id() -> String {
    let path = format!("{}/.trinity/state/clade.json", &project_dir());
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => {
            let json: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();
            json.get("id").and_then(|v| v.as_str()).unwrap_or("unknown").to_string()
        }
        Err(_) => "unknown".to_string(),
    }
}

async fn list_snapshots() -> Vec<String> {
    let dir = format!("{}/.trinity/snapshots", &project_dir());
    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(e) => e,
        Err(_) => return vec![],
    };
    let mut snapshots = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        if let Some(name) = entry.file_name().to_str() {
            if name.starts_with("trios_app-") {
                snapshots.push(name.to_string());
            }
        }
        if snapshots.len() >= 10 {
            break;
        }
    }
    snapshots
}

async fn load_recent_events(limit: usize) -> Vec<String> {
    let path = format!("{}/.trinity/event_log.jsonl", &project_dir());
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => content
            .lines()
            .rev()
            .take(limit)
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect(),
        Err(_) => vec![],
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
        assert!(!b.halted);
    }

    #[test]
    fn dashboard_default_port() {
        assert_eq!(DEFAULT_PORT, 9405);
    }

    #[test]
    fn port_returns_default_without_env() {
        assert_eq!(port(), DEFAULT_PORT);
    }

    #[test]
    fn health_status_serializes() {
        let h = HealthStatus {
            sovereign: true,
            canary: false,
            timestamp: "2026-06-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&h).unwrap_or_default();
        assert!(json.contains("\"sovereign\":true"));
        assert!(json.contains("\"canary\":false"));
    }

    #[test]
    fn dashboard_data_serializes() {
        let budget = default_budget();
        let score = compute_health_score(true, true, &budget, 1);
        let d = DashboardData {
            health: HealthStatus { sovereign: true, canary: true, timestamp: "now".to_string() },
            health_score: score,
            budget,
            clade_id: "test-clade".to_string(),
            snapshots: vec!["snap1".to_string()],
            recent_events: vec![],
        };
        let json = serde_json::to_string(&d).unwrap_or_default();
        assert!(json.contains("\"clade_id\":\"test-clade\""));
        assert!(json.contains("\"snap1\""));
    }

    #[test]
    fn health_score_all_healthy() {
        let budget = default_budget();
        let s = compute_health_score(true, true, &budget, 3);
        assert!((s.score - 1.0).abs() < f64::EPSILON);
        assert_eq!(s.components.len(), 5);
    }

    #[test]
    fn health_score_sovereign_down() {
        let budget = default_budget();
        let s = compute_health_score(false, true, &budget, 3);
        assert!((s.score - 0.6).abs() < f64::EPSILON);
    }

    #[test]
    fn health_score_all_down() {
        let budget = SafetyBudget {
            budget: 0.0, max_budget: 5.0, total_trials: 0, total_failures: 0, halted: true,
        };
        let s = compute_health_score(false, false, &budget, 0);
        assert!((s.score - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn health_score_weights_sum_to_one() {
        let budget = default_budget();
        let s = compute_health_score(true, true, &budget, 1);
        let total_weight: f64 = s.components.iter().map(|c| c.weight).sum();
        assert!((total_weight - 1.0).abs() < f64::EPSILON);
    }
}
