//! Port of the TS host launcher (`lib/agents/host-acp/*`): decides which program runs an ACP agent
//! and with which environment overlay. Built-in adapters prefer BrowserOS's bundled Bun, then the
//! host's `npx`; custom agents run their own command with the login-shell PATH merged in.
//!
//! Unlike TS, the environment rides on the spawned `Command` (acp-client overlays it on the server's
//! own environment) instead of an `env K=V …` argv prefix or the Windows `node --eval` relay. The
//! resolved [`LaunchSpec`] therefore holds only the variables BrowserOS adds or overrides.

use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Mutex, mpsc},
    time::Duration,
};

use acp_client::LaunchSpec;

use crate::definition::{AgentKind, CustomAgentConfig};

/// Process-wide facts the launcher needs; built once at server start.
#[derive(Debug, Clone)]
pub struct HostEnvironment {
    /// `<app>/Resources/BrowserOSServer/default/resources`; holds `bin/third_party/bun`.
    pub resources_dir: Option<PathBuf>,
    pub browseros_dir: PathBuf,
    /// The server's environment. Only PATH, SHELL and `ACPX_CLAUDE_INCLUDE_USER_SETTINGS` are read
    /// here; the child inherits the real environment from the server process regardless.
    pub inherited_env: BTreeMap<String, String>,
}

