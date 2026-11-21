//! Supervision of the engine child: start it, see it ready, watch it, restart it.
//!
//! One attempt owns one child, its launch file, its host-API token and the only write end of
//! its stdin. Closing that stdin is the lifeline: the child exits on EOF, which also happens
//! when this process is killed outright. The loop above an attempt owns no process, only the
//! backoff, so a failed attempt has always reaped its child before the next one starts.
//!
//! ```text
//!   spawn ─▶ /system/ready 200 ─▶ tools/list ─▶ run `return 1` ─▶ Up (poll /system/ready)
//!     │ exit 2 before ready: the port was taken, start again at once on another (3 times)
//!     └ any other exit or a readiness timeout: Down, then 1 s, 5 s, 30 s, 30 s … and again;
//!       a minute up resets the backoff
//! ```

use std::io;
use std::net::TcpListener;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::process::{Child, ChildStdin, Command};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::engine_main::EXIT_PORT_IN_USE;
use super::host::Host;
use super::wire::Wire;
use super::{Connection, EngineLaunch, Link, NeoEngine};
use crate::browser::Tool;

/// From spawn to a real `run`. Probe p1 measured 30 ms to 2.4 s against a running browser.
const READY_WITHIN: Duration = Duration::from_secs(30);
/// How often an up engine's browser link is read for `connected()`.
const LINK_POLL: Duration = Duration::from_secs(2);
/// The child's time to exit after its stdin closes before it is killed: the launcher kills the
/// whole server 5 s after its SIGTERM (CORRECTIONS-1 A11).
const STOP_GRACE: Duration = Duration::from_secs(2);
/// Starts in a row that may end in "port taken" before that counts as a failure.
const PORT_RETRIES: u32 = 3;
/// A closed loopback port for Xvnc: the default, `127.0.0.1:5900`, is Screen Sharing on a Mac.
const CLOSED_XVNC: &str = "127.0.0.1:9";

pub(super) async fn supervise(engine: NeoEngine, launch: EngineLaunch, stop: CancellationToken) {
    let mut backoff = Backoff::default();
    let mut clashes = 0;
    while !stop.is_cancelled() {
        engine.set_link(Link::Starting);
        let (reason, up_for) = match attempt(&engine, &launch, &stop).await {
            Exit::Stopped => break,
            Exit::PortInUse if clashes < PORT_RETRIES => {
                clashes += 1;
                tracing::warn!("the browser engine's port was taken; starting it on another");
                continue;
            }
            Exit::PortInUse => (
                "the browser engine found no free port".to_owned(),
                Duration::ZERO,
            ),
            Exit::Failed { reason, up_for } => (reason, up_for),
        };
        clashes = 0;
        let delay = backoff.after_failure(up_for);
        tracing::warn!(%reason, retry_in_s = delay.as_secs(), "the browser engine is down");
        engine.set_link(Link::Down(reason));
        tokio::select! {
            biased;
            () = stop.cancelled() => break,
            () = tokio::time::sleep(delay) => {}
        }
    }
    engine.set_link(Link::Stopped);
}

/// How one attempt ended. `up_for` is how long the child served after it was ready.
enum Exit {
    Stopped,
    PortInUse,
    Failed { reason: String, up_for: Duration },
}

