//! A tab whose page session Chromium dropped without Neo hearing about it, through real MCP
//! dispatch and a real browser. On a busy browser the `Target.detachedFromTarget` event can be
//! lost, and Neo then kept sending to the dead session: every snapshot, `goto`, act and read on
//! that tab failed with "no session with given id" until the tab was closed. The test drops
//! the session for real and hides only that one detach event from Neo.
//!
//! The contract: snapshot, `goto` and act recover inside the call that meets the dead session,
//! a read may fail once with that session's own error, and every later call runs on a new
//! attachment of the page.
//!
//! Needs a browser with CDP on `BROWSEROS_CDP_PORT` and the sdk-privacy two-origin fixture
//! (`bun serve.ts`: 127.0.0.1:8781 embeds a cross-site OOPIF from localhost:8782):
//! `cargo test -p browseros-mcp --test oopif_snapshot_live -- --ignored`.
use browseros_cdp::{CdpClient, CdpError, CdpEvent, ConnectOptions, SessionId};
use browseros_core::{
    BrowserSession, BrowserSessionHooks, BrowserView, CdpConnection, NativeCapabilities, PageId,
};
use browseros_mcp::{
    BrowserToolDefaults, BrowserToolOptions, StaticTargetStore, TargetStore, ToolCtx, ToolResult,
    catalog, execute_tool, output_file::create_browser_output_file_access,
};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

const FIXTURE: &str = "http://127.0.0.1:8781/";

/// The real connection, except that it swallows `Target.detachedFromTarget` for the sessions
/// in `hidden`, standing in for an event a lagged or racing listener never applied.
struct HidesDetach {
    inner: CdpClient,
    hidden: Arc<Mutex<Vec<String>>>,
    /// When set, the next script whose source contains this text finds its session dropped
    /// just before it runs, after the isolated-world setup on that session already answered.
    drop_before_script: Arc<Mutex<Option<&'static str>>>,
}

impl HidesDetach {
    /// Sends `method`, first dropping its session unannounced when it is the armed script.
    async fn send_script_aware(
        &self,
        method: &str,
        params: Value,
        session: Option<&SessionId>,
        send: BoxFuture<'_, Result<Value, CdpError>>,
    ) -> Result<Value, CdpError> {
        let source = params
            .get("expression")
            .or_else(|| params.get("functionDeclaration"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let armed = matches!(method, "Runtime.evaluate" | "Runtime.callFunctionOn")
            && session.is_some()
            && self
                .drop_before_script
                .lock()
                .is_ok_and(|mut armed| armed.take_if(|needle| source.contains(*needle)).is_some());
        if armed && let Some(session) = session {
            if let Ok(mut hidden) = self.hidden.lock() {
                hidden.push(session.0.clone());
            }
            self.inner
                .send(
                    "Target.detachFromTarget",
                    json!({"sessionId": session.0}),
                    None,
                )
                .await?;
        }
        send.await
    }
}

impl CdpConnection for HidesDetach {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        let send = CdpConnection::send(&self.inner, method, params.clone(), session);
        Box::pin(self.send_script_aware(method, params, session, send))
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        CdpConnection::send_raw_json(&self.inner, method, params_json, session)
    }

    fn send_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        let send = CdpConnection::send_at_epoch(
            &self.inner,
            expected_epoch,
            method,
            params.clone(),
            session,
        );
        Box::pin(self.send_script_aware(method, params, session, send))
    }

    fn send_raw_json_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        CdpConnection::send_raw_json_at_epoch(
            &self.inner,
            expected_epoch,
            method,
            params_json,
            session,
        )
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        let mut source = CdpConnection::events(&self.inner);
        let (tx, rx) = broadcast::channel(4096);
        let hidden = self.hidden.clone();
        tokio::spawn(async move {
            while let Ok(event) = source.recv().await {
                let dropped = event.method == "Target.detachedFromTarget"
                    && event
                        .params
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .is_some_and(|id| {
                            hidden.lock().is_ok_and(|ids| ids.iter().any(|h| h == id))
                        });
                if !dropped && tx.send(event).is_err() {
                    break;
                }
            }
        });
        rx
    }

    fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        CdpConnection::events_targeted(&self.inner, method)
    }

    fn is_connected(&self) -> bool {
        CdpConnection::is_connected(&self.inner)
    }

    fn connection_epoch(&self) -> u64 {
        CdpConnection::connection_epoch(&self.inner)
    }
}

