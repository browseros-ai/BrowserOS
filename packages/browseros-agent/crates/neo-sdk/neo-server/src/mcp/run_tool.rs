//! The `run` tool's definition, and a run's reply as MCP carries it.

use rmcp::handler::server::common::schema_for_type;
use rmcp::model::{CallToolResult, ContentBlock, MetaObject, Tool};
use serde_json::{Map, Value, json};

use super::RunArgs;
use super::run_description::run_description;
use crate::config::Defaults;
use crate::dispatch::Registry;
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
    lines.join("\n")
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
    use crate::run::RunReply;

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
}
