use std::fmt::Display;
use std::net::SocketAddr;
use std::path::{self, PathBuf};
use std::str::FromStr;

use neo_takeover::Xvnc;

use super::{ConfigError, Defaults};

/// The process environment, read once at start. A plain data bag: `from_lookup` validated every
/// field, and nothing else in the crate reads an env var.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EnvConfig {
    /// `BROWSERCLAW_DIR` (trimmed, non-empty), else `.browserclaw` under `home`. Logs, the trace
    /// store and the default host socket live under it.
    pub(crate) state_dir: PathBuf,
    /// `CLAW_DOWNLOADS_DIR`, else the browser's `$HOME/Downloads`. The sole default read root; absent only if no home can be discovered.
    pub(crate) downloads_dir: Option<PathBuf>,
    /// `CLAW_PROFILE_DIR`, with launch-file fallback owned by `AppConfig`.
    pub(crate) profile_dir: Option<PathBuf>,
    /// Both configured CAPTCHA key files, including missing files, protected independently
    /// of whether a solver could read and enable them at startup.
    pub(crate) captcha_key_files: Vec<PathBuf>,
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
    /// Neo's one screen, which neo-takeover's fits resize to the owner's viewer and put
    /// back home:
    /// - `addr`: `CLAW_XVNC_ADDR`, a `host:port`, else `Defaults::XVNC_ADDR` (the image's Xvnc).
    /// - `home`: `NEO_SCREEN`, the image's own `WxH` for Xvnc's geometry, read rather than
    ///   duplicated, else `Defaults::SCREEN`.
    pub(crate) xvnc: Xvnc,
    /// Whether `CLAW_TRACE_VALUE_BYTES` is set. It is read no more: the host sets
    /// `trace.valueBytes` through `/v1/settings` instead. Kept to warn a host that still sets it,
    /// since its values would now be cut without a word.
    pub(crate) ignored_trace_value_bytes: bool,
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
    /// `CLAW_HOST_TOKEN_FILE`, the bearer token the host presents. Without it, or with a token
    /// neo-http refuses (empty, under 32 bytes, not visible ASCII), there is no host
    /// listener at all, and `run` still works.
    pub(crate) token_file: Option<PathBuf>,
    /// `CLAW_HOST_TCP`: a TCP address served in addition to the socket, only by explicit opt-in.
    pub(crate) tcp: Option<SocketAddr>,
}

