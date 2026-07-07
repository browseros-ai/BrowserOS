use crate::{
    app::AppState,
    domain::{AgentRef, ClientInfo, DispatchId, Session, SessionId, color_for_slug},
    services::{
        audit::{DispatchResultSummary, RecordToolDispatchInput},
        tab_activity::RecordToolInput,
    },
};
use base64::Engine;
use browseros_core::{
    PageId,
    screenshot::{ScreenshotCaptureOptions, ScreenshotFormat},
};
use browseros_mcp::{
    BrowserToolDefaults, BrowserToolOptions, McpBeforeToolResult, McpClientInfo, McpHookError,
    McpHookResult, McpHooks, McpSessionClosed, McpSessionStarted, McpToolCall, McpToolTiming,
    OutputFileAccess, ToolCtx, ToolResult, catalog, execute_tool, extract_page_id, result_page_id,
};
use futures_util::future::BoxFuture;
use rmcp::model::ContentBlock;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, info_span, warn};

const NAVIGATE_BLOCKED_SCHEMES: &[&str] = &["javascript", "file", "data"];

#[derive(Clone)]
pub struct ClawMcpHooks {
    state: AppState,
}

impl ClawMcpHooks {
    /// Connects host-owned session, audit, replay, and tab side effects to rmcp calls.
    #[must_use]
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    async fn session_started_inner(&self, event: McpSessionStarted) -> McpHookResult<()> {
        let session_id = SessionId::new(event.session_id);
        if self.state.sessions.contains(&session_id).await {
            return Ok(());
        }
        let client = client_info(event.client_info);
        let profiles = self
            .state
            .agents
            .list_profiles()
            .await
            .map_err(hook_error)?;
        let agent = AgentRef::resolve(&session_id, &client, &profiles);
        let session = self
            .state
            .sessions
            .mint_with_id(session_id, agent, client)
            .await
            .map_err(hook_error)?;
        tracing::info!(
            session_id = %session.id(),
            agent = %session.agent().agent_id(),
            "mcp session initialized"
        );
        Ok(())
    }

    async fn before_tool_inner(&self, call: McpToolCall) -> McpHookResult<McpBeforeToolResult> {
        let session = self.session(&call.session_id).await?;
        session.touch(tokio::time::Instant::now()).await;
        let dispatch_id = DispatchId::from(call.dispatch_id.clone());
        let cancel = linked_cancel_token(session.child_token(), call.cancel.clone());
        session
            .register_dispatch(dispatch_id.clone(), cancel.clone())
            .await;

        let span = info_span!(
            "mcp_dispatch",
            session_id = %session.id(),
            dispatch_id = %dispatch_id,
            agent = %session.agent().agent_id(),
            tool = %call.tool_name
        );
        async move {
            let result = if let Some(result) = navigate_scheme_guard(&call) {
                Some(result)
            } else if let Some(result) = browser_connected_guard(&call) {
                Some(result)
            } else {
                page_ownership_guard(&self.state, &session, &call).await
            };
            Ok(McpBeforeToolResult { cancel, result })
        }
        .instrument(span)
        .await
    }

    async fn after_tool_inner(
        &self,
        call: McpToolCall,
        result: &mut ToolResult,
        timing: McpToolTiming,
    ) -> McpHookResult<()> {
        let session = self.session(&call.session_id).await?;
        apply_post_execution_hooks(&session, &call, result).await;
        TabsResultFilter::apply(&session, &call, result).await;
        let audit = AuditWriter::record(&self.state, &session, &call, result, timing).await;
        if let Some(record) = audit {
            ScreenshotPersister::persist(
                &self.state,
                &session,
                &call,
                result,
                record,
                call.output_files.clone(),
            )
            .await;
        }
        TabActivityTracker::record(&self.state, &session, &call, result).await;
        TabGroupOrchestrator::record(&session, &call, result, call.output_files.clone()).await;
        session
            .unregister_dispatch(&DispatchId::from(call.dispatch_id))
            .await;
        call.cancel.cancel();
        Ok(())
    }

    async fn session(&self, session_id: &str) -> McpHookResult<Arc<Session>> {
        self.state
            .sessions
            .lookup(&SessionId::new(session_id))
            .await
            .ok_or_else(|| McpHookError::new(format!("mcp session {session_id} is not registered")))
    }
}

