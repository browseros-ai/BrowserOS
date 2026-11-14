//! Restarting child supervision. One attempt owns the child, launch file and
//! sole stdin writer. Closing that life line also kills the SDK after host SIGKILL.

#![cfg(unix)]

use std::{
    future::Future,
    io,
    net::TcpListener,
    os::unix::{fs::PermissionsExt, net::SocketAddr},
    path::Path,
    process::Stdio,
    time::Duration,
};

use serde_json::{Value, json};
use tokio::{
    fs,
    process::{Child, ChildStdin, Command},
    time::Instant,
};

use super::{
    EngineLaunch, EngineState, EngineUnavailable, NeoEngine, RunTool, host_api::HostApi, wire::Wire,
};

/// Attempt-owned credentials remain available through normal child shutdown.
struct Spawned {
    child: Child,
    wire: Wire,
    host: HostApi,
    launch_file: tempfile::TempPath,
    host_files: tempfile::TempDir,
    port: u16,
}

/// Failed attempts move through this capped schedule. A minute of healthy
/// service resets it, so a rapid crash loop cannot keep retrying once a second.
#[derive(Default)]
struct RestartBackoff {
    step: usize,
}

impl RestartBackoff {
    fn after_failure(&mut self, ready_for: Duration) -> Duration {
        const DELAYS: [Duration; 3] = [
            Duration::from_secs(1),
            Duration::from_secs(5),
            Duration::from_secs(30),
        ];
        if ready_for >= Duration::from_secs(60) {
            self.step = 0;
        }
        let delay = DELAYS[self.step];
        self.step = (self.step + 1).min(DELAYS.len() - 1);
        delay
    }
}

/// The attempt reports only lifecycle facts. The restart loop never owns a
/// process, which lets tests drive this same loop with a fake child.
enum AttemptExit {
    Stopped,
    Failed {
        reason: String,
        port: Option<u16>,
        ready_for: Duration,
    },
}

pub(super) async fn supervise(engine: &NeoEngine, launch: EngineLaunch) {
    load_saved_tool(engine, &launch.state_dir.join("claw-sdk/run-tool.json")).await;
    supervise_with(engine, launch, run_attempt).await;
}

async fn load_saved_tool(engine: &NeoEngine, path: &Path) {
    if let Ok(saved) = fs::read(path).await
        && let Ok(tool) = serde_json::from_slice::<RunTool>(&saved)
    {
        if !tool.has_compatible_schema() {
            tracing::warn!("the saved Playwright run definition has an incompatible schema");
            return;
        }
        engine
            .inner
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .tool = Some(tool);
        // Discovery may have started before disk I/O completed. Wake watchers
        // even though loading a cached description does not make the child ready.
        engine.inner.state.send_modify(|_| {});
    }
}

async fn supervise_with<F, Fut>(engine: &NeoEngine, launch: EngineLaunch, mut attempt: F)
where
    F: FnMut(NeoEngine, EngineLaunch, Option<u16>) -> Fut,
    Fut: Future<Output = AttemptExit>,
{
    let mut backoff = RestartBackoff::default();
    let mut previous_port = None;
    while !engine.inner.stop.is_cancelled() {
        match attempt(engine.clone(), launch.clone(), previous_port).await {
            AttemptExit::Stopped => break,
            AttemptExit::Failed {
                reason,
                port,
                ready_for,
            } => {
                previous_port = port.or(previous_port);
                let delay = backoff.after_failure(ready_for);
                tracing::warn!(%reason, retry_in_seconds = delay.as_secs(), "the Playwright engine is restarting");
                engine.unavailable_until(reason, Some(std::time::Instant::now() + delay));
                // Shutdown interrupts the wait and the loop condition prevents
                // a final respawn when cancellation races with the deadline.
                tokio::select! {
                    biased;
                    () = engine.inner.stop.cancelled() => break,
                    () = tokio::time::sleep(delay) => {}
                }
            }
        }
    }
    engine.unavailable("the Playwright engine was shut down".to_owned());
}

