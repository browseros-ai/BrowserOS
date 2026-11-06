use std::fmt::Display;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;

use browseros_captcha::config::{CAPSOLVER_KEY_FILE_ENV, TWOCAPTCHA_KEY_FILE_ENV};
use browseros_takeover::Xvnc;

use super::{ConfigError, Defaults};

/// The process environment, read once at start. A plain data bag: `from_lookup` validated every
/// field, and nothing else in the crate reads an env var.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EnvConfig {
    /// `BROWSERCLAW_DIR` (trimmed, non-empty), else `$HOME/.browserclaw`. Logs, the trace store
    /// and the default host socket live under it.
    pub(crate) state_dir: PathBuf,
    /// `HOME` (trimmed, non-empty). The Neo image keeps its CAPTCHA keys under it
    /// (`Defaults::CAPTCHA_KEY_DIR`), and the state dir falls back to it.
    pub(crate) home: Option<PathBuf>,
    /// browseros-captcha's key-file variables (`BROWSEROS_CAPSOLVER_API_KEY_FILE`, then
    /// `BROWSEROS_2CAPTCHA_API_KEY_FILE`), each trimmed and blank as unset, as that crate reads
    /// them. Its `CaptchaConfig` reads each key once and keeps no path, so these are where Neo's
    /// secret files learn the keys' places.
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
    /// Neo's one screen, which browseros-takeover's fits resize to the owner's viewer and put
    /// back home:
    /// - `addr`: `CLAW_XVNC_ADDR`, a `host:port`, else `Defaults::XVNC_ADDR` (the image's Xvnc).
    /// - `home`: `NEO_SCREEN`, the image's own `WxH` for Xvnc's geometry, read rather than
    ///   duplicated, else `Defaults::SCREEN`.
    pub(crate) xvnc: Xvnc,
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
    /// browseros-service refuses (empty, under 32 bytes, not visible ASCII), there is no host
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
        let home = read("HOME").map(PathBuf::from);
        let state_dir = match (read("BROWSERCLAW_DIR"), &home) {
            (Some(dir), _) => PathBuf::from(dir),
            (None, Some(home)) => home.join(".browserclaw"),
            (None, None) => {
                return Err(ConfigError::Env {
                    name: "HOME",
                    why: "unset, and BROWSERCLAW_DIR is unset too, so there is no state directory"
                        .to_owned(),
                });
            }
        };
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
            log_filter: read("CLAW_LOG").unwrap_or_else(|| Defaults::LOG_FILTER.to_owned()),
            hosted_mode,
            native_version: read("CLAW_NATIVE_BROWSEROS_VERSION"),
            ignored_config_file: read("BROWSERCLAW_CONFIG_FILE").map(PathBuf::from),
            host_api,
            xvnc,
            state_dir,
            home,
            captcha_key_files: [CAPSOLVER_KEY_FILE_ENV, TWOCAPTCHA_KEY_FILE_ENV]
                .into_iter()
                .filter_map(&read)
                .map(PathBuf::from)
                .collect(),
        })
    }

    /// `$BROWSERCLAW_DIR/claw-sdk`, the trace store's directory: the one source for where
    /// `open_trace` keeps the store and for the secret file that denies it to uploads, so the
    /// two cannot drift apart.
    #[must_use]
    pub(crate) fn trace_dir(&self) -> PathBuf {
        self.state_dir.join("claw-sdk")
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
    use std::fs;
    use std::net::SocketAddr;
    use std::path::{Path, PathBuf};

    use browseros_captcha::config::{
        CAPSOLVER_KEY_FILE_ENV, CaptchaConfig, TWOCAPTCHA_KEY_FILE_ENV,
    };
    use browseros_takeover::{ScreenSize, Xvnc};

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
                home: Some(PathBuf::from("/home/neo")),
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

    /// Every key file browseros-captcha reads is one Neo names, so no key reaches an upload
    /// through a spelling the two read differently.
    #[test]
    fn reads_the_captcha_key_paths_as_browseros_captcha_trims_them() -> Result<(), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let capsolver = dir.path().join("capsolver-api-key");
        let twocaptcha = dir.path().join("2captcha-api-key");
        fs::write(&capsolver, "capsolver-key")?;
        fs::write(&twocaptcha, "twocaptcha-key")?;
        let vars = HashMap::from([
            ("HOME".to_owned(), "/home/neo".to_owned()),
            (
                CAPSOLVER_KEY_FILE_ENV.to_owned(),
                format!("  {}\n", capsolver.display()),
            ),
            (
                TWOCAPTCHA_KEY_FILE_ENV.to_owned(),
                format!("\t{} ", twocaptcha.display()),
            ),
        ]);
        let lookup = |name: &str| vars.get(name).cloned();
        let solver = CaptchaConfig::from_lookup(lookup);
        assert!(
            solver.capsolver.is_some() && solver.twocaptcha.is_some(),
            "browseros-captcha trims each value and reads the key at that path"
        );
        assert_eq!(
            EnvConfig::from_lookup(&lookup)?.captcha_key_files,
            [capsolver, twocaptcha],
            "Neo names the same two files, trimmed the same way"
        );

        let blank = env(&[
            ("HOME", "/home/neo"),
            (CAPSOLVER_KEY_FILE_ENV, "   "),
            (TWOCAPTCHA_KEY_FILE_ENV, ""),
        ])?;
        assert_eq!(
            blank.captcha_key_files,
            Vec::<PathBuf>::new(),
            "a blank value names no file, as browseros-captcha reads none"
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
}