impl McpHooks for ClawMcpHooks {
    fn session_started(&self, event: McpSessionStarted) -> BoxFuture<'_, McpHookResult<()>> {
        Box::pin(async move { self.session_started_inner(event).await })
    }

    fn session_closed(&self, event: McpSessionClosed) -> BoxFuture<'_, McpHookResult<()>> {
        Box::pin(async move {
            self.state
                .sessions
                .remove(
                    &SessionId::new(event.session_id),
                    "closed",
                    Some(&event.reason),
                )
                .await
                .map_err(hook_error)?;
            Ok(())
        })
    }

    fn before_tool(&self, call: McpToolCall) -> BoxFuture<'_, McpHookResult<McpBeforeToolResult>> {
        Box::pin(async move { self.before_tool_inner(call).await })
    }

    fn after_tool<'a>(
        &'a self,
        call: McpToolCall,
        result: &'a mut ToolResult,
        timing: McpToolTiming,
    ) -> BoxFuture<'a, McpHookResult<()>> {
        Box::pin(async move { self.after_tool_inner(call, result, timing).await })
    }
}

fn client_info(value: McpClientInfo) -> ClientInfo {
    ClientInfo {
        name: clean_client_field(value.name, "agent"),
        version: clean_client_field(value.version, "unknown"),
        title: value.title,
    }
}