async fn attempt(engine: &NeoEngine, launch: &EngineLaunch, stop: &CancellationToken) -> Exit {
    let Spawned {
        mut child,
        wire,
        host,
        port,
        files: _files,
    } = match spawn(launch).await {
        Ok(spawned) => spawned,
        Err(error) => {
            return Exit::Failed {
                reason: format!("the browser engine could not start: {error}"),
                up_for: Duration::ZERO,
            };
        }
    };
    // `Child::wait` closes the child's stdin itself: take the lifeline out first, or watching a
    // healthy child would tell it to exit.
    let mut lifeline = child.stdin.take();
    let pid = child.id();
    tracing::info!(?pid, port, "the browser engine was started");
    let mut up_at = None;
    let exit = async {
        let tool = tokio::select! {
            biased;
            () = stop.cancelled() => return Exit::Stopped,
            status = child.wait() => return exited(status, false),
            ready = become_ready(&wire) => match ready {
                Ok(tool) => tool,
                Err(reason) => return Exit::Failed { reason, up_for: Duration::ZERO },
            },
        };
        engine.set_tool(tool);
        match host.tabs().await {
            Ok(census) => engine.seed_tabs(&census),
            Err(reason) => tracing::warn!(%reason, "the first tab census failed"),
        }
        engine.set_linked(true);
        engine.set_link(Link::Up(Arc::new(Connection {
            wire: wire.clone(),
            host,
            #[cfg(feature = "test-support")]
            pid,
        })));
        up_at = Some(Instant::now());
        tracing::info!(?pid, port, "the browser engine is ready");
        loop {
            tokio::select! {
                biased;
                () = stop.cancelled() => return Exit::Stopped,
                status = child.wait() => return exited(status, true),
                () = tokio::time::sleep(LINK_POLL) => engine.set_linked(wire.ready().await),
            }
        }
    }
    .await;
    // Reap before the next attempt, on every path: a kill-on-drop alone signals without
    // waiting, and a restart could then overlap the old child.
    stop_child(&mut child, &mut lifeline).await;
    match exit {
        Exit::Failed { reason, .. } => Exit::Failed {
            reason,
            up_for: up_at.map_or(Duration::ZERO, |at: Instant| at.elapsed()),
        },
        other => other,
    }
}