impl EnvConfig {
    /// Reads the variables through `lookup`, and the user's home through `users_home` only when
    /// `HOME` is unset or blank, so tests pass values instead of the real environment and passwd
    /// database. Blank values count as unset. Production passes `std::env::home_dir`, which
    /// returns a set `HOME` as is, so a blank one names no home and only an unset one reaches
    /// the passwd entry.
    pub(crate) fn from_lookup(
        lookup: &dyn Fn(&str) -> Option<String>,
        users_home: &dyn Fn() -> Option<PathBuf>,
    ) -> Result<Self, ConfigError> {
        let read = |name: &str| non_blank(lookup(name));
        let home = read("HOME").map(PathBuf::from).or_else(|| {
            users_home().filter(|home| !home.as_os_str().to_string_lossy().trim().is_empty())
        });
        let state_dir = match (read("BROWSERCLAW_DIR"), &home) {
            (Some(dir), _) => PathBuf::from(dir),
            (None, Some(home)) => home.join(".browserclaw"),
            (None, None) => {
                return Err(ConfigError::Env {
                    name: "HOME",
                    why: "unset, the passwd entry names no home, and BROWSERCLAW_DIR is unset \
                          too, so there is no state directory"
                        .to_owned(),
                });
            }
        };
        let downloads_dir = read("CLAW_DOWNLOADS_DIR")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|home| home.join("Downloads")))
            .map(|downloads| {
                path::absolute(downloads).map_err(|error| ConfigError::Env {
                    name: "CLAW_DOWNLOADS_DIR",
                    why: error.to_string(),
                })
            })
            .transpose()?;
        let hosted_mode = match lookup("BROWSERCLAW_HOSTED_MODE") {
            Some(value) if value == "1" => HostedMode::On,
            value => HostedMode::Off { value },
        };
        let host_api = HostApiConfig {
            socket: read("CLAW_HOST_SOCKET")
                .map_or_else(|| state_dir.join("host.sock"), PathBuf::from),
            token_file: read("CLAW_HOST_TOKEN_FILE").map(PathBuf::from),
            tcp: parse(&read, "CLAW_HOST_TCP", "an address like 127.0.0.1:9300")?,
        };
        let xvnc = Xvnc {
            addr: parse(&read, "CLAW_XVNC_ADDR", "an address like 127.0.0.1:5900")?
                .unwrap_or(Defaults::XVNC_ADDR),
            home: parse(&read, "NEO_SCREEN", "a screen size like 1600x1000")?
                .unwrap_or(Defaults::SCREEN),
        };
        Ok(Self {
            ignored_trace_value_bytes: read("CLAW_TRACE_VALUE_BYTES").is_some(),
            log_filter: read("CLAW_LOG").unwrap_or_else(|| Defaults::LOG_FILTER.to_owned()),
            hosted_mode,
            native_version: read("CLAW_NATIVE_BROWSEROS_VERSION"),
            ignored_config_file: read("BROWSERCLAW_CONFIG_FILE").map(PathBuf::from),
            host_api,
            xvnc,
            state_dir,
            downloads_dir,
            profile_dir: read("CLAW_PROFILE_DIR").map(PathBuf::from),
            captcha_key_files: [
                "BROWSEROS_CAPSOLVER_API_KEY_FILE",
                "BROWSEROS_2CAPTCHA_API_KEY_FILE",
            ]
            .into_iter()
            .filter_map(|name| read(name).map(PathBuf::from))
            .collect(),
        })
    }

    /// `$BROWSERCLAW_DIR/claw-sdk`, where `open_trace` keeps the store.
    #[must_use]
    pub(crate) fn trace_dir(&self) -> PathBuf {
        self.state_dir.join("claw-sdk")
    }

    /// Where `/v1/settings` saves what the host set: beside `trace.db`, not in it, because a
    /// trace store that does not open is moved aside, and the settings must not go with it.
    #[must_use]
    pub(crate) fn settings_file(&self) -> PathBuf {
        self.trace_dir().join("settings.json")
    }
}

