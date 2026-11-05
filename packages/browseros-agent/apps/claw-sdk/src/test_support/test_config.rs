//! A process config for tests, without a launch file or the real environment.

use std::path::Path;

use crate::config::{AppConfig, Defaults, EnvConfig, HostApiConfig, HostedMode, LaunchConfig};

/// The config Chromium would give a hosted Neo, with its state under `state_dir` and the agent
/// port left to the OS (port 0; `App::local_addr` reports the one it got). There is no host
/// listener, and the CDP port goes unused once the app starts on a `FakeCdp`.
#[must_use]
pub fn test_config(state_dir: &Path) -> AppConfig {
    AppConfig {
        launch: LaunchConfig {
            server_port: 0,
            cdp_port: Defaults::CDP_PORT,
            resources_dir: state_dir.join("resources"),
            browseros_version: None,
        },
        env: EnvConfig {
            state_dir: state_dir.to_path_buf(),
            log_filter: Defaults::LOG_FILTER.to_owned(),
            hosted_mode: HostedMode::On,
            native_version: None,
            ignored_config_file: None,
            host_api: HostApiConfig {
                socket: state_dir.join("host.sock"),
                token_file: None,
                tcp: None,
            },
        },
    }
}
