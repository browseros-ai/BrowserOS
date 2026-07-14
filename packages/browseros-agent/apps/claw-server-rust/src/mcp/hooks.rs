use crate::{
    app::AppState,
    domain::{AgentRef, ClientInfo, DispatchId, Session, SessionId, color_for_slug},
    mcp::naming::{
        build_session_group_title, client_prefix_from_slug, desired_group_title,
        elicit_session_name, peer_elicit_session_name,
    },
    services::{
        audit::{DispatchResultSummary, RecordToolDispatchInput},
        tab_activity::RecordToolInput,
    },
};
use base64::Engine;
use browseros_core::{
    BrowserSession, PageId,
    screenshot::{ScreenshotCaptureOptions, ScreenshotFormat},
};
use browseros_mcp::{
    BrowserToolDefaults, BrowserToolOptions, McpBeforeToolResult, McpClientInfo, McpHookError,
    McpHookResult, McpHooks, McpSessionClosed, McpSessionStarted, McpToolCall, McpToolTiming,
    OutputFileAccess, ToolCtx, ToolResult, catalog, execute_tool, extract_page_id, result_page_id,
    output_file::create_browser_output_file_access,
};
use futures_util::future::BoxFuture;
use rmcp::{
    RoleServer,
    model::ContentBlock,
    service::{ElicitationMode, Peer},
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, debug, info_span, warn};

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
        if let Some(peer) = event.peer {
            SessionNamer::spawn(self.state.clone(), session, peer);
        }
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
        apply_post_execution_hooks(&self.state, &session, &call, result).await;
        TabsResultFilter::apply(&self.state, &session, &call, result).await;
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
        TabGroupOrchestrator::record(
            &self.state,
            &session,
            &call,
            result,
            call.output_files.clone(),
        )
        .await;
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
    let ownership = state.sessions.ownership();
    let agent_key = session.agent().ownership_key();
    if let Some(owner) = state.sessions.owner_of_page(&page_id).await {
        if page_missing_after_refresh(call.browser_session.as_ref(), &page_id).await {
            ownership.remove_page(&page_id).await;
        } else if owner != agent_key {
            return Some(ToolResult::error(format!(
                "page {} is not owned by this agent; call `tabs` with action=\"new\" to open a fresh page and use the returned page id.",
                page_id.0
            )));
        } else {
            return None;
        }
    }
    ownership.claim_page(agent_key, page_id).await;
    None
}

