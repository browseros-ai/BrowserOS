use crate::{
    AppState,
    api::mcp::dispatch::{
        ToolCall, ToolObserver, ToolObserverContext, extract_page_id, result_page_id,
    },
    db::audit_log::{RecordToolDispatchInput, redacted_args_json, result_meta},
    ids::DispatchId,
    services::{
        audit::{AuditEvent, AuditPreview},
        sessions::Session,
    },
};
use browseros_core::PageId;
use browseros_mcp::{
    ToolResult,
    token_estimate::{
        TOKEN_ESTIMATOR_VERSION, estimate_tool_input_tokens, estimate_tool_output_tokens,
    },
};
use futures_util::future::BoxFuture;
use serde_json::Value;
use std::sync::Arc;
use tracing::warn;

const AUDIT_IDENTITY_TEXT_MAX: usize = 512;
const AUDIT_TOOL_NAME_MAX: usize = 128;
const AUDIT_TARGET_ID_MAX: usize = 512;
const AUDIT_URL_MAX: usize = 4096;
const AUDIT_TITLE_MAX: usize = 1024;

/** Converts the final tool outcome into a bounded event and admits it to the audit worker. */
pub fn apply(context: ToolObserverContext<'_>) -> BoxFuture<'_, anyhow::Result<()>> {
    Box::pin(async move {
        let Some(event) = build_event(
            context.call,
            context.result,
            context.duration_ms,
            context.cancelled,
        )
        .await
        else {
            return Ok(());
        };
        context.call.state.audit_worker.submit(event).await?;
        Ok(())
    })
}

pub struct LocalToolDispatch<'a> {
    pub session: &'a Arc<Session>,
    pub agent_label: &'a str,
    pub tool_name: &'a str,
    pub raw_args: &'a Value,
    pub result: &'a ToolResult,
    pub duration_ms: i64,
    pub dispatch_id: DispatchId,
}

pub async fn record_local_tool_dispatch(
    state: &AppState,
    dispatch: LocalToolDispatch<'_>,
) -> anyhow::Result<()> {
    state
        .audit_worker
        .submit(AuditEvent::without_preview(RecordToolDispatchInput {
            agent_id: bounded_text(
                dispatch.session.convo_id().as_str(),
                AUDIT_IDENTITY_TEXT_MAX,
            ),
            slug: bounded_text(dispatch.session.agent().slug(), AUDIT_IDENTITY_TEXT_MAX),
            agent_label: bounded_text(dispatch.agent_label, AUDIT_IDENTITY_TEXT_MAX),
            session_id: dispatch.session.id().as_str().to_string(),
            tool_name: bounded_text(dispatch.tool_name, AUDIT_TOOL_NAME_MAX),
            page_id: None,
            tab_id: None,
            target_id: None,
            url: None,
            title: None,
            args_json: redacted_args_json(dispatch.raw_args, dispatch.result.audit_hints.as_ref()),
            result_meta: tool_result_meta(dispatch.result, false),
            duration_ms: dispatch.duration_ms,
            // Recorded right after the tool returns, so back-date by its duration: every
            // top-level row is stamped with its start, which readers order calls by.
            created_at: Some(crate::clock::now_epoch_ms().saturating_sub(dispatch.duration_ms)),
            dispatch_id: dispatch.dispatch_id,
            parent_dispatch_id: None,
            tool_input_token_estimate: estimate_tool_input_tokens(
                dispatch.tool_name,
                dispatch.raw_args,
            ),
            tool_output_token_estimate: estimate_tool_output_tokens(&dispatch.result.content),
            token_estimator_version: TOKEN_ESTIMATOR_VERSION,
        }))
        .await?;
    Ok(())
}

