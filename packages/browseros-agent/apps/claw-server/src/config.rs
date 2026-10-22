use browseros_core::NativeCapabilities;
use clap::Parser;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    env, fs,
    num::{NonZeroU16, NonZeroU64},
    path::{Path, PathBuf},
    time::Duration,
};

const DEFAULT_SERVER_PORT: u16 = 9200;
const DEFAULT_CDP_PORT: u16 = 49337;
const DEFAULT_SESSION_IDLE_MS: u64 = 30 * 60 * 1000;
// Retain results so users can inspect agent-opened tabs and browser-side work can settle.
const DEFAULT_SESSION_RETENTION_MS: u64 = 60 * 60 * 1000;
const DEFAULT_SESSION_SWEEP_INTERVAL_MS: u64 = 60 * 1000;
const DEFAULT_REPLAY_RETENTION_DAYS: u64 = 7;
const BROWSERCLAW_DIR_NAME: &str = ".browserclaw";
/// Developer assertion of the native BrowserOS version for builds whose launch config does
/// not carry a usable one (dev/dogfood sidecars, stale local stamps). Logged at startup.
pub const NATIVE_VERSION_ENV: &str = "CLAW_NATIVE_BROWSEROS_VERSION";
/// Selects the [`ServerMode`]. Unset or blank means desktop.
pub const MODE_ENV: &str = "BROWSERCLAW_MODE";
const DEV_BROWSERCLAW_DIR_NAME: &str = ".browserclaw-dev";

#[derive(Debug, Parser)]
#[command(name = "browseros-claw-server-rs")]
pub struct Cli {
    #[arg(long, conflicts_with_all = ["config", "stdio"], help = "Print version")]
    version: bool,
    #[arg(long, required_unless_present = "version")]
    config: Option<PathBuf>,
    #[arg(long)]
    stdio: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CliAction {
    Version,
    Run { config: PathBuf, stdio: bool },
}

impl Cli {
    #[must_use]
    pub fn parse_action() -> CliAction {
        Self::parse().into_action()
    }

    fn into_action(self) -> CliAction {
        match (self.version, self.config) {
            (true, None) => CliAction::Version,
            (false, Some(config)) => CliAction::Run {
                config,
                stdio: self.stdio,
            },
            _ => unreachable!("Clap enforces version and run argument constraints"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub server_port: u16,
    pub cdp_port: u16,
    pub proxy_port: Option<u16>,
    pub resources_dir: PathBuf,
    pub browserclaw_dir: PathBuf,
    pub session_idle: Duration,
    pub session_retention: Duration,
    pub session_sweep_interval: Duration,
    pub replay_retention_days: u64,
    pub dev_mode: bool,
    /// Read once per process. Every discovery and execution path gates native features on
    /// this one value, which `BrowserService` copies into each browser session.
    pub native: NativeIdentity,
    /// Read once per process from [`MODE_ENV`]; the MCP surface follows it.
    pub mode: ServerMode,
}

/// How this server is deployed, fixed for the process by [`MODE_ENV`].
///
/// Desktop is the BrowserOS app on a person's own machine, with their agent harnesses
/// installed beside it. Hosted is BrowserOS neo run as infrastructure (the Docker image) and
/// driven remotely by an agent that brings its own harness. A hosted server cannot link
/// skills into that agent or reach its user, so the MCP layer hides the tools that assume it
/// can (see `api::mcp::hosted`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ServerMode {
    #[default]
    Desktop,
    Hosted,
}

impl ServerMode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Hosted => "hosted",
        }
    }

    #[must_use]
    pub fn is_hosted(self) -> bool {
        self == Self::Hosted
    }
}

