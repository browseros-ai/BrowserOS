//! Process inputs: the CLI (`--version`, `--config <path>`), the Chromium-written sidecar
//! `config.json` (parsed like `@browseros/shared/schemas/sidecar-config`), environment variables,
//! and the exit-code contract Chromium's supervisor reads. Owned by P3.

use serde::Deserialize;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

/// Exit codes Chromium interprets. `PORT_CONFLICT` makes it relaunch on another port; any other
/// non-zero code is a failed launch, so the two must never be folded together.
pub mod exit {
    pub const SUCCESS: i32 = 0;
    pub const GENERAL: i32 = 1;
    pub const PORT_CONFLICT: i32 = 2;
    pub const SIGNAL: i32 = 3;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliAction {
    Version,
    Run { config: PathBuf },
}

/// Message texts match the TS server's stderr lines.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CliError {
    #[error("--config is required")]
    ConfigRequired,
    #[error("--config requires a path")]
    ConfigPathMissing,
    #[error("Unknown argument: {0}")]
    UnknownArgument(String),
}

/// Parses `argv` without the program name. `--version` wins wherever it appears.
pub fn parse_cli(args: impl IntoIterator<Item = OsString>) -> Result<CliAction, CliError> {
    let args: Vec<OsString> = args.into_iter().collect();
    if args.iter().any(|arg| arg == "--version") {
        return Ok(CliAction::Version);
    }
    let mut config = None;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "--config" {
            let path = iter.next().ok_or(CliError::ConfigPathMissing)?;
            config = Some(PathBuf::from(path));
        } else if let Some(path) = arg.to_str().and_then(|arg| arg.strip_prefix("--config=")) {
            config = Some(PathBuf::from(path));
        } else {
            return Err(CliError::UnknownArgument(
                arg.to_string_lossy().into_owned(),
            ));
        }
    }
    config
        .map(|config| CliAction::Run { config })
        .ok_or(CliError::ConfigRequired)
}

/// The resolved process configuration. Directories are absolute (sidecar paths are relative to
/// the config file's directory).
#[derive(Debug, Clone)]
pub struct Config {
    pub server_port: u16,
    pub cdp_port: u16,
    /// Logged only; the proxy forwards LAN MCP traffic to `server_port`.
    pub proxy_port: Option<u16>,
    pub resources_dir: PathBuf,
    pub execution_dir: PathBuf,
    /// Parsed for parity and unused, as today.
    pub allow_remote_in_mcp: bool,
    pub instance: InstanceInfo,
    pub browseros_dir: PathBuf,
    pub env: EnvConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstanceInfo {
    pub client_id: Option<String>,
    pub install_id: Option<String>,
    /// Feeds native capability gating (profile tools appear on >= 0.51.1.3).
    pub browseros_version: Option<String>,
    pub chromium_version: Option<String>,
}

/// Environment the server reads once at startup (names kept from the TS server).
#[derive(Debug, Clone, Default)]
pub struct EnvConfig {
    pub browseros_dir: Option<PathBuf>,
    /// `NODE_ENV`; `development` selects `~/.browseros-dev`.
    pub node_env: Option<String>,
    pub trusted_origins: Vec<String>,
    pub mcp_public_url: Option<String>,
    pub acpx_probe_timeout_ms: Option<u64>,
    pub use_mock_llm: bool,
    pub log_level: Option<String>,
    pub shell: Option<String>,
    pub home: Option<PathBuf>,
}

impl EnvConfig {
    pub fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        Self {
            browseros_dir: var("BROWSEROS_DIR").map(PathBuf::from),
            node_env: var("NODE_ENV"),
            trusted_origins: var("BROWSEROS_TRUSTED_ORIGINS")
                .map(|value| {
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|origin| !origin.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            mcp_public_url: var("BROWSEROS_MCP_PUBLIC_URL"),
            acpx_probe_timeout_ms: var("BROWSEROS_ACPX_PROBE_TIMEOUT_MS")
                .and_then(|value| value.parse().ok()),
            use_mock_llm: var("BROWSEROS_USE_MOCK_LLM").as_deref() == Some("true"),
            log_level: var("LOG_LEVEL"),
            shell: var("SHELL"),
            home: var("HOME").map(PathBuf::from),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Failed to read config {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Invalid config {path}: {message}")]
    Invalid { path: PathBuf, message: String },
}

#[derive(Deserialize)]
struct SidecarFile {
    ports: SidecarPorts,
    directories: SidecarDirectories,
    #[serde(default)]
    flags: SidecarFlags,
    #[serde(default)]
    instance: SidecarInstance,
}

#[derive(Deserialize)]
struct SidecarPorts {
    server: PortValue,
    cdp: PortValue,
    #[serde(default)]
    proxy: Option<PortValue>,
}

/// Ports arrive as integers or numeric strings.
#[derive(Deserialize)]
#[serde(untagged)]
enum PortValue {
    Number(u16),
    Text(String),
}

#[derive(Deserialize)]
struct SidecarDirectories {
    resources: String,
    execution: String,
}

#[derive(Deserialize, Default)]
struct SidecarFlags {
    #[serde(default)]
    allow_remote_in_mcp: bool,
}

#[derive(Deserialize, Default)]
struct SidecarInstance {
    client_id: Option<String>,
    install_id: Option<String>,
    browseros_version: Option<String>,
    chromium_version: Option<String>,
}

impl Config {
    /// Reads and normalizes the sidecar. S0 covers the shape; P3 pins every TS error text and
    /// edge case (blank directories, out-of-range ports) with tests.
    pub fn load(path: &Path, env: &EnvConfig) -> Result<Self, ConfigError> {
        let invalid = |message: String| ConfigError::Invalid {
            path: path.to_path_buf(),
            message,
        };
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let file: SidecarFile =
            serde_json::from_str(&text).map_err(|error| invalid(error.to_string()))?;
        let port = |name: &str, value: PortValue| -> Result<u16, ConfigError> {
            match value {
                PortValue::Number(port) => Ok(port),
                PortValue::Text(text) => text
                    .trim()
                    .parse()
                    .map_err(|_| invalid(format!("ports.{name} must be a port number"))),
            }
        };
        let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let dir = |value: &str| base.join(value.trim());
        Ok(Self {
            server_port: port("server", file.ports.server)?,
            cdp_port: port("cdp", file.ports.cdp)?,
            proxy_port: file
                .ports
                .proxy
                .map(|proxy| port("proxy", proxy))
                .transpose()?,
            resources_dir: dir(&file.directories.resources),
            execution_dir: dir(&file.directories.execution),
            allow_remote_in_mcp: file.flags.allow_remote_in_mcp,
            instance: InstanceInfo {
                client_id: file.instance.client_id,
                install_id: file.instance.install_id,
                browseros_version: file.instance.browseros_version,
                chromium_version: file.instance.chromium_version,
            },
            browseros_dir: crate::paths::resolve_browseros_dir(env),
            env: env.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn cli_actions() {
        assert_eq!(parse_cli(args(&["--version"])), Ok(CliAction::Version));
        assert_eq!(
            parse_cli(args(&["--config", "c.json"])),
            Ok(CliAction::Run {
                config: PathBuf::from("c.json")
            })
        );
        assert_eq!(parse_cli(args(&[])), Err(CliError::ConfigRequired));
        assert_eq!(
            parse_cli(args(&["--config"])),
            Err(CliError::ConfigPathMissing)
        );
    }
}