async fn build_event(
    call: &ToolCall,
    result: &ToolResult,
    duration_ms: i64,
    cancelled: bool,
) -> Option<AuditEvent> {
    let Some(identity) = &call.identity else {
        warn!(
            tool = call.tool().name,
            session_id = %call.session_id,
            "cockpit dispatch missing identity"
        );
        return None;
    };
    let page_id = if call.flags.new_page {
        result_page_id(result)
    } else {
        extract_page_id(call)
    };
    let live = match (&call.browser_session, page_id) {
        (Some(browser), Some(page_id)) => browser.pages.get_info(PageId(page_id)).await,
        _ => None,
    }
    .or_else(|| call.page_snapshot.clone());
    // A sensitive dispatch skips its screenshot: a password or payment form would
    // otherwise sit on disk as an image beside the redacted args.
    let sensitive_script = call
        .sensitive_input
        .load(std::sync::atomic::Ordering::Relaxed);
    let script_hints = browseros_mcp::AuditHints {
        redact_all_values: true,
        skip_screenshot: true,
        ..Default::default()
    };
    // Source can embed credentials even when admission, syntax checking or
    // cancellation prevents the bridge from discovering that call. Keep child
    // step metadata, but never persist raw run source in the audit ledger.
    let audit_hints = if sensitive_script || call.tool().name == "run" {
        Some(&script_hints)
    } else {
        result.audit_hints.as_ref()
    };
    let skip_screenshot = sensitive_script
        || browseros_mcp::tools::sensitive::is_sensitive_tool(call.tool().name)
        || result
            .audit_hints
            .as_ref()
            .is_some_and(|hints| hints.skip_screenshot);
    Some(AuditEvent {
        input: RecordToolDispatchInput {
            agent_id: bounded_text(
                identity.session.convo_id().as_str(),
                AUDIT_IDENTITY_TEXT_MAX,
            ),
            slug: bounded_text(identity.agent.slug(), AUDIT_IDENTITY_TEXT_MAX),
            agent_label: bounded_text(&identity.agent_label, AUDIT_IDENTITY_TEXT_MAX),
            session_id: call.session_id.as_str().to_string(),
            tool_name: bounded_text(call.tool().name, AUDIT_TOOL_NAME_MAX),
            page_id: page_id.map(i64::from),
            tab_id: live.as_ref().map(|page| page.tab_id.0),
            target_id: live
                .as_ref()
                .map(|page| bounded_text(page.target_id.as_str(), AUDIT_TARGET_ID_MAX)),
            url: live
                .as_ref()
                .map(|page| bounded_text(&page.url, AUDIT_URL_MAX)),
            title: live
                .as_ref()
                .map(|page| bounded_text(&page.title, AUDIT_TITLE_MAX)),
            args_json: redacted_args_json(&call.raw_args, audit_hints),
            result_meta: tool_result_meta(result, cancelled),
            duration_ms,
            // Stamp the tool's start so a script dispatch sorts before the child
            // primitives it records while executing; top-level rows have no parent.
            created_at: Some(call.started_at_ms),
            dispatch_id: call.dispatch_id.clone(),
            parent_dispatch_id: None,
            tool_input_token_estimate: estimate_tool_input_tokens(call.tool().name, &call.raw_args),
            tool_output_token_estimate: estimate_tool_output_tokens(&result.content),
            token_estimator_version: TOKEN_ESTIMATOR_VERSION,
        },
        preview: if skip_screenshot {
            None
        } else {
            Some(preview_callback(&call.state, &identity.session))
        },
    })
}

/// The audit worker invokes this after storing the row. Weak references keep
/// queued previews from extending session or service teardown.
pub(in crate::api::mcp) fn preview_callback(
    state: &AppState,
    session: &Arc<Session>,
) -> AuditPreview {
    let session = Arc::downgrade(session);
    let visuals = Arc::downgrade(&state.visuals);
    let screenshots = Arc::downgrade(&state.screenshots);
    Arc::new(move |session_id, row_id| {
        let session = session.clone();
        let visuals = visuals.clone();
        let screenshots = screenshots.clone();
        Box::pin(async move {
            let (Some(session), Some(visuals), Some(screenshots)) =
                (session.upgrade(), visuals.upgrade(), screenshots.upgrade())
            else {
                return Ok(false);
            };
            let Some(bytes) = visuals.capture_for_session(&session).await? else {
                return Ok(false);
            };
            screenshots.write(&session_id, row_id, &bytes).await?;
            Ok(true)
        })
    })
}

/// Captures a screenshot of the session's active owned tab and attaches it to
/// the given audit row. Called synchronously by the code-mode script hook so a
/// script's primitives get per-step screenshots like granular tools; failures
/// are non-fatal.
pub(crate) async fn persist_screenshot(
    state: &AppState,
    session_id: &str,
    dispatch_id: &DispatchId,
    row_id: i64,
) {
    let bytes = match state.visuals.capture(session_id, None).await {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return,
        Err(error) => {
            warn!(error = %error, dispatch_id = %dispatch_id, "audit screenshot capture failed");
            return;
        }
    };
    if let Err(error) = state.screenshots.write(session_id, row_id, &bytes).await {
        warn!(error = %error, dispatch_id = %dispatch_id, "session screenshot write failed");
        return;
    }
    if let Err(error) = state.audit_log.mark_screenshot(row_id).await {
        warn!(error = %error, dispatch_id = %dispatch_id, "audit screenshot marker failed");
    }
}

fn tool_result_meta(result: &ToolResult, cancelled: bool) -> String {
    match result.structured_content.as_ref() {
        Some(structured_content) => result_meta(
            cancelled || result.is_error,
            cancelled,
            structured_content,
            result.content.len(),
        ),
        None => result_meta(
            cancelled || result.is_error,
            cancelled,
            &Value::Null,
            result.content.len(),
        ),
    }
}

fn bounded_text(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes.saturating_sub(1);
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    format!("{}~", &value[..end])
}

