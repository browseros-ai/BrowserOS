//! Exercises the coordinator through its CDP boundary. Chrome's staged status is
//! scripted; the actual guard and scheduling policy run in ExtensionUpdates.
use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::{BrowserSession, BrowserSessionHooks, CdpConnection, SessionId};
use claw_server_rust::services::extension_updates::{ExtensionUpdates, UpdateOutcome};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{Notify, broadcast};

// Poisoning means this scripted boundary itself failed, not a product error.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(error) => panic!("CDP fixture lock poisoned: {error}"),
    }
}

const ID: &str = "pjimfkbpehlcllblajnpfamdfjhhlgkc";
struct Fixture {
    status: Mutex<Value>,
    workers: Mutex<Vec<Value>>,
    calls: Mutex<Vec<String>>,
    events: broadcast::Sender<CdpEvent>,
    hang_evaluation: AtomicBool,
    evaluating: Notify,
    detached: Notify,
}
impl Fixture {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            status: Mutex::new(
                json!({"extensionId":ID,"runningVersion":"0.2.28.0","pendingVersion":"0.2.29.0","detectedAt":1_000,"attemptedVersion":null}),
            ),
            workers: Mutex::new(vec![
                json!({"type":"service_worker","targetId":"ours","url":format!("chrome-extension://{ID}/background.js")}),
            ]),
            calls: Mutex::default(),
            events: broadcast::channel(8).0,
            hang_evaluation: AtomicBool::new(false),
            evaluating: Notify::new(),
            detached: Notify::new(),
        })
    }
    fn browser(self: &Arc<Self>) -> Arc<BrowserSession> {
        BrowserSession::new(self.clone(), BrowserSessionHooks::default())
    }
    fn reloads(&self) -> usize {
        lock(&self.calls)
            .iter()
            .filter(|c| c.as_str() == "reload")
            .count()
    }
}
impl CdpConnection for Fixture {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move {
            lock(&self.calls).push(method.into());
            Ok(match method {
                "Target.getTargets" => {
                    json!({"targetInfos": lock(&self.workers).clone()})
                }
                "Target.attachToTarget" => json!({"sessionId":"attached"}),
                "Target.detachFromTarget" => {
                    self.detached.notify_one();
                    json!({})
                }
                "Runtime.evaluate" => {
                    self.evaluating.notify_one();
                    if self.hang_evaluation.load(Ordering::SeqCst) {
                        std::future::pending::<()>().await;
                    }
                    if params["expression"].as_str().is_some_and(|expression| {
                        expression
                            .starts_with("globalThis.browserosExtensionUpdates.applyPendingUpdate(")
                    }) {
                        lock(&self.calls).push("reload".into());
                        lock(&self.status)["attemptedVersion"] = json!("0.2.29.0");
                        json!({"result":{"value":{"scheduled":true}}})
                    } else {
                        json!({"result":{"value":lock(&self.status).clone()}})
                    }
                }
                _ => panic!("unexpected CDP call {method}"),
            })
        })
    }
    fn send_raw_json<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        _: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async { panic!("unexpected raw CDP") })
    }
    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }
    fn is_connected(&self) -> bool {
        true
    }
    fn connection_epoch(&self) -> u64 {
        1
    }
}

#[tokio::test]
async fn gives_work_a_grace_then_activates_once_and_verifies_the_new_worker() -> Result<(), String>
{
    let fixture = Fixture::new();
    let browser = fixture.browser();
    let service = ExtensionUpdates::default();
    assert_eq!(
        service.check(&browser, false, 10_000).await?,
        UpdateOutcome::Deferred
    );
    assert_eq!(
        service.check(&browser, true, 40_000).await?,
        UpdateOutcome::Deferred
    );
    assert_eq!(
        service.check(&browser, false, 40_000).await?,
        UpdateOutcome::Scheduled("0.2.29.0".into())
    );
    assert_eq!(
        service.check(&browser, false, 41_000).await?,
        UpdateOutcome::AlreadyAttempted
    );
    let restarted = ExtensionUpdates::default();
    assert_eq!(
        restarted.check(&browser, false, 42_000).await?,
        UpdateOutcome::AlreadyAttempted
    );
    assert_eq!(fixture.reloads(), 1);
    {
        let mut status = lock(&fixture.status);
        status["runningVersion"] = json!("0.2.29.0");
        status["pendingVersion"] = Value::Null;
    }
    assert_eq!(
        service.check(&browser, false, 45_000).await?,
        UpdateOutcome::Activated("0.2.29.0".into())
    );
    assert_eq!(
        lock(&fixture.calls)
            .iter()
            .filter(|c| c.as_str() == "Target.detachFromTarget")
            .count(),
        6
    );
    Ok(())
}

#[tokio::test]
async fn a_busy_session_cannot_defer_a_confirmed_update_forever() -> Result<(), String> {
    let fixture = Fixture::new();
    let browser = fixture.browser();
    let service = ExtensionUpdates::default();
    assert_eq!(
        service.check(&browser, true, 841_000).await?,
        UpdateOutcome::Scheduled("0.2.29.0".into())
    );
    Ok(())
}

