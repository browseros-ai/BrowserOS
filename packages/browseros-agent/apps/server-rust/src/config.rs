//! Process inputs: the CLI (`--version`, `--config <path>`), the Chromium-written sidecar
//! `config.json` (parsed like `@browseros/shared/schemas/sidecar-config`), environment variables,
//! and the exit-code contract Chromium's supervisor reads.

use serde_json::Value;
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
            let path = path.to_string_lossy();
            let path = path.trim();
            if path.is_empty() || path.starts_with('-') {
                return Err(CliError::ConfigPathMissing);
            }
            config = Some(PathBuf::from(path));
        } else if let Some(path) = arg.to_str().and_then(|arg| arg.strip_prefix("--config=")) {
            if path.trim().is_empty() {
                return Err(CliError::ConfigPathMissing);
            }
            config = Some(PathBuf::from(path.trim()));
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

impl Config {
    /// Validate the optional sidecar sections before requiring the server's four
    /// fields, matching shared/sidecar-config then config.ts projection order.
    pub fn load(path: &Path, env: &EnvConfig) -> Result<Self, ConfigError> {
        let path = absolute_path(path).map_err(|source| ConfigError::Read {
            path: path.to_owned(),
            source,
        })?;
        let invalid = |message: String| ConfigError::Invalid {
            path: path.clone(),
            message,
        };
        let text = std::fs::read_to_string(&path).map_err(|source| ConfigError::Read {
            path: path.clone(),
            source,
        })?;
        let file: Value = serde_json::from_str(&text)
            .map_err(|error| invalid(format!("Sidecar config file error: {error}")))?;
        if !file.is_object() {
            return Err(invalid(
                "Invalid sidecar configuration: config must be an object".into(),
            ));
        }
        for section in ["ports", "directories", "flags", "instance", "replay"] {
            if let Some(value) = file.get(section)
                && !value.is_object()
            {
                return Err(invalid(format!(
                    "Invalid sidecar configuration: {section} must be an object"
                )));
            }
        }
        let port = |name: &str| -> Result<Option<u16>, ConfigError> {
            let Some(value) = file.get("ports").and_then(|ports| ports.get(name)) else {
                return Ok(None);
            };
            let port = match value {
                Value::Number(number) => number
                    .as_f64()
                    .filter(|n| n.fract() == 0.0 && *n >= 1.0 && *n <= 65535.0)
                    .map(|n| n as u16),
                Value::String(text) => {
                    let text = text.trim();
                    (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
                        .then(|| text.parse::<u16>().ok().filter(|port| *port > 0))
                        .flatten()
                }
                _ => None,
            };
            port.map(Some).ok_or_else(|| invalid(format!("Invalid sidecar configuration:\n  - ports.{name}: must be an integer port between 1 and 65535")))
        };
        let server_port = port("server")?;
        let cdp_port = port("cdp")?;
        let proxy_port = port("proxy")?;
        let dir = |name: &str| -> Result<Option<PathBuf>, ConfigError> {
            let Some(value) = file
                .get("directories")
                .and_then(|directories| directories.get(name))
            else {
                return Ok(None);
            };
            let text = value.as_str().map(str::trim).filter(|text| !text.is_empty()).ok_or_else(|| invalid(format!("Invalid sidecar configuration:\n  - directories.{name}: must be a non-empty path")))?;
            Ok(Some(normalize_path(
                &path.parent().unwrap_or(Path::new(".")).join(text),
            )))
        };
        let resources_dir = dir("resources")?;
        let execution_dir = dir("execution")?;
        let allow_remote_in_mcp = match file.pointer("/flags/allow_remote_in_mcp") {
            Some(value) => value.as_bool().ok_or_else(|| {
                invalid(
                    "Invalid sidecar configuration: flags.allow_remote_in_mcp must be a boolean"
                        .into(),
                )
            })?,
            None => false,
        };
        let instance = |name: &str| -> Result<Option<String>, ConfigError> {
            file.get("instance")
                .and_then(|instance| instance.get(name))
                .map(|value| {
                    value.as_str().map(str::to_owned).ok_or_else(|| {
                        invalid(format!(
                            "Invalid sidecar configuration: instance.{name} must be a string"
                        ))
                    })
                })
                .transpose()
        };
        let instance = InstanceInfo {
            client_id: instance("client_id")?,
            install_id: instance("install_id")?,
            browseros_version: instance("browseros_version")?,
            chromium_version: instance("chromium_version")?,
        };
        if let Some(value) = file.pointer("/replay/retentionDays")
            && !value
                .as_f64()
                .is_some_and(|number| number.is_finite() && number > 0.0 && number.fract() == 0.0)
        {
            return Err(invalid(
                "Invalid sidecar configuration: replay.retentionDays must be a positive integer"
                    .into(),
            ));
        }
        let missing: Vec<&str> = [
            ("ports.server", server_port.is_none()),
            ("ports.cdp", cdp_port.is_none()),
            ("directories.resources", resources_dir.is_none()),
            ("directories.execution", execution_dir.is_none()),
        ]
        .into_iter()
        .filter_map(|(name, missing)| missing.then_some(name))
        .collect();
        if !missing.is_empty() {
            return Err(invalid(format!(
                "Invalid server sidecar configuration:\n{}",
                missing
                    .iter()
                    .map(|name| format!("  - {name}: Required"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )));
        }
        if env.node_env.as_deref() == Some("production") {
            let missing: Vec<&str> = [
                ("SENTRY_DSN", option_env!("SENTRY_DSN")),
                ("POSTHOG_API_KEY", option_env!("POSTHOG_API_KEY")),
            ]
            .into_iter()
            .filter_map(|(name, value)| {
                value
                    .filter(|value| !value.is_empty())
                    .is_none()
                    .then_some(name)
            })
            .collect();
            if !missing.is_empty() {
                return Err(invalid(format!(
                    "Missing required environment variables for production:\n{}",
                    missing
                        .iter()
                        .map(|name| format!("  - {name}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                )));
            }
        }
        // All four are proved present above; no fallback ports may escape into production.
        match (server_port, cdp_port, resources_dir, execution_dir) {
            (Some(server_port), Some(cdp_port), Some(resources_dir), Some(execution_dir)) => {
                Ok(Self {
                    server_port,
                    cdp_port,
                    proxy_port,
                    resources_dir,
                    execution_dir,
                    allow_remote_in_mcp,
                    instance,
                    browseros_dir: crate::paths::resolve_browseros_dir(env),
                    env: env.clone(),
                })
            }
            _ => Err(invalid("Invalid server sidecar configuration".into())),
        }
    }
}

fn absolute_path(path: &Path) -> std::io::Result<PathBuf> {
    Ok(normalize_path(&if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    }))
}

/// Lexical resolution like node:path.resolve: directories need not exist yet.
fn normalize_path(path: &Path) -> PathBuf {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                output.pop();
            }
            component => output.push(component.as_os_str()),
        }
    }
    output
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