const _: ToolObserver = apply;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::audit_log::ListDispatchesQuery;
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use browseros_core::{BrowserSession, CdpConnection};
    use browseros_mcp::token_estimate::{
        TOKEN_ESTIMATOR_VERSION, estimate_tool_input_tokens, estimate_tool_output_tokens,
    };
    use rmcp::model::ContentBlock;
    use serde_json::json;
    use std::sync::{
        Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    };
    use tokio::{
        sync::Semaphore,
        time::{Duration, timeout},
    };

    fn png_header(width: u32, height: u32) -> String {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        STANDARD.encode(bytes)
    }

    #[tokio::test]
    async fn records_ordinary_errors_and_cancellations() -> anyhow::Result<()> {
        let call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        let failed = ToolResult::error("failed");
        apply(ToolObserverContext {
            call: &call,
            result: &failed,
            cancelled: false,
            duration_ms: 4,
        })
        .await?;

        let cancelled = ToolResult {
            content: vec![ContentBlock::text("Operation cancelled by the User")],
            is_error: true,
            structured_content: Some(json!({
                "cancellationReason": "Operation cancelled by the User",
                "cancellationKind": "cockpit.operator-cancelled"
            })),
            audit_hints: None,
        };
        apply(ToolObserverContext {
            call: &call,
            result: &cancelled,
            cancelled: true,
            duration_ms: 5,
        })
        .await?;
        call.state
            .audit_worker
            .flush_session(call.session_id.as_str())
            .await?;

        let rows = call
            .state
            .audit_log
            .list_dispatches(ListDispatchesQuery::default())
            .await?
            .rows;
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|row| row.token_estimator_version == TOKEN_ESTIMATOR_VERSION)
        );
        assert!(rows.iter().all(|row| {
            row.tool_input_token_estimate
                == estimate_tool_input_tokens(call.tool().name, &call.raw_args)
        }));
        assert_eq!(
            rows[0].tool_output_token_estimate,
            estimate_tool_output_tokens(&cancelled.content)
        );
        assert_eq!(
            rows[1].tool_output_token_estimate,
            estimate_tool_output_tokens(&failed.content)
        );
        assert!(rows.iter().all(|row| {
            row.result_meta
                .as_deref()
                .is_some_and(|meta| meta.contains("\"isError\":true"))
        }));
        Ok(())
    }

    #[tokio::test]
    async fn local_tool_uses_the_shared_mixed_content_estimator() -> anyhow::Result<()> {
        let call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        let session = &call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?
            .session;
        let raw_args = json!({ "name": "focus" });
        let result = ToolResult {
            content: vec![
                ContentBlock::text("abc"),
                ContentBlock::image(png_header(33, 65), "image/png"),
            ],
            is_error: false,
            structured_content: Some(json!({ "ignored": true })),
            audit_hints: None,
        };

        record_local_tool_dispatch(
            &call.state,
            LocalToolDispatch {
                session,
                agent_label: "Codex",
                tool_name: "name_session",
                raw_args: &raw_args,
                result: &result,
                duration_ms: 1,
                dispatch_id: DispatchId::new(),
            },
        )
        .await?;
        call.state
            .audit_worker
            .flush_session(call.session_id.as_str())
            .await?;

        let row = call
            .state
            .audit_log
            .list_dispatches(ListDispatchesQuery::default())
            .await?
            .rows
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("dispatch missing"))?;
        assert_eq!(row.token_estimator_version, TOKEN_ESTIMATOR_VERSION);
        assert_eq!(
            row.tool_input_token_estimate,
            estimate_tool_input_tokens("name_session", &raw_args)
        );
        assert_eq!(row.tool_output_token_estimate, 7);
        Ok(())
    }

    #[tokio::test]
    async fn local_tool_rows_are_stamped_with_their_start() -> anyhow::Result<()> {
        let call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        let session = &call
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity missing"))?
            .session;
        let result = ToolResult {
            content: vec![ContentBlock::text("done")],
            is_error: false,
            structured_content: None,
            audit_hints: None,
        };
        let finished_at = crate::clock::now_epoch_ms();

        record_local_tool_dispatch(
            &call.state,
            LocalToolDispatch {
                session,
                agent_label: "Codex",
                tool_name: "save_skill",
                raw_args: &json!({}),
                result: &result,
                duration_ms: 5_000,
                dispatch_id: DispatchId::new(),
            },
        )
        .await?;
        call.state
            .audit_worker
            .flush_session(call.session_id.as_str())
            .await?;

        let row = call
            .state
            .audit_log
            .list_dispatches(ListDispatchesQuery::default())
            .await?
            .rows
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("dispatch missing"))?;
        // Recorded right after it finished, so its start is about five seconds back.
        assert!(row.created_at <= finished_at - 4_000, "{}", row.created_at);
        assert!(row.created_at >= finished_at - 6_000, "{}", row.created_at);
        Ok(())
    }

    #[tokio::test]
    async fn event_bounds_variable_size_fields_before_submission() -> anyhow::Result<()> {
        let mut call =
            crate::api::mcp::test_support::tool_call("tabs", json!({ "value": "x".repeat(9000) }))
                .await?;
        call.page_snapshot = Some(browseros_core::pages::PageInfo {
            page_id: PageId(7),
            tab_id: browseros_core::TabId(8),
            target_id: browseros_core::TargetId::from("t".repeat(9000)),
            url: "u".repeat(9000),
            title: "i".repeat(9000),
            is_active: true,
            is_loading: false,
            load_progress: 1.0,
            is_pinned: false,
            window_id: None,
            index: None,
            group_id: None,
            browser_profile_id: None,
            live_browser_context_id: None,
        });
        let result = ToolResult::text(
            "x".repeat(9000),
            Some(json!({
                "x".repeat(9000): "y"
            })),
        );
        let event = build_event(&call, &result, 1, false)
            .await
            .ok_or_else(|| anyhow::anyhow!("event missing"))?;

        assert!(event.input.args_json.len() <= 4096);
        assert!(event.input.result_meta.len() <= 4096);
        assert!(
            event
                .input
                .target_id
                .as_deref()
                .is_some_and(|value| value.len() <= 512)
        );
        assert!(
            event
                .input
                .url
                .as_deref()
                .is_some_and(|value| value.len() <= 4096)
        );
        assert!(
            event
                .input
                .title
                .as_deref()
                .is_some_and(|value| value.len() <= 1024)
        );
        Ok(())
    }

    #[tokio::test]
    async fn sensitive_dispatch_redacts_args_and_skips_screenshot() -> anyhow::Result<()> {
        let call = crate::api::mcp::test_support::tool_call(
            "act",
            json!({ "page": 1, "kind": "fill", "ref": "e1", "value": "hunter2" }),
        )
        .await?;
        let result = ToolResult {
            content: vec![ContentBlock::text("ok (fill)")],
            is_error: false,
            structured_content: Some(json!({ "kind": "fill" })),
            audit_hints: Some(browseros_mcp::AuditHints {
                sensitive_refs: vec!["e1".to_string()],
                redact_all_values: false,
                skip_screenshot: true,
            }),
        };

        let event = build_event(&call, &result, 1, false)
            .await
            .ok_or_else(|| anyhow::anyhow!("event missing"))?;

        assert!(event.input.args_json.contains("[redacted]"));
        assert!(!event.input.args_json.contains("hunter2"));
        // A sensitive dispatch is never screenshotted.
        assert!(event.preview.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn credential_early_failure_redacts_password_without_runtime_hints() -> anyhow::Result<()>
    {
        let call = crate::api::mcp::test_support::tool_call(
            "credentials",
            json!({
                "action":"fill", "page":1, "username":"synthetic-user",
                "password":{"unexpected":"synthetic-credential-secret"}
            }),
        )
        .await?;
        let result = ToolResult::error("unsupported native browser");
        let event = build_event(&call, &result, 1, true)
            .await
            .ok_or_else(|| anyhow::anyhow!("event missing"))?;
        assert!(event.input.args_json.contains("synthetic-user"));
        assert!(event.input.args_json.contains("[redacted]"));
        assert!(
            !event
                .input
                .args_json
                .contains("synthetic-credential-secret")
        );
        assert!(event.preview.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn card_early_failure_redacts_card_without_runtime_hints() -> anyhow::Result<()> {
        let call = crate::api::mcp::test_support::tool_call(
            "payment_cards",
            json!({
                "action":"fill", "page":1,
                "card":{"number":"4111111111111111","expiryMonth":12,"expiryYear":2030,"cvc":"987"}
            }),
        )
        .await?;
        let result = ToolResult::error("unsupported native browser");
        let event = build_event(&call, &result, 1, true)
            .await
            .ok_or_else(|| anyhow::anyhow!("event missing"))?;
        assert!(event.input.args_json.contains("\"action\":\"fill\""));
        assert!(event.input.args_json.contains("[redacted]"));
        assert!(!event.input.args_json.contains("4111111111111111"));
        assert!(!event.input.args_json.contains("987"));
        assert!(event.preview.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn cancelled_credential_script_redacts_parent_source_and_suppresses_preview()
    -> anyhow::Result<()> {
        let call = crate::api::mcp::test_support::tool_call(
            "run",
            json!({
                "code":"await browser.credentials({action:'add',password:'synthetic-run-secret'});"
            }),
        )
        .await?;
        call.sensitive_input
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let result = ToolResult::error("cancelled");
        let event = build_event(&call, &result, 1, true)
            .await
            .ok_or_else(|| anyhow::anyhow!("missing event"))?;
        assert!(event.input.args_json.contains("[redacted]"));
        assert!(!event.input.args_json.contains("synthetic-run-secret"));
        assert!(event.preview.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn credential_script_is_sensitive_even_before_the_bridge_runs() -> anyhow::Result<()> {
        let call = crate::api::mcp::test_support::tool_call("run", json!({
            "code":"await sleep(99999); await browser.credentials({action:'add',password:'synthetic-unreached-secret'});"
        })).await?;
        let result = ToolResult::error("cancelled before execution");
        let event = build_event(&call, &result, 1, true)
            .await
            .ok_or_else(|| anyhow::anyhow!("missing event"))?;
        assert!(!event.input.args_json.contains("synthetic-unreached-secret"));
        assert!(event.preview.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn ordinary_dispatch_keeps_its_value_and_screenshot() -> anyhow::Result<()> {
        let call = crate::api::mcp::test_support::tool_call(
            "act",
            json!({ "page": 1, "kind": "fill", "ref": "e1", "value": "london" }),
        )
        .await?;
        let result = ToolResult {
            content: vec![ContentBlock::text("ok (fill)")],
            is_error: false,
            structured_content: Some(json!({ "kind": "fill" })),
            audit_hints: None,
        };

        let event = build_event(&call, &result, 1, false)
            .await
            .ok_or_else(|| anyhow::anyhow!("event missing"))?;

        // No hints: the ledger is unchanged and the screenshot is still attached.
        assert!(event.input.args_json.contains("london"));
        assert!(event.preview.is_some());
        Ok(())
    }

    /// A controllable browser boundary for the real preview, actor, ledger and file path.
    /// Screenshot bytes and loader identity change independently so a stale image is observable.
    struct PreviewConnection {
        inner: Arc<crate::api::mcp::test_support::ProfileConnection>,
        image: StdMutex<Vec<u8>>,
        document: StdMutex<String>,
        returned_images: StdMutex<Vec<Vec<u8>>>,
        pause_capture: AtomicBool,
        capture_entered: Semaphore,
        capture_release: Semaphore,
        pause_frame: AtomicBool,
        frame_entered: Semaphore,
        frame_release: Semaphore,
        frame_reads: Semaphore,
        inventories: Semaphore,
    }

    impl PreviewConnection {
        fn new() -> Arc<Self> {
            let inner = crate::api::mcp::test_support::ProfileConnection::new();
            inner
                .tabs
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .retain(|tab| tab["tabId"] == 101 || tab["tabId"] == 102);
            Arc::new(Self {
                inner,
                image: StdMutex::new(b"\xff\xd8clean-page".to_vec()),
                document: StdMutex::new("document-one".into()),
                returned_images: StdMutex::new(Vec::new()),
                pause_capture: AtomicBool::new(false),
                capture_entered: Semaphore::new(0),
                capture_release: Semaphore::new(0),
                pause_frame: AtomicBool::new(false),
                frame_entered: Semaphore::new(0),
                frame_release: Semaphore::new(0),
                frame_reads: Semaphore::new(0),
                inventories: Semaphore::new(0),
            })
        }

        fn sensitive(&self) {
            *self.image.lock().unwrap_or_else(|p| p.into_inner()) =
                b"\xff\xd8later-secret".to_vec();
        }

        fn navigate(&self) {
            *self.document.lock().unwrap_or_else(|p| p.into_inner()) = "document-two".into();
            *self.image.lock().unwrap_or_else(|p| p.into_inner()) =
                b"\xff\xd8new-document".to_vec();
        }

        fn drain(semaphore: &Semaphore) {
            while let Ok(permit) = semaphore.try_acquire() {
                permit.forget();
            }
        }

        async fn signal(semaphore: &Semaphore, permits: u32) -> anyhow::Result<()> {
            timeout(Duration::from_secs(2), semaphore.acquire_many(permits))
                .await??
                .forget();
            Ok(())
        }
    }

    impl CdpConnection for PreviewConnection {
        fn send<'a>(
            &'a self,
            method: &'a str,
            params: Value,
            session: Option<&'a browseros_core::SessionId>,
        ) -> BoxFuture<'a, Result<Value, browseros_cdp::CdpError>> {
            self.send_at_epoch(self.connection_epoch(), method, params, session)
        }

        fn send_at_epoch<'a>(
            &'a self,
            epoch: u64,
            method: &'a str,
            params: Value,
            session: Option<&'a browseros_core::SessionId>,
        ) -> BoxFuture<'a, Result<Value, browseros_cdp::CdpError>> {
            Box::pin(async move {
                let actual = self.connection_epoch();
                if epoch != actual {
                    return Err(browseros_cdp::CdpError::ConnectionEpochChanged {
                        expected: epoch,
                        actual,
                    });
                }
                match method {
                    "Page.getFrameTree" => {
                        if self.pause_frame.swap(false, Ordering::SeqCst) {
                            self.frame_entered.add_permits(1);
                            self.frame_release
                                .acquire()
                                .await
                                .map_err(|error| browseros_cdp::CdpError::Protocol {
                                    code: -1,
                                    message: error.to_string(),
                                })?
                                .forget();
                        }
                        let document = self
                            .document
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .clone();
                        self.frame_reads.add_permits(1);
                        Ok(json!({"frameTree":{"frame":{
                            "id":"main-frame", "loaderId":document,
                            "url":"https://synthetic.test/form", "securityOrigin":"https://synthetic.test",
                            "mimeType":"text/html"
                        }}}))
                    }
                    "Page.captureScreenshot" => {
                        self.capture_entered.add_permits(1);
                        if self.pause_capture.swap(false, Ordering::SeqCst) {
                            self.capture_release
                                .acquire()
                                .await
                                .map_err(|error| browseros_cdp::CdpError::Protocol {
                                    code: -1,
                                    message: error.to_string(),
                                })?
                                .forget();
                        }
                        let image = self.image.lock().unwrap_or_else(|p| p.into_inner()).clone();
                        self.returned_images
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .push(image.clone());
                        Ok(json!({"data":STANDARD.encode(image)}))
                    }
                    "Browser.getTabs" => {
                        let result = self
                            .inner
                            .send_at_epoch(epoch, method, params, session)
                            .await;
                        self.inventories.add_permits(1);
                        result
                    }
                    _ => {
                        self.inner
                            .send_at_epoch(epoch, method, params, session)
                            .await
                    }
                }
            })
        }

        fn send_raw_json<'a>(
            &'a self,
            method: &'a str,
            params: &'a str,
            session: Option<&'a browseros_core::SessionId>,
        ) -> BoxFuture<'a, Result<String, browseros_cdp::CdpError>> {
            self.send_raw_json_at_epoch(self.connection_epoch(), method, params, session)
        }

        fn send_raw_json_at_epoch<'a>(
            &'a self,
            epoch: u64,
            method: &'a str,
            params: &'a str,
            session: Option<&'a browseros_core::SessionId>,
        ) -> BoxFuture<'a, Result<String, browseros_cdp::CdpError>> {
            Box::pin(async move {
                let params = serde_json::from_str(params).map_err(|error| {
                    browseros_cdp::CdpError::Protocol {
                        code: -1,
                        message: error.to_string(),
                    }
                })?;
                Ok(self
                    .send_at_epoch(epoch, method, params, session)
                    .await?
                    .to_string())
            })
        }

        fn events(&self) -> tokio::sync::broadcast::Receiver<browseros_cdp::CdpEvent> {
            self.inner.events()
        }
        fn events_targeted(
            &self,
            method: &str,
        ) -> tokio::sync::mpsc::UnboundedReceiver<browseros_cdp::CdpEvent> {
            self.inner.events_targeted(method)
        }
        fn is_connected(&self) -> bool {
            self.inner.is_connected()
        }
        fn connection_epoch(&self) -> u64 {
            self.inner.connection_epoch()
        }
    }

    struct PreviewFixture {
        state: AppState,
        session: Arc<Session>,
        other: Arc<Session>,
        connection: Arc<PreviewConnection>,
    }

    impl PreviewFixture {
        async fn new() -> anyhow::Result<Self> {
            use crate::{
                identity::{ClientIdentity, ConversationIdentity},
                ids::SessionId,
            };
            let call =
                crate::api::mcp::test_support::tool_call("tabs", json!({"action":"list"})).await?;
            let session = call
                .identity
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("session missing"))?
                .session
                .clone();
            let other = Session::new(
                SessionId::new("s2"),
                ClientIdentity::Ephemeral {
                    slug: "codex".into(),
                    label: "Codex".into(),
                },
                ConversationIdentity::new("codex", "other-preview".into()),
                "Codex".into(),
                tokio::time::Instant::now(),
            );
            call.state.sessions.insert_for_testing(other.clone()).await;
            let connection = PreviewConnection::new();
            let browser: Arc<BrowserSession> =
                crate::api::mcp::test_support::current_session(connection.clone());
            let pages = browser.pages.list().await?;
            call.state.browser.set_session_for_testing(browser).await;
            for (session, tab) in [(&session, 101), (&other, 102)] {
                call.state
                    .audit_log
                    .record_session_start(
                        session.id().as_str(),
                        session.convo_id().as_str(),
                        "codex",
                        "Codex",
                        "codex",
                        "1",
                    )
                    .await?;
                let page = pages
                    .iter()
                    .find(|page| page.tab_id.0 == tab)
                    .ok_or_else(|| anyhow::anyhow!("owned page missing"))?;
                call.state.session_tabs.enqueue_claim_tab_for_session(
                    tab,
                    Some(page.target_id.as_str().into()),
                    session.id().as_str().into(),
                    session.convo_id().as_str().into(),
                    1,
                );
                call.state
                    .tab_activity
                    .record_tool(crate::services::cockpit::RecordToolInput {
                        target_id: page.target_id.clone(),
                        tab_id: tab,
                        page_id: page.page_id.0,
                        session_id: session.id().as_str().into(),
                        agent_id: session.convo_id().as_str().into(),
                        slug: "codex".into(),
                        tool_name: "run".into(),
                    })
                    .await;
            }
            call.state.session_tabs.drain_writes().await;
            Ok(Self {
                state: call.state,
                session,
                other,
                connection,
            })
        }

        fn event(
            &self,
            session: &Arc<Session>,
            tool: &str,
            preview: Option<AuditPreview>,
        ) -> AuditEvent {
            AuditEvent {
                input: RecordToolDispatchInput {
                    agent_id: session.convo_id().as_str().into(),
                    slug: "codex".into(),
                    agent_label: "Codex".into(),
                    session_id: session.id().as_str().into(),
                    tool_name: tool.into(),
                    page_id: None,
                    tab_id: None,
                    target_id: None,
                    url: None,
                    title: None,
                    args_json: "{}".into(),
                    result_meta: "{}".into(),
                    duration_ms: 1,
                    created_at: None,
                    dispatch_id: DispatchId::new(),
                    parent_dispatch_id: None,
                    tool_input_token_estimate: 0,
                    tool_output_token_estimate: 0,
                    token_estimator_version: TOKEN_ESTIMATOR_VERSION,
                },
                preview,
            }
        }

        async fn submit(
            &self,
            session: &Arc<Session>,
            tool: &str,
            preview: Option<AuditPreview>,
        ) -> anyhow::Result<DispatchId> {
            let event = self.event(session, tool, preview);
            let id = event.input.dispatch_id.clone();
            self.state
                .audit_worker
                .submit_tracked(event)
                .await?
                .wait()
                .await?;
            Ok(id)
        }

        async fn preview(&self, tool: &str) -> anyhow::Result<DispatchId> {
            self.submit(
                &self.session,
                tool,
                Some(preview_callback(&self.state, &self.session)),
            )
            .await
        }

        async fn assert_picture(
            &self,
            dispatch: &DispatchId,
            expected: Option<&[u8]>,
        ) -> anyhow::Result<()> {
            self.state
                .audit_worker
                .flush_session(self.session.id().as_str())
                .await?;
            let row = self
                .state
                .audit_log
                .list_dispatches(ListDispatchesQuery {
                    session_id: Some(self.session.id().as_str().into()),
                    ..Default::default()
                })
                .await?
                .rows
                .into_iter()
                .find(|row| row.dispatch_id.as_deref() == Some(dispatch.as_str()))
                .ok_or_else(|| anyhow::anyhow!("preview parent missing"))?;
            assert_eq!(row.has_screenshot, expected.is_some());
            let pictures = self
                .state
                .screenshots
                .list(self.session.id().as_str())
                .await?
                .ok_or_else(|| anyhow::anyhow!("screenshot session missing"))?;
            assert_eq!(
                pictures
                    .iter()
                    .any(|picture| picture.screenshot_id == row.id),
                expected.is_some()
            );
            assert_eq!(
                tokio::fs::try_exists(
                    self.state
                        .screenshots
                        .path_for(self.session.id().as_str(), row.id)
                )
                .await?,
                expected.is_some()
            );
            if let Some(expected) = expected {
                assert_eq!(
                    self.state
                        .screenshots
                        .read(self.session.id().as_str(), row.id)
                        .await?,
                    expected
                );
            }
            Ok(())
        }

        async fn unsafe_run(&self, session: &Arc<Session>) -> anyhow::Result<()> {
            PreviewConnection::drain(&self.connection.frame_reads);
            let run = self.state.visuals.begin_script_run();
            self.connection.sensitive();
            self.submit(session, "neo.cdp", None).await?;
            drop(run);
            PreviewConnection::signal(&self.connection.frame_reads, 2).await
        }
    }

    #[tokio::test]
    async fn queued_preview_never_stores_later_cdp_pixels_from_either_session() -> anyhow::Result<()>
    {
        for another_session in [false, true] {
            let fixture = PreviewFixture::new().await?;
            let entered = Arc::new(Semaphore::new(0));
            let release = Arc::new(Semaphore::new(0));
            let blocker: AuditPreview = Arc::new({
                let entered = entered.clone();
                let release = release.clone();
                move |_, _| {
                    let entered = entered.clone();
                    let release = release.clone();
                    Box::pin(async move {
                        entered.add_permits(1);
                        release
                            .acquire()
                            .await
                            .map_err(|error| crate::error::AppError::Internal(error.to_string()))?
                            .forget();
                        Ok(false)
                    })
                }
            });
            fixture
                .submit(&fixture.session, "block-first-slot", Some(blocker.clone()))
                .await?;
            fixture
                .submit(&fixture.other, "block-second-slot", Some(blocker))
                .await?;
            PreviewConnection::signal(&entered, 2).await?;
            let parent = fixture.preview("queued-clean-run").await?;
            fixture
                .submit(&fixture.session, "queue-admitted", None)
                .await?;
            let later = if another_session {
                &fixture.other
            } else {
                &fixture.session
            };
            fixture.unsafe_run(later).await?;
            release.add_permits(2);
            fixture.assert_picture(&parent, None).await?;
            assert!(
                fixture
                    .connection
                    .returned_images
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .is_empty()
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn in_flight_preview_discards_later_cdp_pixels_from_either_session() -> anyhow::Result<()>
    {
        for another_session in [false, true] {
            let fixture = PreviewFixture::new().await?;
            fixture
                .connection
                .pause_capture
                .store(true, Ordering::SeqCst);
            let parent = fixture.preview("in-flight-clean-run").await?;
            PreviewConnection::signal(&fixture.connection.capture_entered, 1).await?;
            let later = if another_session {
                &fixture.other
            } else {
                &fixture.session
            };
            fixture.unsafe_run(later).await?;
            fixture.connection.capture_release.add_permits(1);
            fixture.assert_picture(&parent, None).await?;
            assert_eq!(
                *fixture
                    .connection
                    .returned_images
                    .lock()
                    .unwrap_or_else(|p| p.into_inner()),
                [b"\xff\xd8later-secret".to_vec()]
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn earlier_cdp_hold_survives_clean_work_and_navigation_restores_pictures()
    -> anyhow::Result<()> {
        let fixture = PreviewFixture::new().await?;
        fixture.unsafe_run(&fixture.other).await?;
        fixture.state.visuals.begin_script_run().clean();
        let held = fixture.preview("clean-on-held-document").await?;
        fixture.assert_picture(&held, None).await?;
        fixture.connection.navigate();
        fixture.state.visuals.begin_script_run().clean();
        let navigated = fixture.preview("clean-new-document").await?;
        fixture
            .assert_picture(&navigated, Some(b"\xff\xd8new-document"))
            .await?;
        Ok(())
    }

    #[tokio::test]
    async fn another_sessions_active_clean_run_suppresses_only_overlapping_capture()
    -> anyhow::Result<()> {
        let fixture = PreviewFixture::new().await?;
        fixture
            .submit(&fixture.other, "active-clean-run", None)
            .await?;
        let run = fixture.state.visuals.begin_script_run();
        let overlapping = fixture.preview("overlapping-preview").await?;
        fixture.assert_picture(&overlapping, None).await?;
        run.clean();
        let after = fixture.preview("after-clean-completion").await?;
        fixture
            .assert_picture(&after, Some(b"\xff\xd8clean-page"))
            .await?;
        Ok(())
    }

    #[tokio::test]
    async fn unsafe_drop_blocks_captures_until_document_holds_are_applied() -> anyhow::Result<()> {
        let fixture = PreviewFixture::new().await?;
        fixture.connection.pause_frame.store(true, Ordering::SeqCst);
        let run = fixture.state.visuals.begin_script_run();
        fixture.connection.sensitive();
        drop(run);
        PreviewConnection::signal(&fixture.connection.frame_entered, 1).await?;
        let applying = fixture.preview("while-hold-is-being-applied").await?;
        fixture.assert_picture(&applying, None).await?;
        fixture.connection.frame_release.add_permits(1);
        PreviewConnection::signal(&fixture.connection.frame_reads, 2).await?;
        let held = fixture.preview("after-hold-applied").await?;
        fixture.assert_picture(&held, None).await?;
        fixture.connection.navigate();
        let recovered = fixture.preview("after-new-document").await?;
        fixture
            .assert_picture(&recovered, Some(b"\xff\xd8new-document"))
            .await?;
        Ok(())
    }

    #[tokio::test]
    async fn failed_unsafe_inventory_blocks_until_recovery_applies_document_holds()
    -> anyhow::Result<()> {
        let fixture = PreviewFixture::new().await?;
        PreviewConnection::drain(&fixture.connection.inventories);
        PreviewConnection::drain(&fixture.connection.frame_reads);
        fixture
            .connection
            .inner
            .fail_inventory
            .store(true, Ordering::SeqCst);
        drop(fixture.state.visuals.begin_script_run());
        PreviewConnection::signal(&fixture.connection.inventories, 1).await?;
        let unavailable = fixture.preview("while-inventory-unavailable").await?;
        fixture.assert_picture(&unavailable, None).await?;
        fixture
            .connection
            .inner
            .fail_inventory
            .store(false, Ordering::SeqCst);
        PreviewConnection::signal(&fixture.connection.frame_reads, 2).await?;
        fixture.state.visuals.begin_script_run().clean();
        let held = fixture.preview("after-inventory-recovery").await?;
        fixture.assert_picture(&held, None).await?;
        assert!(
            fixture
                .connection
                .returned_images
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .is_empty()
        );
        fixture.connection.navigate();
        let recovered = fixture
            .preview("new-document-after-inventory-recovery")
            .await?;
        fixture
            .assert_picture(&recovered, Some(b"\xff\xd8new-document"))
            .await?;
        Ok(())
    }
}