#[tokio::test]
async fn unrelated_or_ambiguous_workers_are_never_reloaded() -> Result<(), String> {
    let fixture = Fixture::new();
    let browser = fixture.browser();
    let service = ExtensionUpdates::default();
    lock(&fixture.workers).push(json!({"type":"service_worker","targetId":"incognito","url":format!("chrome-extension://{ID}/background.js")}));
    assert_eq!(
        service.check(&browser, false, 841_000).await?,
        UpdateOutcome::Deferred
    );
    lock(&fixture.workers).retain(|t| t["targetId"] == "never");
    lock(&fixture.workers).push(json!({"type":"service_worker","targetId":"foreign","url":"chrome-extension://other/background.js"}));
    assert_eq!(
        service.check(&browser, false, 841_000).await?,
        UpdateOutcome::Deferred
    );
    assert_eq!(fixture.reloads(), 0);
    Ok(())
}

#[tokio::test]
async fn native_status_errors_detach_and_never_reload() -> Result<(), String> {
    let fixture = Fixture::new();
    let browser = fixture.browser();
    let service = ExtensionUpdates::default();
    *lock(&fixture.status) = Value::Null;
    assert!(service.check(&browser, false, 841_000).await.is_err());
    assert_eq!(fixture.reloads(), 0);
    assert_eq!(
        lock(&fixture.calls).last().map(String::as_str),
        Some("Target.detachFromTarget")
    );
    Ok(())
}

#[tokio::test]
async fn cancelled_checks_release_the_worker_debugger_attachment() -> Result<(), String> {
    let fixture = Fixture::new();
    fixture.hang_evaluation.store(true, Ordering::SeqCst);
    let browser = fixture.browser();
    let check = tokio::spawn(async move {
        ExtensionUpdates::default()
            .check(&browser, false, 841_000)
            .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        fixture.evaluating.notified(),
    )
    .await
    .map_err(|error| error.to_string())?;
    check.abort();
    assert!(check.await.is_err_and(|error| error.is_cancelled()));
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        fixture.detached.notified(),
    )
    .await
    .map_err(|error| error.to_string())?;
    assert_eq!(fixture.reloads(), 0);
    Ok(())
}

#[tokio::test]
async fn running_coordinator_reads_sessions_and_recovers_on_notification_and_timer()
-> anyhow::Result<()> {
    use claw_server_rust::{
        AppState,
        config::Config,
        identity::{ClientIdentity, ClientInfo},
        ids::DispatchId,
    };
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    let dir = tempfile::tempdir()?;
    let config = Arc::new(Config {
        server_port: 0,
        cdp_port: 0,
        proxy_port: None,
        resources_dir: dir.path().join("resources"),
        browserclaw_dir: dir.path().join("browserclaw"),
        session_idle: Duration::from_secs(300),
        session_retention: Duration::from_secs(300),
        session_sweep_interval: Duration::from_secs(60),
        replay_retention_days: 7,
        dev_mode: false,
    });
    let state = AppState::new_with_home(config, dir.path().join("home")).await?;
    let fixture = Fixture::new();
    lock(&fixture.status)["detectedAt"] = json!(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64
            - 40_000
    );
    state
        .browser
        .set_session_for_testing(fixture.browser())
        .await;
    let session = state
        .sessions
        .mint(
            ClientIdentity::Ephemeral {
                slug: "test".into(),
                label: "Test".into(),
            },
            ClientInfo {
                name: "Test".into(),
                version: "1".into(),
                title: None,
            },
        )
        .await?;
    let dispatch = DispatchId::new();
    assert!(
        session
            .try_register_dispatch(dispatch.clone(), CancellationToken::new())
            .await
    );

    tokio::time::pause();
    let cancel = CancellationToken::new();
    let task = state.extension_updates.start(
        state.browser.clone(),
        state.sessions.clone(),
        cancel.clone(),
    );
    fixture.detached.notified().await;
    assert_eq!(
        fixture.reloads(),
        0,
        "startup respects a registered dispatch"
    );

    assert!(session.finish_dispatch(&dispatch).await);
    state.extension_updates.notify();
    fixture.detached.notified().await;
    assert_eq!(
        fixture.reloads(),
        1,
        "notification wakes the now-idle coordinator"
    );

    {
        let mut status = lock(&fixture.status);
        status["runningVersion"] = json!("0.2.29.0");
        status["pendingVersion"] = Value::Null;
    }
    tokio::time::advance(Duration::from_secs(30)).await;
    fixture.detached.notified().await;
    assert_eq!(
        fixture.reloads(),
        1,
        "periodic verification does not reload again"
    );
    cancel.cancel();
    task.await?;
    tokio::time::resume();
    state.audit_worker.shutdown().await?;
    Ok(())
}