impl HostEnvironment {
    /// Snapshot of this server process's environment (non-UTF-8 entries are skipped).
    pub fn from_process(resources_dir: Option<PathBuf>, browseros_dir: PathBuf) -> Self {
        let inherited_env = std::env::vars_os()
            .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
            .collect();
        Self {
            resources_dir,
            browseros_dir,
            inherited_env,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LaunchSource {
    BundledBun,
    HostNpx,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLaunch {
    pub spec: LaunchSpec,
    pub source: LaunchSource,
}

/// An npm-published ACP adapter run through `bun x` / `npx`. The version range also keys Bun's
/// executable cache, so raising the minimum forces every install onto the newer adapter.
#[derive(Debug, Clone, Copy)]
pub struct AdapterPackage {
    pub package_spec: &'static str,
    pub bin: &'static str,
}

pub const CLAUDE_ADAPTER: AdapterPackage = AdapterPackage {
    package_spec: "@agentclientprotocol/claude-agent-acp@^0.75.1",
    bin: "claude-agent-acp",
};
pub const CODEX_ADAPTER: AdapterPackage = AdapterPackage {
    package_spec: "@agentclientprotocol/codex-acp@^1.10.0",
    bin: "codex-acp",
};

/// Messages are the TS launcher's, verbatim.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LaunchError {
    #[error("Custom ACP agent is missing a launch command")]
    MissingCommand,
    #[error("Command has an unterminated quote")]
    UnterminatedQuote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Platform {
    Unix,
    Windows,
}

impl Platform {
    pub(crate) const CURRENT: Self = if cfg!(windows) {
        Self::Windows
    } else {
        Self::Unix
    };

    fn delimiter(self) -> char {
        match self {
            Self::Unix => ':',
            Self::Windows => ';',
        }
    }

    /// Windows env keys are case-insensitive; reuse whichever spelling the environment has.
    fn path_key(self, env: &BTreeMap<String, String>) -> String {
        match self {
            Self::Unix => "PATH".into(),
            Self::Windows => env
                .keys()
                .find(|key| key.eq_ignore_ascii_case("path"))
                .cloned()
                .unwrap_or_else(|| "Path".into()),
        }
    }
}

/// Bundled Bun (`bun x --bun --silent --package <spec> <bin>`, node shim on PATH,
/// `BUN_INSTALL_CACHE_DIR`) → host `npx -y <spec>` → custom argv with the login-shell PATH merged
/// in. A `custom` config forces the custom path even for built-in kinds (the probe of an unsaved
/// command), exactly like TS. Writes the node shim, so call it off the async executor.
pub fn resolve_launch(
    kind: AgentKind,
    custom: Option<&CustomAgentConfig>,
    extra_env: &BTreeMap<String, String>,
    cwd: &Path,
    host: &HostEnvironment,
) -> Result<ResolvedLaunch, LaunchError> {
    resolve_launch_on(Platform::CURRENT, kind, custom, extra_env, cwd, host)
}

pub(crate) fn resolve_launch_on(
    platform: Platform,
    kind: AgentKind,
    custom: Option<&CustomAgentConfig>,
    extra_env: &BTreeMap<String, String>,
    cwd: &Path,
    host: &HostEnvironment,
) -> Result<ResolvedLaunch, LaunchError> {
    if kind == AgentKind::Custom || custom.is_some() {
        return resolve_custom(platform, custom, extra_env, cwd, host);
    }
    let adapter = if kind == AgentKind::Claude {
        CLAUDE_ADAPTER
    } else {
        CODEX_ADAPTER
    };
    if let Some(bun) = resolve_bundled_bun(host.resources_dir.as_deref(), platform) {
        let mut env = bundled_bun_env(&bun, &host.browseros_dir, &host.inherited_env, platform);
        prepend_native_dir(&mut env, host.resources_dir.as_deref(), platform);
        env.extend(extra_env.clone());
        let args = [
            "x",
            "--bun",
            "--silent",
            "--package",
            adapter.package_spec,
            adapter.bin,
        ];
        return Ok(ResolvedLaunch {
            spec: LaunchSpec {
                program: bun,
                args: args.map(String::from).to_vec(),
                env,
                cwd: cwd.to_path_buf(),
            },
            source: LaunchSource::BundledBun,
        });
    }

    let key = platform.path_key(&host.inherited_env);
    let mut host_env: BTreeMap<String, String> = host
        .inherited_env
        .get(&key)
        .map(|path| BTreeMap::from([(key.clone(), path.clone())]))
        .unwrap_or_default();
    let before = host_env.get(&key).cloned();
    prepend_native_dir(&mut host_env, host.resources_dir.as_deref(), platform);
    // Only an added bundled-native dir justifies overriding the inherited PATH.
    let mut env = if host_env.get(&key) != before.as_ref() {
        host_env
    } else {
        BTreeMap::new()
    };
    env.extend(extra_env.clone());
    // DECIDED: on Windows run `npx.cmd` directly instead of TS's `cmd.exe /d /s /c "<escaped>"`.
    // WHY: Rust's std applies batch-file argument escaping itself; pre-escaping would double it.
    // ASSUMPTION: untested on Windows.
    let program = match platform {
        Platform::Unix => "npx",
        Platform::Windows => "npx.cmd",
    };
    Ok(ResolvedLaunch {
        spec: LaunchSpec {
            program: program.into(),
            args: vec!["-y".into(), adapter.package_spec.into()],
            env,
            cwd: cwd.to_path_buf(),
        },
        source: LaunchSource::HostNpx,
    })
}

fn resolve_custom(
    platform: Platform,
    custom: Option<&CustomAgentConfig>,
    extra_env: &BTreeMap<String, String>,
    cwd: &Path,
    host: &HostEnvironment,
) -> Result<ResolvedLaunch, LaunchError> {
    let command = custom
        .map(|config| config.command.trim())
        .filter(|command| !command.is_empty())
        .ok_or(LaunchError::MissingCommand)?;
    let argv = split_command_line(command)?;
    let (program, args) = argv.split_first().ok_or(LaunchError::MissingCommand)?;
    let mut env = extra_env.clone();
    // A GUI-launched server lacks the shell-profile PATH (homebrew, nvm, ~/.local/bin). Like TS,
    // this replaces any PATH from the custom env with login PATH + inherited PATH.
    if let Some(login) = login_shell_path(platform, &host.inherited_env) {
        let inherited = host
            .inherited_env
            .get(&platform.path_key(&host.inherited_env))
            .map(String::as_str)
            .unwrap_or_default();
        env.insert("PATH".into(), merge_path(&login, inherited));
    }
    Ok(ResolvedLaunch {
        spec: LaunchSpec {
            program: program.into(),
            args: args.to_vec(),
            env,
            cwd: cwd.to_path_buf(),
        },
        source: LaunchSource::Custom,
    })
}

/// Splits a custom agent's command line. Single and double quotes are literal and may join
/// adjacent text; backslashes stay literal so Windows paths survive; an unterminated quote fails
/// up front rather than spawning the wrong argv.
pub fn split_command_line(command: &str) -> Result<Vec<String>, LaunchError> {
    let mut argv = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut has_token = false;
    for ch in command.chars() {
        if let Some(open) = quote {
            if ch == open {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                has_token = true;
            }
            ' ' | '\t' | '\n' | '\r' => {
                if has_token {
                    argv.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            _ => {
                current.push(ch);
                has_token = true;
            }
        }
    }
    if quote.is_some() {
        return Err(LaunchError::UnterminatedQuote);
    }
    if has_token {
        argv.push(current);
    }
    Ok(argv)
}

fn non_empty(path: Option<&Path>) -> Option<&Path> {
    path.filter(|path| {
        path.to_str()
            .map_or(!path.as_os_str().is_empty(), |text| !text.trim().is_empty())
    })
}

fn resolve_bundled_bun(resources_dir: Option<&Path>, platform: Platform) -> Option<PathBuf> {
    let name = match platform {
        Platform::Unix => "bun",
        Platform::Windows => "bun.exe",
    };
    let candidate = non_empty(resources_dir)?
        .join("bin")
        .join("third_party")
        .join(name);
    let metadata = std::fs::metadata(&candidate).ok()?;
    (metadata.is_file() && (platform == Platform::Windows || is_executable(&metadata)))
        .then_some(candidate)
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_: &std::fs::Metadata) -> bool {
    true
}

/// PATH = [node shim, bun's dir, inherited PATH…] (deduped) plus a private Bun install cache.
/// Adapter entrypoints are `#!/usr/bin/env node`; a GUI app often has no Node on PATH, so the shim
/// named `node` execs the bundled Bun instead.
fn bundled_bun_env(
    bun: &Path,
    browseros_dir: &Path,
    inherited: &BTreeMap<String, String>,
    platform: Platform,
) -> BTreeMap<String, String> {
    let key = platform.path_key(inherited);
    let browseros_dir = non_empty(Some(browseros_dir));
    let shim = browseros_dir.and_then(|dir| ensure_node_shim(bun, dir, platform));
    let bun_dir = bun.parent().map(|dir| dir.to_string_lossy().into_owned());
    let inherited_path = inherited.get(&key).map(String::as_str).unwrap_or_default();
    let entries = shim
        .into_iter()
        .chain(bun_dir)
        .chain(inherited_path.split(platform.delimiter()).map(String::from));
    let mut env = BTreeMap::from([(key, dedupe_join(entries, platform.delimiter()))]);
    if let Some(dir) = browseros_dir {
        env.insert(
            "BUN_INSTALL_CACHE_DIR".into(),
            dir.join("cache")
                .join("bun-install")
                .to_string_lossy()
                .into_owned(),
        );
    }
    env
}

/// Writes `<browserosDir>/cache/acp-node-shim/node` and returns its directory; `None` on Windows or
/// on any write failure (the launch then relies on a host Node, as TS does).
fn ensure_node_shim(bun: &Path, browseros_dir: &Path, platform: Platform) -> Option<String> {
    if platform == Platform::Windows {
        return None;
    }
    let dir = browseros_dir.join("cache").join("acp-node-shim");
    let shim = dir.join("node");
    let script = format!(
        "#!/bin/sh\nexec {} \"$@\"\n",
        shell_quote(&bun.to_string_lossy())
    );
    let written = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(&shim, script))
        .and_then(|()| make_executable(&shim));
    match written {
        Ok(()) => Some(dir.to_string_lossy().into_owned()),
        Err(error) => {
            tracing::warn!(%error, "Unable to write the ACP node shim");
            None
        }
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn make_executable(_: &Path) -> std::io::Result<()> {
    Ok(())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Prepends `<resources>/bin/third_party` (where bundled native CLIs would live) when it exists.
fn prepend_native_dir(
    env: &mut BTreeMap<String, String>,
    resources_dir: Option<&Path>,
    platform: Platform,
) {
    let Some(resources_dir) = non_empty(resources_dir) else {
        return;
    };
    let dir = resources_dir.join("bin").join("third_party");
    if !dir.is_dir() {
        return;
    }
    let dir = dir.to_string_lossy().into_owned();
    let key = platform.path_key(env);
    let existing = env.get(&key).cloned().unwrap_or_default();
    let parts = existing
        .split(platform.delimiter())
        .filter(|part| !part.is_empty() && *part != dir)
        .map(String::from);
    let joined = std::iter::once(dir.clone())
        .chain(parts)
        .collect::<Vec<_>>()
        .join(&platform.delimiter().to_string());
    env.insert(key, joined);
}

fn dedupe_join(entries: impl Iterator<Item = String>, delimiter: char) -> String {
    let mut seen = Vec::<String>::new();
    for entry in entries.filter(|entry| !entry.is_empty()) {
        if !seen.contains(&entry) {
            seen.push(entry);
        }
    }
    seen.join(&delimiter.to_string())
}

/// Login PATH first, then inherited PATH, deduplicated (always `:`; Unix only).
fn merge_path(login: &str, inherited: &str) -> String {
    dedupe_join(
        login
            .split(':')
            .chain(inherited.split(':'))
            .map(String::from),
        ':',
    )
}

/// Login-shell PATH per shell, resolved once per process like TS's module cache (a failed
/// resolution is cached too, so a broken profile costs one 5 s timeout, not one per launch).
static LOGIN_PATHS: Mutex<BTreeMap<String, Option<String>>> = Mutex::new(BTreeMap::new());
const LOGIN_SHELL_TIMEOUT: Duration = Duration::from_secs(5);

fn login_shell_path(platform: Platform, env: &BTreeMap<String, String>) -> Option<String> {
    if platform == Platform::Windows {
        return None;
    }
    let shell = env
        .get("SHELL")
        .cloned()
        .unwrap_or_else(|| "/bin/bash".into());
    let mut cache = LOGIN_PATHS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cache
        .entry(shell)
        .or_insert_with_key(|shell| query_login_shell(shell))
        .clone()
}

fn query_login_shell(shell: &str) -> Option<String> {
    let mut child = std::process::Command::new(shell)
        .args(["-lc", "printf %s \"$PATH\""])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    // Read on a helper thread: a profile that backgrounds a process can hold the pipe open past
    // the shell's exit, and the deadline must still fire.
    std::thread::spawn(move || {
        let mut output = String::new();
        let _ = tx.send(stdout.read_to_string(&mut output).map(|_| output));
    });
    let output = rx.recv_timeout(LOGIN_SHELL_TIMEOUT);
    if output.is_err() {
        let _ = child.kill();
    }
    let _ = child.wait();
    let path = output.ok()?.ok()?.trim().to_owned();
    (!path.is_empty()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(dir: &Path, path: &str) -> HostEnvironment {
        HostEnvironment {
            resources_dir: None,
            browseros_dir: dir.into(),
            inherited_env: BTreeMap::from([("Path".into(), path.into())]),
        }
    }

    #[test]
    fn windows_npx_keeps_the_existing_path_key_spelling() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let resources = dir.path().join("resources");
        std::fs::create_dir_all(resources.join("bin").join("third_party"))?;
        let mut host = host(dir.path(), r"C:\Windows;C:\node");
        host.resources_dir = Some(resources.clone());
        let launch = resolve_launch_on(
            Platform::Windows,
            AgentKind::Claude,
            None,
            &BTreeMap::new(),
            dir.path(),
            &host,
        )?;
        assert_eq!(launch.source, LaunchSource::HostNpx);
        assert_eq!(launch.spec.program, PathBuf::from("npx.cmd"));
        let native = resources.join("bin").join("third_party");
        assert_eq!(
            launch.spec.env.get("Path"),
            Some(&format!(r"{};C:\Windows;C:\node", native.display()))
        );
        Ok(())
    }

    #[test]
    fn windows_never_resolves_a_login_shell_path() {
        assert_eq!(login_shell_path(Platform::Windows, &BTreeMap::new()), None);
    }
}
