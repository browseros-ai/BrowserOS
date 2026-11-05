use std::net::SocketAddr;
use std::path::PathBuf;

use super::{ConfigError, Defaults};

/// The process environment, read once at start. A plain data bag: `from_lookup` validated every
/// field, and nothing else in the crate reads an env var.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EnvConfig {
    /// `BROWSERCLAW_DIR` (trimmed, non-empty), else `$HOME/.browserclaw`. Logs, the trace store
    /// and the default host socket live under it.
    pub(crate) state_dir: PathBuf,
    /// `CLAW_LOG`, else `Defaults::LOG_FILTER`.
    pub(crate) log_filter: String,
    /// `BROWSERCLAW_HOSTED_MODE`.
    pub(crate) hosted_mode: HostedMode,
    /// `CLAW_NATIVE_BROWSEROS_VERSION`: a developer's assertion of the native browser version,
    /// which wins over the launch file's (dev and dogfood launchers write a stale one).
    pub(crate) native_version: Option<String>,
    /// `BROWSERCLAW_CONFIG_FILE`. The image still sets it; Neo reads no config file and logs
    /// that once at start.
    pub(crate) ignored_config_file: Option<PathBuf>,
    /// Where the host listener goes.
    pub(crate) host_api: HostApiConfig,
}

/// Whether Chromium runs in hosted mode, where popups open as tabs Neo can drive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HostedMode {
    /// `BROWSERCLAW_HOSTED_MODE=1`, Chromium's exact opt-in.
    On,
    /// Anything else. Neo still starts, and logs one error line naming the value, because popups
    /// will open as windows it cannot drive.
    Off {
        /// The value as set, or `None` when unset.
        value: Option<String>,
    },
}

/// Where the host listener binds (design D1). zuzu points these at its shared `/run/mirror-neo/`.
/// A plain data bag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HostApiConfig {
    /// `CLAW_HOST_SOCKET`, else `$BROWSERCLAW_DIR/host.sock`.
    pub(crate) socket: PathBuf,
    /// `CLAW_HOST_TOKEN_FILE`, the bearer token the host presents. Without it there is no host
    /// listener at all, and `run` still works.
    pub(crate) token_file: Option<PathBuf>,
    /// `CLAW_HOST_TCP`: a TCP address served in addition to the socket, only by explicit opt-in.
    pub(crate) tcp: Option<SocketAddr>,
}

impl EnvConfig {
    /// Reads the variables through `lookup`, so tests pass a map instead of the real
    /// environment. Blank values count as unset.
    pub(crate) fn from_lookup(
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, ConfigError> {
        let read = |name: &str| non_blank(lookup(name));
        let state_dir = if let Some(dir) = read("BROWSERCLAW_DIR") {
            PathBuf::from(dir)
        } else {
            let home = read("HOME").ok_or_else(|| ConfigError::Env {
                name: "HOME",
                why: "unset, and BROWSERCLAW_DIR is unset too, so there is no state directory"
                    .to_owned(),
            })?;
            PathBuf::from(home).join(".browserclaw")
        };
        let hosted_mode = match lookup("BROWSERCLAW_HOSTED_MODE") {
            Some(value) if value == "1" => HostedMode::On,
            value => HostedMode::Off { value },
        };
        let tcp = match read("CLAW_HOST_TCP") {
            Some(text) => {
                let address: SocketAddr = text.parse().map_err(|error| ConfigError::Env {
                    name: "CLAW_HOST_TCP",
                    why: format!("{text:?} is not an address like 127.0.0.1:9300: {error}"),
                })?;
                Some(address)
            }
            None => None,
        };
        let host_api = HostApiConfig {
            socket: read("CLAW_HOST_SOCKET")
                .map_or_else(|| state_dir.join("host.sock"), PathBuf::from),
            token_file: read("CLAW_HOST_TOKEN_FILE").map(PathBuf::from),
            tcp,
        };
        Ok(Self {
            log_filter: read("CLAW_LOG").unwrap_or_else(|| Defaults::LOG_FILTER.to_owned()),
            hosted_mode,
            native_version: read("CLAW_NATIVE_BROWSEROS_VERSION"),
            ignored_config_file: read("BROWSERCLAW_CONFIG_FILE").map(PathBuf::from),
            host_api,
            state_dir,
        })
    }
}

fn non_blank(value: Option<String>) -> Option<String> {
    let trimmed = value?.trim().to_owned();
    (!trimmed.is_empty()).then_some(trimmed)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::error::Error;
    use std::net::SocketAddr;
    use std::path::{Path, PathBuf};

    use super::{EnvConfig, HostApiConfig, HostedMode};
    use crate::config::{ConfigError, Defaults};

    fn env(vars: &[(&str, &str)]) -> Result<EnvConfig, ConfigError> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        EnvConfig::from_lookup(&|name| vars.get(name).cloned())
    }