/// Ready means the browser is linked, the tool is listed, and a real script ran.
async fn become_ready(wire: &Wire) -> Result<Tool, String> {
    let ready = async {
        while !wire.ready().await {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let tool = wire.tool().await?;
        let probe = wire
            .run(json!({ "code": "return 1" }), None)
            .await
            .map_err(|error| error.to_string())?;
        if probe["structuredContent"]["value"] != json!(1) {
            return Err(format!(
                "the engine's first run did not return 1: {}",
                probe["content"]
            ));
        }
        Ok(tool)
    };
    tokio::time::timeout(READY_WITHIN, ready)
        .await
        .map_err(|_| {
            format!(
                "the browser engine was not linked to the browser within {} s",
                READY_WITHIN.as_secs()
            )
        })?
}

async fn stop_child(child: &mut Child, lifeline: &mut Option<ChildStdin>) {
    drop(lifeline.take());
    if tokio::time::timeout(STOP_GRACE, child.wait())
        .await
        .is_err()
    {
        tracing::warn!("the browser engine did not exit after its stdin closed; killing it");
        let _ = child.kill().await;
    }
}

fn exited(status: io::Result<ExitStatus>, was_up: bool) -> Exit {
    match status {
        Ok(status) if !was_up && status.code() == Some(i32::from(EXIT_PORT_IN_USE)) => {
            Exit::PortInUse
        }
        Ok(status) => Exit::Failed {
            reason: format!("the browser engine exited ({status})"),
            up_for: Duration::ZERO,
        },
        Err(error) => Exit::Failed {
            reason: format!("the browser engine could not be watched: {error}"),
            up_for: Duration::ZERO,
        },
    }
}

/// 1 s, 5 s, then 30 s for each failure in a row; a minute of healthy service starts over, so
/// one crash a day restarts at once while a crash loop settles at one start every 30 s.
#[derive(Default)]
struct Backoff {
    step: usize,
}

impl Backoff {
    fn after_failure(&mut self, up_for: Duration) -> Duration {
        const DELAYS: [Duration; 3] = [
            Duration::from_secs(1),
            Duration::from_secs(5),
            Duration::from_secs(30),
        ];
        if up_for >= Duration::from_secs(60) {
            self.step = 0;
        }
        let delay = DELAYS[self.step.min(DELAYS.len() - 1)];
        self.step += 1;
        delay
    }
}

/// One started child and what belongs to its attempt. `files` are removed when it drops.
struct Spawned {
    child: Child,
    wire: Wire,
    host: Host,
    port: u16,
    files: AttemptFiles,
}

struct AttemptFiles {
    _launch: tempfile::TempPath,
    _host: tempfile::TempDir,
}

async fn spawn(launch: &EngineLaunch) -> io::Result<Spawned> {
    let state_dir = std::path::absolute(&launch.state_dir)?;
    let sdk_dir = state_dir.join("claw-sdk");
    tokio::fs::create_dir_all(&sdk_dir).await?;
    write_settings(&sdk_dir.join("settings.json"))?;

    // The engine refuses port 0, so a port is picked here and released for the child to bind.
    // Another process can take it in between; the child then exits 2 and the supervisor starts
    // it again on another, which is cheaper and simpler than passing a bound socket across.
    let port = free_port(None)?;
    let host_port = free_port(Some(port))?;
    let launch_file = private_json(&state_dir, &launch_json(launch, port)?)?;

    // The token file and the socket live in a short private folder of their own: the socket
    // is needed even with TCP on, and macOS caps a socket path at about 103 bytes.
    let host_dir = host_files_dir(&std::env::temp_dir())?;
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let token_path = host_dir.path().join("token");
    write_private(&token_path, token.as_bytes())?;

    let wire = Wire::new(port).map_err(io::Error::other)?;
    let host = Host::new(host_port, token).map_err(io::Error::other)?;
    let exe = match &launch.exe {
        Some(exe) => exe.clone(),
        None => std::env::current_exe()?,
    };
    let child = Command::new(exe)
        .arg("__neo-engine")
        .arg("--config")
        .arg(&launch_file)
        .env("BROWSERCLAW_DIR", &state_dir)
        .env("BROWSERCLAW_HOSTED_MODE", "0")
        .env("CLAW_XVNC_ADDR", CLOSED_XVNC)
        .env("CLAW_HOST_TCP", format!("127.0.0.1:{host_port}"))
        .env("CLAW_HOST_TOKEN_FILE", &token_path)
        .env("CLAW_HOST_SOCKET", host_dir.path().join("h.sock"))
        // A developer's shell may point these at another browser's profile or folder; the
        // engine derives the profile from `directories.execution` instead.
        .env_remove("CLAW_PROFILE_DIR")
        .env_remove("CLAW_DOWNLOADS_DIR")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()?;
    Ok(Spawned {
        child,
        wire,
        host,
        port,
        files: AttemptFiles {
            _launch: launch_file,
            _host: host_dir,
        },
    })
}

/// The engine's launch JSON: the keys it reads, nothing else.
fn launch_json(launch: &EngineLaunch, port: u16) -> io::Result<Value> {
    let mut config = json!({
        "ports": { "server": port, "cdp": launch.cdp_port },
        "directories": { "execution": std::path::absolute(&launch.execution_dir)? },
    });
    if let Some(version) = &launch.browseros_version {
        config["instance"] = json!({ "browseros_version": version });
    }
    Ok(config)
}

/// A desktop browser's settings, merged over whatever the file holds (design 3.5): every run
/// carries a task yet must see the person's tabs; the idle-tab janitor cannot be turned off, so
/// it waits its longest; and no automatic pictures of the person's pages go to disk.
fn write_settings(path: &Path) -> io::Result<()> {
    let mut settings = match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<serde_json::Map<String, Value>>(&bytes)
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "the engine's settings file did not parse; replacing it");
                serde_json::Map::new()
            }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => serde_json::Map::new(),
        Err(error) => return Err(error),
    };
    for (section, key, value) in [
        ("tabs", "scope", json!("browser")),
        ("tabs", "idleCloseSeconds", json!(86_400)),
        ("trace", "screenshots", json!("off")),
    ] {
        let section = settings.entry(section).or_insert_with(|| json!({}));
        if !section.is_object() {
            *section = json!({});
        }
        section[key] = value;
    }
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("the settings file has no folder"))?;
    // Written whole under another name and renamed, as the engine asks of a host.
    private_json(dir, &Value::Object(settings))?
        .persist(path)
        .map_err(|error| error.error)?;
    Ok(())
}

/// A 0600 JSON file in `dir`, removed when the returned path drops (unless persisted).
fn private_json(dir: &Path, value: &Value) -> io::Result<tempfile::TempPath> {
    let file = tempfile::Builder::new()
        .prefix("neo-")
        .suffix(".json")
        .permissions(std::fs::Permissions::from_mode(0o600))
        .tempfile_in(dir)?;
    std::fs::write(file.path(), serde_json::to_vec_pretty(value)?)?;
    Ok(file.into_temp_path())
}

fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)
}

