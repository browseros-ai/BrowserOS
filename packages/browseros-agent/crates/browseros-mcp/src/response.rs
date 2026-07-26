//! Builds a tool's primary result, then appends registered post-action context in order.
//! Post-actions are best-effort: failures and timeouts add unavailable context without changing
//! primary success, while cancellation terminates response assembly.

use crate::{
    constants::TOOL_POST_ACTION_CAPTURE_TIMEOUT,
    format::{diff::format_diff_result, snapshot::format_snapshot_result},
    framework::{ToolCtx, ToolError, ToolExecResult, ToolResult, merge_structured},
};
use browseros_core::{ConsoleEntry, PageId, settle::SettleOutcome};
use rmcp::model::ContentBlock;
use serde_json::{Value, json};

#[derive(Debug, Clone)]
enum PostAction {
    Snapshot { page: u32 },
    Diff { page: u32, include_structured: bool },
    Pages,
    Screenshot { page: u32 },
}

#[derive(Debug, Clone)]
struct ConsoleSummary {
    page: u32,
    since: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ToolResponse {
    content: Vec<ContentBlock>,
    has_error: bool,
    structured_content: Option<Value>,
    post_actions: Vec<PostAction>,
    console_summaries: Vec<ConsoleSummary>,
}

#[derive(Debug, Clone)]
pub struct BuiltToolResponse {
    pub content: Vec<ContentBlock>,
    pub is_error: bool,
    pub structured_content: Option<Value>,
    pub metadata_tab_id: Option<i64>,
}

impl ToolResponse {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text(&mut self, value: impl Into<String>) {
        self.content.push(ContentBlock::text(value));
    }

    pub fn image(&mut self, data: impl Into<String>, mime_type: impl Into<String>) {
        self.content.push(ContentBlock::image(data, mime_type));
    }

    pub fn error(&mut self, message: impl Into<String>) {
        self.has_error = true;
        self.text(message);
    }

    pub fn data(&mut self, value: Value) {
        merge_structured(&mut self.structured_content, value);
    }

    pub fn append_result(&mut self, result: ToolResult) {
        self.content.extend(result.content);
        if result.is_error {
            self.has_error = true;
        }
        if let Some(structured) = result.structured_content {
            self.data(structured);
        }
    }

    pub fn include_snapshot(&mut self, page: u32) {
        self.post_actions.push(PostAction::Snapshot { page });
    }

    pub fn include_diff(&mut self, page: u32, include_structured: bool) {
        self.post_actions.push(PostAction::Diff {
            page,
            include_structured,
        });
    }

    pub fn include_pages(&mut self) {
        self.post_actions.push(PostAction::Pages);
    }

    pub fn include_screenshot(&mut self, page: u32) {
        self.post_actions.push(PostAction::Screenshot { page });
    }

    pub fn include_console_summary(&mut self, page: u32, since: u64) {
        self.console_summaries.push(ConsoleSummary { page, since });
    }

    pub async fn build_for_session(
        &mut self,
        ctx: &ToolCtx,
        primary_page: Option<PageId>,
    ) -> ToolExecResult<BuiltToolResponse> {
        if !self.post_actions.is_empty() {
            self.text("\n--- Additional context (auto-included) ---");
        }

        for action in self.post_actions.clone() {
            ctx.throw_if_cancelled()?;
            self.settle_for_post_action(&action, ctx).await?;
            let result = tokio::select! {
                () = ctx.cancel.cancelled() => return Err(ToolError::Cancelled),
                result = tokio::time::timeout(
                    TOOL_POST_ACTION_CAPTURE_TIMEOUT,
                    self.run_session_post_action(action.clone(), ctx)
                ) => result,
            };
            match result {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    tracing::debug!("browser MCP post-action failed: {err}");
                    self.text(action.unavailable_text(err.to_string()));
                }
                Err(_elapsed) => {
                    let reason = format!(
                        "timed out after {}ms",
                        TOOL_POST_ACTION_CAPTURE_TIMEOUT.as_millis()
                    );
                    tracing::debug!(
                        "browser MCP post-action timed out: {}",
                        action.unavailable_text(&reason)
                    );
                    self.text(action.unavailable_text(reason));
                }
            }
        }

        for summary in self.console_summaries.clone() {
            self.append_console_summary(ctx, summary);
        }
        self.prepend_page_signal_lines(ctx, primary_page);

