use std::fs;
use std::num::NonZeroU16;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use super::{ConfigError, Defaults};

/// The launch facts Neo uses from the JSON file Chromium writes for its sidecar
/// (`browseros_server_config.cc`) and passes as `--config`. A plain data bag: `read` validated
/// every field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LaunchConfig {
    /// The agent listener's port, `ports.server`. Chromium proxies its public port here, and
    /// relaunches Neo on the next port when Neo exits 2.
    pub(crate) server_port: u16,
    /// Chromium's CDP port, `ports.cdp`, where the browser link connects.
    pub(crate) cdp_port: u16,
    /// `directories.resources`, resolved against the launch file's directory.
    pub(crate) resources_dir: PathBuf,
    /// `instance.browseros_version`: the native browser's own version, which gates core's
    /// native capabilities. `None` when missing, blank or not a string, because an unknown
    /// version only hides gated features and must never fail the start.
    pub(crate) browseros_version: Option<String>,
}

impl LaunchConfig {
    /// Reads and parses the launch file at `path`.
    pub(crate) fn read(path: &Path) -> Result<Self, ConfigError> {
        let text = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let config_dir = path.parent().unwrap_or_else(|| Path::new(""));
        Self::parse(&text, config_dir).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Parses the file's text. Unknown keys are ignored, because Chromium adds launch fields
    /// without coordinating with the sidecar; a known key with the wrong type is an error.
    fn parse(text: &str, config_dir: &Path) -> Result<Self, serde_json::Error> {
        let file: LaunchFile = serde_json::from_str(text)?;
        let resources_dir = match file.directories.resources {
            Some(resources) => config_dir.join(resources),
            None => config_dir.join("resources"),
        };
        let browseros_version = file
            .instance
            .as_ref()
            .and_then(|instance| instance.get("browseros_version"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|version| !version.is_empty())
            .map(str::to_owned);
        Ok(Self {
            server_port: file
                .ports
                .server
                .map_or(Defaults::SERVER_PORT, NonZeroU16::get),
            cdp_port: file.ports.cdp.map_or(Defaults::CDP_PORT, NonZeroU16::get),
            resources_dir,
            browseros_version,
        })
    }
}

/// The file as Chromium writes it: `ports`, `directories`, `flags` and `instance`. Only the
/// keys Neo reads are named.
#[derive(Deserialize)]
struct LaunchFile {
    #[serde(default)]
    ports: Ports,
    #[serde(default)]
    directories: Directories,
    /// Untyped on purpose: a missing, malformed or wrong-typed identity leaves the version
    /// unknown instead of failing the parse.
    #[serde(default)]
    instance: Option<Value>,
}

#[derive(Default, Deserialize)]
struct Ports {
    server: Option<NonZeroU16>,
    cdp: Option<NonZeroU16>,
}

#[derive(Default, Deserialize)]
struct Directories {
    resources: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;
    use std::path::Path;

    use super::LaunchConfig;
    use crate::config::{ConfigError, Defaults};

    /// The shape `browseros_server_config.cc` writes, plus keys Neo has never heard of.
    const CHROMIUM_LAUNCH_JSON: &str = r#"{
        "ports": { "cdp": 9101, "server": 9201, "proxy": 9010, "extension": 9300 },
        "directories": { "resources": "resources", "execution": "/srv/exec", "cache": "c" },
        "flags": { "allow_remote_in_mcp": false, "future_flag": 3 },
        "instance": {
            "install_id": "abc",
            "browseros_version": "0.51.1.3",
            "chromium_version": "155.0.8311.26"
        },
        "telemetry": { "enabled": true }
    }"#;

    #[test]
    fn reads_chromium_launch_json_with_unknown_keys() -> Result<(), Box<dyn Error>> {
        let config = LaunchConfig::parse(CHROMIUM_LAUNCH_JSON, Path::new("/opt/neo"))?;
        assert_eq!(
            config,
            LaunchConfig {
                server_port: 9201,
                cdp_port: 9101,
                resources_dir: Path::new("/opt/neo/resources").to_path_buf(),
                browseros_version: Some("0.51.1.3".to_owned()),
            },
            "Chromium adds launch keys freely, so unknown ones must not fail the start"
        );
        Ok(())
    }

    #[test]
    fn defaults_fill_a_bare_file() -> Result<(), Box<dyn Error>> {
        let config = LaunchConfig::parse("{}", Path::new("/opt/neo"))?;
        assert_eq!(
            config.server_port,
            Defaults::SERVER_PORT,
            "Chromium's own default"
        );
        assert_eq!(
            config.cdp_port,
            Defaults::CDP_PORT,
            "Chromium's own default"
        );
        assert_eq!(
            config.resources_dir,
            Path::new("/opt/neo/resources"),
            "resources sit beside the launch file by default"
        );
        assert_eq!(config.browseros_version, None, "no instance, no version");
        Ok(())
    }

    #[test]
    fn an_absolute_resources_dir_is_kept() -> Result<(), Box<dyn Error>> {
        let text = r#"{ "directories": { "resources": "/srv/resources" } }"#;
        let config = LaunchConfig::parse(text, Path::new("/opt/neo"))?;
        assert_eq!(
            config.resources_dir,
            Path::new("/srv/resources"),
            "join keeps an absolute path, so only relative ones move under the config dir"
        );
        Ok(())
    }

    #[test]
    fn a_bad_instance_leaves_the_version_unknown() -> Result<(), Box<dyn Error>> {
        for instance in [
            r#"{"browseros_version": ""}"#,
            r#"{"browseros_version": "  "}"#,
            r#"{"browseros_version": 51}"#,
            r#"{"chromium_version": "155"}"#,
            r#""0.51.1.3""#,
            "null",
            "[]",
        ] {
            let text = format!(r#"{{ "instance": {instance} }}"#);
            let config = LaunchConfig::parse(&text, Path::new(""))?;
            assert_eq!(
                config.browseros_version, None,
                "instance {instance} carries no usable version, and must not fail the start"
            );
        }
        Ok(())
    }

    #[test]
    fn a_wrong_typed_port_fails() {
        for text in [
            r#"{ "ports": { "server": "9200" } }"#,
            r#"{ "ports": { "server": 0 } }"#,
            r#"{ "ports": { "cdp": 70000 } }"#,
        ] {
            assert!(
                LaunchConfig::parse(text, Path::new("")).is_err(),
                "{text}: a port Neo cannot bind or dial is a broken launch, so it exits 1"
            );
        }
    }

    #[test]
    fn read_resolves_against_the_files_directory() -> Result<(), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("server_config.json");
        fs::write(&path, CHROMIUM_LAUNCH_JSON)?;
        let config = LaunchConfig::read(&path)?;
        assert_eq!(
            config.resources_dir,
            dir.path().join("resources"),
            "relative to the file, not the process's working directory"
        );

        let missing = LaunchConfig::read(&dir.path().join("absent.json"));
        assert!(
            matches!(missing, Err(ConfigError::Read { .. })),
            "a missing file is a read error that names the path"
        );
        fs::write(&path, "{ not json")?;
        assert!(
            matches!(LaunchConfig::read(&path), Err(ConfigError::Parse { .. })),
            "broken JSON is a parse error that names the path"
        );
        Ok(())
    }
}