/// The connected browser's native identity as this process learned it.
///
/// The browser that launches claw-server writes its compiled-in BrowserOS version into the
/// launch config (`instance.browseros_version`), separately from whichever OTA server build
/// it starts, and normally keeps this server as its child; a different browser comes with a
/// relaunched server and its own config. Dev and dogfood launchers write the config
/// themselves, so their value is only as good as the launcher's knowledge.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NativeIdentity {
    pub capabilities: NativeCapabilities,
    /// True when [`NATIVE_VERSION_ENV`] supplied the version instead of the launch config.
    pub asserted: bool,
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
    replay: Option<SidecarReplay>,
    /// Untyped on purpose: a missing, malformed or wrong-typed identity must leave the
    /// native unknown (gated features hidden), never fail startup and with it all browsing.
    #[serde(default)]
    instance: Option<serde_json::Value>,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SidecarReplay {
    retention_days: NonZeroU64,
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        Self::load_with_env(path, &ConfigEnv::from_process())
    }

    pub fn load_with_env(path: impl AsRef<Path>, env: &ConfigEnv) -> anyhow::Result<Self> {
        Self::load_with_env_and_default_dev_mode(path, env, dev_mode_fallback())
    }

    fn load_with_env_and_default_dev_mode(
        path: impl AsRef<Path>,
        env: &ConfigEnv,
        default_dev_mode: bool,
    ) -> anyhow::Result<Self> {
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
        let dev_mode = sidecar.flags.dev_mode.unwrap_or(default_dev_mode);
        let browserclaw_dir = resolve_browserclaw_dir(env, dev_mode, &cwd);
        let native = resolve_native_identity(env, sidecar.instance.as_ref());
        let mode = resolve_mode(env)?;

        Ok(Self {
            server_port,
            cdp_port,
            proxy_port,
            resources_dir,
            browserclaw_dir,
            session_idle: Duration::from_millis(read_positive_ms(
                env,
                "CLAW_SESSION_IDLE_MS",
                DEFAULT_SESSION_IDLE_MS,
            )),
            session_retention: Duration::from_millis(read_positive_ms(
                env,
                "CLAW_SESSION_RETENTION_MS",
                DEFAULT_SESSION_RETENTION_MS,
            )),
            session_sweep_interval: Duration::from_millis(read_positive_ms(
                env,
                "CLAW_SESSION_SWEEP_INTERVAL_MS",
                DEFAULT_SESSION_SWEEP_INTERVAL_MS,
            )),
            replay_retention_days: sidecar
                .replay
                .map(|replay| replay.retention_days.get())
                .unwrap_or(DEFAULT_REPLAY_RETENTION_DAYS),
            dev_mode,
            native,
            mode,
        })
    }

    /// Base URL external tools should reach BrowserClaw at: the Chrome-assigned
    /// proxy port when present (the source of truth), else the direct server
    /// port (dev, where the proxy is unavailable).
    #[must_use]
    pub fn public_base_url(&self) -> String {
        format!(
            "http://127.0.0.1:{}",
            self.proxy_port.unwrap_or(self.server_port)
        )
    }

    #[must_use]
    pub fn public_mcp_url(&self) -> String {
        format!("{}/mcp", self.public_base_url())
    }

    #[must_use]
    pub fn local_server_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.server_port)
    }
}

/// Selects Claw dev mode from the Rust build profile when config does not override it.
fn dev_mode_fallback() -> bool {
    cfg!(debug_assertions)
}

/// Dev mode never implies native support: only a reported or explicitly asserted version
/// counts.
fn resolve_native_identity(
    env: &ConfigEnv,
    instance: Option<&serde_json::Value>,
) -> NativeIdentity {
    if let Some(asserted) = env.get(NATIVE_VERSION_ENV).and_then(clean_string) {
        return NativeIdentity {
            capabilities: NativeCapabilities::from_browseros_version(Some(&asserted)),
            asserted: true,
        };
    }
    let reported = instance
        .and_then(|instance| instance.get("browseros_version"))
        .and_then(serde_json::Value::as_str);
    NativeIdentity {
        capabilities: NativeCapabilities::from_browseros_version(reported),
        asserted: false,
    }
}