        Ok(BuiltToolResponse {
            content: self.content.clone(),
            is_error: self.has_error,
            structured_content: self.structured_content.clone(),
            metadata_tab_id: None,
        })
    }

    async fn run_session_post_action(
        &mut self,
        action: PostAction,
        ctx: &ToolCtx,
    ) -> ToolExecResult<()> {
        match action {
            PostAction::Snapshot { page } => {
                self.throw_if_dialog_open(ctx, page)?;
                let observer = ctx.session.observe(browseros_core::PageId(page)).await;
                let snapshot = observer.snapshot().await?;
                let formatted = format_snapshot_result(&snapshot.text, &snapshot.url, ctx).await;
                self.text(format!("[Page {page} snapshot]\n{}", formatted.text));
                Ok(())
            }
            PostAction::Diff {
                page,
                include_structured,
            } => {
                self.throw_if_dialog_open(ctx, page)?;
                let diff = ctx
                    .session
                    .observe(browseros_core::PageId(page))
                    .await
                    .diff()
                    .await?;
                let origin = diff.after_url.as_deref().map(ToString::to_string);
                let origin = match origin {
                    Some(origin) => origin,
                    None => ctx
                        .session
                        .pages
                        .get_info(browseros_core::PageId(page))
                        .await
                        .map(|info| info.url)
                        .unwrap_or_else(|| "unknown".to_string()),
                };
                let formatted = format_diff_result(&diff, &origin, ctx).await;
                self.text(format!("[Page {page} diff]\n{}", formatted.text));
                if include_structured {
                    let mut structured = json!({ "changed": diff.changed });
                    if diff.url_changed
                        && let Value::Object(object) = &mut structured
                    {
                        object.insert("urlChanged".to_string(), Value::Bool(true));
                        object.insert(
                            "beforeUrl".to_string(),
                            diff.before_url.map(Value::String).unwrap_or(Value::Null),
                        );
                        object.insert(
                            "afterUrl".to_string(),
                            diff.after_url.map(Value::String).unwrap_or(Value::Null),
                        );
                    }
                    self.data(structured);
                }
                Ok(())
            }
            PostAction::Pages => {
                let pages = ctx.session.pages.list().await?;
                if pages.is_empty() {
                    self.text("[Open pages] None");
                    return Ok(());
                }
                let lines = pages
                    .iter()
                    .map(|page| {
                        format!(
                            "  {}. {} - {}{}",
                            page.page_id.0,
                            if page.title.is_empty() {
                                "(untitled)"
                            } else {
                                &page.title
                            },
                            page.url,
                            if page.is_active { " [ACTIVE]" } else { "" }
                        )
                    })
                    .collect::<Vec<_>>();
                self.text(format!("[Open pages]\n{}", lines.join("\n")));
                Ok(())
            }
            PostAction::Screenshot { page } => {
                let page_session = ctx
                    .session
                    .pages
                    .get_session(browseros_core::PageId(page))
                    .await?;
                #[derive(serde::Deserialize)]
                struct CaptureScreenshotResult {
                    data: String,
                }
                let result: CaptureScreenshotResult = page_session
                    .session
                    .send(
                        "Page.captureScreenshot",
                        json!({ "format": "png", "captureBeyondViewport": false }),
                    )
                    .await?;
                self.text(format!("[Page {page} screenshot]"));
                self.image(result.data, "image/png");
                Ok(())
            }
        }
    }

    async fn settle_for_post_action(
        &self,
        action: &PostAction,
        ctx: &ToolCtx,
    ) -> ToolExecResult<()> {
        let Some(page) = action.settle_page() else {
            return Ok(());
        };
        if ctx
            .session
            .page_signals
            .pending_dialog_line(&PageId(page))
            .is_some()
        {
            tracing::debug!("browser MCP post-action settle skipped for page {page}: dialog open");
            return Ok(());
        }
        let outcome = tokio::select! {
            () = ctx.cancel.cancelled() => return Err(ToolError::Cancelled),
            outcome = browseros_core::settle::wait_for_action_settle(
                ctx.session.pages.as_ref(),
                PageId(page),
                browseros_core::timeouts::ACTION_SETTLE_DEFAULT_TIMEOUT,
            ) => outcome,
        };
        match outcome {
            SettleOutcome::Settled { .. } => {}
            SettleOutcome::BudgetExpired => {
                tracing::debug!("browser MCP post-action settle budget expired for page {page}");
            }
            SettleOutcome::Skipped { reason } => {
                tracing::debug!("browser MCP post-action settle skipped for page {page}: {reason}");
            }
        }
        Ok(())
    }

