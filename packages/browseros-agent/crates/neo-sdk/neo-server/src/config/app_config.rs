use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{env, io};

use neo_http::{Settings, SettingsDefaults};
use neo_playwright::{FileAccess, ProtectedPath};

use neo_captcha::config::CaptchaConfig;

use super::{EnvConfig, LaunchConfig};

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
        let launch = LaunchConfig::read(path)?;
        let env = EnvConfig::from_lookup(&lookup, &env::home_dir)?;
        Ok(Self {
            launch,
            env,
            // A key file that cannot be read leaves its provider off, as the crate decides;
            // app.rs logs the outcome once logging is up.
            captcha: CaptchaConfig::from_lookup(lookup),
        })
    }

    /// The application owns filesystem layout; settings stores only the host's overrides.
    pub(crate) fn settings_defaults(&self) -> SettingsDefaults {
        if self.env.downloads_dir.is_none() {
            tracing::warn!(
                "downloads directory could not be discovered; files.readDirs defaults to an empty list"
            );
        }
        SettingsDefaults {
            read_dirs: self.env.downloads_dir.iter().cloned().collect(),
        }
    }

    /// Connects live settings to the library's path policy once for either startup mode.
    /// Protected paths stay configured even before their files or directories exist.
    pub(crate) fn file_access(&self, settings: Settings) -> Arc<FileAccess> {
        let mut own = vec![ProtectedPath::Directory(self.env.state_dir.clone())];
        let profile = self.env.profile_dir.as_deref().or_else(|| {
            self.launch
                .execution_dir
                .as_deref()
                .filter(|execution| {
                    execution
                        .file_name()
                        .is_some_and(|name| name == ".browseros")
                })
                .and_then(Path::parent)
        });
        if let Some(profile) = profile {
            own.push(ProtectedPath::Directory(profile.to_owned()));
        } else {
            tracing::warn!(
                "browser profile is not protected: set CLAW_PROFILE_DIR when the launch execution directory does not identify .browseros"
            );
        }
        own.extend(
            self.env
                .host_api
                .token_file
                .iter()
                .cloned()
                .map(ProtectedPath::File),
        );
        own.extend(
            self.env
                .captcha_key_files
                .iter()
                .cloned()
                .map(ProtectedPath::File),
        );
        Arc::new(FileAccess::new(move || settings.read_dirs(), own))
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
    use std::error::Error;
    use std::path::PathBuf;

    use neo_captcha::config::CaptchaConfig;

    use super::AppConfig;
    use crate::config::{EnvConfig, HostApiConfig, HostedMode, LaunchConfig};
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
                downloads_dir: Some(PathBuf::from("/downloads")),
                profile_dir: None,
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
                ignored_trace_value_bytes: false,
            },
            captcha: CaptchaConfig::default(),
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
    #[tokio::test]
    async fn file_policy_uses_the_launch_profile_and_retains_missing_protected_files()
    -> Result<(), Box<dyn Error>> {
        use neo_http::{Settings, SettingsDefaults};
        use neo_playwright::FileAccessError;

        let dir = tempfile::tempdir()?;
        let mut config = config(None, None);
        let profile = dir.path().join("profile");
        config.launch.execution_dir = Some(profile.join(".browseros"));
        config.env.host_api.token_file = Some(dir.path().join("missing-token"));
        config.env.captcha_key_files = vec![
            dir.path().join("missing-key-one"),
            dir.path().join("missing-key-two"),
        ];
        let settings = Settings::defaults(SettingsDefaults::new(vec!["/".into()])?);
        let policy = config.file_access(settings.clone());
        for path in [
            profile.join("Local State"),
            dir.path().join("missing-token"),
            dir.path().join("missing-key-one"),
            dir.path().join("missing-key-two"),
        ] {
            assert_eq!(
                policy.read(&path).await.err(),
                Some(FileAccessError::OwnFiles)
            );
            assert_eq!(
                policy.write(&path).await.err(),
                Some(FileAccessError::OwnFiles)
            );
        }
        config.env.profile_dir = Some(dir.path().join("override"));
        let policy = config.file_access(settings);
        assert!(
            policy.write(&profile.join("ordinary")).await.is_ok(),
            "explicit profile wins over launch discovery"
        );
        assert_eq!(
            policy
                .write(&dir.path().join("override/Local State"))
                .await
                .err(),
            Some(FileAccessError::OwnFiles)
        );
        Ok(())
    }
}