async fn context(session: Arc<BrowserSession>) -> anyhow::Result<ToolCtx> {
    let target = Arc::new(StaticTargetStore::new());
    let binding = target.get().await.map_err(anyhow::Error::msg)?;
    Ok(ToolCtx::new(BrowserToolOptions {
        browser: BrowserView::capture(session, binding, None).await,
        target: Some(target),
        defaults: BrowserToolDefaults::default(),
        cancel: CancellationToken::new(),
        output_files: create_browser_output_file_access(),
        inner_call_hook: None,
        preloaded_helpers: Vec::new(),
    }))
}

async fn call(ctx: &ToolCtx, name: &str, args: Value) -> anyhow::Result<ToolResult> {
    let tool = catalog()
        .into_iter()
        .find(|tool| tool.name == name)
        .ok_or_else(|| anyhow::anyhow!("missing tool {name}"))?;
    Ok(execute_tool(&tool, args, ctx).await?)
}

fn text(result: &ToolResult) -> String {
    serde_json::to_string(&result.content).unwrap_or_default()
}

/// Fails with the tool's own error text, which is what an agent would have seen.
fn ok(label: &str, result: &ToolResult) -> anyhow::Result<String> {
    let body = text(result);
    anyhow::ensure!(!result.is_error, "{label} failed: {body}");
    Ok(body)
}

#[tokio::test]
#[ignore = "needs a browser with CDP on BROWSEROS_CDP_PORT and the sdk-privacy fixture"]
async fn tests_that_a_tab_recovers_after_its_page_session_dies_unannounced() -> anyhow::Result<()> {
    let port: u16 = std::env::var("BROWSEROS_CDP_PORT")
        .ok()
        .and_then(|port| port.parse().ok())
        .unwrap_or(9005);
    // As the claw server runs: scripts go to an isolated world, two commands each.
    browseros_core::page_js::set_isolated_world_enabled(true);
    let client = CdpClient::connect(ConnectOptions::new(port)).await?;
    let hidden = Arc::new(Mutex::new(Vec::new()));
    let drop_before_script = Arc::new(Mutex::new(None));
    let session = BrowserSession::new_with_native(
        Arc::new(HidesDetach {
            inner: client.clone(),
            hidden: hidden.clone(),
            drop_before_script: drop_before_script.clone(),
        }),
        BrowserSessionHooks::default(),
        NativeCapabilities::from_browseros_version(Some("0.51.2.7")),
    );
    let ctx = context(session.clone()).await?;
    let opened = call(
        &ctx,
        "run",
        json!({"code": format!("const p = await browser.open('{FIXTURE}'); return p.id;")}),
    )
    .await?;
    ok("open", &opened)?;
    let page = opened
        .structured_content
        .as_ref()
        .and_then(|value| value["value"].as_u64())
        .and_then(|id| u32::try_from(id).ok())
        .ok_or_else(|| anyhow::anyhow!("open returned no page id"))?;
    let result = exercise(&ctx, &session, &client, &hidden, page).await;
    let result = match result {
        Ok(()) => checks_survive_a_drop_mid_script(&ctx, &session, &drop_before_script, page).await,
        failed => failed,
    };
    let _ = call(&ctx, "tabs", json!({"action": "close", "page": page})).await;
    result
}