async fn run_attempt(
    engine: NeoEngine,
    launch: EngineLaunch,
    previous_port: Option<u16>,
) -> AttemptExit {
    let spawned = tokio::select! {
        biased;
        () = engine.inner.stop.cancelled() => return AttemptExit::Stopped,
        spawned = spawn(&launch, previous_port) => spawned,
    };
    let Spawned {
        mut child,
        wire,
        host,
        launch_file: _launch_file,
        host_files: _host_files,
        port,
    } = match spawned {
        Ok(spawned) => spawned,
        Err(error) => {
            return AttemptExit::Failed {
                reason: format!("the Playwright engine could not start: {error}"),
                port: previous_port,
                ready_for: Duration::ZERO,
            };
        }
    };
    // Child::wait closes child.stdin itself. Take the writer out first, or
    // simply monitoring an otherwise healthy child would tell it to exit.
    let mut lifeline = child.stdin.take();
    let mut ready_at = None;
    let outcome = async {
        let tool = tokio::select! {
            biased;
            () = engine.inner.stop.cancelled() => return None,
            status = child.wait() => return Some(exited(status).reason),
            result = wait_ready(&wire) => match result {
                Ok(tool) => tool,
                Err(error) => return Some(error.reason),
            },
        };
        if !tool.has_compatible_schema() {
            return Some("the Playwright engine returned an incompatible run schema".to_owned());
        }
        let saved_tool = launch.state_dir.join("claw-sdk/run-tool.json");
        tokio::select! {
            biased;
            () = engine.inner.stop.cancelled() => return None,
            result = save_tool(&saved_tool, &tool) => {
                if let Err(error) = result {
                    tracing::warn!(%error, "the Playwright run definition could not be cached");
                }
            }
        }
        {
            let mut connection = engine
                .inner
                .connection
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            connection.wire = Some(wire);
            connection.host = Some(host);
            connection.tool = Some(tool);
        }
        ready_at = Some(Instant::now());
        tracing::info!(pid = child.id(), port, "the Playwright engine is ready");
        engine.inner.state.send_replace(EngineState::Ready);
        tokio::select! {
            biased;
            () = engine.inner.stop.cancelled() => None,
            status = child.wait() => Some(exited(status).reason),
        }
    }
    .await;
    // Readiness failure must reap the child too. Merely dropping a kill_on_drop
    // handle sends a signal without waiting, and a retry could overlap it.
    stop_child(&mut child, &mut lifeline).await;
    match outcome {
        None => AttemptExit::Stopped,
        Some(reason) => AttemptExit::Failed {
            reason,
            port: Some(port),
            ready_for: ready_at.map_or(Duration::ZERO, |ready_at| ready_at.elapsed()),
        },
    }
}

