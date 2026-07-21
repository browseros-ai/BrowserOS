pub const SOVEREIGN_PORT: u16 = 9105;
pub const CANARY_PORT: u16 = 9205;
pub const A2A_PORT: u16 = 9200;
pub const DASHBOARD_PORT: u16 = 9300;
pub const CDP_PORT: u16 = 9106;
pub const DEV_PORT: u16 = 9305;

pub const GITHUB_ORG: &str = "gHashTag";

pub fn sovereign_health_url() -> String {
    format!("http://127.0.0.1:{}/health", port_or_env("SOVEREIGN_PORT", SOVEREIGN_PORT))
}

pub fn canary_health_url() -> String {
    format!("http://127.0.0.1:{}/health", port_or_env("CANARY_PORT", CANARY_PORT))
}

pub fn project_dir() -> String {
    std::env::var("TRIOS_ROOT").unwrap_or_else(|_| {
        match std::env::current_dir() {
            Ok(p) => p.to_string_lossy().into_owned(),
            Err(e) => {
                eprintln!("[FAIL] TRIOS_ROOT not set and current_dir unavailable: {}", e);
                std::process::exit(1);
            }
        }
    })
}

pub fn github_api_url(path: &str) -> String {
    format!("https://api.github.com/repos/{}/{}", GITHUB_ORG, path)
}

pub fn new_correlation_id() -> String {
    let ts = chrono::Utc::now().timestamp_millis() as u64;
    let pid = std::process::id();
    format!("{:x}-{:x}", ts, pid)
}

pub fn log_event(correlation_id: &str, event: &str, details: &str) {
    use std::io::Write;
    let ts = chrono::Utc::now().to_rfc3339();
    let escaped_details = details.replace('\\', "\\\\").replace('"', "\\\"");
    let escaped_event = event.replace('\\', "\\\\").replace('"', "\\\"");
    let line = format!(
        r#"{{"timestamp":"{}","correlation_id":"{}","event":"{}","details":"{}"}}"#,
        ts, correlation_id, escaped_event, escaped_details
    );
    let path = format!("{}/.trinity/event_log.jsonl", project_dir());
    if let Err(e) = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)
        .and_then(|mut f| writeln!(f, "{}", line))
    {
        eprintln!("[trios-config] Failed to write event log: {}", e);
    }
}

fn port_or_env(env_name: &str, default: u16) -> u16 {
    std::env::var(env_name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sovereign_url_default() {
        let url = sovereign_health_url();
        assert!(url.contains("9105"));
        assert!(url.ends_with("/health"));
    }

    #[test]
    fn canary_url_default() {
        let url = canary_health_url();
        assert!(url.contains("9205"));
        assert!(url.ends_with("/health"));
    }

    #[test]
    fn github_api_url_format() {
        let url = github_api_url("trios/pulls");
        assert!(url.contains("gHashTag"));
        assert!(url.ends_with("trios/pulls"));
    }

    #[test]
    fn project_dir_not_empty() {
        assert!(!project_dir().is_empty());
    }

    #[test]
    fn port_or_env_uses_default() {
        let port = port_or_env("NONEXISTENT_PORT_TEST_VAR", 1234);
        assert_eq!(port, 1234);
    }

    #[test]
    fn ports_are_distinct() {
        let ports = [SOVEREIGN_PORT, CANARY_PORT, A2A_PORT, DASHBOARD_PORT, CDP_PORT, DEV_PORT];
        for i in 0..ports.len() {
            for j in (i + 1)..ports.len() {
                assert_ne!(ports[i], ports[j], "ports at {} and {} collide", i, j);
            }
        }
    }

    #[test]
    fn correlation_id_not_empty() {
        let id = new_correlation_id();
        assert!(!id.is_empty());
        assert!(id.contains('-'));
    }

    #[test]
    fn correlation_id_unique() {
        let a = new_correlation_id();
        std::thread::sleep(std::time::Duration::from_millis(1));
        let b = new_correlation_id();
        assert_ne!(a, b);
    }

    #[test]
    fn github_org_is_set() {
        assert_eq!(GITHUB_ORG, "gHashTag");
    }
}