async fn exercise(
    ctx: &ToolCtx,
    session: &BrowserSession,
    client: &CdpClient,
    hidden: &Mutex<Vec<String>>,
    page: u32,
) -> anyhow::Result<()> {
    let first = complete_snapshot(ctx, "first snapshot", page).await?;
    anyhow::ensure!(
        first.contains("Cross-site password"),
        "the cross-site iframe was not read: {first}"
    );

    let dropped = drop_page_session_unannounced(session, client, hidden, page).await?;
    let snapshot = complete_snapshot(ctx, "snapshot after the page session died", page).await?;
    anyhow::ensure!(
        snapshot.contains("Cross-site password"),
        "the recovered snapshot lost the cross-site iframe: {snapshot}"
    );
    ensure_replaced(session, page, &dropped).await?;

    let dropped = drop_page_session_unannounced(session, client, hidden, page).await?;
    let goto = call(
        ctx,
        "run",
        json!({"code": format!("await browser.page({page}).goto('{FIXTURE}plain'); return (await browser.page({page}).info()).url;")}),
    )
    .await?;
    anyhow::ensure!(
        ok("goto after the page session died", &goto)?.contains("/plain"),
        "goto did not navigate: {}",
        text(&goto)
    );
    ensure_replaced(session, page, &dropped).await?;

    let dropped = drop_page_session_unannounced(session, client, hidden, page).await?;
    let read = call(ctx, "read", json!({"page": page})).await?;
    anyhow::ensure!(
        !read.is_error || text(&read).contains("no session with given id"),
        "read after the page session died failed with something else: {}",
        text(&read)
    );
    ok(
        "read after the one that met the dead session",
        &call(ctx, "read", json!({"page": page})).await?,
    )?;
    ensure_replaced(session, page, &dropped).await?;

    // `/plain` serves the same forms without the cross-site iframe.
    let dropped = drop_page_session_unannounced(session, client, hidden, page).await?;
    let snapshot = complete_snapshot(ctx, "snapshot before act", page).await?;
    anyhow::ensure!(
        snapshot.contains("[ref=e1]"),
        "no ref to act on: {snapshot}"
    );
    ensure_replaced(session, page, &dropped).await?;

    let dropped = drop_page_session_unannounced(session, client, hidden, page).await?;
    ok(
        "act after the page session died",
        &call(
            ctx,
            "act",
            json!({"page": page, "kind": "fill", "ref": "e1", "value": "a@example.test"}),
        )
        .await?,
    )?;
    ensure_replaced(session, page, &dropped).await?;

    // The audit layer redacts a fill only when the field reads as sensitive. That check
    // resolves the ref first, so it must recover too, or the value is logged in the clear.
    anyhow::ensure!(
        // `ok` returns the content as JSON, so the quotes arrive escaped.
        snapshot.contains(r#"textbox \"Password\" [ref=e2]"#),
        "no password ref: {snapshot}"
    );
    let dropped = drop_page_session_unannounced(session, client, hidden, page).await?;
    let fill = call(
        ctx,
        "act",
        json!({"page": page, "kind": "fill", "ref": "e2", "value": "synthetic-pass"}),
    )
    .await?;
    ok("password fill after the page session died", &fill)?;
    anyhow::ensure!(
        fill.audit_hints
            .as_ref()
            .is_some_and(|hints| hints.sensitive_refs.iter().any(|r| r == "e2")),
        "the password fill was not marked for redaction: {:?}",
        fill.audit_hints
    );
    ensure_replaced(session, page, &dropped).await?;

    let dropped = drop_page_session_unannounced(session, client, hidden, page).await?;
    ok(
        "click after the page session died",
        &call(
            ctx,
            "act",
            json!({"page": page, "kind": "click", "ref": "e1"}),
        )
        .await?,
    )?;
    ensure_replaced(session, page, &dropped).await?;

    // Typing classifies the focused field first. That check must recover with the typing.
    ok(
        "focus the password field",
        &call(
            ctx,
            "act",
            json!({"page": page, "kind": "click", "ref": "e2"}),
        )
        .await?,
    )?;
    let dropped = drop_page_session_unannounced(session, client, hidden, page).await?;
    let typed = call(
        ctx,
        "act",
        json!({"page": page, "kind": "type", "text": "synthetic-typed"}),
    )
    .await?;
    ok("typing after the page session died", &typed)?;
    anyhow::ensure!(
        typed
            .audit_hints
            .as_ref()
            .is_some_and(|hints| hints.redact_all_values),
        "typing into the password field was not marked for redaction: {:?}",
        typed.audit_hints
    );
    ensure_replaced(session, page, &dropped).await?;
    Ok(())
}

/// The audit layer's sensitive-field checks each run a script in two commands. A session that
/// dies between them must not read as "not sensitive" while the act that follows re-attaches
/// and types the secret.
async fn checks_survive_a_drop_mid_script(
    ctx: &ToolCtx,
    session: &BrowserSession,
    drop_before_script: &Mutex<Option<&'static str>>,
    page: u32,
) -> anyhow::Result<()> {
    let arm = |needle| -> anyhow::Result<()> {
        *drop_before_script
            .lock()
            .map_err(|_| anyhow::anyhow!("arm poisoned"))? = Some(needle);
        Ok(())
    };

    // The focused-field check reads `document.activeElement`.
    let before = attached_session(session, page).await?;
    arm("activeElement")?;
    let typed = call(
        ctx,
        "act",
        json!({"page": page, "kind": "type", "text": "synthetic-mid-script"}),
    )
    .await?;
    ok("typing after a drop mid-check", &typed)?;
    anyhow::ensure!(
        typed
            .audit_hints
            .as_ref()
            .is_some_and(|hints| hints.redact_all_values),
        "typing was not marked for redaction after a drop mid-check: {:?}",
        typed.audit_hints
    );
    ensure_replaced(session, page, &before).await?;

    // The ref check calls the shared predicate, which names `current-password`.
    let before = attached_session(session, page).await?;
    arm("current-password")?;
    let fill = call(
        ctx,
        "act",
        json!({"page": page, "kind": "fill", "ref": "e2", "value": "synthetic-mid-script"}),
    )
    .await?;
    ok("password fill after a drop mid-check", &fill)?;
    anyhow::ensure!(
        fill.audit_hints
            .as_ref()
            .is_some_and(|hints| hints.sensitive_refs.iter().any(|r| r == "e2")),
        "the password fill was not marked for redaction after a drop mid-check: {:?}",
        fill.audit_hints
    );
    ensure_replaced(session, page, &before).await
}

/// A snapshot that read every frame it found.
async fn complete_snapshot(ctx: &ToolCtx, label: &str, page: u32) -> anyhow::Result<String> {
    let result = call(ctx, "snapshot", json!({"page": page})).await?;
    let body = ok(label, &result)?;
    let structured = result
        .structured_content
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("{label} has no structured result"))?;
    anyhow::ensure!(
        structured["partial"] == json!(false) && structured.get("frameErrors").is_none(),
        "{label} reported unread frames: {structured}"
    );
    Ok(body)
}

