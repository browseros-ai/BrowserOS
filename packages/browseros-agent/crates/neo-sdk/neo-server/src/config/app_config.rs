use std::path::{Path, PathBuf};
use std::{env, io};

use neo_captcha::config::CaptchaConfig;

use super::{EnvConfig, LaunchConfig, SecretFiles};

/// Everything a process starts with, parsed once at the boundary (`main`) and trusted after.
#[derive(Clone, Debug)]
pub struct AppConfig {
    pub(crate) launch: LaunchConfig,
    pub(crate) env: EnvConfig,
    /// `neo.solveCaptcha`'s providers, from neo-captcha's own variables
    /// (`BROWSEROS_CAPSOLVER_API_KEY_FILE`, `BROWSEROS_2CAPTCHA_API_KEY_FILE` and their
    /// neighbors). The key files are read here, once, so the solver never touches the
    /// environment or the disk on a run. Its `Debug` redacts the keys.
    pub(crate) captcha: CaptchaConfig,
    /// Where Neo's own secrets are on disk, which `setInputFiles` never uploads. Located here,
    /// once: neo-playwright's hooks are fixed when the app builds its `Playwright`, so the
    /// list must exist before then.
    pub(crate) secret_files: SecretFiles,
}

/// A config Neo cannot start with. `main` prints it on one line and exits 1, never 2, which
/// Chromium reads as "port in use".
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The launch file is missing or unreadable.
    #[error("reading {}: {source}", path.display())]
    Read {
        /// The `--config` path.
        path: PathBuf,
        /// Why it could not be read.
        source: io::Error,
    },
    /// The launch file is not the JSON Chromium writes.
    #[error("parsing {}: {source}", path.display())]
    Parse {
        /// The `--config` path.
        path: PathBuf,
        /// Where and why the JSON failed.
        source: serde_json::Error,
    },
    /// An env var has a value Neo cannot use.
    #[error("{name}: {why}")]
    Env {
        /// The variable.
        name: &'static str,
        /// What is wrong with its value.
        why: String,
    },
}

impl AppConfig {
    /// Reads Chromium's launch file at `path` and the process environment.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let lookup = |name: &str| env::var(name).ok();
        let cwd = env::current_dir();
        let launch = LaunchConfig::read(path)?;
        let env = EnvConfig::from_lookup(&lookup, &env::home_dir)?;
        Ok(Self {
            secret_files: SecretFiles::locate(&env, &launch, &cwd),
            launch,
            env,
            // A key file that cannot be read leaves its provider off, as the crate decides;
            // app.rs logs the outcome once logging is up.
            captcha: CaptchaConfig::from_lookup(lookup),
        })
    }

    /// The native browser's version for core's capability gates: the developer's
    /// `CLAW_NATIVE_BROWSEROS_VERSION` when set, else what Chromium wrote in the launch file.
    #[must_use]
    pub(crate) fn browseros_version(&self) -> Option<&str> {
        self.env
            .native_version
            .as_deref()
            .or(self.launch.browseros_version.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use neo_captcha::config::CaptchaConfig;

    use super::AppConfig;
    use crate::config::{EnvConfig, HostApiConfig, HostedMode, LaunchConfig, SecretFiles};
    use crate::test_support::CLOSED_XVNC;

    fn config(launch_version: Option<&str>, env_version: Option<&str>) -> AppConfig {
        AppConfig {
            launch: LaunchConfig {
                server_port: 9200,
                cdp_port: 49337,
                resources_dir: PathBuf::from("resources"),
                execution_dir: None,
                browseros_version: launch_version.map(str::to_owned),
            },
            env: EnvConfig {
                state_dir: PathBuf::from("/state"),
                home: None,
                captcha_key_files: Vec::new(),
                log_filter: "info".to_owned(),
                hosted_mode: HostedMode::On,
                native_version: env_version.map(str::to_owned),
                ignored_config_file: None,
                host_api: HostApiConfig {
                    socket: PathBuf::from("/state/host.sock"),
                    token_file: None,
                    tcp: None,
                },
                xvnc: CLOSED_XVNC,
            },
            captcha: CaptchaConfig::default(),
            secret_files: SecretFiles::Named(Vec::new()),
        }
    }

    #[test]
    fn the_env_version_overrides_the_launch_file() {
        assert_eq!(
            config(Some("0.51.1.3"), Some("0.52.0.0")).browseros_version(),
            Some("0.52.0.0"),
            "a developer's assertion wins over a launcher's stale stamp"
        );
        assert_eq!(
            config(Some("0.51.1.3"), None).browseros_version(),
            Some("0.51.1.3"),
            "otherwise the browser's own report"
        );
        assert_eq!(config(None, None).browseros_version(), None, "unknown");
    }
}