/// Parses a set variable, failing the start on a malformed value: a deployment that set one
/// expects it honored, never silently replaced by the default.
fn parse<T>(
    read: &dyn Fn(&str) -> Option<String>,
    name: &'static str,
    like: &str,
) -> Result<Option<T>, ConfigError>
where
    T: FromStr,
    T::Err: Display,
{
    read(name)
        .map(|text| {
            text.parse().map_err(|error| ConfigError::Env {
                name,
                why: format!("{text:?} is not {like}: {error}"),
            })
        })
        .transpose()
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

    use neo_takeover::{ScreenSize, Xvnc};

    use super::{EnvConfig, HostApiConfig, HostedMode};
    use crate::config::{ConfigError, Defaults};

    fn env(vars: &[(&str, &str)]) -> Result<EnvConfig, ConfigError> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        EnvConfig::from_lookup(&|name| vars.get(name).cloned(), &|| None)
    }

    #[test]
    fn defaults_with_only_home() -> Result<(), Box<dyn Error>> {
        let config = env(&[("HOME", "/home/neo")])?;
        assert_eq!(
            config,
            EnvConfig {
                state_dir: PathBuf::from("/home/neo/.browserclaw"),
                downloads_dir: Some(PathBuf::from("/home/neo/Downloads")),
                profile_dir: None,
                captcha_key_files: Vec::new(),
                log_filter: Defaults::LOG_FILTER.to_owned(),
                hosted_mode: HostedMode::Off { value: None },
                native_version: None,
                ignored_config_file: None,
                host_api: HostApiConfig {
                    socket: PathBuf::from("/home/neo/.browserclaw/host.sock"),
                    token_file: None,
                    tcp: None,
                },
                xvnc: Xvnc {
                    addr: Xvnc::NEO_ADDR,
                    home: ScreenSize {
                        width: 1600,
                        height: 1000
                    },
                },
                ignored_trace_value_bytes: false,
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
    fn the_trace_dir_follows_the_state_dir() -> Result<(), Box<dyn Error>> {
        assert_eq!(
            env(&[
                ("HOME", "/home/neo"),
                ("BROWSERCLAW_DIR", "/data/browserclaw")
            ])?
            .trace_dir(),
            Path::new("/data/browserclaw/claw-sdk"),
            "the image's store, under its state volume"
        );
        assert_eq!(
            env(&[("HOME", "/home/neo")])?.trace_dir(),
            Path::new("/home/neo/.browserclaw/claw-sdk"),
            "and under HOME's default state dir"
        );
        Ok(())
    }

    /// `HOME` wins, as every Unix program reads it; an `env -i` launch, which has none, still
    /// gets the user's home from the passwd entry.
    #[test]
    fn home_is_home_else_the_passwd_entrys() -> Result<(), Box<dyn Error>> {
        let passwd = || Some(PathBuf::from("/home/neo"));
        let set = EnvConfig::from_lookup(
            &|name| (name == "HOME").then(|| "/tmp/rig/home".to_owned()),
            &passwd,
        )?;
        assert_eq!(set.state_dir, Path::new("/tmp/rig/home/.browserclaw"));
        let unset = EnvConfig::from_lookup(&|_| None, &passwd)?;
        assert_eq!(
            unset.state_dir,
            Path::new("/home/neo/.browserclaw"),
            "the state dir falls back to that home too"
        );
        Ok(())
    }

    #[test]
    fn no_home_and_no_state_dir_fails() {
        assert!(
            matches!(env(&[]), Err(ConfigError::Env { name: "HOME", .. })),
            "without either, or a passwd home, there is nowhere to put logs or the trace"
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
            ("CLAW_XVNC_ADDR", " 127.0.0.1:5901 "),
            ("NEO_SCREEN", "1280x800"),
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
        let xvnc: SocketAddr = "127.0.0.1:5901".parse()?;
        assert_eq!(
            config.xvnc,
            Xvnc {
                addr: xvnc,
                home: ScreenSize {
                    width: 1280,
                    height: 800
                },
            },
            "an image that moves Xvnc or resizes its screen"
        );
        Ok(())
    }

    #[test]
    fn the_old_value_size_variable_is_only_noticed() -> Result<(), Box<dyn Error>> {
        assert!(!env(&[("HOME", "/h")])?.ignored_trace_value_bytes);
        let config = env(&[("HOME", "/h"), ("CLAW_TRACE_VALUE_BYTES", "256KiB")])?;
        assert!(
            config.ignored_trace_value_bytes,
            "set, even unreadable, it fails nothing and is only warned about: trace.valueBytes replaced it"
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

    #[test]
    fn a_bad_xvnc_address_or_screen_fails() {
        for (name, value) in [
            ("CLAW_XVNC_ADDR", "localhost:5900"),
            ("CLAW_XVNC_ADDR", "5900"),
            ("NEO_SCREEN", "1600"),
            ("NEO_SCREEN", "1600X1000"),
            ("NEO_SCREEN", "0x1000"),
        ] {
            let error = env(&[("HOME", "/h"), (name, value)]);
            assert!(
                matches!(&error, Err(ConfigError::Env { name: failed, .. }) if *failed == name),
                "{name}={value:?} must fail the start naming {name}, not fit a screen it was not \
                 given: {error:?}"
            );
        }
    }
    #[test]
    fn filesystem_layout_keeps_configured_missing_paths_and_no_home_startup()
    -> Result<(), Box<dyn Error>> {
        let config = env(&[
            ("BROWSERCLAW_DIR", "/state"),
            ("CLAW_DOWNLOADS_DIR", "/downloads"),
            ("CLAW_PROFILE_DIR", "/profile"),
            ("BROWSEROS_CAPSOLVER_API_KEY_FILE", "/keys/one"),
            ("BROWSEROS_2CAPTCHA_API_KEY_FILE", "/keys/two"),
        ])?;
        assert_eq!(config.downloads_dir, Some(PathBuf::from("/downloads")));
        assert_eq!(config.profile_dir, Some(PathBuf::from("/profile")));
        assert_eq!(
            config.captcha_key_files,
            vec![PathBuf::from("/keys/one"), PathBuf::from("/keys/two")]
        );
        let config = env(&[("BROWSERCLAW_DIR", "/state")])?;
        assert_eq!(
            config.downloads_dir, None,
            "no home must not invent a browser downloads directory"
        );
        Ok(())
    }
}
