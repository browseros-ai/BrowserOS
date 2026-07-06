use clap::Parser;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    env, fs,
    num::NonZeroU16,
    path::{Path, PathBuf},
    time::Duration,
};

const DEFAULT_SERVER_PORT: u16 = 9200;
const DEFAULT_CDP_PORT: u16 = 49337;
const DEFAULT_SESSION_IDLE_MS: u64 = 5 * 60 * 1000;
const DEFAULT_SESSION_SWEEP_INTERVAL_MS: u64 = 60 * 1000;
const BROWSERCLAW_DIR_NAME: &str = ".browserclaw";
const DEV_BROWSERCLAW_DIR_NAME: &str = ".browserclaw-dev";

#[derive(Debug, Parser)]
#[command(name = "browseros-claw-server-rs")]
pub struct Cli {
    #[arg(long)]
    pub config: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub server_port: u16,
    pub cdp_port: u16,
    pub proxy_port: Option<u16>,
    pub resources_dir: PathBuf,
    pub browserclaw_dir: PathBuf,
    pub claw_dir: PathBuf,
    pub session_idle: Duration,
    pub session_sweep_interval: Duration,
    pub screencast_screenshot_fallback: bool,
    pub dev_mode: bool,
    pub auth_token: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ConfigEnv {
    vars: BTreeMap<String, String>,
    home_dir: Option<PathBuf>,
}

impl ConfigEnv {
    #[must_use]
    pub fn from_process() -> Self {
        Self {
            vars: env::vars().collect(),
            home_dir: env::var_os("HOME").map(PathBuf::from),
        }
    }

    #[must_use]
    pub fn with_vars(vars: BTreeMap<String, String>, home_dir: PathBuf) -> Self {
        Self {
            vars,
            home_dir: Some(home_dir),
        }
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(String::as_str)
    }
}

#[derive(Debug, Deserialize)]
struct SidecarConfig {
    #[serde(default)]
    ports: SidecarPorts,
    #[serde(default)]
    directories: SidecarDirectories,
    #[serde(default)]
    flags: SidecarFlags,
    #[serde(default)]
    auth: SidecarAuth,
}

#[derive(Debug, Default, Deserialize)]
struct SidecarPorts {
    server: Option<NonZeroU16>,
    cdp: Option<NonZeroU16>,
    proxy: Option<NonZeroU16>,
}

#[derive(Debug, Default, Deserialize)]
struct SidecarDirectories {
    resources: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SidecarFlags {
    dev_mode: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct SidecarAuth {
    token: Option<String>,
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        Self::load_with_env(path, &ConfigEnv::from_process())
    }

    pub fn load_with_env(path: impl AsRef<Path>, env: &ConfigEnv) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let raw = fs::read_to_string(path)?;
        let sidecar: SidecarConfig = serde_json::from_str(&raw)?;
        let cwd = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let server_port = sidecar
            .ports
            .server
            .map(NonZeroU16::get)
            .unwrap_or(DEFAULT_SERVER_PORT);
        let cdp_port = sidecar
            .ports
            .cdp
            .map(NonZeroU16::get)
            .unwrap_or(DEFAULT_CDP_PORT);
        let proxy_port = sidecar.ports.proxy.map(NonZeroU16::get);
        let resources_dir = sidecar
            .directories
            .resources
            .map(|path| resolve_path(&cwd, path))
            .unwrap_or_else(|| cwd.join("resources"));
        let dev_mode = sidecar.flags.dev_mode.unwrap_or_else(|| {
            env.get("NODE_ENV")
                .map(|value| value == "development")
                .unwrap_or(false)
        });
        let browserclaw_dir = resolve_browserclaw_dir(env, dev_mode, &cwd);
        let claw_dir = browserclaw_dir.clone();
        let auth_token = sidecar
            .auth
            .token
            .and_then(|token| clean_string(token.as_str()));

        Ok(Self {
            server_port,
            cdp_port,
            proxy_port,
            resources_dir,
            browserclaw_dir,
            claw_dir,
            session_idle: Duration::from_millis(read_positive_ms(
                env,
                "CLAW_SESSION_IDLE_MS",
                DEFAULT_SESSION_IDLE_MS,
            )),
            session_sweep_interval: Duration::from_millis(read_positive_ms(
                env,
                "CLAW_SESSION_SWEEP_INTERVAL_MS",
                DEFAULT_SESSION_SWEEP_INTERVAL_MS,
            )),
            screencast_screenshot_fallback: read_bool_default_true(
                env,
                "CLAW_SCREENCAST_SCREENSHOT_FALLBACK",
            ),
            dev_mode,
            auth_token,
        })
    }

