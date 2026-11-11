use std::env;
use std::path::PathBuf;

use neo_captcha::config::CaptchaConfig;

use super::HostedError;
use crate::config::{AppConfig, EnvConfig, LaunchConfig};

/// The host's launch facts. No process environment is changed.
#[derive(Clone, Debug, Default)]
pub struct HostedOptions {
    /// Required state root; the trace is stored in `claw-sdk/trace.db` underneath it.
    pub state_dir: PathBuf,
    /// Legacy browser launch fact, accepted for compatibility; no upload policy uses it.
    pub execution_dir: Option<PathBuf>,
    /// The installed browser's version, as `instance.browseros_version` names it.
    pub browseros_version: Option<String>,
}

impl HostedOptions {
    pub(super) fn config(self) -> Result<AppConfig, HostedError> {
        if self.state_dir.as_os_str().is_empty() {
            return Err(HostedError(
                "state_dir must name a private state directory".to_owned(),
            ));
        }
        // Read the product settings without mutating Node's process environment. The host
        // owns its state root and asserts hosted operation.
        let lookup = |name: &str| match name {
            "BROWSERCLAW_DIR" => Some("/".to_owned()),
            "BROWSERCLAW_HOSTED_MODE" => Some("1".to_owned()),
            _ => env::var(name).ok(),
        };
        let mut env =
            EnvConfig::from_lookup(&lookup, &env::home_dir).map_err(HostedError::from_error)?;
        env.state_dir = self.state_dir;
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
