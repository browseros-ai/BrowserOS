//! Exercises host operation ordering through central dispatch and actual QuickJS scripts.
//! These in-memory transports retain the real BrowserView/handler/response path; notifications
//! pause the async handoffs so cancellation and guard lifetime do not depend on sleeps.

use super::{check_fixed_fixture_epoch, fake_ctx};
use crate::{
    framework::{
        InnerCallHook, InnerCallRecord, ToolCtx, ToolDef, ToolError, ToolExecResult, ToolResult,
        execute_tool,
    },
    response::ToolResponse,
    tools,
};
use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::{BrowserSession, BrowserSessionHooks, BrowserView, CdpConnection, SessionId};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex as AsyncMutex, Notify, OwnedMutexGuard, broadcast};

/// One isolated conversation's lock and observable async phases. The hook and transport
/// share this state so assertions cover the handler's native write and later readback.
#[derive(Default)]
struct GuardState {
    gate: Arc<AsyncMutex<()>>,
    attempted: Notify,
    handler_started: Notify,
    release_handler: Notify,
    readback_started: Notify,
    release_readback: Notify,
    reject: AtomicBool,
    fail_handler: AtomicBool,
    pause_handler: AtomicBool,
    pause_readback: AtomicBool,
    active: AtomicBool,
    writes: AtomicUsize,
    abandoned: AtomicUsize,
    released: AtomicUsize,
    acquisitions: Mutex<Vec<(String, Value)>>,
    calls: Mutex<Vec<(String, bool)>>,
    audits: Mutex<Vec<(String, bool)>>,
}

struct QueuedAcquisition {
    state: Arc<GuardState>,
    acquired: bool,
}

impl Drop for QueuedAcquisition {
    fn drop(&mut self) {
        if !self.acquired {
            self.state.abandoned.fetch_add(1, Ordering::SeqCst);
        }
    }
}

struct HostGuard {
    _lock: OwnedMutexGuard<()>,
    state: Arc<GuardState>,
}

impl Drop for HostGuard {
    fn drop(&mut self) {
        self.state.active.store(false, Ordering::SeqCst);
        self.state.released.fetch_add(1, Ordering::SeqCst);
    }
}

struct GuardHook(Arc<GuardState>);

impl InnerCallHook for GuardHook {
    fn authorize<'a>(&'a self, _page: Option<u32>) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    fn record<'a>(&'a self, record: InnerCallRecord<'a>) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.0
                .audits
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((
                    record.method.to_owned(),
                    self.0.active.load(Ordering::SeqCst),
                ));
        })
    }

    fn on_page_created<'a>(&'a self, _page: u32) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    fn acquire_tool_guard<'a>(
        &'a self,
        tool_name: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Option<Box<dyn Send>>, String>> {
        Box::pin(async move {
            self.0
                .acquisitions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((tool_name.to_owned(), args.clone()));
            // Match a host's operation-specific policy: the outer run and unrelated
            // primitives keep progressing while this conversation's group lock is held.
            if tool_name != "guard_probe" && tool_name != "tab_groups" {
                return Ok(None);
            }
            let mut queued = QueuedAcquisition {
                state: self.0.clone(),
                acquired: false,
            };
            self.0.attempted.notify_one();
            if self.0.reject.load(Ordering::SeqCst) {
                return Err("host rejected the group operation".to_owned());
            }
            let lock = self.0.gate.clone().lock_owned().await;
            queued.acquired = true;
            self.0.active.store(true, Ordering::SeqCst);
            Ok(Some(Box::new(HostGuard {
                _lock: lock,
                state: self.0.clone(),
            }) as Box<dyn Send>))
        })
    }
}

/// Immutable-epoch native-shaped transport: publication records whether the host guard
/// is held, and response gates let the real handler/response futures remain in flight.
struct GuardConnection {
    state: Arc<GuardState>,
    events: broadcast::Sender<CdpEvent>,
}