/// The page's attached session, as Neo will address the next command to it.
async fn attached_session(session: &BrowserSession, page: u32) -> anyhow::Result<String> {
    let attached = session
        .pages
        .get_attached_session(PageId(page))
        .await
        .ok_or_else(|| anyhow::anyhow!("page {page} has no attached session"))?;
    Ok(attached
        .session_id()
        .ok_or_else(|| anyhow::anyhow!("page session has no id"))?
        .0
        .clone())
}

async fn ensure_replaced(session: &BrowserSession, page: u32, dropped: &str) -> anyhow::Result<()> {
    let current = attached_session(session, page).await?;
    anyhow::ensure!(
        current != dropped,
        "page {page} still addresses the dropped session {dropped}"
    );
    Ok(())
}

/// Detaches the page's current session in Chromium while hiding that detach from Neo, and
/// returns the dropped session's id.
async fn drop_page_session_unannounced(
    session: &BrowserSession,
    client: &CdpClient,
    hidden: &Mutex<Vec<String>>,
    page: u32,
) -> anyhow::Result<String> {
    let id = attached_session(session, page).await?;
    hidden
        .lock()
        .map_err(|_| anyhow::anyhow!("hidden set poisoned"))?
        .push(id.clone());
    client
        .send("Target.detachFromTarget", json!({"sessionId": id}), None)
        .await?;
    Ok(id)
}
