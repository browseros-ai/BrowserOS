//! The `run` tool's definition, and a run's reply as MCP carries it.

use rmcp::handler::server::common::schema_for_type;
use rmcp::model::{CallToolResult, ContentBlock, MetaObject, Tool};
use serde_json::{Map, Value, json};

use super::RunArgs;
use super::run_description::run_description;
use crate::config::Defaults;
use crate::dispatch::{Registry, TaskKind, TaskNote};
use crate::run::{OpNote, RUN_META_KEY, ReplyError, RunReply};
use crate::runtime::{LogLevel, LogLine};

/// The one tool, built once at startup (the registry is frozen). Protocol translation only:
/// every field of a result was decided by the run service, so a replayed reply renders exactly
/// as the first one did.
#[derive(Debug)]
pub(crate) struct RunTool {
    tool: Tool,
}

impl RunTool {
    /// The one tool's name.
    pub(crate) const NAME: &'static str = "run";

    /// `run { code, timeout? }`, its schema from [`RunArgs`] and its description from the
    /// registry's listed methods.
    #[must_use]
    pub(crate) fn new(registry: &Registry) -> Self {
        Self {
            tool: Tool::new(
                Self::NAME,
                run_description(registry),
                schema_for_type::<RunArgs>(),
            ),
        }
    }

    /// The definition, for `tools/list` and for rmcp's per-tool header checks.
    #[must_use]
    pub(crate) fn tool(&self) -> Tool {
        self.tool.clone()
    }

    /// A run's reply as an MCP result:
    /// - text: the value as JSON, or the error line; then the console lines and one line per
    ///   write that was never confirmed or op the script left running;
    /// - `structuredContent`: `{ "value": … }`, or `{ "error": … }`;
    /// - `_meta["com.browseros.neo/run"]`: `{ id, status, effects, error?, unknown }`, the
    ///   committed contract's names (MIGRATION.md: the run id is `.id`);
    /// - `isError` unless the status is `ok`. A failed run is a tool result, never a JSON-RPC
    ///   error, so the model reads why.
    #[must_use]
    pub(crate) fn result(reply: &RunReply) -> CallToolResult {
        let mut result = match &reply.error {
            None => CallToolResult::structured(json!({ "value": reply.value })),
            Some(error) => CallToolResult::structured_error(json!({ "error": error })),
        };
        result.content = vec![ContentBlock::text(text(reply))];
        let mut meta = MetaObject::new();
        meta.insert(RUN_META_KEY.to_owned(), run_meta(reply));
        result.with_meta(Some(meta))
    }
}

/// `_meta["com.browseros.neo/run"]` of a result.
fn run_meta(reply: &RunReply) -> Value {
    let mut meta = Map::new();
    meta.insert("id".to_owned(), json!(reply.id));
    meta.insert("status".to_owned(), json!(reply.status));
    meta.insert("effects".to_owned(), json!(reply.effects));
    if let Some(error) = &reply.error {
        meta.insert("error".to_owned(), json!(error));
    }
    let unknown: Vec<Value> = reply
        .unknown
        .iter()
        .map(|note| json!({ "operationId": note.operation_id, "name": note.name }))
        .collect();
    meta.insert("unknown".to_owned(), Value::Array(unknown));
    meta.insert("tasks".to_owned(), json!(reply.tasks));
    Value::Object(meta)
}

/// What the model reads.
fn text(reply: &RunReply) -> String {
    let mut lines = vec![match &reply.error {
        None => reply.value.to_string(),
        Some(error) => error_line(error),
    }];
    if !reply.logs.is_empty() || reply.logs_dropped > 0 {
        lines.push("console:".to_owned());
        lines.extend(reply.logs.iter().map(log_line));
        if reply.logs_dropped > 0 {
            lines.push(format!(
                "[{} more console lines were dropped: a run keeps {} lines and {} KiB]",
                reply.logs_dropped,
                Defaults::MAX_LOG_LINES,
                Defaults::MAX_LOG_BYTES / 1024
            ));
        }
    }
    for note in &reply.unknown {
        lines.push(format!(
            "[unknown outcome] {} was sent but not confirmed. Take a snapshot before repeating it.",
            op_label(note)
        ));
    }
    for note in &reply.not_awaited {
        lines.push(format!(
            "[not awaited] {} was still running when the script returned; it was cancelled.",
            op_label(note)
        ));
    }
    lines.extend(reply.tasks.iter().map(task_line));
    lines.join("\n")
}

fn task_line(task: &TaskNote) -> String {
    let id = &task.id;
    if task.kind == TaskKind::Closed {
        return format!("[task {id} closed · its tabs were closed]");
    }
    let mut tabs: Vec<String> = task
        .tabs
        .iter()
        .take(10)
        .map(|tab| {
            let mut url: String = tab.url.chars().take(120).collect();
            if tab.url.chars().count() > 120 {
                url.push('…');
            }
            format!("{} {url}", tab.tab)
        })
        .collect();
    if task.tabs.len() > 10 {
        tabs.push(format!("+{} more", task.tabs.len() - 10));
    }
    let tabs = if tabs.is_empty() {
        "none".to_owned()
    } else {
        tabs.join(", ")
    };
    match task.kind {
        TaskKind::BuiltinPage => {
            format!("[new task {id} was created for this script's built-in page · tabs: {tabs}]")
        }
        TaskKind::Created | TaskKind::Fetched => format!("[task {id} · tabs: {tabs}]"),
        TaskKind::Closed => format!("[task {id} closed · its tabs were closed]"),
    }
}