/// Unlike a missing credential, an unrecognized mode fails startup: only an operator sets this
/// variable, and quietly serving desktop tools to a hosted agent would hide their typo.
fn resolve_mode(env: &ConfigEnv) -> anyhow::Result<ServerMode> {
    let Some(raw) = env.get(MODE_ENV).and_then(clean_string) else {
        return Ok(ServerMode::Desktop);
    };
    match raw.to_ascii_lowercase().as_str() {
        "desktop" => Ok(ServerMode::Desktop),
        "hosted" => Ok(ServerMode::Hosted),
        _ => anyhow::bail!("{MODE_ENV} must be \"desktop\" or \"hosted\", got {raw:?}"),
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
    parse_positive_int_prefix(raw).unwrap_or(fallback)
}

/// Accepts the established leading-integer format: trailing text is ignored
/// ("500ms" is 500), while digit-less or non-positive values fall back.
fn parse_positive_int_prefix(raw: &str) -> Option<u64> {
    let trimmed = raw.trim_start();
    let unsigned = trimmed.strip_prefix('+').unwrap_or(trimmed);
    let end = unsigned
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(unsigned.len());
    unsigned[..end]
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
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
    /// A blank or whitespace credential is no credential, which is what keeps a
    /// half-set variable from advertising a tool that cannot run. Asserted as a
    /// shape rather than through the environment, which no test here owns.
    #[test]
    fn the_offering_is_a_yes_or_a_no() {
        assert!(super::PursueOffering::Offered.is_offered());
        assert!(!super::PursueOffering::Absent.is_offered());
    }

    use super::{Cli, CliAction, Config, ConfigEnv, MODE_ENV, NATIVE_VERSION_ENV, ServerMode};
    use browseros_core::{BrowserOsVersion, NativeFeature};
    use clap::{Parser, error::ErrorKind};
    use std::{collections::BTreeMap, fs, path::PathBuf, time::Duration};
    use tempfile::tempdir;

    #[test]
    fn version_action_does_not_require_config() -> anyhow::Result<()> {
        let cli = Cli::try_parse_from(["browseros-claw-server-rs", "--version"])?;
        assert_eq!(cli.into_action(), CliAction::Version);
        Ok(())
    }

    #[test]
    fn run_action_requires_config() -> anyhow::Result<()> {
        let Err(error) = Cli::try_parse_from(["browseros-claw-server-rs"]) else {
            anyhow::bail!("run mode parsed without --config");
        };
        assert_eq!(error.kind(), ErrorKind::MissingRequiredArgument);
        Ok(())
    }

    #[test]
    fn version_action_rejects_run_flags() -> anyhow::Result<()> {
        for args in [
            &["browseros-claw-server-rs", "--version", "--stdio"][..],
            &[
                "browseros-claw-server-rs",
                "--version",
                "--config",
                "sidecar.json",
            ],
        ] {
            let Err(error) = Cli::try_parse_from(args) else {
                anyhow::bail!("version mode accepted run-only arguments: {args:?}");
            };
            assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
        }
        Ok(())
    }

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
        vars.insert("CLAW_SESSION_RETENTION_MS".to_string(), "2000".to_string());
        let cfg = Config::load_with_env(
            &config_path,
            &ConfigEnv::with_vars(vars, PathBuf::from("/tmp/home")),
        )?;
        assert_eq!(cfg.server_port, 9200);
        assert_eq!(cfg.cdp_port, 49337);
        assert_eq!(cfg.proxy_port, None);
        assert_eq!(cfg.session_idle, Duration::from_millis(1000));
        assert_eq!(cfg.session_retention, Duration::from_millis(2000));
        assert_eq!(cfg.replay_retention_days, 7);
        assert!(cfg.browserclaw_dir.ends_with("browserclaw"));
        assert_eq!(cfg.public_mcp_url(), "http://127.0.0.1:9200/mcp");
        // No proxy configured (dev): falls back to the direct server port.
        assert_eq!(cfg.public_base_url(), "http://127.0.0.1:9200");
        Ok(())
    }

    #[test]
    fn reads_replay_retention_days_from_sidecar_config() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let config_path = dir.path().join("sidecar.json");
        fs::write(&config_path, r#"{"replay":{"retentionDays":14}}"#)?;
        let cfg = Config::load_with_env(
            &config_path,
            &ConfigEnv::with_vars(BTreeMap::new(), dir.path().join("home")),
        )?;
        assert_eq!(cfg.replay_retention_days, 14);
        Ok(())
    }

    #[test]
    fn uses_build_profile_dev_mode_fallback_without_sidecar_flag() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let config_path = dir.path().join("sidecar.json");
        fs::write(&config_path, r#"{"ports":{},"directories":{}}"#)?;
        let env = ConfigEnv::with_vars(BTreeMap::new(), dir.path().join("home"));

        let debug_cfg = Config::load_with_env_and_default_dev_mode(&config_path, &env, true)?;
        assert!(debug_cfg.dev_mode);
        assert!(debug_cfg.browserclaw_dir.ends_with(".browserclaw-dev"));

        let release_cfg = Config::load_with_env_and_default_dev_mode(&config_path, &env, false)?;
        assert!(!release_cfg.dev_mode);
        assert!(release_cfg.browserclaw_dir.ends_with(".browserclaw"));
        Ok(())
    }

    #[test]
    fn ignores_node_env_when_sidecar_flag_is_missing() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let config_path = dir.path().join("sidecar.json");
        fs::write(&config_path, r#"{"ports":{},"directories":{}}"#)?;
        let mut vars = BTreeMap::new();
        vars.insert("NODE_ENV".to_string(), "development".to_string());
        let env = ConfigEnv::with_vars(vars, dir.path().join("home"));
        let release_cfg = Config::load_with_env_and_default_dev_mode(&config_path, &env, false)?;
        assert!(!release_cfg.dev_mode);
        assert!(release_cfg.browserclaw_dir.ends_with(".browserclaw"));

        let mut vars = BTreeMap::new();
        vars.insert("NODE_ENV".to_string(), "production".to_string());
        let env = ConfigEnv::with_vars(vars, dir.path().join("home"));
        let debug_cfg = Config::load_with_env_and_default_dev_mode(&config_path, &env, true)?;
        assert!(debug_cfg.dev_mode);
        assert!(debug_cfg.browserclaw_dir.ends_with(".browserclaw-dev"));
        Ok(())
    }

    #[test]
    fn sidecar_dev_mode_overrides_build_profile_fallback() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let env = ConfigEnv::with_vars(BTreeMap::new(), dir.path().join("home"));

        let prod_config_path = dir.path().join("prod-sidecar.json");
        fs::write(&prod_config_path, r#"{"flags":{"devMode":false}}"#)?;
        let prod_cfg = Config::load_with_env_and_default_dev_mode(&prod_config_path, &env, true)?;
        assert!(!prod_cfg.dev_mode);
        assert!(prod_cfg.browserclaw_dir.ends_with(".browserclaw"));

        let dev_config_path = dir.path().join("dev-sidecar.json");
        fs::write(&dev_config_path, r#"{"flags":{"devMode":true}}"#)?;
        let dev_cfg = Config::load_with_env_and_default_dev_mode(&dev_config_path, &env, false)?;
        assert!(dev_cfg.dev_mode);
        assert!(dev_cfg.browserclaw_dir.ends_with(".browserclaw-dev"));
        Ok(())
    }

    #[test]
    fn env_ms_overrides_fall_back_on_garbage_and_accept_padded_values() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let config_path = dir.path().join("sidecar.json");
        fs::write(&config_path, r#"{"ports":{},"directories":{}}"#)?;
        let home = dir.path().join("home");

        let cases: &[(&str, &str, Duration, Duration, Duration)] = &[
            // (idle raw, sweep raw, expected idle, expected retention, expected sweep)
            (
                "garbage",
                "-500",
                Duration::from_secs(30 * 60),
                Duration::from_millis(3_600_000),
                Duration::from_millis(60_000),
            ),
            (
                "0",
                "0x10",
                Duration::from_secs(30 * 60),
                Duration::from_millis(3_600_000),
                Duration::from_millis(60_000),
            ),
            // Number.parseInt parity: integer prefix wins, trailing garbage
            // ignored, surrounding whitespace tolerated.
            (
                " 2500 ",
                "500ms",
                Duration::from_millis(2500),
                Duration::from_millis(500),
                Duration::from_millis(500),
            ),
        ];
        for (idle_raw, sweep_raw, expected_idle, expected_retention, expected_sweep) in cases {
            let mut vars = BTreeMap::new();
            vars.insert("CLAW_SESSION_IDLE_MS".to_string(), (*idle_raw).to_string());
            vars.insert(
                "CLAW_SESSION_RETENTION_MS".to_string(),
                (*sweep_raw).to_string(),
            );
            vars.insert(
                "CLAW_SESSION_SWEEP_INTERVAL_MS".to_string(),
                (*sweep_raw).to_string(),
            );
            let cfg =
                Config::load_with_env(&config_path, &ConfigEnv::with_vars(vars, home.clone()))?;
            assert_eq!(cfg.session_idle, *expected_idle, "idle raw: {idle_raw:?}");
            assert_eq!(
                cfg.session_retention, *expected_retention,
                "retention raw: {sweep_raw:?}"
            );
            assert_eq!(
                cfg.session_sweep_interval, *expected_sweep,
                "sweep raw: {sweep_raw:?}"
            );
        }

        let cfg = Config::load_with_env(
            &config_path,
            &ConfigEnv::with_vars(BTreeMap::new(), home.clone()),
        )?;
        assert_eq!(cfg.session_idle, Duration::from_secs(30 * 60));
        assert_eq!(cfg.session_retention, Duration::from_millis(3_600_000));
        assert_eq!(cfg.session_sweep_interval, Duration::from_millis(60_000));
        Ok(())
    }

    #[test]
    fn blank_browserclaw_dir_override_is_ignored() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let config_path = dir.path().join("sidecar.json");
        fs::write(&config_path, r#"{"ports":{},"directories":{}}"#)?;
        let mut vars = BTreeMap::new();
        vars.insert("BROWSERCLAW_DIR".to_string(), "   ".to_string());
        let cfg = Config::load_with_env(
            &config_path,
            &ConfigEnv::with_vars(vars, dir.path().join("home")),
        )?;
        assert!(cfg.browserclaw_dir.starts_with(dir.path().join("home")));
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
        assert_eq!(cfg.public_mcp_url(), "http://127.0.0.1:9444/mcp");
        // Proxy configured: the source of truth is the proxy port.
        assert_eq!(cfg.public_base_url(), "http://127.0.0.1:9444");
        Ok(())
    }

    fn load_native(sidecar: &str, vars: &[(&str, &str)]) -> anyhow::Result<Config> {
        let dir = tempdir()?;
        let config_path = dir.path().join("sidecar.json");
        fs::write(&config_path, sidecar)?;
        let vars = vars
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect();
        Config::load_with_env(
            &config_path,
            &ConfigEnv::with_vars(vars, dir.path().join("home")),
        )
    }

    #[test]
    fn reads_the_browser_written_native_version() -> anyhow::Result<()> {
        // Shape written by the native BuildServerConfigJson.
        let cfg = load_native(
            r#"{"ports":{"cdp":9000,"server":9100,"proxy":9100},"flags":{"allow_remote_in_mcp":false},
                "instance":{"install_id":"i","browseros_version":"0.51.1.3","chromium_version":"155.0.8311.26"}}"#,
            &[],
        )?;
        assert_eq!(
            cfg.native.capabilities.browseros_version(),
            Some(BrowserOsVersion::new(0, 51, 1, 3))
        );
        assert!(
            cfg.native
                .capabilities
                .supports(NativeFeature::ProfileTargeting)
        );
        assert!(!cfg.native.asserted);
        let older = load_native(r#"{"instance":{"browseros_version":"0.51.1.2"}}"#, &[])?;
        assert!(
            !older
                .native
                .capabilities
                .supports(NativeFeature::ProfileTargeting)
        );
        Ok(())
    }

    #[test]
    fn missing_blank_or_wrong_typed_instance_is_unknown_and_still_starts() -> anyhow::Result<()> {
        for sidecar in [
            r#"{}"#,
            r#"{"instance":{"browseros_version":"","chromium_version":""}}"#,
            r#"{"instance":{"browseros_version":51}}"#,
            r#"{"instance":"0.51.1.3"}"#,
            r#"{"instance":null}"#,
            r#"{"instance":{"browseros_version":"not-a-version"}}"#,
        ] {
            let cfg = load_native(sidecar, &[])?;
            assert_eq!(
                cfg.native.capabilities.browseros_version(),
                None,
                "{sidecar}"
            );
            assert!(!cfg.native.asserted);
        }
        Ok(())
    }

    #[test]
    fn developer_assertion_overrides_and_is_marked() -> anyhow::Result<()> {
        let cfg = load_native(
            r#"{"instance":{"browseros_version":""}}"#,
            &[(NATIVE_VERSION_ENV, "0.51.1.3")],
        )?;
        assert!(
            cfg.native
                .capabilities
                .supports(NativeFeature::ProfileTargeting)
        );
        assert!(cfg.native.asserted);
        let blank = load_native(
            r#"{"instance":{"browseros_version":"0.51.1.2"}}"#,
            &[(NATIVE_VERSION_ENV, "  ")],
        )?;
        assert!(!blank.native.asserted);
        assert_eq!(
            blank.native.capabilities.browseros_version(),
            Some(BrowserOsVersion::new(0, 51, 1, 2))
        );
        Ok(())
    }

    #[test]
    fn mode_defaults_to_desktop_and_reads_hosted_from_the_environment() -> anyhow::Result<()> {
        assert_eq!(load_native("{}", &[])?.mode, ServerMode::Desktop);
        assert_eq!(
            load_native("{}", &[(MODE_ENV, "  ")])?.mode,
            ServerMode::Desktop
        );
        assert_eq!(
            load_native("{}", &[(MODE_ENV, "desktop")])?.mode,
            ServerMode::Desktop
        );
        for hosted in ["hosted", " Hosted ", "HOSTED"] {
            assert_eq!(
                load_native("{}", &[(MODE_ENV, hosted)])?.mode,
                ServerMode::Hosted,
                "{hosted:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn an_unrecognized_mode_fails_startup_and_names_the_variable() {
        let error = load_native("{}", &[(MODE_ENV, "hostd")])
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default();
        assert!(error.contains(MODE_ENV), "{error}");
        assert!(error.contains("\"hostd\""), "{error}");
    }
}

// The decision loop is one environment variable away from being a different
// product, and more than one layer needs the answer: the MCP catalogue, the
// server instructions, run's own description and the managed skill all behave
// differently when it is present. It sits with the other configuration because
// the skill loader is a service and must not reach up into the API layer to ask.
/// Where the decision model credential comes from.
///
/// Three states rather than two, because the environment case is real and a boolean cannot say
/// it: the loop works, the page shows nothing, and the user pastes a token they did not need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSource {
    /// Saved through the API. Takes precedence.
    Settings,
    /// `JEV_TOKEN` in the server's environment.
    Environment,
    /// Neither, so the pursue tool is not offered.
    None,
}

impl TokenSource {
    /// Whether a credential exists at all, by either route.
    #[must_use]
    pub fn is_present(self) -> bool {
        self != Self::None
    }
}

/// Whether the decision loop is offered by this server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PursueOffering {
    /// There is a credential, so the loop can run and is advertised.
    Offered,
    /// There is none. Everything behaves exactly as it did before the loop
    /// existed, which is what every production install sees.
    Absent,
}

impl PursueOffering {
    /// Reads the answer off the credential's source.
    ///
    /// A token saved in the settings and `JEV_TOKEN` both offer the loop. Which of the two wins
    /// is decided in the settings store, so that nothing here has to know there are two places
    /// to look, and so the answer cannot differ between layers.
    #[must_use]
    pub fn of(source: TokenSource) -> Self {
        if source.is_present() {
            Self::Offered
        } else {
            Self::Absent
        }
    }

    #[must_use]
    pub fn is_offered(self) -> bool {
        self == Self::Offered
    }
}

// The credential itself is read from the settings store, which owns the precedence between a
// saved token and JEV_TOKEN. A second reader of the environment here is how the two answers
// drift apart, so there is not one.