impl CdpConnection for GuardConnection {
    fn send<'a>(
        &'a self,
        method: &'a str,
        _params: Value,
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move {
            self.state
                .calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((method.to_owned(), self.state.active.load(Ordering::SeqCst)));
            match method {
                "Browser.closeTabGroup" => {
                    self.state.writes.fetch_add(1, Ordering::SeqCst);
                    self.state.handler_started.notify_one();
                    if self.state.pause_handler.load(Ordering::SeqCst) {
                        self.state.release_handler.notified().await;
                    }
                    if self.state.fail_handler.load(Ordering::SeqCst) {
                        return Err(CdpError::Protocol {
                            code: -1,
                            message: "forced handler failure".to_owned(),
                        });
                    }
                    Ok(json!({}))
                }
                "Browser.getTabs" => {
                    self.state.readback_started.notify_one();
                    if self.state.pause_readback.load(Ordering::SeqCst) {
                        self.state.release_readback.notified().await;
                    }
                    Ok(json!({ "tabs": [] }))
                }
                _ => Err(CdpError::Protocol {
                    code: -1,
                    message: format!("unexpected guard fixture method: {method}"),
                }),
            }
        })
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            Ok(self
                .send(method, serde_json::from_str(params)?, session)
                .await?
                .to_string())
        })
    }

    fn send_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move {
            check_fixed_fixture_epoch(expected_epoch, self.connection_epoch())?;
            self.send(method, params, session).await
        })
    }

    fn send_raw_json_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            check_fixed_fixture_epoch(expected_epoch, self.connection_epoch())?;
            self.send_raw_json(method, params, session).await
        })
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

fn guarded_ctx(state: Arc<GuardState>) -> ToolCtx {
    let (events, _) = broadcast::channel(8);
    let session = BrowserSession::new(
        Arc::new(GuardConnection {
            state: state.clone(),
            events,
        }),
        BrowserSessionHooks::default(),
    );
    let mut ctx = fake_ctx();
    ctx.browser = BrowserView::unbound(session, None);
    ctx.inner_call_hook = Some(Arc::new(GuardHook(state)));
    ctx
}

fn probe_handler<'a>(
    args: Value,
    ctx: &'a ToolCtx,
    response: &'a mut ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        ctx.browser
            .cdp(
                "Browser.closeTabGroup",
                json!({ "groupId": "probe-group" }),
                None,
            )
            .await?;
        response.text("primary response");
        if args.get("readback").and_then(Value::as_bool) == Some(true) {
            response.include_pages();
        }
        Ok(None)
    })
}

fn probe_def() -> ToolDef {
    let mut def = tools::tab_groups::definition();
    def.name = "guard_probe";
    def.handler = probe_handler;
    def
}

async fn reach_phase<F: Future>(call: Pin<&mut F>, notification: &Notify) -> anyhow::Result<()> {
    tokio::select! {
        () = notification.notified() => Ok(()),
        _ = call => anyhow::bail!("call completed before reaching the guarded phase"),
        () = tokio::time::sleep(Duration::from_secs(2)) => anyhow::bail!("guarded phase was not reached"),
    }
}

fn assert_dropped_queue(state: &GuardState) {
    assert_eq!(
        state.abandoned.load(Ordering::SeqCst),
        1,
        "queued acquisition must be dropped"
    );
    assert_eq!(
        state.writes.load(Ordering::SeqCst),
        0,
        "queued operation must never publish a native write"
    );
    assert!(!state.active.load(Ordering::SeqCst));
    assert!(
        state.gate.try_lock().is_ok(),
        "dropped acquisition must not retain its queue position"
    );
}

#[tokio::test]
async fn tool_guard_hook_rejection_has_zero_handler_writes() -> anyhow::Result<()> {
    let state = Arc::new(GuardState::default());
    state.reject.store(true, Ordering::SeqCst);
    let ctx = guarded_ctx(state.clone());
    let result = execute_tool(&probe_def(), json!({ "sentinel": 17 }), &ctx).await;
    assert!(
        matches!(result, Err(ToolError::Message(reason)) if reason == "host rejected the group operation")
    );
    assert_eq!(state.writes.load(Ordering::SeqCst), 0);
    assert_eq!(
        *state
            .acquisitions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![("guard_probe".to_owned(), json!({ "sentinel": 17 }))]
    );
    Ok(())
}

