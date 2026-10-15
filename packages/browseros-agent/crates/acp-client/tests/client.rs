//! These tests cross the public client seam with a real child and byte pipes.
//! No model/network is involved: the example peer scripts protocol ordering,
//! cancellation races, malformed future updates, errors, and process failure.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use acp_client::{
    AcpError, AgentConnection, ApproveAll, ClientOptions, LaunchSpec, PermissionDecision,
    PermissionPolicy, RejectAll, SessionParams, Turn, TurnEvent, TurnOutcome,
    schema::{ContentBlock, RequestPermissionRequest, SessionUpdate, StopReason, TextContent},
};
use futures_util::future::BoxFuture;
use tempfile::TempDir;

fn options() -> ClientOptions {
    ClientOptions {
        client_name: "acp-client-test".into(),
        client_version: "1".into(),
        control_timeout: Duration::from_secs(3),
        stderr_tail_lines: 2,
    }
}
fn fake_spec(dir: &TempDir, scenario: &str) -> Result<LaunchSpec, Box<dyn std::error::Error>> {
    Ok(LaunchSpec {
        program: PathBuf::from(env!("CARGO_BIN_EXE_acp-client-fake-agent")),
        args: vec![scenario.into()],
        env: BTreeMap::from([("ACP_TEST_OVERLAY".into(), "overlay-value".into())]),
        cwd: dir.path().into(),
    })
}
fn params(dir: &TempDir) -> SessionParams {
    SessionParams {
        cwd: dir.path().into(),
        mcp_servers: vec![],
        meta: None,
    }
}
fn text(value: &str) -> Vec<ContentBlock> {
    vec![ContentBlock::Text(TextContent::new(value))]
}
async fn connect(
    dir: &TempDir,
    scenario: &str,
    policy: Arc<dyn PermissionPolicy>,
) -> Result<AgentConnection, Box<dyn std::error::Error>> {
    Ok(AgentConnection::connect(fake_spec(dir, scenario)?, options(), policy).await?)
}
async fn collect(
    turn: &mut Turn,
) -> Result<(Vec<SessionUpdate>, TurnOutcome), Box<dyn std::error::Error>> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut updates = vec![];
        while let Some(event) = turn.next().await {
            match event {
                TurnEvent::Update(update) => updates.push(update),
                TurnEvent::Done(result) => {
                    assert!(turn.next().await.is_none(), "Done must be the last event");
                    return Ok((updates, result?));
                }
            }
        }
        Err("turn ended without Done".into())
    })
    .await?
}
fn visible(updates: &[SessionUpdate]) -> String {
    updates
        .iter()
        .filter_map(|update| match update {
            SessionUpdate::AgentMessageChunk(chunk) => match &chunk.content {
                ContentBlock::Text(text) => Some(text.text.as_str()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}
async fn next_text(turn: &mut Turn, expected: &str) -> Result<(), Box<dyn std::error::Error>> {
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(event) = turn.next().await {
            if let TurnEvent::Update(update) = event
                && visible(&[update]).contains(expected)
            {
                return Ok(());
            }
        }
        Err("missing expected text".into())
    })
    .await?
}

#[tokio::test]
async fn streams_routes_controls_and_swallows_load_replay() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = TempDir::new()?;
    let conn = connect(&dir, "", Arc::new(ApproveAll)).await?;
    assert!(conn.pid().is_some());
    let features = conn.features();
    assert!(
        features.load_session
            && features.resume_session
            && features.close_session
            && features.mcp_http
            && features.mcp_sse
            && features.prompt_image
            && features.prompt_audio
    );
    assert_eq!(
        conn.initialize_response()
            .agent_info
            .as_ref()
            .map(|info| info.name.as_str()),
        Some("fake")
    );
    let first = conn.new_session(params(&dir)).await?;
    let second = conn.new_session(params(&dir)).await?;
    assert!(first.modes().is_some());
    assert_eq!(first.config_options().len(), 1);
    first.set_config_option("model", "fake").await?;
    first.set_mode("code").await?;
    let mut turn = first.prompt(text("normal"))?;
    assert!(matches!(
        first.prompt(text("busy")),
        Err(AcpError::TurnInProgress)
    ));
    assert!(matches!(
        conn.load_session(first.id().clone(), params(&dir)).await,
        Err(AcpError::TurnInProgress)
    ));
    let mut other = second.prompt(text("normal"))?;
    let (updates, result) = collect(&mut turn).await?;
    assert_eq!(visible(&updates), "hello:s1 world");
    assert_eq!(result.stop_reason, StopReason::EndTurn);
    let meta = result.meta.ok_or("missing prompt meta")?;
    assert_eq!(
        meta["controls"],
        serde_json::json!(["session/set_config_option", "session/set_mode"])
    );
    assert_eq!(visible(&collect(&mut other).await?.0), "hello:s2 world");
    let loaded = conn.load_session(first.id().clone(), params(&dir)).await?;
    let mut turn = loaded.prompt(text("normal"))?;
    assert_eq!(visible(&collect(&mut turn).await?.0), "hello:s1 world");
    let resumed = conn
        .resume_session(first.id().clone(), params(&dir))
        .await?;
    let mut turn = resumed.prompt(text("normal"))?;
    assert_eq!(visible(&collect(&mut turn).await?.0), "hello:s1 world");
    resumed.close().await?;
    assert!(matches!(
        first.prompt(text("closed alias")),
        Err(AcpError::Closed)
    ));
    let report = conn.shutdown(Duration::from_secs(1)).await;
    assert!(report.status.is_some_and(|status| status.success()));
    Ok(())
}

#[tokio::test]
async fn cancel_is_idempotent_drains_and_old_cancellers_cannot_cancel_next_turn()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    let conn = connect(&dir, "", Arc::new(ApproveAll)).await?;
    let session = conn.new_session(params(&dir)).await?;
    let mut turn = session.prompt(text("hang"))?;
    let cancel = turn.canceller();
    next_text(&mut turn, "started").await?;
    cancel.cancel();
    cancel.clone().cancel();
    assert!(matches!(
        session.prompt(text("still busy")),
        Err(AcpError::TurnInProgress)
    ));
    let (updates, outcome) = collect(&mut turn).await?;
    assert_eq!(visible(&updates), "after cancel 1after cancel 2");
    assert_eq!(outcome.stop_reason, StopReason::Cancelled);
    let mut next = session.prompt(text("normal"))?;
    cancel.cancel();
    let (_, outcome) = collect(&mut next).await?;
    assert_eq!(outcome.meta.ok_or("missing meta")?["cancels"], 1);
    // Cancel immediately, before the child has read the prompt. Wire ordering
    // must still place session/prompt before session/cancel.
    let mut immediate = session.prompt(text("hang"))?;
    immediate.canceller().cancel();
    assert_eq!(
        collect(&mut immediate).await?.1.stop_reason,
        StopReason::Cancelled
    );
    conn.shutdown(Duration::from_secs(1)).await;
    Ok(())
}