async fn page_missing_after_refresh(
    browser: Option<&Arc<BrowserSession>>,
    page_id: &PageId,
) -> bool {
    let Some(browser) = browser else {
        return false;
    };
    if browser.pages.get_info(page_id.clone()).await.is_some() {
        return false;
    }
    match browser.pages.list().await {
        Ok(pages) => !pages.iter().any(|page| page.page_id == *page_id),
        Err(err) => {
            warn!(error = %err, page_id = page_id.0, "page ownership stale-prune refresh failed");
            false
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
        let page_id = result_page_id(result)
            .or_else(|| extract_page_id(call.hooks.accepts_page_arg, &call.raw_args));
        if wrote {
            // A tool-carried image counts as the page's visual anchor, so
            // later read-only dispatches skip the fallback capture.
            if let Some(page_id) = page_id {
                session.mark_first_capture_done(PageId(page_id)).await;
            }
            return;
        }
        // Fallback capture is an opt-out feature (CLAW_SCREENCAST_SCREENSHOT_FALLBACK);
        // disabling it must not affect the explicit tool-image branch above.
        if !state.config.screencast_screenshot_fallback {
            return;
        }
        let Some(browser) = &call.browser_session else {
            return;
        };
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

/// Dispatches one `tab_groups` call through the shared tool framework,
/// folding tool-missing, transport, and tool-error outcomes into Err(reason).
async fn dispatch_tab_groups(
    browser: &Arc<BrowserSession>,
    cancel: CancellationToken,
    output_files: OutputFileAccess,
    args: Value,
) -> Result<ToolResult, String> {
    let Some(tab_groups) = catalog().into_iter().find(|tool| tool.name == "tab_groups") else {
        return Err("tab_groups tool missing from catalog".to_string());
    };
    let tool_ctx = ToolCtx::new(BrowserToolOptions {
        session: browser.clone(),
        defaults: BrowserToolDefaults::default(),
        cancel,
        output_files,
    });
    match execute_tool(&tab_groups, args, &tool_ctx).await {
        Ok(result) if !result.is_error => Ok(result),
        Ok(result) => Err(first_text(&result)),
        Err(err) => Err(err.to_string()),
    }
}

struct SessionNamer;

impl SessionNamer {
    /// Fire-and-forget session-name elicitation; must never block or fail
    /// initialize, so every failure path is a log line at most.
    fn spawn(state: AppState, session: Arc<Session>, peer: Peer<RoleServer>) {
        if !peer
            .supported_elicitation_modes()
            .contains(&ElicitationMode::Form)
        {
            return;
        }
        tokio::spawn(async move {
            Self::run(state, session, peer).await;
        });
    }

    async fn run(state: AppState, session: Arc<Session>, peer: Peer<RoleServer>) {
        let prefix = client_prefix_from_slug(session.agent().slug()).to_string();
        // Session teardown cancels the elicitation so an abandoned prompt
        // does not park this task (and the peer handle) for the full 120s.
        let name = tokio::select! {
            name = elicit_session_name(|| peer_elicit_session_name(&peer, &prefix)) => name,
            () = session.child_token().cancelled_owned() => {
                debug!(session_id = %session.id(), "session closed during naming elicitation");
                return;
            }
        };
        let Some(name) = name else {
            return;
        };
        // The idle sweeper may have torn the session down mid-elicitation.
        if state.sessions.lookup(session.id()).await.is_none() {
            debug!(session_id = %session.id(), "session closed before naming applied");
            return;
        }
        session.set_session_label(name.clone()).await;
        let title = build_session_group_title(&prefix, &name);
        tracing::info!(session_id = %session.id(), title = %title, "mcp session named");
        Self::retitle_existing_group(&state, &session, &title).await;
    }

    /// Late-name path: the tab group already exists, so push the new title.
    /// When no group exists yet, the create path reads the label instead.
    async fn retitle_existing_group(state: &AppState, session: &Arc<Session>, title: &str) {
        let Some(group_id) = state
            .sessions
            .ownership()
            .tab_group_ref(&session.agent().ownership_key())
            .await
        else {
            return;
        };
        let Some(browser) = state.browser.session().await else {
            return;
        };
        if let Err(reason) = dispatch_tab_groups(
            &browser,
            session.child_token(),
            create_browser_output_file_access(),
            json!({ "action": "update", "groupId": group_id, "title": title }),
        )
        .await
        {
            warn!(
                session_id = %session.id(),
                error = %reason,
                "session name tab group retitle failed"
            );
        }
    }
}

struct TabGroupOrchestrator;

impl TabGroupOrchestrator {
    async fn record(
        state: &AppState,
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
        let ownership = state.sessions.ownership();
        let agent_key = session.agent().ownership_key();
        let group_id = ownership.tab_group_ref(&agent_key).await;
        let color = ownership
            .tab_group_color(&agent_key)
            .await
            .unwrap_or_else(|| color_for_slug(session.agent().slug()));
        // creation_title is Some only when this call creates the group.
        let (args, creation_title) = if let Some(group_id) = group_id {
            (
                json!({ "action": "create", "groupId": group_id, "pages": [page_id] }),
                None,
            )
        } else {
            let title = desired_group_title(session).await;
            (
                json!({ "action": "create", "pages": [page_id], "title": title }),
                Some(title),
            )
        };
        let group_result =
            match dispatch_tab_groups(browser, call.cancel.clone(), output_files.clone(), args)
                .await
            {
                Ok(result) => result,
                Err(reason) => {
                    warn!(
                        dispatch_id = %call.dispatch_id,
                        error = %reason,
                        "tab group orchestration failed"
                    );
                    return;
                }
            };
        let Some(group_id) = group_result
            .structured_content
            .as_ref()
            .and_then(|value| value.get("group"))
            .and_then(|value| value.get("groupId"))
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            return;
        };
        ownership
            .set_tab_group(agent_key, Some(group_id.clone()), Some(color))
            .await;
        let Some(creation_title) = creation_title else {
            return;
        };
        // Lock the colour separately: `tab_groups create` does not accept a
        // colour today; update does. A failure leaves the default colour.
        if let Err(reason) = dispatch_tab_groups(
            browser,
            call.cancel.clone(),
            output_files.clone(),
            json!({ "action": "update", "groupId": group_id, "color": color }),
        )
        .await
        {
            warn!(
                dispatch_id = %call.dispatch_id,
                group_color = %color,
                error = %reason,
                "tab group color lock failed"
            );
        }
        // Dedicated late-title apply, independent of the color lock: a label
        // that landed while the create was in flight must not be lost to a
        // color failure (mirrors the TS applyAgentTabGroupTitle update).
        let desired_title = desired_group_title(session).await;
        if desired_title != creation_title
            && let Err(reason) = dispatch_tab_groups(
                browser,
                call.cancel.clone(),
                output_files,
                json!({ "action": "update", "groupId": group_id, "title": desired_title }),
            )
            .await
        {
            warn!(
                dispatch_id = %call.dispatch_id,
                error = %reason,
                "tab group late title apply failed"
            );
        }
    }
}

struct TabsResultFilter;

impl TabsResultFilter {
    async fn apply(
        state: &AppState,
        session: &Arc<Session>,
        call: &McpToolCall,
        result: &mut ToolResult,
    ) {
        if result.is_error || !call.hooks.filter_tabs_list {
            return;
        }
        let Some(Value::Object(structured)) = result.structured_content.as_ref() else {
            return;
        };
        let Some(pages) = structured.get("pages").and_then(Value::as_array) else {
            return;
        };
        let live_page_ids = pages
            .iter()
            .filter_map(|page| {
                page.get("page")
                    .and_then(Value::as_u64)
                    .and_then(|value| u32::try_from(value).ok())
                    .map(PageId)
            })
            .collect::<BTreeSet<_>>();
        let ownership = state.sessions.ownership();
        ownership.prune_missing_pages(&live_page_ids).await;
        let owned = ownership
            .owned_pages(&session.agent().ownership_key())
            .await;
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
    state: &AppState,
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
        state
            .sessions
            .ownership()
            .claim_page(session.agent().ownership_key(), PageId(page_id))
            .await;
    }
    if call.hooks.close_page
        && let Some(page_id) = extract_page_id(call.hooks.accepts_page_arg, &call.raw_args)
    {
        let page_id = PageId(page_id);
        state.sessions.ownership().remove_page(&page_id).await;
        session.forget_first_capture(&page_id).await;
    }
}

fn hook_error(error: impl std::fmt::Display) -> McpHookError {
    McpHookError::new(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        domain::{AgentId, AgentRef},
    };
    use browseros_mcp::ToolCallHooks;
    use std::{collections::HashSet, path::PathBuf, time::Duration};
    use tempfile::TempDir;
    use tokio::sync::Mutex;

    struct TestState {
        state: AppState,
        _dir: TempDir,
    }

    async fn test_state() -> anyhow::Result<TestState> {
        test_state_with_fallback(true).await
    }

    async fn test_state_with_fallback(
        screencast_screenshot_fallback: bool,
    ) -> anyhow::Result<TestState> {
        let dir = tempfile::tempdir()?;
        let root = dir.path().join("browserclaw");
        let config = Arc::new(Config {
            server_port: 9200,
            cdp_port: 49337,
            proxy_port: None,
            resources_dir: dir.path().join("resources"),
            browserclaw_dir: root.clone(),
            claw_dir: root,
            session_idle: Duration::from_secs(300),
            session_sweep_interval: Duration::from_secs(60),
            screencast_screenshot_fallback,
            dev_mode: false,
            auth_token: None,
        });
        let state = AppState::new_with_home(config, None, dir.path().join("home")).await?;
        Ok(TestState { state, _dir: dir })
    }

    fn test_session(session_id: &str, agent_id: &str, slug: &str) -> Arc<Session> {
        Session::new(
            SessionId::new(session_id),
            AgentRef::Ephemeral {
                agent_id: AgentId::new(agent_id),
                slug: slug.to_string(),
                label: slug.to_string(),
            },
            tokio::time::Instant::now(),
        )
    }

    fn output_files() -> OutputFileAccess {
        Arc::new(Mutex::new(HashSet::<PathBuf>::new()))
    }

    fn page_call(page_id: u32) -> McpToolCall {
        McpToolCall {
            session_id: "session".to_string(),
            dispatch_id: "dispatch".to_string(),
            tool_name: "navigate",
            raw_args: json!({ "page": page_id }),
            hooks: ToolCallHooks {
                accepts_page_arg: true,
                ..ToolCallHooks::default()
            },
            browser_session: None,
            cancel: CancellationToken::new(),
            output_files: output_files(),
        }
    }

    fn tabs_list_call() -> McpToolCall {
        McpToolCall {
            session_id: "session".to_string(),
            dispatch_id: "dispatch".to_string(),
            tool_name: "tabs",
            raw_args: json!({ "action": "list" }),
            hooks: ToolCallHooks {
                filter_tabs_list: true,
                ..ToolCallHooks::default()
            },
            browser_session: None,
            cancel: CancellationToken::new(),
            output_files: output_files(),
        }
    }

    fn tabs_close_call(page_id: u32) -> McpToolCall {
        McpToolCall {
            session_id: "session".to_string(),
            dispatch_id: "dispatch".to_string(),
            tool_name: "tabs",
            raw_args: json!({ "action": "close", "page": page_id }),
            hooks: ToolCallHooks {
                accepts_page_arg: true,
                close_page: true,
                ..ToolCallHooks::default()
            },
            browser_session: None,
            cancel: CancellationToken::new(),
            output_files: output_files(),
        }
    }

    #[tokio::test]
    async fn tabs_filter_reuses_same_slug_pages_after_reconnect() -> anyhow::Result<()> {
        let app = test_state().await?;
        let session1 = test_session("s1", "codex-a", "codex");
        let session2 = test_session("s2", "codex-b", "codex");
        let key1 = session1.agent().ownership_key();
        let key2 = session2.agent().ownership_key();
        app.state
            .sessions
            .ownership()
            .claim_page(key1.clone(), PageId(1))
            .await;
        app.state
            .sessions
            .ownership()
            .claim_page(key1.clone(), PageId(2))
            .await;
        app.state
            .sessions
            .ownership()
            .set_tab_group(
                key1,
                Some("group-1".to_string()),
                Some(crate::domain::TabGroupColor::Purple),
            )
            .await;

        let mut result = ToolResult::text(
            "[1] https://one\n[2] https://two\n[3] https://three",
            Some(json!({
                "pages": [
                    { "page": 1, "url": "https://one", "title": "One" },
                    { "page": 2, "url": "https://two", "title": "Two" },
                    { "page": 3, "url": "https://three", "title": "Three" }
                ]
            })),
        );
        TabsResultFilter::apply(&app.state, &session2, &tabs_list_call(), &mut result).await;

        assert_eq!(key2.as_str(), "codex");
        assert_eq!(
            result
                .structured_content
                .as_ref()
                .and_then(|value| value.get("pages").and_then(Value::as_array).map(Vec::len)),
            Some(2)
        );
        assert_eq!(
            app.state
                .sessions
                .ownership()
                .tab_group_ref(&key2)
                .await
                .as_deref(),
            Some("group-1")
        );
        assert_eq!(
            first_text(&result),
            "[1] https://one (One)\n[2] https://two (Two)"
        );
        Ok(())
    }

    #[tokio::test]
    async fn page_ownership_guard_denies_different_agent_key_with_existing_message()
    -> anyhow::Result<()> {
        let app = test_state().await?;
        let cowork = test_session("cowork-session", "cowork-a", "cowork");
        let codex = test_session("codex-session", "codex-a", "codex");
        app.state
            .sessions
            .ownership()
            .claim_page(cowork.agent().ownership_key(), PageId(7))
            .await;

        let result = page_ownership_guard(&app.state, &codex, &page_call(7))
            .await
            .ok_or_else(|| anyhow::anyhow!("expected ownership denial"))?;

        assert!(result.is_error);
        assert_eq!(
            first_text(&result),
            "page 7 is not owned by this agent; call `tabs` with action=\"new\" to open a fresh page and use the returned page id."
        );
        Ok(())
    }

    #[tokio::test]
    async fn page_ownership_guard_allows_same_slug_concurrent_sessions() -> anyhow::Result<()> {
        let app = test_state().await?;
        let first = test_session("s1", "codex-a", "codex");
        let second = test_session("s2", "codex-b", "codex");

        assert!(
            page_ownership_guard(&app.state, &first, &page_call(4))
                .await
                .is_none()
        );
        assert!(
            page_ownership_guard(&app.state, &second, &page_call(4))
                .await
                .is_none()
        );
        assert_eq!(
            app.state.sessions.owner_of_page(&PageId(4)).await,
            Some(second.agent().ownership_key())
        );
        Ok(())
    }

    #[tokio::test]
    async fn persist_writes_explicit_tool_image_when_fallback_disabled() -> anyhow::Result<()> {
        let app = test_state_with_fallback(false).await?;
        let session = test_session("s1", "codex-a", "codex");
        let call = page_call(1);
        // "anBlZw==" is base64 for "jpeg".
        let result = ToolResult::image("anBlZw==", "image/jpeg", json!({}));

        ScreenshotPersister::persist(
            &app.state,
            &session,
            &call,
            &result,
            AuditRecord { row_id: 7 },
            output_files(),
        )
        .await;

        assert_eq!(app.state.screenshots.read("7").await?, b"jpeg");
        assert_eq!(app.state.screenshots.read("dispatch").await?, b"jpeg");
        assert!(
            session.has_first_capture(&PageId(1)).await,
            "tool-carried image should count as the page's first capture"
        );
        Ok(())
    }

    #[tokio::test]
    async fn close_page_hook_removes_agent_owned_page() -> anyhow::Result<()> {
        let app = test_state().await?;
        let session = test_session("s1", "codex-a", "codex");
        let page_id = PageId(9);
        app.state
            .sessions
            .ownership()
            .claim_page(session.agent().ownership_key(), page_id.clone())
            .await;
        session.mark_first_capture_done(page_id.clone()).await;
        let result = ToolResult::text("closed page 9", Some(json!({ "page": 9 })));

        apply_post_execution_hooks(&app.state, &session, &tabs_close_call(9), &result).await;

        assert_eq!(app.state.sessions.owner_of_page(&page_id).await, None);
        assert!(!session.has_first_capture(&page_id).await);
        Ok(())
    }
}