#[tokio::test]
async fn tool_guard_queued_cancellation_has_zero_handler_writes() -> anyhow::Result<()> {
    let state = Arc::new(GuardState::default());
    let owner = state.gate.clone().lock_owned().await;
    let ctx = guarded_ctx(state.clone());
    let def = probe_def();
    let call = execute_tool(&def, json!({}), &ctx);
    tokio::pin!(call);
    reach_phase(call.as_mut(), &state.attempted).await?;
    ctx.cancel.cancel();
    let result = tokio::time::timeout(Duration::from_secs(1), call).await?;
    assert!(matches!(result, Err(ToolError::Cancelled)));
    drop(owner);
    assert_dropped_queue(&state);
    Ok(())
}

#[tokio::test]
async fn tool_guard_is_retained_through_response_readback() -> anyhow::Result<()> {
    let state = Arc::new(GuardState::default());
    state.pause_readback.store(true, Ordering::SeqCst);
    let ctx = guarded_ctx(state.clone());
    let def = probe_def();
    let call = execute_tool(&def, json!({ "readback": true }), &ctx);
    tokio::pin!(call);
    reach_phase(call.as_mut(), &state.readback_started).await?;
    assert!(state.active.load(Ordering::SeqCst));
    assert_eq!(state.released.load(Ordering::SeqCst), 0);
    assert!(state.gate.try_lock().is_err());
    state.release_readback.notify_one();
    let result = call.await?;
    assert!(!result.is_error);
    assert_eq!(
        *state
            .calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![
            ("Browser.closeTabGroup".to_owned(), true),
            ("Browser.getTabs".to_owned(), true)
        ]
    );
    assert_eq!(state.released.load(Ordering::SeqCst), 1);
    assert!(state.gate.try_lock().is_ok());
    Ok(())
}

#[tokio::test]
async fn tool_guard_handler_error_releases_the_guard() -> anyhow::Result<()> {
    let state = Arc::new(GuardState::default());
    state.fail_handler.store(true, Ordering::SeqCst);
    let ctx = guarded_ctx(state.clone());
    let result = execute_tool(&probe_def(), json!({}), &ctx).await?;
    assert!(result.is_error);
    assert_eq!(state.writes.load(Ordering::SeqCst), 1);
    assert_eq!(state.released.load(Ordering::SeqCst), 1);
    assert!(state.gate.try_lock().is_ok());
    Ok(())
}

#[tokio::test]
async fn tool_guard_response_cancellation_releases_the_guard() -> anyhow::Result<()> {
    let state = Arc::new(GuardState::default());
    state.pause_readback.store(true, Ordering::SeqCst);
    let ctx = guarded_ctx(state.clone());
    let def = probe_def();
    let call = execute_tool(&def, json!({ "readback": true }), &ctx);
    tokio::pin!(call);
    reach_phase(call.as_mut(), &state.readback_started).await?;
    assert!(state.active.load(Ordering::SeqCst));
    ctx.cancel.cancel();
    let result = tokio::time::timeout(Duration::from_secs(1), call).await?;
    assert!(matches!(result, Err(ToolError::Cancelled)));
    assert_eq!(state.released.load(Ordering::SeqCst), 1);
    assert!(state.gate.try_lock().is_ok());
    Ok(())
}

const GROUP_SCRIPT: &str = "await browser.pages.list(); console.log('before-group'); return await browser.tabGroups({ action: 'close', groupId: 'script-group' });";