struct PendingPolicy;
impl PermissionPolicy for PendingPolicy {
    fn decide(&self, _: &RequestPermissionRequest) -> BoxFuture<'static, PermissionDecision> {
        Box::pin(std::future::pending())
    }
}

#[tokio::test]
async fn cancelling_pending_permission_does_not_block_dispatch()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    let conn = connect(&dir, "", Arc::new(PendingPolicy)).await?;
    let session = conn.new_session(params(&dir)).await?;
    let mut turn = session.prompt(text("permission"))?;
    next_text(&mut turn, "permission pending").await?;
    turn.canceller().cancel();
    let (updates, _) = collect(&mut turn).await?;
    assert!(visible(&updates).contains("cancelled"));
    conn.shutdown(Duration::from_secs(1)).await;
    Ok(())
}

#[tokio::test]
async fn permission_decisions_choose_only_offered_options() -> Result<(), Box<dyn std::error::Error>>
{
    for (policy, prompt, expected) in [
        (
            Arc::new(ApproveAll) as Arc<dyn PermissionPolicy>,
            "permission",
            "allow",
        ),
        (Arc::new(RejectAll), "permission", "reject"),
        (Arc::new(ApproveAll), "missing-option", "cancelled"),
    ] {
        let dir = TempDir::new()?;
        let conn = connect(&dir, "", policy).await?;
        let session = conn.new_session(params(&dir)).await?;
        let mut turn = session.prompt(text(prompt))?;
        let (updates, _) = collect(&mut turn).await?;
        assert!(matches!(updates.first(), Some(SessionUpdate::ToolCall(_))));
        assert!(visible(&updates).contains(expected));
        conn.shutdown(Duration::from_secs(1)).await;
    }
    Ok(())
}

#[tokio::test]
async fn spawn_environment_and_exit_diagnostics() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    let mut missing = fake_spec(&dir, "")?;
    missing.program = PathBuf::from("/definitely/missing/acp-agent");
    assert!(matches!(
        AgentConnection::connect(missing, options(), Arc::new(ApproveAll)).await,
        Err(AcpError::Spawn { .. })
    ));
    let conn = connect(&dir, "", Arc::new(ApproveAll)).await?;
    let session = conn.new_session(params(&dir)).await?;
    let mut env = session.prompt(text("environment"))?;
    let (updates, _) = collect(&mut env).await?;
    assert!(visible(&updates).contains("overlay-value:"));
    // macOS may canonicalize /var to /private/var in current_dir().
    assert!(visible(&updates).contains(&dir.path().canonicalize()?.display().to_string()));
    let mut crash = session.prompt(text("crash"))?;
    match tokio::time::timeout(Duration::from_secs(3), crash.next()).await? {
        Some(TurnEvent::Done(Err(AcpError::Exited {
            status,
            stderr_tail,
        }))) => {
            assert_eq!(status.and_then(|status| status.code()), Some(17));
            assert_eq!(stderr_tail, ["second diagnostic", "crash detail"]);
        }
        other => return Err(format!("expected Exited, got {other:?}").into()),
    }
    assert_eq!(
        conn.closed().await.stderr_tail,
        ["second diagnostic", "crash detail"]
    );
    Ok(())
}