/// A port the OS says is free now, other than `avoid`.
fn free_port(avoid: Option<u16>) -> io::Result<u16> {
    let first = TcpListener::bind("127.0.0.1:0")?;
    let port = first.local_addr()?.port();
    if Some(port) != avoid {
        return Ok(port);
    }
    // Hold the clashing one while asking for another.
    let second = TcpListener::bind("127.0.0.1:0")?;
    Ok(second.local_addr()?.port())
}

/// A fresh 0700 folder whose `h.sock` fits the platform's socket path limit: under the temp
/// folder when that is short enough, else under `/tmp`.
fn host_files_dir(temp_root: &Path) -> io::Result<tempfile::TempDir> {
    let mut builder = tempfile::Builder::new();
    builder
        .prefix("neo-host-")
        .permissions(std::fs::Permissions::from_mode(0o700));
    if socket_fits(&temp_root.join("neo-host-XXXXXX").join("h.sock")) {
        let dir = builder.tempdir_in(temp_root)?;
        if socket_fits(&dir.path().join("h.sock")) {
            return Ok(dir);
        }
    }
    let dir = builder.tempdir_in(PathBuf::from("/tmp"))?;
    if socket_fits(&dir.path().join("h.sock")) {
        Ok(dir)
    } else {
        Err(io::Error::other(
            "no temporary folder is short enough for the host socket",
        ))
    }
}

fn socket_fits(path: &Path) -> bool {
    std::os::unix::net::SocketAddr::from_pathname(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_climbs_to_thirty_seconds_and_a_long_run_resets_it() {
        let mut backoff = Backoff::default();
        let delays: Vec<u64> = (0..4)
            .map(|_| backoff.after_failure(Duration::ZERO).as_secs())
            .collect();
        assert_eq!(delays, vec![1, 5, 30, 30]);
        assert_eq!(backoff.after_failure(Duration::from_secs(61)).as_secs(), 1);
    }

    #[test]
    fn settings_keep_other_keys_and_force_the_desktop_policy() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"tabs":{"scope":"task","idleCloseSeconds":60},"trace":"broken","files":{"readDirs":["/x"]}}"#,
        )?;
        write_settings(&path)?;
        let written: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        assert_eq!(
            written,
            json!({
                "tabs": { "scope": "browser", "idleCloseSeconds": 86400 },
                "trace": { "screenshots": "off" },
                "files": { "readDirs": ["/x"] }
            })
        );
        let mode = std::fs::metadata(&path)?.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        Ok(())
    }

    #[test]
    fn the_launch_file_names_both_ports_and_the_profile() -> io::Result<()> {
        let launch = EngineLaunch {
            cdp_port: 9101,
            state_dir: PathBuf::from("/s/neo-engine"),
            execution_dir: PathBuf::from("/profile/.browseros"),
            browseros_version: Some("0.50.5".into()),
            exe: None,
        };
        assert_eq!(
            launch_json(&launch, 9300)?,
            json!({
                "ports": { "server": 9300, "cdp": 9101 },
                "directories": { "execution": "/profile/.browseros" },
                "instance": { "browseros_version": "0.50.5" }
            })
        );
        Ok(())
    }

    #[test]
    fn the_host_folder_fits_a_socket_even_under_a_deep_temp_folder() -> io::Result<()> {
        let root = tempfile::tempdir_in("/tmp")?;
        let deep = root.path().join("d".repeat(120));
        std::fs::create_dir(&deep)?;
        let dir = host_files_dir(&deep)?;
        assert!(!dir.path().starts_with(&deep));
        let _socket = std::os::unix::net::UnixListener::bind(dir.path().join("h.sock"))?;
        assert_eq!(dir.path().metadata()?.permissions().mode() & 0o777, 0o700);
        Ok(())
    }

    #[test]
    fn a_clash_on_the_engine_port_before_ready_is_a_port_retry() {
        use std::os::unix::process::ExitStatusExt;
        let clash = ExitStatus::from_raw(i32::from(EXIT_PORT_IN_USE) << 8);
        assert!(matches!(exited(Ok(clash), false), Exit::PortInUse));
        assert!(matches!(exited(Ok(clash), true), Exit::Failed { .. }));
    }
}
