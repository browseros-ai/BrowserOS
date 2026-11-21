//! The browser's launch file, the state directory and the install id: everything the process
//! reads before it opens the store.
//!
//! The browser writes `<execution dir>/config.json` and passes it as `--config`. Its own rule for
//! any server binary is that it "must tolerate the full key set; it must not reject unknown
//! keys", so every struct here ignores unknown keys and only four values are required.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The parsed launch file plus what is derived from it. Unknown keys are tolerated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub server_port: u16,
    pub cdp_port: u16,
    pub resources: PathBuf,
    pub execution: PathBuf,
    /// `$BROWSEROS_DIR`, else `~/.browseros`, or `~/.browseros-dev` in dev mode.
    pub state_dir: PathBuf,
    pub dev_mode: bool,
    pub browseros_version: Option<String>,
    /// From `<state dir>/installation.json`; `None` when the file is damaged, and then the
    /// sign-in routes answer `503` instead of storing tokens under a made-up id.
    pub install_id: Option<String>,
    /// Extra origins the trust rule accepts, from `BROWSEROS_TRUSTED_ORIGINS` (development).
    pub trusted_origins: Vec<String>,
}

/// Why the launch file could not be used. Each is exit code 1.
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("reading {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("parsing {path}: {source}")]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("the launch file has no {0}")]
    Missing(&'static str),
    #[error("no home directory to put the state directory in; set BROWSEROS_DIR")]
    NoHome,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LaunchFile {
    ports: Ports,
    directories: Directories,
    flags: Flags,
    instance: Instance,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Ports {
    server: Option<u16>,
    cdp: Option<u16>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Directories {
    resources: Option<PathBuf>,
    execution: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Flags {
    dev_mode: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Instance {
    browseros_version: Option<String>,
}

/// What `Launch::read` takes from the process environment, so a test can supply its own.
#[derive(Debug, Clone, Default)]
pub struct LaunchEnv {
    pub browseros_dir: Option<String>,
    pub node_env: Option<String>,
    pub home: Option<PathBuf>,
    pub trusted_origins: Option<String>,
}

impl LaunchEnv {
    pub fn from_process() -> Self {
        Self {
            browseros_dir: std::env::var("BROWSEROS_DIR").ok(),
            node_env: std::env::var("NODE_ENV").ok(),
            home: std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(PathBuf::from),
            trusted_origins: std::env::var("BROWSEROS_TRUSTED_ORIGINS").ok(),
        }
    }
}

impl Launch {
    /// Reads the launch file. The install id is read later, once the state directory exists.
    pub fn read(path: &Path, env: &LaunchEnv) -> Result<Launch, LaunchError> {
        let text = std::fs::read_to_string(path).map_err(|source| LaunchError::Read {
            path: path.to_owned(),
            source,
        })?;
        let file: LaunchFile =
            serde_json::from_str(&text).map_err(|source| LaunchError::Parse {
                path: path.to_owned(),
                source,
            })?;
        let dev_mode = file.flags.dev_mode || env.node_env.as_deref() == Some("development");
        Ok(Launch {
            server_port: file
                .ports
                .server
                .ok_or(LaunchError::Missing("ports.server"))?,
            cdp_port: file.ports.cdp.ok_or(LaunchError::Missing("ports.cdp"))?,
            resources: file
                .directories
                .resources
                .ok_or(LaunchError::Missing("directories.resources"))?,
            execution: file
                .directories
                .execution
                .ok_or(LaunchError::Missing("directories.execution"))?,
            state_dir: resolve_state_dir(env, dev_mode)?,
            dev_mode,
            browseros_version: file.instance.browseros_version,
            install_id: None,
            trusted_origins: env
                .trusted_origins
                .as_deref()
                .map(crate::http::parse_trusted_origins)
                .unwrap_or_default(),
        })
    }

    /// `<resources>/bin/third_party/bun`, when the resources ship it. acpx and the coding-agent
    /// adapters run under it.
    pub fn bundled_bun(&self) -> Option<PathBuf> {
        let name = if cfg!(windows) { "bun.exe" } else { "bun" };
        let bun = self.resources.join("bin").join("third_party").join(name);
        bun.is_file().then_some(bun)
    }
}

/// `$BROWSEROS_DIR`, else `~/.browseros`, or `~/.browseros-dev` in dev mode: the same folder
/// the old server used, so the same database and install id.
pub fn resolve_state_dir(env: &LaunchEnv, dev_mode: bool) -> Result<PathBuf, LaunchError> {
    if let Some(dir) = env.browseros_dir.as_deref().map(str::trim)
        && !dir.is_empty()
    {
        return Ok(PathBuf::from(dir));
    }
    let home = env.home.as_ref().ok_or(LaunchError::NoHome)?;
    Ok(home.join(if dev_mode {
        ".browseros-dev"
    } else {
        ".browseros"
    }))
}

const INSTALLATION_FILE: &str = "installation.json";

#[derive(Deserialize)]
struct InstallationFile {
    install_id: String,
}

/// Reads `<state dir>/installation.json`, creating it when absent, with the old server's exact
/// semantics: the key is `install_id`, a UUID. The browser may create the file at the same
/// moment, so a new one is published with a no-clobber hard link of a complete temporary file,
/// and a loser adopts the winner's id. A damaged file is never replaced: it is an error, which
/// the caller logs, and the server runs without sign-in rather than under a second identity.
pub fn read_install_id(state_dir: &Path) -> Result<String, String> {
    let path = state_dir.join(INSTALLATION_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => return parse_install_id(&path, &text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("reading {}: {error}", path.display())),
    }
    std::fs::create_dir_all(state_dir)
        .map_err(|error| format!("creating {}: {error}", state_dir.display()))?;
    let candidate = uuid::Uuid::new_v4().to_string();
    let contents = format!("{{\n  \"install_id\": \"{candidate}\"\n}}\n");
    let mut temporary = tempfile::Builder::new()
        .prefix(&format!(".{INSTALLATION_FILE}."))
        .suffix(".tmp")
        .tempfile_in(state_dir)
        .map_err(|error| {
            format!(
                "creating a temporary file in {}: {error}",
                state_dir.display()
            )
        })?;
    temporary
        .write_all(contents.as_bytes())
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|error| format!("writing {}: {error}", temporary.path().display()))?;
    match std::fs::hard_link(temporary.path(), &path) {
        Ok(()) => Ok(candidate),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let text = std::fs::read_to_string(&path)
                .map_err(|error| format!("reading {}: {error}", path.display()))?;
            parse_install_id(&path, &text)
        }
        Err(error) => Err(format!("publishing {}: {error}", path.display())),
    }
}

fn parse_install_id(path: &Path, text: &str) -> Result<String, String> {
    let file: InstallationFile = serde_json::from_str(text)
        .map_err(|_| format!("invalid installation identity file: {}", path.display()))?;
    uuid::Uuid::parse_str(&file.install_id)
        .ok()
        .filter(|id| {
            id.hyphenated()
                .to_string()
                .eq_ignore_ascii_case(&file.install_id)
        })
        .map(|_| file.install_id)
        .ok_or_else(|| format!("invalid installation identity file: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(dir: &Path) -> LaunchEnv {
        LaunchEnv {
            browseros_dir: Some(dir.display().to_string()),
            ..LaunchEnv::default()
        }
    }

    #[test]
    fn unknown_keys_at_every_level_are_tolerated() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{
              "ports": { "cdp": 9100, "server": 9200, "proxy": 9000, "future": 1 },
              "directories": { "resources": "/r", "execution": "/e", "other": "/o" },
              "flags": { "allow_remote_in_mcp": false, "devMode": false },
              "instance": { "install_id": "x", "browseros_version": "0.53.0", "chromium_version": "146" },
              "invented": { "nested": [1, 2] }
            }"#,
        )?;
        let launch = Launch::read(&path, &env(dir.path()))?;
        assert_eq!((launch.server_port, launch.cdp_port), (9200, 9100));
        assert_eq!(launch.browseros_version.as_deref(), Some("0.53.0"));
        assert_eq!(launch.state_dir, dir.path());
        Ok(())
    }

    #[test]
    fn a_missing_required_value_is_named() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{ "ports": { "cdp": 1 } }"#)?;
        let error = Launch::read(&path, &env(dir.path()))
            .err()
            .map(|e| e.to_string());
        assert_eq!(
            error.as_deref(),
            Some("the launch file has no ports.server")
        );
        Ok(())
    }

    #[test]
    fn the_install_id_is_created_once_and_a_damaged_file_is_kept()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let first = read_install_id(dir.path())?;
        assert_eq!(read_install_id(dir.path())?, first);
        std::fs::write(dir.path().join(INSTALLATION_FILE), "{ broken")?;
        assert!(read_install_id(dir.path()).is_err());
        assert_eq!(
            std::fs::read_to_string(dir.path().join(INSTALLATION_FILE))?,
            "{ broken"
        );
        Ok(())
    }

    #[test]
    fn concurrent_creation_settles_on_one_id() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().to_owned();
        let ids: Vec<Result<String, String>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| scope.spawn(|| read_install_id(&path)))
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap_or_else(|_| Err("panicked".into())))
                .collect()
        });
        let ids: Result<Vec<String>, String> = ids.into_iter().collect();
        let ids = ids?;
        assert!(ids.windows(2).all(|pair| pair[0] == pair[1]));
        Ok(())
    }
}