/// `script / script_error: TypeError: x is not a function (run:2:5)`, plus the op and its
/// delivery for an op's error that escaped the script.
fn error_line(error: &ReplyError) -> String {
    let line = format!(
        "{} / {}: {}",
        error.owner.as_str(),
        error.code,
        error.message
    );
    match (error.operation_id, error.delivery) {
        (Some(operation), Some(delivery)) => format!(
            "{line} (op {}, delivery {})",
            operation.seq,
            delivery.as_str()
        ),
        _ => line,
    }
}

fn log_line(line: &LogLine) -> String {
    match line.level {
        LogLevel::Log => line.text.clone(),
        LogLevel::Info => format!("[info] {}", line.text),
        LogLevel::Warn => format!("[warn] {}", line.text),
        LogLevel::Error => format!("[error] {}", line.text),
        LogLevel::Debug => format!("[debug] {}", line.text),
    }
}

/// `page.goto at run:3:7 (op 9)`.
fn op_label(note: &OpNote) -> String {
    match &note.site {
        Some(site) => format!("{} at {site} (op {})", note.name, note.operation_id.seq),
        None => format!("{} (op {})", note.name, note.operation_id.seq),
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use neo_trace::{OperationId, RunId};
    use serde_json::json;

    use super::text;
    use crate::run::{RUN_META_KEY, RunReply};
    use crate::test_rig::{self, TestError, TestRig};

    #[test]
    fn a_write_never_confirmed_tells_the_model_to_look_before_repeating_it()
    -> Result<(), Box<dyn Error>> {
        let id = RunId::new();
        let reply: RunReply = serde_json::from_value(json!({
            "id": id,
            "status": "ok",
            "effects": "unknown",
            "value": null,
            "unknown": [{
                "operationId": OperationId::new(id, 7),
                "name": "locator.click",
                "site": "run:3:7",
            }],
        }))?;
        assert_eq!(
            text(&reply).lines().last(),
            Some(
                "[unknown outcome] locator.click at run:3:7 (op 7) was sent but not confirmed. \
                 Take a snapshot before repeating it."
            ),
            "sdk-traces' wording, which zuzu's own warning used"
        );
        Ok(())
    }
    #[test]
    fn task_lines_bound_urls_and_tab_counts_and_closed_tasks_have_no_tabs()
    -> Result<(), Box<dyn Error>> {
        let id = RunId::new();
        let tabs: Vec<_> = (1..=12)
            .map(|tab| json!({ "tabId": tab, "url": "é".repeat(121) }))
            .collect();
        let reply: RunReply = serde_json::from_value(json!({
            "id": id, "status": "ok", "effects": "none", "value": null,
            "tasks": [
                { "id": "one", "name": null, "kind": "fetched", "tabs": tabs },
                { "id": "two", "name": null, "kind": "closed", "tabs": [] }
            ]
        }))?;
        let text = text(&reply);
        assert!(text.contains(&format!("1 {}…", "é".repeat(120))));
        assert!(text.contains(", +2 more]"));
        assert!(!text.contains("11 é"));
        assert!(text.ends_with("[task two closed · its tabs were closed]"));
        let wire = serde_json::to_value(super::RunTool::result(&reply))?;
        assert_eq!(
            wire["_meta"][RUN_META_KEY]["tasks"][0]["tabs"]
                .as_array()
                .map(Vec::len),
            Some(12),
            "only the text is clipped"
        );
        Ok(())
    }
    #[test]
    fn an_auto_task_is_named_in_reply_text_metadata_and_replay() -> Result<(), TestError> {
        use crate::run::{RunMeta, RunRequest, RunTimeout};
        use neo_trace::RequestId;
        use std::time::Duration;

        test_rig::on_neo_runtime(async {
            let rig = TestRig::new().await?;
            let service = rig.run_service(Duration::from_secs(30));
            let make_request = || -> Result<RunRequest, TestError> {
                Ok(RunRequest {
                    code: "await __neo.invoke('test.touch', { page }); throw new Error('after opening');".to_owned(),
                    timeout: RunTimeout::from_millis(None)?,
                    meta: RunMeta { request_id: Some(RequestId::parse("auto-reply")?), ..RunMeta::default() },
                })
            };
            let reply = service.run(make_request()?).await;
            assert_eq!(reply.tasks.len(), 1);
            let task = &reply.tasks[0];
            let mcp = serde_json::to_value(super::RunTool::result(&reply))?;
            assert!(mcp["content"][0]["text"].as_str().ok_or("text")?.ends_with(&format!(
                "[new task {} was created for this script's built-in page · tabs: {} about:blank]",
                task.id, task.tabs[0].tab,
            )));
            assert_eq!(
                mcp["_meta"][RUN_META_KEY]["tasks"],
                serde_json::to_value(&reply.tasks)?
            );
            assert_eq!(mcp["structuredContent"], json!({ "error": reply.error }));
            let replay = service.run(make_request()?).await;
            assert_eq!(serde_json::to_value(super::RunTool::result(&replay))?, mcp);
            Ok(())
        })
    }
}
