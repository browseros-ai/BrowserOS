use std::env;
use std::path::PathBuf;

use neo_captcha::config::CaptchaConfig;

use super::HostedError;
use crate::config::{AppConfig, EnvConfig, LaunchConfig, NeoHome};

/// The host's launch facts. No process environment is changed.
#[derive(Clone, Debug, Default)]
pub struct HostedOptions {
    /// Required state root; the trace is stored in `claw-sdk/trace.db` underneath it.
    pub state_dir: PathBuf,
    /// Optional absolute folders for path reads. Omission uses the browser's downloads folder;
    /// an empty list denies path reads. Neo's own files remain protected with any list.
    pub read_dirs: Option<Vec<PathBuf>>,
    /// Browser launch fact, used to discover a .browseros profile when `CLAW_PROFILE_DIR` is unset.
    pub execution_dir: Option<PathBuf>,
    /// The installed browser's version, as `instance.browseros_version` names it.
    pub browseros_version: Option<String>,
    /// Whether the browser was launched in hosted mode (`BROWSERCLAW_HOSTED_MODE=1` in
    /// Chromium's environment). The sidecar reads that variable from its own environment, which
    /// the image shares with the browser; a host in Node launched the browser itself, so it says
    /// which mode it chose. Off by default: then Neo waits for console and page errors, which
    /// hosted BrowserOS never sends (`events.wait` refuses those in hosted mode).
    pub hosted_mode: bool,
}

impl HostedOptions {
    pub(super) fn config(self) -> Result<AppConfig, HostedError> {
        if self.state_dir.as_os_str().is_empty() {
            return Err(HostedError(
                "state_dir must name a private state directory".to_owned(),
            ));
        }
        // Read the product settings without mutating Node's process environment. The host
        // owns its state root and states the browser's mode.
        let hosted_mode = self.hosted_mode;
        let lookup = |name: &str| match name {
            "BROWSERCLAW_DIR" => Some("/".to_owned()),
            "BROWSERCLAW_HOSTED_MODE" => hosted_mode.then(|| "1".to_owned()),
            _ => env::var(name).ok(),
        };
        let mut env =
            EnvConfig::from_lookup(&lookup, &env::home_dir).map_err(HostedError::from_error)?;
        env.state_dir = self.state_dir;
        // The host launched the browser under its own home, which may hold the files it hands
        // a script (the spec suite writes its results there); it is not Neo's to protect.
        env.neo_home = NeoHome::NotNeos;
        let launch = LaunchConfig {
            server_port: 0,
            cdp_port: 0,
            resources_dir: PathBuf::new(),
            execution_dir: self.execution_dir,
            browseros_version: self.browseros_version,
        };
        Ok(AppConfig {
            captcha: CaptchaConfig::from_lookup(lookup),
            env,
            launch,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::path::PathBuf;

    use super::HostedOptions;
    use crate::config::NeoHome;

    #[test]
    fn the_node_embedding_never_protects_its_home() -> Result<(), Box<dyn Error>> {
        for hosted_mode in [true, false] {
            let config = HostedOptions {
                state_dir: PathBuf::from("/tmp/neo-state"),
                hosted_mode,
                ..HostedOptions::default()
            }
            .config()?;
            assert_eq!(
                config.env.neo_home,
                NeoHome::NotNeos,
                "hosted_mode {hosted_mode}: the host's home holds what it hands a script"
            );
        }
        Ok(())
    }
}