    fn append_console_summary(&mut self, ctx: &ToolCtx, summary: ConsoleSummary) {
        let entries = ctx
            .session
            .page_signals
            .console_entries_since(&PageId(summary.page), summary.since);
        let Some(line) = console_summary_line(summary.page, &entries) else {
            return;
        };
        self.text(line);
    }

    fn prepend_page_signal_lines(&mut self, ctx: &ToolCtx, primary_page: Option<PageId>) {
        let mut pages = Vec::new();
        if let Some(page) = primary_page {
            push_page_once(&mut pages, page);
        }
        if let Some(page) = self.structured_page_id() {
            push_page_once(&mut pages, page);
        }
        for action in &self.post_actions {
            if let Some(page) = action.page() {
                push_page_once(&mut pages, PageId(page));
            }
        }

        let mut lines = Vec::new();
        for page in pages {
            lines.extend(ctx.session.page_signals.take_alert_note_lines(&page));
            if let Some(line) = ctx.session.page_signals.pending_dialog_line(&page) {
                lines.push(line);
            }
        }
        self.prepend_lines(lines);
    }

    fn prepend_lines(&mut self, lines: Vec<String>) {
        if lines.is_empty() {
            return;
        }
        let existing_first_text: &str = self
            .content
            .first()
            .and_then(|content| content.as_text())
            .map(|content| content.text.as_ref())
            .unwrap_or_default();
        let missing = lines
            .into_iter()
            .filter(|line| !existing_first_text.starts_with(line))
            .collect::<Vec<_>>();
        if missing.is_empty() {
            return;
        }
        self.content
            .insert(0, ContentBlock::text(missing.join("\n")));
    }

    fn structured_page_id(&self) -> Option<PageId> {
        self.structured_content
            .as_ref()
            .and_then(|value| value.get("page"))
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .map(PageId)
    }

    fn throw_if_dialog_open(&self, ctx: &ToolCtx, page: u32) -> ToolExecResult<()> {
        match ctx.session.page_signals.pending_dialog_line(&PageId(page)) {
            Some(message) => Err(ToolError::message(message)),
            None => Ok(()),
        }
    }
}

impl BuiltToolResponse {
    #[must_use]
    pub fn into_tool_result(self) -> ToolResult {
        let _ = self.metadata_tab_id;
        ToolResult {
            content: self.content,
            is_error: self.is_error,
            structured_content: self.structured_content,
        }
    }
}

impl PostAction {
    fn settle_page(&self) -> Option<u32> {
        match self {
            Self::Snapshot { page } | Self::Diff { page, .. } => Some(*page),
            Self::Pages | Self::Screenshot { .. } => None,
        }
    }

    fn page(&self) -> Option<u32> {
        match self {
            Self::Snapshot { page } | Self::Diff { page, .. } | Self::Screenshot { page } => {
                Some(*page)
            }
            Self::Pages => None,
        }
    }

    fn unavailable_text(&self, reason: impl AsRef<str>) -> String {
        let reason = reason.as_ref();
        match self {
            Self::Snapshot { page } => format!("[page {page} snapshot unavailable: {reason}]"),
            Self::Diff { page, .. } => format!("[page {page} diff unavailable: {reason}]"),
            Self::Pages => format!("[pages unavailable: {reason}]"),
            Self::Screenshot { page } => format!("[page {page} screenshot unavailable: {reason}]"),
        }
    }
}

fn push_page_once(pages: &mut Vec<PageId>, page: PageId) {
    if !pages.contains(&page) {
        pages.push(page);
    }
}

fn console_summary_line(page: u32, entries: &[ConsoleEntry]) -> Option<String> {
    let first = entries.first()?;
    let noun = if entries.iter().all(ConsoleEntry::is_warning) {
        "warning"
    } else {
        "error"
    };
    let suffix = if entries.len() == 1 { "" } else { "s" };
    Some(format!(
        "[page {page} console] {} {noun}{suffix} during action, e.g.: {}",
        entries.len(),
        first.summary_text()
    ))
}