#[tokio::test]
async fn tool_guard_actual_run_uses_plain_name_and_releases_before_audit() -> anyhow::Result<()> {
    let state = Arc::new(GuardState::default());
    let owner = state.gate.clone().lock_owned().await;
    let ctx = guarded_ctx(state.clone());
    let def = tools::run::definition();
    let call = execute_tool(&def, json!({ "code": GROUP_SCRIPT }), &ctx);
    tokio::pin!(call);
    reach_phase(call.as_mut(), &state.attempted).await?;
    assert_eq!(state.writes.load(Ordering::SeqCst), 0);
    assert_eq!(
        *state
            .calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![("Browser.getTabs".to_owned(), false)]
    );
    drop(owner);
    let result = call.await?;
    assert!(!result.is_error);
    let data = result
        .structured_content
        .ok_or_else(|| anyhow::anyhow!("missing run result"))?;
    assert_eq!(
        data,
        json!({ "ok": true, "value": { "groupId": "script-group" }, "logs": ["before-group"] })
    );
    assert_eq!(
        *state
            .acquisitions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![
            ("run".to_owned(), json!({ "code": GROUP_SCRIPT })),
            (
                "tab_groups".to_owned(),
                json!({ "action": "close", "groupId": "script-group" })
            )
        ]
    );
    assert_eq!(
        *state
            .audits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![
            ("pages.list".to_owned(), false),
            ("tool:tab_groups".to_owned(), false)
        ]
    );
    assert_eq!(state.writes.load(Ordering::SeqCst), 1);
    assert_eq!(state.released.load(Ordering::SeqCst), 1);
    assert!(state.gate.try_lock().is_ok());
    Ok(())
}

#[tokio::test]
async fn tool_guard_actual_run_queued_cancellation_has_zero_handler_writes() -> anyhow::Result<()> {
    let state = Arc::new(GuardState::default());
    let owner = state.gate.clone().lock_owned().await;
    let ctx = guarded_ctx(state.clone());
    let def = tools::run::definition();
    let call = execute_tool(&def, json!({ "code": GROUP_SCRIPT }), &ctx);
    tokio::pin!(call);
    reach_phase(call.as_mut(), &state.attempted).await?;
    ctx.cancel.cancel();
    let result = tokio::time::timeout(Duration::from_secs(1), call).await?;
    assert!(matches!(result, Err(ToolError::Cancelled)));
    drop(owner);
    assert_dropped_queue(&state);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn tool_guard_actual_run_queued_timeout_has_zero_handler_writes() -> anyhow::Result<()> {
    let state = Arc::new(GuardState::default());
    let owner = state.gate.clone().lock_owned().await;
    let ctx = guarded_ctx(state.clone());
    let def = tools::run::definition();
    let call = execute_tool(&def, json!({ "code": GROUP_SCRIPT, "timeout": 1000 }), &ctx);
    tokio::pin!(call);
    reach_phase(call.as_mut(), &state.attempted).await?;
    tokio::time::advance(Duration::from_millis(1000)).await;
    let result = call.await?;
    assert!(result.is_error);
    let data = result
        .structured_content
        .ok_or_else(|| anyhow::anyhow!("missing timeout result"))?;
    assert_eq!(
        data,
        json!({ "ok": false, "error": "run exceeded 1000ms", "logs": ["before-group"] })
    );
    drop(owner);
    assert_dropped_queue(&state);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn tool_guard_actual_run_timeout_releases_acquired_guard() -> anyhow::Result<()> {
    let state = Arc::new(GuardState::default());
    state.pause_handler.store(true, Ordering::SeqCst);
    let ctx = guarded_ctx(state.clone());
    let def = tools::run::definition();
    let call = execute_tool(&def, json!({ "code": GROUP_SCRIPT, "timeout": 1000 }), &ctx);
    tokio::pin!(call);
    reach_phase(call.as_mut(), &state.handler_started).await?;
    assert!(state.active.load(Ordering::SeqCst));
    assert_eq!(state.writes.load(Ordering::SeqCst), 1);
    tokio::time::advance(Duration::from_millis(1000)).await;
    let result = call.await?;
    assert!(result.is_error);
    let data = result
        .structured_content
        .ok_or_else(|| anyhow::anyhow!("missing timeout result"))?;
    assert_eq!(data["error"], json!("run exceeded 1000ms"));
    assert_eq!(state.released.load(Ordering::SeqCst), 1);
    assert!(!state.active.load(Ordering::SeqCst));
    assert!(state.gate.try_lock().is_ok());
    Ok(())
}