async fn spawn(launch: &EngineLaunch, previous_port: Option<u16>) -> io::Result<Spawned> {
    let state_dir = std::path::absolute(&launch.state_dir)?;
    let sdk_dir = state_dir.join("claw-sdk");
    fs::create_dir_all(&sdk_dir).await?;
    configure_settings(&sdk_dir.join("settings.json")).await?;
    // The SDK refuses port 0. Hold the reservation until spawn, and avoid the
    // previous port even if the OS offers it again after an exit-code-2 clash.
    let reservation = reserve_port(previous_port)?;
    let port = reservation.local_addr()?.port();
    let config = launch_config(launch, port).await?;
    let config_path = temporary_json(&sdk_dir, &config).await?;
    let wire = Wire::new(port).map_err(io::Error::other)?;
    // Each attempt has its own 0700 directory and socket. Shared desktop state
    // can host several stdio parents, so fixed credential/socket paths would race.
    // Keep sockets outside the possibly deep state directory while the explicit
    // token path still gets SDK protection. The chooser also handles deep TMPDIRs.
    let host_files = host_files_dir(&std::env::temp_dir())?;
    fs::set_permissions(host_files.path(), std::fs::Permissions::from_mode(0o700)).await?;
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let token_path = host_files.path().join("token");
    fs::write(&token_path, &token).await?;
    fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o600)).await?;
    let host_reservation = reserve_port(None)?;
    let host_port = host_reservation.local_addr()?.port();
    let host = HostApi::new(host_port, token).map_err(io::Error::other)?;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("__neo-sdk")
        .arg("--config")
        .arg(&config_path)
        .env("BROWSERCLAW_DIR", &state_dir)
        .env("BROWSERCLAW_HOSTED_MODE", "0")
        .env("CLAW_XVNC_ADDR", "127.0.0.1:9")
        .env("CLAW_HOST_TCP", format!("127.0.0.1:{host_port}"))
        .env("CLAW_HOST_SOCKET", host_files.path().join("host.sock"))
        .env("CLAW_HOST_TOKEN_FILE", &token_path)
        .env_remove("CLAW_DOWNLOADS_DIR")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    drop(reservation);
    drop(host_reservation);
    let child = command.spawn()?;
    tracing::info!(pid = child.id(), port, "the Playwright engine was started");
    // Keep this private path for the whole attempt so an operator can inspect
    // the actual version/flags/port used by this particular child.
    Ok(Spawned {
        child,
        wire,
        host,
        launch_file: config_path,
        host_files,
        port,
    })
}

/// Keep attempt-owned credentials private and the SDK's mandatory Unix socket
/// bindable, even when the inherited temporary root has a long byte path.
fn host_files_dir(temp_root: &Path) -> io::Result<tempfile::TempDir> {
    let mut builder = tempfile::Builder::new();
    builder
        .prefix("neo-host-")
        .permissions(std::fs::Permissions::from_mode(0o700));
    // Reject impossible roots before allocation can hit the filesystem path limit.
    if validate_host_socket_path(&temp_root.join("host.sock")).is_ok() {
        let inherited = builder.tempdir_in(temp_root)?;
        if validate_host_socket_path(&inherited.path().join("host.sock")).is_ok() {
            return Ok(inherited);
        }
    }
    let fallback = builder.tempdir_in("/tmp")?;
    validate_host_socket_path(&fallback.path().join("host.sock"))?;
    Ok(fallback)
}

/// Use the same byte limit as the SDK's UnixListener::bind and retain the
/// offending path in the startup error before the child's exit can obscure it.
fn validate_host_socket_path(socket_path: &Path) -> io::Result<()> {
    SocketAddr::from_pathname(socket_path).map(|_| ()).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "host socket {} ({} bytes) exceeds the platform Unix socket pathname limit: {error}",
                socket_path.display(),
                socket_path.as_os_str().len(),
            ),
        )
    })
}

fn reserve_port(previous_port: Option<u16>) -> io::Result<TcpListener> {
    let reservation = TcpListener::bind("127.0.0.1:0")?;
    if Some(reservation.local_addr()?.port()) == previous_port {
        // Keep the rejected reservation bound while asking for another port.
        return TcpListener::bind("127.0.0.1:0");
    }
    Ok(reservation)
}

async fn launch_config(launch: &EngineLaunch, port: u16) -> io::Result<Value> {
    let original: Value = serde_json::from_slice(&fs::read(&launch.config_path).await?)?;
    let mut instance = original
        .get("instance")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(version) = &launch.browseros_version {
        // Use the identity the host resolved, including an explicit developer
        // assertion. Never stamp the independently updated server's version.
        instance.insert("browseros_version".to_owned(), json!(version));
    }
    let mut config = json!({
        "ports": {"cdp": launch.cdp_port, "server": port},
        "directories": {"resources": std::path::absolute(&launch.resources_dir)?},
        "flags": {"script_cdp": true},
        "instance": instance,
    });
    if let Some(execution) = original["directories"]["execution"]
        .as_str()
        .filter(|value| !value.is_empty())
    {
        let original_dir = launch.config_path.parent().unwrap_or_else(|| Path::new(""));
        config["directories"]["execution"] =
            json!(std::path::absolute(original_dir.join(execution))?);
    }
    Ok(config)
}