    #[must_use]
    pub fn public_mcp_url(&self) -> String {
        format!(
            "http://127.0.0.1:{}/mcp",
            self.proxy_port.unwrap_or(self.server_port)
        )
    }

    #[must_use]
    pub fn local_server_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.server_port)
    }
}

fn resolve_path(cwd: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

fn resolve_browserclaw_dir(env: &ConfigEnv, dev_mode: bool, cwd: &Path) -> PathBuf {
    if let Some(raw) = env.get("BROWSERCLAW_DIR").and_then(clean_string) {
        return PathBuf::from(raw);
    }
    let home = env.home_dir.clone().unwrap_or_else(|| cwd.to_path_buf());
    home.join(if dev_mode {
        DEV_BROWSERCLAW_DIR_NAME
    } else {
        BROWSERCLAW_DIR_NAME
    })
}

fn read_positive_ms(env: &ConfigEnv, key: &str, fallback: u64) -> u64 {
    let Some(raw) = env.get(key) else {
        return fallback;
    };
    raw.parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .unwrap_or(fallback)
}

fn read_bool_default_true(env: &ConfigEnv, key: &str) -> bool {
    let Some(raw) = env.get(key) else {
        return true;
    };
    let normalized = raw.trim().to_ascii_lowercase();
    normalized != "0" && normalized != "false"
}

fn clean_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, ConfigEnv};
    use std::{collections::BTreeMap, fs, path::PathBuf, time::Duration};
    use tempfile::tempdir;

    #[test]
    fn parses_sidecar_defaults_and_browserclaw_dir_override() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let config_path = dir.path().join("sidecar.json");
        fs::write(&config_path, r#"{"ports":{},"directories":{}}"#)?;
        let mut vars = BTreeMap::new();
        vars.insert(
            "BROWSERCLAW_DIR".to_string(),
            dir.path().join("browserclaw").to_string_lossy().to_string(),
        );
        vars.insert("CLAW_SESSION_IDLE_MS".to_string(), "1000".to_string());
        let cfg = Config::load_with_env(
            &config_path,
            &ConfigEnv::with_vars(vars, PathBuf::from("/tmp/home")),
        )?;
        assert_eq!(cfg.server_port, 9200);
        assert_eq!(cfg.cdp_port, 49337);
        assert_eq!(cfg.proxy_port, None);
        assert_eq!(cfg.session_idle, Duration::from_millis(1000));
        assert!(cfg.browserclaw_dir.ends_with("browserclaw"));
        assert_eq!(cfg.claw_dir, cfg.browserclaw_dir);
        assert_eq!(cfg.public_mcp_url(), "http://127.0.0.1:9200/mcp");
        Ok(())
    }

    #[test]
    fn honors_ports_proxy_and_development_dir() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let config_path = dir.path().join("sidecar.json");
        fs::write(
            &config_path,
            r#"{"ports":{"server":9300,"cdp":49338,"proxy":9444},"flags":{"devMode":true}}"#,
        )?;
        let cfg = Config::load_with_env(
            &config_path,
            &ConfigEnv::with_vars(BTreeMap::new(), dir.path().join("home")),
        )?;
        assert_eq!(cfg.server_port, 9300);
        assert_eq!(cfg.cdp_port, 49338);
        assert_eq!(cfg.proxy_port, Some(9444));
        assert!(cfg.browserclaw_dir.ends_with(".browserclaw-dev"));
        assert_eq!(cfg.claw_dir, cfg.browserclaw_dir);
        assert_eq!(cfg.public_mcp_url(), "http://127.0.0.1:9444/mcp");
        Ok(())
    }
}