fn clean_client_field(value: String, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

/// Links rmcp request cancellation into the session-owned dispatch token.
fn linked_cancel_token(
    session_cancel: CancellationToken,
    request_cancel: CancellationToken,
) -> CancellationToken {
    let cancel_on_request = session_cancel.clone();
    let exit_on_completion = session_cancel.clone();
    tokio::spawn(async move {
        tokio::select! {
            () = request_cancel.cancelled_owned() => cancel_on_request.cancel(),
            () = exit_on_completion.cancelled_owned() => {}
        }
    });
    session_cancel
}

fn navigate_scheme_guard(call: &McpToolCall) -> Option<ToolResult> {
    if call.tool_name != "navigate" {
        return None;
    }
    let url = call.raw_args.get("url").and_then(Value::as_str)?;
    let (scheme, _rest) = url.split_once(':')?;
    let scheme = scheme.to_ascii_lowercase();
    if NAVIGATE_BLOCKED_SCHEMES.contains(&scheme.as_str()) {
        return Some(ToolResult::error(format!(
            "navigate refuses {scheme}: URLs; only http(s) is allowed"
        )));
    }
    None
}

fn browser_connected_guard(call: &McpToolCall) -> Option<ToolResult> {
    if call.browser_session.is_some() {
        None
    } else {
        Some(ToolResult::error(
            "browser not connected (retrying); try again once BrowserOS reconnects",
        ))
    }
}

async fn page_ownership_guard(
    state: &AppState,
    session: &Arc<Session>,
    call: &McpToolCall,
) -> Option<ToolResult> {
    let page_id = extract_page_id(call.hooks.accepts_page_arg, &call.raw_args)?;
    let page_id = PageId(page_id);
    match state.sessions.owner_of_page(&page_id).await {
        Some(owner) if owner != *session.id() => Some(ToolResult::error(format!(
            "page {} is not owned by this agent; call `tabs` with action=\"new\" to open a fresh page and use the returned page id.",
            page_id.0
        ))),
        Some(_) => None,
        None => {
            session.add_owned_page(page_id).await;
            None
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct AuditRecord {
    row_id: i64,
}

struct AuditWriter;

impl AuditWriter {
    async fn record(
        state: &AppState,
        session: &Arc<Session>,
        call: &McpToolCall,
        result: &ToolResult,
        timing: McpToolTiming,
    ) -> Option<AuditRecord> {
        let page_id = result_page_id(result)
            .or_else(|| extract_page_id(call.hooks.accepts_page_arg, &call.raw_args));
        let live = match (&call.browser_session, page_id) {
            (Some(browser), Some(page_id)) => browser.pages.get_info(PageId(page_id)).await,
            _ => None,
        };
        let content = serde_json::to_value(&result.content).unwrap_or_else(|err| {
            warn!(error = %err, "tool content serialization failed");
            json!([])
        });
        let structured_content = result.structured_content.clone().unwrap_or(Value::Null);
        match state
            .audit
            .record_tool_dispatch(RecordToolDispatchInput {
                agent_id: session.agent().agent_id().as_str().to_string(),
                slug: session.agent().slug().to_string(),
                agent_label: session.agent().label().to_string(),
                session_id: session.id().as_str().to_string(),
                tool_name: call.tool_name.to_string(),
                page_id: page_id.map(i64::from),
                target_id: live
                    .as_ref()
                    .map(|page| page.target_id.as_str().to_string()),
                url: live.as_ref().map(|page| page.url.clone()),
                title: live.as_ref().map(|page| page.title.clone()),
                raw_args: call.raw_args.clone(),
                duration_ms: timing.duration_ms,
                dispatch_id: DispatchId::from(call.dispatch_id.clone()),
                result: DispatchResultSummary {
                    is_error: result.is_error,
                    structured_content,
                    content,
                },
            })
            .await
        {
            Ok(row_id) => Some(AuditRecord { row_id }),
            Err(err) => {
                warn!(error = %err, dispatch_id = %call.dispatch_id, "audit writer failed");
                None
            }
        }
    }
}

struct ScreenshotPersister;

impl ScreenshotPersister {
    async fn persist(
        state: &AppState,
        session: &Arc<Session>,
        call: &McpToolCall,
        result: &ToolResult,
        record: AuditRecord,
        output_files: OutputFileAccess,
    ) {
        if result.is_error {
            return;
        }
        let mut wrote = false;
        for image in result.content.iter().filter_map(image_data) {
            match base64::engine::general_purpose::STANDARD.decode(image.data.as_bytes()) {
                Ok(bytes) => {
                    if write_screenshot_files(state, call, record, &bytes).await {
                        wrote = true;
                        break;
                    }
                }
                Err(err) => {
                    warn!(error = %err, dispatch_id = %call.dispatch_id, "tool-result image decode failed")
                }
            }
        }
        if wrote {
            return;
        }
        let Some(browser) = &call.browser_session else {
            return;
        };
        let page_id = result_page_id(result)
            .or_else(|| extract_page_id(call.hooks.accepts_page_arg, &call.raw_args));
        let Some(page_id) = page_id else {
            return;
        };
        let page = PageId(page_id);
        let should_capture = call.hooks.capture_new_page || !session.has_first_capture(&page).await;
        if !should_capture {
            return;
        }
        let options = ScreenshotCaptureOptions {
            format: Some(ScreenshotFormat::Jpeg),
            quality: Some(50),
            full_page: Some(false),
            annotate: Some(false),
            clip: None,
        };
        match browser.screenshot(page.clone(), options).await {
            Ok(capture) => match base64::engine::general_purpose::STANDARD.decode(capture.data) {
                Ok(bytes) => {
                    if write_screenshot_files(state, call, record, &bytes).await {
                        session.mark_first_capture_done(page).await;
                    }
                }
                Err(err) => {
                    warn!(error = %err, dispatch_id = %call.dispatch_id, "fallback screenshot decode failed")
                }
            },
            Err(err) => {
                warn!(error = %err, dispatch_id = %call.dispatch_id, "fallback screenshot capture failed")
            }
        }
        drop(output_files);
    }
}

async fn write_screenshot_files(
    state: &AppState,
    call: &McpToolCall,
    record: AuditRecord,
    bytes: &[u8],
) -> bool {
    let row_key = record.row_id.to_string();
    let write_row = state.screenshots.write(&row_key, bytes).await;
    let write_dispatch = state.screenshots.write(&call.dispatch_id, bytes).await;
    if let Err(err) = write_row {
        warn!(error = %err, dispatch_id = %call.dispatch_id, "screenshot row-id write failed");
        return false;
    }
    if let Err(err) = write_dispatch {
        warn!(error = %err, dispatch_id = %call.dispatch_id, "screenshot dispatch-id write failed");
    }
    if let Err(err) = state.audit.mark_screenshot(record.row_id).await {
        warn!(error = %err, dispatch_id = %call.dispatch_id, "audit screenshot marker failed");
    }
    true
}

struct ImageRef<'a> {
    data: &'a str,
}

fn image_data(block: &ContentBlock) -> Option<ImageRef<'_>> {
    match block {
        ContentBlock::Image(image) => Some(ImageRef { data: &image.data }),
        _ => None,
    }
}

struct TabActivityTracker;

impl TabActivityTracker {
    async fn record(
        state: &AppState,
        session: &Arc<Session>,
        call: &McpToolCall,
        result: &ToolResult,
    ) {
        if result.is_error {
            return;
        }
        let Some(browser) = &call.browser_session else {
            return;
        };
        let Some(page_id) = result_page_id(result)
            .or_else(|| extract_page_id(call.hooks.accepts_page_arg, &call.raw_args))
        else {
            return;
        };
        let Some(info) = browser.pages.get_info(PageId(page_id)).await else {
            return;
        };
        state
            .tab_activity
            .record_tool(RecordToolInput {
                target_id: info.target_id,
                page_id,
                url: info.url,
                title: info.title,
                agent_id: session.agent().agent_id().as_str().to_string(),
                slug: session.agent().slug().to_string(),
                tool_name: call.tool_name.to_string(),
            })
            .await;
    }
}

struct TabGroupOrchestrator;

impl TabGroupOrchestrator {
    async fn record(
        session: &Arc<Session>,
        call: &McpToolCall,
        result: &ToolResult,
        output_files: OutputFileAccess,
    ) {
        if result.is_error || !call.hooks.capture_new_page {
            return;
        }
        let Some(browser) = &call.browser_session else {
            return;
        };
        let Some(page_id) = result_page_id(result) else {
            return;
        };
        let Some(tab_groups) = catalog().into_iter().find(|tool| tool.name == "tab_groups") else {
            warn!("tab_groups tool missing from catalog");
            return;
        };
        let group_id = session.tab_group_ref().await;
        let created_new_group = group_id.is_none();
        let color = color_for_slug(session.agent().slug());
        let args = if let Some(group_id) = group_id {
            json!({ "action": "create", "groupId": group_id, "pages": [page_id] })
        } else {
            json!({
                "action": "create",
                "pages": [page_id],
                "title": session.agent().slug()
            })
        };
        let tool_ctx = ToolCtx::new(BrowserToolOptions {
            session: browser.clone(),
            defaults: BrowserToolDefaults::default(),
            cancel: call.cancel.clone(),
            output_files,
        });
        match execute_tool(&tab_groups, args, &tool_ctx).await {
            Ok(group_result) if !group_result.is_error => {
                if let Some(group_id) = group_result
                    .structured_content
                    .as_ref()
                    .and_then(|value| value.get("group"))
                    .and_then(|value| value.get("groupId"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                {
                    session.set_tab_group_ref(Some(group_id.clone())).await;
                    session.set_tab_group_color(Some(color)).await;
                    if created_new_group {
                        match execute_tool(
                            &tab_groups,
                            json!({ "action": "update", "groupId": group_id, "color": color }),
                            &tool_ctx,
                        )
                        .await
                        {
                            Ok(result) if !result.is_error => {}
                            Ok(result) => warn!(
                                dispatch_id = %call.dispatch_id,
                                group_color = %color,
                                error = first_text(&result),
                                "tab group color lock returned error"
                            ),
                            Err(err) => warn!(
                                error = %err,
                                dispatch_id = %call.dispatch_id,
                                group_color = %color,
                                "tab group color lock failed"
                            ),
                        }
                    }
                }
            }
            Ok(group_result) => warn!(
                dispatch_id = %call.dispatch_id,
                error = first_text(&group_result),
                "tab group orchestration returned error"
            ),
            Err(err) => {
                warn!(error = %err, dispatch_id = %call.dispatch_id, "tab group orchestration failed")
            }
        }
    }
}

struct TabsResultFilter;

impl TabsResultFilter {
    async fn apply(session: &Arc<Session>, call: &McpToolCall, result: &mut ToolResult) {
        if result.is_error || !call.hooks.filter_tabs_list {
            return;
        }
        let owned = session.owned_pages().await;
        let Some(Value::Object(structured)) = result.structured_content.as_ref() else {
            return;
        };
        let Some(pages) = structured.get("pages").and_then(Value::as_array) else {
            return;
        };
        let surviving = pages
            .iter()
            .filter(|page| {
                page.get("page")
                    .and_then(Value::as_u64)
                    .and_then(|value| u32::try_from(value).ok())
                    .map(|page_id| owned.contains(&PageId(page_id)))
                    .unwrap_or(false)
            })
            .cloned()
            .collect::<Vec<_>>();
        let lines = surviving
            .iter()
            .filter_map(format_tab_line)
            .collect::<Vec<_>>();
        result.content = vec![ContentBlock::text(if lines.is_empty() {
            "(no open pages)".to_string()
        } else {
            lines.join("\n")
        })];
        result.structured_content = Some(json!({ "pages": surviving }));
        result.is_error = false;
    }
}

fn format_tab_line(page: &Value) -> Option<String> {
    let page_id = page.get("page").and_then(Value::as_u64)?;
    let url = page.get("url").and_then(Value::as_str).unwrap_or_default();
    let title = page
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if title.is_empty() {
        Some(format!("[{page_id}] {url}"))
    } else {
        Some(format!("[{page_id}] {url} ({title})"))
    }
}

fn first_text(result: &ToolResult) -> String {
    result
        .content
        .iter()
        .find_map(|block| match block {
            ContentBlock::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "unknown error".to_string())
}

async fn apply_post_execution_hooks(
    session: &Arc<Session>,
    call: &McpToolCall,
    result: &ToolResult,
) {
    if result.is_error {
        return;
    }
    if call.hooks.capture_new_page
        && let Some(page_id) = result_page_id(result)
    {
        session.add_owned_page(PageId(page_id)).await;
    }
    if call.hooks.close_page
        && let Some(page_id) = extract_page_id(call.hooks.accepts_page_arg, &call.raw_args)
    {
        session.remove_owned_page(&PageId(page_id)).await;
    }
}

fn hook_error(error: impl std::fmt::Display) -> McpHookError {
    McpHookError::new(error.to_string())
}