/// Desktop owns these two settings even when an earlier SDK launch saved overrides.
/// Merge atomically so a crash cannot restore hosted defaults or drop unrelated settings.
async fn configure_settings(path: &Path) -> io::Result<()> {
    let mut settings: serde_json::Map<String, Value> = match fs::read(path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            tracing::warn!(%error, "invalid SDK settings replaced with desktop policy");
            Default::default()
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Default::default(),
        Err(error) => return Err(error),
    };
    for (section, key, value) in [
        ("tabs", "idleCloseSeconds", json!(86400)),
        ("trace", "screenshots", json!("off")),
    ] {
        let section = settings.entry(section).or_insert_with(|| json!({}));
        if !section.is_object() {
            tracing::warn!(
                key,
                "invalid SDK settings section replaced with desktop policy"
            );
            *section = json!({});
        }
        section[key] = value;
    }
    save_json(path, &Value::Object(settings)).await
}

/// Each writer owns its temporary path until publication or child exit.
/// Reusing a fixed .tmp name would race across independent stdio server processes.
async fn temporary_json(dir: &Path, value: &Value) -> io::Result<tempfile::TempPath> {
    let temporary = tempfile::Builder::new()
        .prefix("neo-")
        .suffix(".json")
        .tempfile_in(dir)?
        .into_temp_path();
    fs::write(&temporary, serde_json::to_vec_pretty(value)?).await?;
    Ok(temporary)
}

async fn save_json(path: &Path, value: &Value) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("SDK state file has no parent"))?;
    temporary_json(dir, value)
        .await?
        .persist(path)
        .map_err(|error| error.error)
}

