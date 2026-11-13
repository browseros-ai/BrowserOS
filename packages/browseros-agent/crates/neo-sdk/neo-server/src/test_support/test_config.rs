//! A process config for tests, without a launch file or the real environment.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::Path;

use neo_captcha::config::CaptchaConfig;
use neo_takeover::Xvnc;

use crate::config::{
    AppConfig, Defaults, EnvConfig, HostApiConfig, HostedMode, LaunchConfig, NeoHome,
};

/// The config Chromium would give a hosted Neo, with its state and its browser profile under
/// `state_dir` and the agent port left to the OS (port 0; `App::local_addr` reports the one it
/// got). There is no host listener and no CAPTCHA provider key, whatever the test's environment
/// holds, and the CDP port goes unused once the app starts on a `FakeCdp`. Neo's screen is
/// [`CLOSED_XVNC`].
#[must_use]
pub fn test_config(state_dir: &Path) -> AppConfig {
    let launch = LaunchConfig {
        server_port: 0,
        cdp_port: Defaults::CDP_PORT,
        resources_dir: state_dir.join("resources"),
        execution_dir: Some(state_dir.join("profile").join(".browseros")),
        browseros_version: None,
    };
    let env = EnvConfig {
        state_dir: state_dir.to_path_buf(),
        downloads_dir: Some(state_dir.join("downloads")),
        profile_dir: Some(state_dir.join("profile")),
        neo_home: NeoHome::NotNeos,
        captcha_key_files: Vec::new(),
        log_filter: Defaults::LOG_FILTER.to_owned(),
        hosted_mode: HostedMode::On,
        native_version: None,
        ignored_config_file: None,
        host_api: HostApiConfig {
            socket: state_dir.join("host.sock"),
            token_file: None,
            tcp: None,
        },
        xvnc: CLOSED_XVNC,
        ignored_trace_value_bytes: false,
    };
    AppConfig {
        launch,
        env,
        captcha: CaptchaConfig::default(),
    }
}

/// Neo's screen at a local port nothing listens on (discard, 9). The service puts the screen home
/// as it starts; at Xvnc's real 5900 that could resize a developer's macOS Screen Sharing, while
/// here the dial is refused at once and only logged.
pub const CLOSED_XVNC: Xvnc = Xvnc {
    addr: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 9)),
    home: Defaults::SCREEN,
};