    #[test]
    fn defaults_with_only_home() -> Result<(), Box<dyn Error>> {
        let config = env(&[("HOME", "/home/neo")])?;
        assert_eq!(
            config,
            EnvConfig {
                state_dir: PathBuf::from("/home/neo/.browserclaw"),
                log_filter: Defaults::LOG_FILTER.to_owned(),
                hosted_mode: HostedMode::Off { value: None },
                native_version: None,
                ignored_config_file: None,
                host_api: HostApiConfig {
                    socket: PathBuf::from("/home/neo/.browserclaw/host.sock"),
                    token_file: None,
                    tcp: None,
                },
            },
            "an image that sets nothing gets the documented defaults"
        );
        Ok(())
    }

    #[test]
    fn browserclaw_dir_is_trimmed_and_blank_is_unset() -> Result<(), Box<dyn Error>> {
        let set = env(&[("HOME", "/home/neo"), ("BROWSERCLAW_DIR", "  /state \n")])?;
        assert_eq!(set.state_dir, Path::new("/state"), "trimmed");
        assert_eq!(
            set.host_api.socket,
            Path::new("/state/host.sock"),
            "the default socket follows the state dir"
        );
        let blank = env(&[("HOME", "/home/neo"), ("BROWSERCLAW_DIR", "   ")])?;
        assert_eq!(
            blank.state_dir,
            Path::new("/home/neo/.browserclaw"),
            "a blank override is no override"
        );
        Ok(())
    }

    #[test]
    fn no_home_and_no_state_dir_fails() {
        assert!(
            matches!(env(&[]), Err(ConfigError::Env { name: "HOME", .. })),
            "without either there is nowhere to put logs or the trace"
        );
    }

    #[test]
    fn only_exactly_one_turns_hosted_mode_on() -> Result<(), Box<dyn Error>> {
        assert_eq!(
            env(&[("HOME", "/h"), ("BROWSERCLAW_HOSTED_MODE", "1")])?.hosted_mode,
            HostedMode::On,
            "Chromium's exact opt-in"
        );
        for value in ["", "0", "true", " 1", "yes"] {
            assert_eq!(
                env(&[("HOME", "/h"), ("BROWSERCLAW_HOSTED_MODE", value)])?.hosted_mode,
                HostedMode::Off {
                    value: Some(value.to_owned())
                },
                "{value:?} is not Chromium's opt-in; Neo starts anyway and names the value"
            );
        }
        Ok(())
    }

    #[test]
    fn reads_every_override() -> Result<(), Box<dyn Error>> {
        let config = env(&[
            ("HOME", "/h"),
            ("CLAW_LOG", "debug"),
            ("CLAW_NATIVE_BROWSEROS_VERSION", " 0.52.0.1 "),
            ("BROWSERCLAW_CONFIG_FILE", "/etc/neo.toml"),
            ("CLAW_HOST_SOCKET", "/run/mirror-neo/host.sock"),
            ("CLAW_HOST_TOKEN_FILE", "/run/mirror-neo/token"),
            ("CLAW_HOST_TCP", "127.0.0.1:9300"),
        ])?;
        assert_eq!(config.log_filter, "debug", "CLAW_LOG");
        assert_eq!(
            config.native_version.as_deref(),
            Some("0.52.0.1"),
            "trimmed"
        );
        assert_eq!(
            config.ignored_config_file,
            Some(PathBuf::from("/etc/neo.toml")),
            "kept only to log that it is ignored"
        );
        let tcp: SocketAddr = "127.0.0.1:9300".parse()?;
        assert_eq!(
            config.host_api,
            HostApiConfig {
                socket: PathBuf::from("/run/mirror-neo/host.sock"),
                token_file: Some(PathBuf::from("/run/mirror-neo/token")),
                tcp: Some(tcp),
            },
            "zuzu's shared directory layout"
        );
        Ok(())
    }

    #[test]
    fn a_bad_tcp_address_fails() {
        assert!(
            matches!(
                env(&[("HOME", "/h"), ("CLAW_HOST_TCP", "localhost")]),
                Err(ConfigError::Env {
                    name: "CLAW_HOST_TCP",
                    ..
                })
            ),
            "an opt-in that cannot be honored must not silently serve only the socket"
        );
    }
}