#[tokio::test]
async fn errors_capabilities_and_control_timeouts() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    for scenario in ["auth", "rpc"] {
        let conn = connect(&dir, scenario, Arc::new(ApproveAll)).await?;
        let result = conn.new_session(params(&dir)).await;
        match (scenario, result) {
            ("auth", Err(AcpError::AuthRequired { methods, message })) => {
                assert_eq!(methods, ["fake-login"]);
                assert_eq!(message, "scripted refusal");
            }
            (
                "rpc",
                Err(AcpError::Rpc {
                    code,
                    message,
                    data,
                }),
            ) => {
                assert_eq!(code, -32602);
                assert_eq!(message, "scripted refusal");
                assert_eq!(data, Some(serde_json::json!({"detail":42})));
            }
            _ => return Err("unexpected error mapping".into()),
        }
        conn.shutdown(Duration::from_secs(1)).await;
    }
    let conn = connect(&dir, "no-caps", Arc::new(ApproveAll)).await?;
    assert!(matches!(
        conn.load_session("x".into(), params(&dir)).await,
        Err(AcpError::Unsupported("session/load"))
    ));
    assert!(matches!(
        conn.resume_session("x".into(), params(&dir)).await,
        Err(AcpError::Unsupported("session/resume"))
    ));
    let session = conn.new_session(params(&dir)).await?;
    session.close().await?;
    conn.shutdown(Duration::from_secs(1)).await;
    assert!(
        matches!(connect(&dir, "v2", Arc::new(ApproveAll)).await, Err(error) if matches!(error.downcast_ref::<AcpError>(), Some(AcpError::ProtocolVersion(_))))
    );
    let mut short = options();
    short.control_timeout = Duration::from_millis(60);
    assert!(matches!(
        AgentConnection::connect(
            fake_spec(&dir, "hang-init")?,
            short.clone(),
            Arc::new(ApproveAll)
        )
        .await,
        Err(AcpError::Timeout {
            method: "initialize",
            ..
        })
    ));
    let conn = AgentConnection::connect(
        fake_spec(&dir, "hang-control")?,
        short,
        Arc::new(ApproveAll),
    )
    .await?;
    let session = conn.new_session(params(&dir)).await?;
    assert!(matches!(
        session.set_mode("code").await,
        Err(AcpError::Timeout {
            method: "session/set_mode",
            ..
        })
    ));
    tokio::time::timeout(Duration::from_secs(3), conn.closed()).await?;
    assert!(matches!(
        session.prompt(text("retired")),
        Err(AcpError::Exited { .. }) | Err(AcpError::Closed)
    ));
    Ok(())
}

#[tokio::test]
async fn shutdown_escalates_and_last_handle_drop_reaps() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    let conn = connect(&dir, "ignore-eof", Arc::new(ApproveAll)).await?;
    let report = tokio::time::timeout(
        Duration::from_secs(3),
        conn.shutdown(Duration::from_millis(50)),
    )
    .await?;
    assert!(report.status.is_some_and(|status| !status.success()));
    let conn = connect(&dir, "", Arc::new(ApproveAll)).await?;
    let pid = conn.pid().ok_or("missing PID")?;
    drop(conn);
    #[cfg(unix)]
    {
        let pid = rustix::process::Pid::from_raw(i32::try_from(pid)?).ok_or("invalid PID")?;
        tokio::time::timeout(Duration::from_secs(3), async {
            while rustix::process::test_kill_process(pid).is_ok() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
    }
    Ok(())
}

#[derive(Clone)]
struct Capture(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| std::io::Error::other("capture poisoned"))?
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn unknown_notifications_are_logged_without_payload_and_turn_survives()
-> Result<(), Box<dyn std::error::Error>> {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let capture = Capture(bytes.clone());
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::WARN)
        .with_writer(move || capture.clone())
        .finish();
    // The SDK can poll transport work on its own executor threads, so a
    // thread-local test subscriber would miss the actual dispatch warning.
    tracing::subscriber::set_global_default(subscriber)?;
    let dir = TempDir::new()?;
    let conn = connect(&dir, "", Arc::new(ApproveAll)).await?;
    let session = conn.new_session(params(&dir)).await?;
    let mut turn = session.prompt(text("normal"))?;
    assert_eq!(visible(&collect(&mut turn).await?.0), "hello:s1 world");
    conn.shutdown(Duration::from_secs(1)).await;
    let log = String::from_utf8(bytes.lock().map_err(|_| "capture poisoned")?.clone())?;
    assert!(
        log.contains("future_unknown_tag"),
        "missing fallback warning: {log}"
    );
    assert!(!log.contains("never log payload"));
    Ok(())
}
