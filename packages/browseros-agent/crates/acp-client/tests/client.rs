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

// These are failure caps, not expected runtimes. A runnable peer completes
// immediately; CPU contention must not turn scheduler latency into a test failure.
const WAIT_CAP: Duration = Duration::from_secs(30);
const IO_GRACE: Duration = Duration::from_secs(10);

fn options() -> ClientOptions {
    ClientOptions {
        client_name: "acp-client-test".into(),
        client_version: "1".into(),
        control_timeout: WAIT_CAP,
        stderr_tail_lines: 2,
        shutdown_grace: IO_GRACE,
        response_grace: IO_GRACE,
        stderr_grace: IO_GRACE,
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
    tokio::time::timeout(WAIT_CAP, async {
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
    tokio::time::timeout(WAIT_CAP, async {
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
    let report = conn.shutdown(IO_GRACE).await;
    assert!(report.status.is_some_and(|status| status.success()));
    Ok(())
}

#[tokio::test]
async fn cancel_is_idempotent_drains_and_old_cancellers_cannot_cancel_next_turn()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    let conn = connect(&dir, "pause-cancel", Arc::new(ApproveAll)).await?;
    let session = conn.new_session(params(&dir)).await?;
    let mut turn = session.prompt(text("hang"))?;
    let cancel = turn.canceller();
    next_text(&mut turn, "started").await?;
    cancel.cancel();
    cancel.clone().cancel();
    wait_for_file(&dir.path().join("cancel-received")).await?;
    assert!(matches!(
        session.prompt(text("still busy")),
        Err(AcpError::TurnInProgress)
    ));
    std::fs::write(dir.path().join("release-cancel"), "ready")?;
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
    conn.shutdown(IO_GRACE).await;
    Ok(())
}

struct ChoicePolicy(PermissionDecision);
impl PermissionPolicy for ChoicePolicy {
    fn decide(&self, _: &RequestPermissionRequest) -> BoxFuture<'static, PermissionDecision> {
        let decision = self.0;
        Box::pin(async move { decision })
    }
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
    conn.shutdown(IO_GRACE).await;
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
        (
            Arc::new(ChoicePolicy(PermissionDecision::AllowAlways)),
            "permission",
            "allow-always",
        ),
        (
            Arc::new(ChoicePolicy(PermissionDecision::RejectAlways)),
            "permission",
            "reject-always",
        ),
        (
            Arc::new(ChoicePolicy(PermissionDecision::Cancel)),
            "permission",
            "cancelled",
        ),
        (Arc::new(ApproveAll), "missing-option", "cancelled"),
    ] {
        let dir = TempDir::new()?;
        let conn = connect(&dir, "", policy).await?;
        let session = conn.new_session(params(&dir)).await?;
        let mut turn = session.prompt(text(prompt))?;
        let (updates, _) = collect(&mut turn).await?;
        assert!(matches!(updates.first(), Some(SessionUpdate::ToolCall(_))));
        assert!(visible(&updates).contains(expected));
        conn.shutdown(IO_GRACE).await;
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
    assert!(
        matches!(AgentConnection::connect(fake_spec(&dir, "crash-init")?, options(), Arc::new(ApproveAll)).await,
        Err(AcpError::Exited { status, stderr_tail }) if status.and_then(|status| status.code()) == Some(18) && stderr_tail == ["startup crash"])
    );
    let conn = connect(&dir, "", Arc::new(ApproveAll)).await?;
    let session = conn.new_session(params(&dir)).await?;
    let mut env = session.prompt(text("environment"))?;
    let (updates, _) = collect(&mut env).await?;
    assert!(visible(&updates).contains("overlay-value:"));
    // macOS may canonicalize /var to /private/var in current_dir().
    assert!(visible(&updates).contains(&dir.path().canonicalize()?.display().to_string()));
    let mut crash = session.prompt(text("crash"))?;
    match tokio::time::timeout(WAIT_CAP, crash.next()).await? {
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
    assert!(
        matches!(AgentConnection::connect(fake_spec(&dir, "auth-init")?, options(), Arc::new(ApproveAll)).await,
        Err(AcpError::AuthRequired { methods, message }) if methods.is_empty() && message.contains("OAuth session expired"))
    );
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
        conn.shutdown(IO_GRACE).await;
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
    for server in [
        serde_json::json!({"type":"http", "name":"unsupported", "url":"http://127.0.0.1:9", "headers":[]}),
        serde_json::json!({"type":"sse", "name":"unsupported", "url":"http://127.0.0.1:9", "headers":[]}),
    ] {
        let mut unsupported = params(&dir);
        unsupported
            .mcp_servers
            .push(serde_json::from_value(server)?);
        assert!(matches!(
            conn.new_session(unsupported).await,
            Err(AcpError::Unsupported(_))
        ));
    }
    let session = conn.new_session(params(&dir)).await?;
    session.close().await?;
    conn.shutdown(IO_GRACE).await;
    assert!(
        matches!(connect(&dir, "v2", Arc::new(ApproveAll)).await, Err(error) if matches!(error.downcast_ref::<AcpError>(), Some(AcpError::ProtocolVersion(_))))
    );
    let initialize_received = dir.path().join("initialize-received");
    let mut initializing = Box::pin(AgentConnection::connect(
        fake_spec(&dir, "hang-init")?,
        options(),
        Arc::new(ApproveAll),
    ));
    tokio::select! {
        _ = &mut initializing => return Err("initialize unexpectedly completed before its timeout".into()),
        ready = wait_for_file(&initialize_received) => ready?,
    }
    advance_deadline().await;
    assert!(
        matches!(initializing.await, Err(AcpError::Timeout { method: "initialize", after }) if after == WAIT_CAP)
    );
    let conn = connect(&dir, "hang-control", Arc::new(ApproveAll)).await?;
    let session = conn.new_session(params(&dir)).await?;
    let control_received = dir.path().join("control-received");
    let mut setting = Box::pin(session.set_mode("code"));
    tokio::select! {
        _ = &mut setting => return Err("set_mode unexpectedly completed before its timeout".into()),
        ready = wait_for_file(&control_received) => ready?,
    }
    advance_deadline().await;
    assert!(
        matches!(setting.await, Err(AcpError::Timeout { method: "session/set_mode", after }) if after == WAIT_CAP)
    );
    tokio::time::timeout(WAIT_CAP, conn.closed()).await?;
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
    let report = tokio::time::timeout(WAIT_CAP, conn.shutdown(Duration::ZERO)).await?;
    assert!(report.status.is_some_and(|status| !status.success()));
    let conn = connect(&dir, "", Arc::new(ApproveAll)).await?;
    let pid = conn.pid().ok_or("missing PID")?;
    drop(conn);
    #[cfg(unix)]
    {
        let pid = rustix::process::Pid::from_raw(i32::try_from(pid)?).ok_or("invalid PID")?;
        tokio::time::timeout(WAIT_CAP, async {
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
    conn.shutdown(IO_GRACE).await;
    let log = String::from_utf8(bytes.lock().map_err(|_| "capture poisoned")?.clone())?;
    assert!(
        log.contains("future_unknown_tag"),
        "missing fallback warning: {log}"
    );
    assert!(!log.contains("never log payload"));
    Ok(())
}

#[tokio::test]
async fn dropping_turn_drains_before_releasing_session_and_shutdown_finishes_active_turn()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    let conn = connect(&dir, "", Arc::new(ApproveAll)).await?;
    let session = conn.new_session(params(&dir)).await?;
    let mut abandoned = session.prompt(text("hang"))?;
    next_text(&mut abandoned, "started").await?;
    drop(abandoned);
    let mut next = tokio::time::timeout(WAIT_CAP, async {
        loop {
            match session.prompt(text("normal")) {
                Ok(turn) => return Ok::<_, AcpError>(turn),
                Err(AcpError::TurnInProgress) => tokio::time::sleep(Duration::from_millis(5)).await,
                Err(error) => return Err(error),
            }
        }
    })
    .await??;
    assert_eq!(
        collect(&mut next).await?.1.meta.ok_or("missing meta")?["cancels"],
        1
    );
    let mut active = session.prompt(text("hang"))?;
    next_text(&mut active, "started").await?;
    let waiter = conn.clone();
    conn.shutdown(IO_GRACE).await;
    assert!(matches!(
        active.next().await,
        Some(TurnEvent::Done(Err(
            AcpError::Closed | AcpError::Exited { .. }
        )))
    ));
    assert!(waiter.closed().await.status.is_some());
    Ok(())
}

#[tokio::test]
async fn dropping_control_future_retires_ambiguous_connection()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    let conn = connect(&dir, "hang-control", Arc::new(ApproveAll)).await?;
    let session = conn.new_session(params(&dir)).await?;
    let control_received = dir.path().join("control-received");
    let mut setting = Box::pin(session.set_mode("code"));
    tokio::select! {
        _ = &mut setting => return Err("set_mode unexpectedly completed before cancellation".into()),
        ready = wait_for_file(&control_received) => ready?,
    }
    drop(setting);
    tokio::time::timeout(WAIT_CAP, conn.closed()).await?;
    assert!(matches!(
        session.prompt(text("must not reuse")),
        Err(AcpError::Closed | AcpError::Exited { .. })
    ));
    Ok(())
}

#[tokio::test]
async fn unterminated_stderr_is_bounded_and_preserves_tail()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    let conn = connect(&dir, "", Arc::new(ApproveAll)).await?;
    let session = conn.new_session(params(&dir)).await?;
    let mut turn = session.prompt(text("long-stderr"))?;
    match tokio::time::timeout(WAIT_CAP, turn.next()).await? {
        Some(TurnEvent::Done(Err(AcpError::Exited { stderr_tail, .. }))) => {
            assert_eq!(stderr_tail.len(), 1);
            assert_eq!(stderr_tail[0].len(), 8192);
            assert!(stderr_tail[0].ends_with("TAIL"));
        }
        other => return Err(format!("expected bounded exit report, got {other:?}").into()),
    }
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn shutdown_kills_wrapper_grandchildren() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    let (conn, _session, lifetime) = grandchild_fixture(&dir).await?;
    tokio::time::timeout(WAIT_CAP, conn.shutdown(Duration::ZERO)).await?;
    assert_descendant_exited(lifetime)?;
    Ok(())
}

/// Regression: the group was SIGKILLed the instant the leader exited on EOF,
/// so a descendant still flushing (Claude's CLI writing its transcript) lost
/// its write. Shutdown must let it finish within the grace period.
#[cfg(unix)]
#[tokio::test]
async fn shutdown_lets_descendants_finish_after_the_leader_exits()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    let conn = connect(&dir, "spawn-flusher", Arc::new(ApproveAll)).await?;
    wait_for_file(&dir.path().join("descendant-ready")).await?;
    let report = tokio::time::timeout(WAIT_CAP, conn.shutdown(IO_GRACE)).await?;
    assert!(report.status.is_some_and(|status| status.success()));
    // The report is published only after the whole group is gone.
    assert_eq!(
        std::fs::read_to_string(dir.path().join("flushed"))?,
        "flushed after the leader exited"
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn shutdown_terminates_lingering_descendants_before_killing()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    let conn = connect(&dir, "spawn-term-flusher", Arc::new(ApproveAll)).await?;
    wait_for_file(&dir.path().join("descendant-ready")).await?;
    tokio::time::timeout(WAIT_CAP, conn.shutdown(Duration::from_millis(100))).await?;
    assert_eq!(
        std::fs::read_to_string(dir.path().join("flushed"))?,
        "flushed on SIGTERM"
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn shutdown_kills_descendants_that_ignore_sigterm() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = TempDir::new()?;
    let (spec, listener) = watched_spec(&dir, "spawn-term-immune")?;
    let conn = AgentConnection::connect(spec, options(), Arc::new(ApproveAll)).await?;
    let lifetime = lifetime_socket(listener).await?;
    tokio::time::timeout(WAIT_CAP, conn.shutdown(Duration::ZERO)).await?;
    assert_descendant_exited(lifetime)?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn final_response_is_drained_before_observing_process_exit()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    for _ in 0..8 {
        let conn = connect(&dir, "", Arc::new(ApproveAll)).await?;
        let session = conn.new_session(params(&dir)).await?;
        let mut turn = session.prompt(text("exit-after-result"))?;
        let (updates, result) = collect(&mut turn).await?;
        assert_eq!(visible(&updates), "final output before exit");
        assert_eq!(result.stop_reason, StopReason::EndTurn);
        assert!(
            conn.closed()
                .await
                .status
                .is_some_and(|status| status.success())
        );
    }
    Ok(())
}

#[tokio::test]
async fn policy_also_answers_setup_permissions_and_failed_load_does_not_revive_closed_alias()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    let conn = connect(&dir, "setup-permission", Arc::new(ApproveAll)).await?;
    let session = conn.new_session(params(&dir)).await?;
    assert_eq!(session.id().to_string(), "setup-session");
    conn.shutdown(IO_GRACE).await;

    let conn = connect(&dir, "load-failure", Arc::new(ApproveAll)).await?;
    let session = conn.new_session(params(&dir)).await?;
    session.close().await?;
    assert!(matches!(
        conn.load_session(session.id().clone(), params(&dir)).await,
        Err(AcpError::Rpc { .. })
    ));
    assert!(matches!(
        session.prompt(text("closed")),
        Err(AcpError::Closed)
    ));
    conn.shutdown(IO_GRACE).await;
    Ok(())
}

/// Consume the public stream, including its single terminal event, without
/// requiring success; crash regressions assert on the actual error variant.
async fn terminal(
    turn: &mut Turn,
) -> Result<Result<TurnOutcome, AcpError>, Box<dyn std::error::Error>> {
    tokio::time::timeout(WAIT_CAP, async {
        while let Some(event) = turn.next().await {
            if let TurnEvent::Done(result) = event {
                assert!(turn.next().await.is_none());
                return Ok(result);
            }
        }
        Err("turn ended without Done".into())
    })
    .await?
}

async fn wait_for_file(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    tokio::time::timeout(WAIT_CAP, async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    Ok(())
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_handles_during_ignored_cancel_stops_agent()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    let conn = connect(&dir, "ignore-cancel", Arc::new(ApproveAll)).await?;
    let pid = conn.pid().ok_or("missing pid")?;
    let session = conn.new_session(params(&dir)).await?;
    let mut turn = session.prompt(text("hang"))?;
    next_text(&mut turn, "started").await?;
    turn.canceller().cancel();
    drop(session);
    drop(conn);
    // Keeping the stream and a canceller alive must not lease the connection.
    assert!(matches!(terminal(&mut turn).await?, Err(AcpError::Closed)));
    let pid = rustix::process::Pid::from_raw(i32::try_from(pid)?).ok_or("invalid pid")?;
    tokio::time::timeout(WAIT_CAP, async {
        while rustix::process::test_kill_process(pid).is_ok() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    Ok(())
}

#[cfg(unix)]
struct EscapedChild(rustix::process::Pid);
#[cfg(unix)]
impl Drop for EscapedChild {
    fn drop(&mut self) {
        let _ = rustix::process::kill_process(self.0, rustix::process::Signal::KILL);
    }
}

#[cfg(unix)]
#[tokio::test]
async fn inherited_stdout_cannot_hold_turn_open_after_exit()
-> Result<(), Box<dyn std::error::Error>> {
    for cancel in [false, true] {
        let dir = TempDir::new()?;
        let (spec, listener) = watched_spec(&dir, "")?;
        let mut client_options = options();
        // Only this fixture deliberately withholds a response. Its stderr and
        // startup budgets remain generous while the product drain grace is short.
        client_options.response_grace = Duration::from_millis(50);
        let conn = AgentConnection::connect(spec, client_options, Arc::new(ApproveAll)).await?;
        let session = conn.new_session(params(&dir)).await?;
        let mut turn = session.prompt(text("inherited-stdout"))?;
        let ready = dir.path().join("stdout-holder-ready");
        wait_for_file(&ready).await?;
        let _lifetime = lifetime_socket(listener).await?;
        let pid: i32 = std::fs::read_to_string(&ready)?.parse()?;
        let _cleanup =
            EscapedChild(rustix::process::Pid::from_raw(pid).ok_or("invalid holder pid")?);
        let report = tokio::time::timeout(WAIT_CAP, conn.closed()).await?;
        assert_eq!(report.status.and_then(|status| status.code()), Some(23));
        if cancel {
            turn.canceller().cancel();
        }
        assert!(
            matches!(terminal(&mut turn).await?, Err(AcpError::Exited { status, stderr_tail })
            if status.and_then(|status| status.code()) == Some(23) && stderr_tail == ["inherited stdout crash"])
        );
        assert!(matches!(
            session.set_mode("code").await,
            Err(AcpError::Exited { .. })
        ));
    }
    Ok(())
}

#[tokio::test]
async fn transport_failures_are_closed_or_exited_and_retire_connection()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    let conn = connect(&dir, "crash-control", Arc::new(ApproveAll)).await?;
    let session = conn.new_session(params(&dir)).await?;
    assert!(
        matches!(session.set_mode("code").await, Err(AcpError::Exited { status, stderr_tail })
        if status.and_then(|status| status.code()) == Some(24) && stderr_tail == ["control crash"])
    );
    let conn = connect(&dir, "", Arc::new(ApproveAll)).await?;
    let session = conn.new_session(params(&dir)).await?;
    let mut turn = session.prompt(text("broken-transport"))?;
    assert!(matches!(
        terminal(&mut turn).await?,
        Err(AcpError::Closed | AcpError::Exited { .. })
    ));
    assert!(matches!(
        session.prompt(text("must not reuse")),
        Err(AcpError::Closed | AcpError::Exited { .. })
    ));
    conn.shutdown(Duration::ZERO).await;
    Ok(())
}

#[tokio::test]
async fn config_setter_returns_agent_options_and_refreshes_snapshot()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    let conn = connect(&dir, "returned-config", Arc::new(ApproveAll)).await?;
    let session = conn.new_session(params(&dir)).await?;
    let returned = serde_json::to_value(session.set_config_option("model", "requested").await?)?;
    assert_eq!(returned[0]["currentValue"], "agent-selected");
    assert_eq!(serde_json::to_value(session.config_options())?, returned);
    conn.shutdown(IO_GRACE).await;
    Ok(())
}

#[tokio::test]
async fn idle_control_notifications_refresh_all_session_aliases()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    let conn = connect(&dir, "idle-controls", Arc::new(ApproveAll)).await?;
    let session = conn.new_session(params(&dir)).await?;
    let alias = conn
        .load_session(session.id().clone(), params(&dir))
        .await?;
    let mut turn = session.prompt(text("normal"))?;
    collect(&mut turn).await?;
    session.set_mode("code").await?;
    for current in [&session, &alias] {
        assert_eq!(
            serde_json::to_value(current.modes())?["currentModeId"],
            "code"
        );
        assert_eq!(
            serde_json::to_value(current.config_options())?[0]["currentValue"],
            "agent-selected"
        );
    }
    // Idle control state is retained, but no idle transcript update is replayed.
    let mut turn = alias.prompt(text("normal"))?;
    assert_eq!(visible(&collect(&mut turn).await?.0), "hello:s1 world");
    conn.shutdown(IO_GRACE).await;
    Ok(())
}

#[tokio::test]
async fn permission_fallbacks_match_acpx_preferences() -> Result<(), Box<dyn std::error::Error>> {
    for (decision, prompt, expected) in [
        (
            PermissionDecision::AllowOnce,
            "always-options",
            Some("allow-always"),
        ),
        (PermissionDecision::AllowOnce, "reject-only", Some("reject")),
        (
            PermissionDecision::RejectOnce,
            "always-options",
            Some("reject-always"),
        ),
        (PermissionDecision::RejectOnce, "allow-only", None),
        (
            PermissionDecision::AllowOnce,
            "reversed-options",
            Some("allow"),
        ),
        (
            PermissionDecision::RejectOnce,
            "reversed-options",
            Some("reject"),
        ),
        (PermissionDecision::AllowAlways, "allow-only", Some("allow")),
        (
            PermissionDecision::RejectAlways,
            "reject-only",
            Some("reject"),
        ),
        (PermissionDecision::Cancel, "always-options", None),
    ] {
        let dir = TempDir::new()?;
        let conn = connect(&dir, "", Arc::new(ChoicePolicy(decision))).await?;
        let session = conn.new_session(params(&dir)).await?;
        let mut turn = session.prompt(text(prompt))?;
        let (updates, _) = collect(&mut turn).await?;
        let response = visible(&updates);
        let outcome: serde_json::Value = serde_json::from_str(
            response
                .strip_prefix("permission pendingpermission:")
                .ok_or("missing permission response")?,
        )?;
        match expected {
            Some(option) => {
                assert_eq!(outcome["outcome"], "selected");
                assert_eq!(outcome["optionId"], option);
            }
            None => assert_eq!(outcome["outcome"], "cancelled"),
        }
        conn.shutdown(IO_GRACE).await;
    }
    Ok(())
}

#[tokio::test]
async fn auth_words_do_not_override_rpc_codes_or_mcp_failures()
-> Result<(), Box<dyn std::error::Error>> {
    for (scenario, expected) in [
        ("auth-invalid-params", -32602),
        ("auth-mcp", -32603),
        ("rpc-local-looking", -32603),
    ] {
        let dir = TempDir::new()?;
        let conn = connect(&dir, scenario, Arc::new(ApproveAll)).await?;
        assert!(
            matches!(conn.new_session(params(&dir)).await, Err(AcpError::Rpc { code, data: Some(_), .. }) if code == expected),
            "{scenario}"
        );
        conn.shutdown(IO_GRACE).await;
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn dropping_runtime_kills_supervised_process_group() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    // Retain public handles outside the runtime so only the dropped supervisor
    // can kill the group. Socket EOF observes that exit without a scheduler guess.
    let (_conn, _session, lifetime) = runtime.block_on(grandchild_fixture(&dir))?;
    drop(runtime);
    assert_descendant_exited(lifetime)?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_response_cannot_overwrite_later_notification()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    let conn = connect(&dir, "post-config-update", Arc::new(ApproveAll)).await?;
    let session = conn.new_session(params(&dir)).await?;
    for _ in 0..8 {
        let returned = session.set_config_option("model", "requested").await?;
        assert_eq!(serde_json::to_value(returned)?[0]["currentValue"], "fake");
        // A later reply establishes that the batched notification was dispatched.
        session.set_mode("code").await?;
        assert_eq!(
            serde_json::to_value(session.config_options())?[0]["currentValue"],
            "newer-notification"
        );
    }
    conn.shutdown(IO_GRACE).await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn genuine_rpc_error_survives_concurrent_child_exit() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = TempDir::new()?;
    let conn = connect(&dir, "", Arc::new(ApproveAll)).await?;
    let session = conn.new_session(params(&dir)).await?;
    let mut turn = session.prompt(text("rpc-error-then-exit"))?;
    assert!(
        matches!(terminal(&mut turn).await?, Err(AcpError::Rpc { code: -32602, message, data })
        if message == "scripted invalid prompt" && data == Some(serde_json::json!({"detail":42})))
    );
    assert_eq!(
        conn.closed().await.status.and_then(|status| status.code()),
        Some(25)
    );
    Ok(())
}

#[tokio::test]
async fn notifications_immediately_after_attachment_refresh_snapshots()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    let conn = connect(&dir, "immediate-controls", Arc::new(ApproveAll)).await?;
    let mut sessions = vec![conn.new_session(params(&dir)).await?];
    for stage in 0..3 {
        if stage > 0 {
            let id = sessions[0].id().clone();
            sessions.push(if stage == 1 {
                conn.load_session(id, params(&dir)).await?
            } else {
                conn.resume_session(id, params(&dir)).await?
            });
        }
        // A later control reply provides a barrier after each attachment batch.
        sessions[stage].set_mode("code").await?;
        for session in &sessions {
            assert_eq!(
                serde_json::to_value(session.modes())?["currentModeId"],
                "code"
            );
            assert_eq!(
                serde_json::to_value(session.config_options())?[0]["currentValue"],
                "after-attachment"
            );
        }
    }
    conn.shutdown(IO_GRACE).await;
    Ok(())
}

/// Pause only after real I/O acknowledged receipt. Resuming before awaiting the
/// timeout keeps subsequent process cleanup on wall time instead of auto-advancing it.
async fn advance_deadline() {
    tokio::time::pause();
    tokio::time::advance(WAIT_CAP).await;
    tokio::time::resume();
}

#[cfg(unix)]
fn watched_spec(
    dir: &TempDir,
    scenario: &str,
) -> Result<(LaunchSpec, tokio::net::UnixListener), Box<dyn std::error::Error>> {
    let path = dir.path().join("life.sock");
    let listener = tokio::net::UnixListener::bind(&path)?;
    let mut spec = fake_spec(dir, scenario)?;
    spec.env.insert(
        "ACP_TEST_LIFETIME_SOCKET".into(),
        path.to_string_lossy().into_owned(),
    );
    Ok((spec, listener))
}

#[cfg(unix)]
async fn lifetime_socket(
    listener: tokio::net::UnixListener,
) -> Result<std::os::unix::net::UnixStream, Box<dyn std::error::Error>> {
    use tokio::io::AsyncReadExt;
    let (mut socket, _) = tokio::time::timeout(WAIT_CAP, listener.accept()).await??;
    let mut ready = [0];
    tokio::time::timeout(WAIT_CAP, socket.read_exact(&mut ready)).await??;
    assert_eq!(&ready, b"r");
    // A standard socket survives dropping the test runtime, unlike a Tokio
    // registration owned by its reactor. Its read timeout is only a failure cap.
    let socket = socket.into_std()?;
    socket.set_nonblocking(false)?;
    socket.set_read_timeout(Some(WAIT_CAP))?;
    Ok(socket)
}

#[cfg(unix)]
async fn grandchild_fixture(
    dir: &TempDir,
) -> Result<
    (
        AgentConnection,
        acp_client::Session,
        std::os::unix::net::UnixStream,
    ),
    Box<dyn std::error::Error>,
> {
    let (spec, listener) = watched_spec(dir, "ignore-eof")?;
    let conn = AgentConnection::connect(spec, options(), Arc::new(ApproveAll)).await?;
    let session = conn.new_session(params(dir)).await?;
    let mut turn = session.prompt(text("grandchild"))?;
    collect(&mut turn).await?;
    let lifetime = lifetime_socket(listener).await?;
    Ok((conn, session, lifetime))
}

#[cfg(unix)]
fn assert_descendant_exited(
    mut lifetime: std::os::unix::net::UnixStream,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Read;
    assert_eq!(
        lifetime.read(&mut [0])?,
        0,
        "descendant survived group cleanup"
    );
    Ok(())
}