async fn wait_ready(wire: &Wire) -> Result<RunTool, EngineUnavailable> {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if wire.healthy().await {
                return wire.tool().await;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .map_err(|_| {
        EngineUnavailable::new("the Playwright engine did not become healthy within 30 s")
    })?
}

async fn stop_child(child: &mut Child, lifeline: &mut Option<ChildStdin>) {
    drop(lifeline.take());
    if tokio::time::timeout(Duration::from_secs(2), child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
    }
}

fn exited(status: io::Result<std::process::ExitStatus>) -> EngineUnavailable {
    match status {
        Ok(status) => EngineUnavailable::new(format!("the Playwright engine exited ({status})")),
        Err(error) => EngineUnavailable::new(format!(
            "the Playwright engine could not be observed: {error}"
        )),
    }
}

async fn save_tool(path: &Path, tool: &RunTool) -> io::Result<()> {
    let definition = json!({
        "name": "run", "description": tool.description, "inputSchema": tool.input_schema
    });
    save_json(path, &definition).await
}

#[cfg(test)]
mod tests {
    use std::{
        os::unix::{
            fs::PermissionsExt,
            net::{SocketAddr, UnixListener},
        },
        path::PathBuf,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use super::{
        AttemptExit, RestartBackoff, configure_settings, host_files_dir, launch_config,
        load_saved_tool, reserve_port, save_tool, supervise_with, temporary_json,
    };
    use crate::services::neo::{EngineLaunch, EngineState, NeoEngine, RunTool};
    use serde_json::{Value, json};

    #[test]
    fn host_files_dir_keeps_short_temp_root() -> anyhow::Result<()> {
        let root = tempfile::tempdir_in("/tmp")?;
        let first = host_files_dir(root.path())?;
        let second = host_files_dir(root.path())?;
        assert_eq!(first.path().parent(), Some(root.path()));
        assert_eq!(second.path().parent(), Some(root.path()));
        assert_ne!(first.path(), second.path());
        assert_eq!(first.path().metadata()?.permissions().mode() & 0o777, 0o700);
        let socket = UnixListener::bind(first.path().join("host.sock"))?;
        let path = first.path().to_owned();
        drop(socket);
        drop(first);
        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn host_files_dir_falls_back_when_temp_root_is_too_long() -> anyhow::Result<()> {
        let root = tempfile::tempdir_in("/tmp")?;
        let long_root = root.path().join("é".repeat(35));
        std::fs::create_dir(&long_root)?;
        // The root fits, but the generated suffix exceeds the limit in bytes.
        assert!(SocketAddr::from_pathname(long_root.join("host.sock")).is_ok());
        let first = host_files_dir(&long_root)?;
        let second = host_files_dir(&long_root)?;
        let socket = UnixListener::bind(first.path().join("host.sock"))?;
        assert!(!first.path().starts_with(&long_root));
        assert_ne!(first.path(), second.path());
        assert_eq!(first.path().metadata()?.permissions().mode() & 0o777, 0o700);
        assert_eq!(std::fs::read_dir(&long_root)?.count(), 0);
        let path = first.path().to_owned();
        drop(socket);
        drop(first);
        assert!(!path.exists());
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn host_files_dir_falls_back_before_directory_creation_exceeds_path_limit() -> anyhow::Result<()>
    {
        let root = tempfile::tempdir_in("/tmp")?;
        let mut long_root = root.path().to_owned();
        // This root fits macOS's filesystem limit, but adding neo-host-XXXXXX
        // would exceed it before the generated socket path could be validated.
        while long_root.as_os_str().len() + 201 < 1010 {
            long_root.push("x".repeat(200));
        }
        long_root.push("x".repeat(1010 - long_root.as_os_str().len() - 1));
        std::fs::create_dir_all(&long_root)?;
        let files = host_files_dir(&long_root)?;
        let _socket = UnixListener::bind(files.path().join("host.sock"))?;
        assert!(!files.path().starts_with(&long_root));
        assert_eq!(std::fs::read_dir(&long_root)?.count(), 0);
        Ok(())
    }

    fn fake_launch() -> EngineLaunch {
        EngineLaunch {
            state_dir: PathBuf::new(),
            resources_dir: PathBuf::new(),
            config_path: PathBuf::new(),
            cdp_port: 49337,
            browseros_version: None,
            hosted: false,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn fake_child_restarts_with_capped_backoff_and_keeps_previous_ports() {
        let engine = NeoEngine::disabled("fake child");
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&attempts);
        let began = tokio::time::Instant::now();
        let mut count = 0_u16;
        supervise_with(&engine, fake_launch(), move |engine, _, previous_port| {
            count += 1;
            let count = count;
            observed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((began.elapsed().as_secs(), previous_port));
            async move {
                if count == 5 {
                    engine.inner.stop.cancel();
                    return AttemptExit::Stopped;
                }
                AttemptExit::Failed {
                    reason: "fake child exited with code 2".to_owned(),
                    port: Some(count),
                    ready_for: Duration::ZERO,
                }
            }
        })
        .await;
        assert_eq!(
            *attempts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            vec![
                (0, None),
                (1, Some(1)),
                (6, Some(2)),
                (36, Some(3)),
                (66, Some(4))
            ],
        );
        assert!(matches!(
            engine.state(),
            EngineState::Unavailable { retry_at: None, .. }
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_during_backoff_preserves_failure_and_prevents_another_child()
    -> anyhow::Result<()> {
        let engine = NeoEngine::disabled("fake child");
        let supervised = engine.clone();
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = Arc::clone(&attempts);
        let mut states = engine.watch();
        let task = tokio::spawn(async move {
            supervise_with(&supervised, fake_launch(), move |_, _, _| {
                count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                async {
                    AttemptExit::Failed {
                        reason: "permission denied at claw-sdk".to_owned(),
                        port: None,
                        ready_for: Duration::ZERO,
                    }
                }
            })
            .await;
        });
        states.changed().await?;
        let reason = engine.not_ready().to_string();
        assert!(reason.contains("permission denied at claw-sdk"));
        assert!(reason.contains("restarting"));
        assert!(matches!(
            engine.state(),
            EngineState::Unavailable {
                retry_at: Some(_),
                ..
            }
        ));
        engine.shutdown().await;
        task.await?;
        tokio::time::advance(Duration::from_secs(60)).await;
        assert_eq!(attempts.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert!(matches!(
            engine.state(),
            EngineState::Unavailable { retry_at: None, .. }
        ));
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn fake_child_can_recover_and_shutdown_while_ready() -> anyhow::Result<()> {
        let engine = NeoEngine::disabled("fake child");
        let supervised = engine.clone();
        let mut states = engine.watch();
        let mut count = 0;
        let task = tokio::spawn(async move {
            supervise_with(&supervised, fake_launch(), move |engine, _, _| {
                count += 1;
                let first = count == 1;
                async move {
                    if first {
                        return AttemptExit::Failed {
                            reason: "fake child died".to_owned(),
                            port: Some(1234),
                            ready_for: Duration::ZERO,
                        };
                    }
                    engine.inner.state.send_replace(EngineState::Ready);
                    engine.inner.stop.cancelled().await;
                    AttemptExit::Stopped
                }
            })
            .await;
        });
        loop {
            states.changed().await?;
            if matches!(*states.borrow_and_update(), EngineState::Ready) {
                break;
            }
        }
        engine.shutdown().await;
        task.await?;
        assert!(engine.not_ready().reason.contains("shut down"));
        Ok(())
    }

    #[test]
    fn healthy_service_resets_backoff_but_rapid_ready_crashes_do_not() {
        let mut backoff = RestartBackoff::default();
        assert_eq!(
            backoff.after_failure(Duration::ZERO),
            Duration::from_secs(1)
        );
        assert_eq!(
            backoff.after_failure(Duration::from_secs(1)),
            Duration::from_secs(5)
        );
        assert_eq!(
            backoff.after_failure(Duration::from_secs(59)),
            Duration::from_secs(30)
        );
        assert_eq!(
            backoff.after_failure(Duration::from_secs(60)),
            Duration::from_secs(1)
        );
    }

    #[tokio::test]
    async fn saved_definition_is_used_only_for_the_current_argument_dialect() -> anyhow::Result<()>
    {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("run-tool.json");
        let mut expected = RunTool::fallback();
        expected.description = "the exact SDK description".to_owned();
        save_tool(&path, &expected).await?;
        let engine = NeoEngine::disabled("fake child");
        load_saved_tool(&engine, &path).await;
        assert_eq!(engine.tool_snapshot().description, expected.description);
        assert_eq!(engine.tool().await.description, expected.description);

        for invalid_schema in [
            json!({"type":"object","properties":{"script":{"type":"string"}},"required":["script"],"additionalProperties":false}),
            json!({"type":"object","properties":{"code":{"type":"string"},"timeout":{"type":"string"}},"required":["code"],"additionalProperties":false}),
            json!({"type":"object","properties":{"code":{"type":"string"},"timeout":{"type":"integer"},"session":{"type":"string"}},"required":["code"],"additionalProperties":false}),
        ] {
            let value = json!({"description":"stale dialect","inputSchema":invalid_schema});
            tokio::fs::write(&path, serde_json::to_vec(&value)?).await?;
            let engine = NeoEngine::disabled("fake child");
            load_saved_tool(&engine, &path).await;
            assert_ne!(engine.tool_snapshot().description, "stale dialect");
            assert_eq!(
                engine.tool_snapshot().input_schema,
                RunTool::fallback().input_schema
            );
        }
        tokio::fs::write(&path, b"{invalid JSON").await?;
        let engine = NeoEngine::disabled("fake child");
        load_saved_tool(&engine, &path).await;
        assert_eq!(
            engine.tool_snapshot().input_schema,
            RunTool::fallback().input_schema
        );
        Ok(())
    }

    #[tokio::test]
    async fn launch_file_carries_resolved_browser_version_and_script_cdp() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let config_path = dir.path().join("config.json");
        tokio::fs::write(
            &config_path,
            serde_json::to_vec(&json!({
                "instance":{"install_id":"installed-browser","browseros_version":"0.1.2.3"},
                "directories":{"execution":"profile/.browseros"},
            }))?,
        )
        .await?;
        let launch = EngineLaunch {
            config_path,
            resources_dir: dir.path().join("resources"),
            browseros_version: Some("0.51.2.7".to_owned()),
            ..fake_launch()
        };
        let config = launch_config(&launch, 32001).await?;
        assert_eq!(config["instance"]["browseros_version"], "0.51.2.7");
        assert_eq!(config["instance"]["install_id"], "installed-browser");
        assert_eq!(config["flags"]["script_cdp"], true);
        assert_eq!(config["ports"]["server"], 32001);
        assert_eq!(config["ports"]["cdp"], launch.cdp_port);
        assert_eq!(
            config["directories"]["execution"],
            json!(dir.path().join("profile/.browseros"))
        );
        Ok(())
    }

    #[test]
    fn new_reservation_avoids_the_previous_childs_port() -> anyhow::Result<()> {
        let previous = reserve_port(None)?;
        let port = previous.local_addr()?.port();
        drop(previous);
        let next = reserve_port(Some(port))?;
        assert_ne!(next.local_addr()?.port(), port);
        Ok(())
    }

    #[tokio::test]
    async fn desktop_settings_override_existing_defaults_and_preserve_other_values()
    -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("settings.json");
        for existing in [
            json!({}),
            json!({"tabs":null,"trace":[]}),
            json!({"tabs":{"scope":"task","idleCloseSeconds":1800},"trace":{"screenshots":"operations","valueBytes":42}}),
        ] {
            tokio::fs::write(&path, serde_json::to_vec(&existing)?).await?;
            configure_settings(&path).await?;
            let saved: Value = serde_json::from_slice(&tokio::fs::read(&path).await?)?;
            assert_eq!(saved["tabs"]["scope"], "browser");
            assert_eq!(saved["tabs"]["idleCloseSeconds"], 86400);
            assert_eq!(saved["trace"]["screenshots"], "off");
            assert_eq!(
                saved["trace"]["valueBytes"],
                existing["trace"]["valueBytes"]
            );
        }
        tokio::fs::write(&path, b"malformed JSON").await?;
        configure_settings(&path).await?;
        let saved: Value = serde_json::from_slice(&tokio::fs::read(&path).await?)?;
        assert_eq!(
            saved,
            json!({"tabs":{"scope":"browser","idleCloseSeconds":86400},"trace":{"screenshots":"off"}})
        );
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_parents_keep_separate_launch_ports_and_can_publish_shared_settings()
    -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let first_config = json!({"ports":{"server":12345}});
        let second_config = json!({"ports":{"server":54321}});
        let (first, second) = tokio::try_join!(
            temporary_json(dir.path(), &first_config),
            temporary_json(dir.path(), &second_config),
        )?;
        assert_ne!(&*first, &*second);
        assert_eq!(
            serde_json::from_slice::<Value>(&tokio::fs::read(&first).await?)?,
            first_config
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&tokio::fs::read(&second).await?)?,
            second_config
        );
        let path = dir.path().join("settings.json");
        for _ in 0..10 {
            tokio::try_join!(configure_settings(&path), configure_settings(&path))?;
        }
        let saved: Value = serde_json::from_slice(&tokio::fs::read(&path).await?)?;
        assert_eq!(
            saved,
            json!({"tabs":{"scope":"browser","idleCloseSeconds":86400},"trace":{"screenshots":"off"}})
        );
        Ok(())
    }
}
