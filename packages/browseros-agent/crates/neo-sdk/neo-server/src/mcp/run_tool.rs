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

/// Neo's sections, in wire order. Rendering, escaping and parsing share this list.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Section {
    Result,
    Error,
    Console,
    Notes,
    Tips,
}

impl Section {
    const ALL: [Self; 5] = [
        Self::Result,
        Self::Error,
        Self::Console,
        Self::Notes,
        Self::Tips,
    ];

    fn header(self) -> &'static str {
        match self {
            Self::Result => "### Result",
            Self::Error => "### Error",
            Self::Console => "### Console",
            Self::Notes => "### Notes",
            Self::Tips => "### Tips",
        }
    }

    fn from_header(line: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|section| section.header() == line)
    }
}

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
    /// - one text block: Result or Error, then Console, Notes and Tips when present;
    /// - no `structuredContent`: the returned value appears only in the text;
    /// - `_meta["com.browseros.neo/run"]`: `{ id, status, effects, error?, unknown, tasks }`, the
    ///   committed contract's names (MIGRATION.md: the run id is `.id`);
    /// - `isError` unless the status is `ok`. A failed run is a tool result, never a JSON-RPC
    ///   error, so the model reads why.
    #[must_use]
    pub(crate) fn result(reply: &RunReply) -> CallToolResult {
        let content = vec![ContentBlock::text(text(reply))];
        let result = match &reply.error {
            None => CallToolResult::success(content),
            Some(_) => CallToolResult::error(content),
        };
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
    let mut sections = Vec::new();
    for section in Section::ALL {
        let mut lines = Vec::new();
        match section {
            Section::Result => {
                if reply.error.is_none() {
                    lines.push(reply.value.to_string());
                }
            }
            Section::Error => {
                if let Some(error) = &reply.error {
                    lines.push(error_line(error));
                }
            }
            Section::Console => {
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
            Section::Notes => {
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
            }
            Section::Tips => {
                if reply.error.is_none() {
                    lines.extend(reply.tips.iter().map(|tip| tip.line().to_owned()));
                }
            }
        }
        if !lines.is_empty() {
            sections.push(format!(
                "{}\n{}",
                section.header(),
                escape_headers(&lines.join("\n"))
            ));
        }
    }
    sections.join("\n")
}

/// Untrusted messages cannot introduce a section of their own.
fn escape_headers(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            if Section::from_header(line).is_some() {
                format!("\\{line}")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The model-visible reply, decoded for tests without a second copy of the value on the wire.
#[cfg(feature = "test-support")]
#[derive(Debug, PartialEq)]
pub struct ReplyText {
    /// The script's JSON return value, or the full Error section.
    pub value: Result<Value, String>,
    /// Console lines, with markdown header escapes removed.
    pub console: Vec<String>,
    /// Warnings about unconfirmed writes and unfinished operations.
    pub notes: Vec<String>,
    /// Optional advice following a successful run.
    pub tips: Vec<String>,
}

/// Reads Neo's section format; malformed or unrelated text is not a run reply.
#[cfg(feature = "test-support")]
#[must_use]
pub fn parse_text(text: &str) -> Option<ReplyText> {
    let (header, body) = text.split_once('\n')?;
    let first = Section::from_header(header)?;
    if !matches!(first, Section::Result | Section::Error) {
        return None;
    }
    let mut current = first;
    let mut primary = Vec::new();
    let mut reply = ReplyText {
        value: Ok(Value::Null),
        console: Vec::new(),
        notes: Vec::new(),
        tips: Vec::new(),
    };
    for line in body.split('\n') {
        if let Some(section) = Section::from_header(line) {
            if section <= current || section == Section::Error {
                return None;
            }
            current = section;
            continue;
        }
        let line = line
            .strip_prefix('\\')
            .filter(|line| Section::from_header(line).is_some())
            .unwrap_or(line)
            .to_owned();
        match current {
            Section::Result | Section::Error => primary.push(line),
            Section::Console => reply.console.push(line),
            Section::Notes => reply.notes.push(line),
            Section::Tips => reply.tips.push(line),
        }
    }
    reply.value = if first == Section::Result {
        let [value] = primary.as_slice() else {
            return None;
        };
        Ok(serde_json::from_str(value).ok()?)
    } else {
        Err(primary.join("\n"))
    };
    Some(reply)
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

    use super::{RunTool, Section, parse_text, text};
    use crate::config::Defaults;
    use crate::run::{RUN_META_KEY, RunReply};
    use crate::test_rig::{self, TestError, TestRig};

    #[test]
    fn values_appear_once_as_compact_json_in_one_text_block() -> Result<(), Box<dyn Error>> {
        for value in [
            json!({ "a": 1 }),
            json!(null),
            json!(42),
            json!(true),
            json!([1, 2]),
            json!("- banner [ref=e1]:\n  - link \"Home\" [ref=e2]\n".repeat(10_000)),
        ] {
            let reply: RunReply = serde_json::from_value(json!({
                "id": RunId::new(), "status": "ok", "effects": "none", "value": value,
            }))?;
            let wire = serde_json::to_value(RunTool::result(&reply))?;
            let rendered = format!("### Result\n{value}");
            assert_eq!(
                wire["content"],
                json!([{ "type": "text", "text": rendered }])
            );
            assert!(
                wire.get("structuredContent").is_none(),
                "large values must travel once"
            );
            assert_eq!(wire["isError"], false);
            assert_eq!(
                wire["_meta"][RUN_META_KEY],
                json!({
                    "id": reply.id, "status": "ok", "effects": "none", "unknown": [], "tasks": [],
                })
            );
            let parsed = parse_text(&rendered).ok_or("a rendered result parses")?;
            assert_eq!(parsed.value, Ok(value));
            assert!(parsed.console.is_empty() && parsed.notes.is_empty() && parsed.tips.is_empty());
        }
        Ok(())
    }

    #[test]
    fn errors_and_console_cannot_forge_sections() -> Result<(), Box<dyn Error>> {
        let headers = Section::ALL.map(Section::header).join("\n");
        let id = RunId::new();
        let reply: RunReply = serde_json::from_value(json!({
            "id": id, "status": "error", "effects": "unknown",
            "error": { "owner": "script", "code": "script_error",
                "message": format!("failed\n{headers}\ncall log\n"),
                "operationId": OperationId::new(id, 9), "delivery": "unknown", "retryable": false },
            "logs": [ { "level": "log", "text": format!("hello\n{headers}\n") },
                { "level": "warn", "text": "slow" } ],
            "logsDropped": 3,
            "unknown": [{ "operationId": OperationId::new(id, 7), "name": "locator.click", "site": "run:3:7" }],
            "notAwaited": [{ "operationId": OperationId::new(id, 8), "name": "page.goto", "site": "run:4:1" }],
        }))?;
        let rendered = text(&reply);
        for header in Section::ALL.map(Section::header) {
            assert_eq!(
                rendered.matches(&format!("\\{header}\n")).count(),
                2,
                "{rendered}"
            );
        }
        let parsed = parse_text(&rendered).ok_or("escaped headers stay in their section")?;
        assert_eq!(
            parsed.value,
            Err(format!(
                "script / script_error: failed\n{headers}\ncall log\n (op 9, delivery unknown)"
            ))
        );
        assert_eq!(
            &parsed.console[..8],
            [
                "hello",
                "### Result",
                "### Error",
                "### Console",
                "### Notes",
                "### Tips",
                "",
                "[warn] slow"
            ]
        );
        assert_eq!(
            parsed.console[8],
            format!(
                "[3 more console lines were dropped: a run keeps {} lines and {} KiB]",
                Defaults::MAX_LOG_LINES,
                Defaults::MAX_LOG_BYTES / 1024
            )
        );
        assert_eq!(
            parsed.notes,
            [
                "[unknown outcome] locator.click at run:3:7 (op 7) was sent but not confirmed. Take a snapshot before repeating it.",
                "[not awaited] page.goto at run:4:1 (op 8) was still running when the script returned; it was cancelled.",
            ]
        );
        assert!(parsed.tips.is_empty());
        let wire = serde_json::to_value(RunTool::result(&reply))?;
        assert_eq!(wire["isError"], true);
        assert!(wire.get("structuredContent").is_none());
        assert_eq!(
            wire["_meta"][RUN_META_KEY]["error"],
            serde_json::to_value(&reply.error)?
        );
        let replayed: RunReply = serde_json::from_value(serde_json::to_value(&reply)?)?;
        assert_eq!(
            serde_json::to_string(&RunTool::result(&reply))?,
            serde_json::to_string(&RunTool::result(&replayed))?
        );
        Ok(())
    }

    #[test]
    fn parser_rejects_non_run_text_invalid_values_and_reordered_sections() {
        for text in [
            "",
            "42",
            "### Console\nhello",
            "### Result",
            "### Result\nno JSON",
            "### Result\n{}\nextra",
            "### Result\n{}\n### Error\nfailed",
            "### Result\n{}\n### Notes\nnote\n### Console\nhello",
            "### Result\n{}\n### Result\n{}",
        ] {
            assert!(parse_text(text).is_none(), "invalid run text: {text}");
        }
    }

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
    fn the_delta_tip_is_the_last_line_after_the_console_and_the_notes() -> Result<(), Box<dyn Error>>
    {
        let id = RunId::new();
        let reply: RunReply = serde_json::from_value(json!({
            "id": id,
            "status": "ok",
            "effects": "unknown",
            "value": "- button \"Go\" [ref=e2]\nTip: not a line of its own",
            "logs": [{ "level": "log", "text": "hello" }],
            "unknown": [{ "operationId": OperationId::new(id, 3), "name": "locator.click" }],
            "tips": ["snapshotDelta"],
        }))?;
        let text = text(&reply);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines.get(1),
            Some(&"\"- button \\\"Go\\\" [ref=e2]\\nTip: not a line of its own\""),
            "the value is JSON text, so page content cannot print a line of its own"
        );
        assert_eq!(lines[0], "### Result");
        assert_eq!(lines[2..5], ["### Console", "hello", "### Notes"]);
        assert!(lines[5].starts_with("[unknown outcome] locator.click"));
        assert_eq!(
            lines.get(6..),
            Some(
                &[
                    "### Tips",
                    "page.ariaSnapshot({ delta: true }) returns only what changed since \
                     your last snapshot."
                ][..]
            )
        );
        // The tip is stored with the reply, so a replay renders it again.
        let replayed: RunReply = serde_json::from_value(serde_json::to_value(&reply)?)?;
        assert_eq!(
            serde_json::to_string(&RunTool::result(&replayed))?,
            serde_json::to_string(&RunTool::result(&reply))?
        );
        Ok(())
    }

    #[test]
    fn task_metadata_never_changes_the_reply_text() -> Result<(), Box<dyn Error>> {
        let tabs: Vec<_> = (1..=12)
            .map(|tab| json!({ "tabId": tab, "url": "é".repeat(121) }))
            .collect();
        let mut reply: RunReply = serde_json::from_value(json!({
            "id": RunId::new(), "status": "ok", "effects": "none", "value": "one",
            "tasks": [
                { "id": "one", "name": null, "kind": "fetched", "tabs": tabs },
                { "id": "two", "name": null, "kind": "closed", "tabs": [] }
            ]
        }))?;
        let wire = serde_json::to_value(RunTool::result(&reply))?;
        assert_eq!(
            wire["content"][0]["text"],
            format!("### Result\n{}", reply.value)
        );
        assert_eq!(
            wire["_meta"][RUN_META_KEY]["tasks"],
            serde_json::to_value(&reply.tasks)?
        );
        reply.tasks.clear();
        let plain = serde_json::to_value(RunTool::result(&reply))?;
        assert_eq!(wire["content"], plain["content"]);
        assert!(wire.get("structuredContent").is_none());
        assert!(plain.get("structuredContent").is_none());
        Ok(())
    }

    #[test]
    fn task_create_fetch_close_failure_and_replay_only_report_tasks_in_metadata()
    -> Result<(), TestError> {
        use crate::run::{RunMeta, RunRequest, RunTimeout};
        use neo_trace::RequestId;
        use std::time::Duration;

        test_rig::on_neo_runtime(async {
            let rig = TestRig::new().await?;
            let service = rig.run_service(Duration::from_secs(30));
            let request = |code: String, id: &str| -> Result<RunRequest, TestError> {
                Ok(RunRequest {
                    code,
                    timeout: RunTimeout::from_millis(None)?,
                    meta: RunMeta {
                        request_id: Some(RequestId::parse(id)?),
                        ..RunMeta::default()
                    },
                })
            };
            let code = "const task = await neo.createTask('reply'); return task.id;";
            let created = service.run(request(code.to_owned(), "created")?).await;
            assert!(created.error.is_none(), "{created:?}");
            let id = created.value.to_string();
            let fetched = service
                .run(request(
                    format!("const task = await neo.task({id}); return task.id;"),
                    "fetched",
                )?)
                .await;
            let failed = service
                .run(request(
                    format!("await neo.task({id}); throw new Error('after fetching');"),
                    "failed",
                )?)
                .await;
            let closed = service
                .run(request(
                    format!(
                        "const task = await neo.task({id}); await task.close(); return task.id;"
                    ),
                    "closed",
                )?)
                .await;
            for (reply, kind) in [
                (&created, "created"),
                (&fetched, "fetched"),
                (&failed, "fetched"),
                (&closed, "closed"),
            ] {
                let mcp = serde_json::to_value(RunTool::result(reply))?;
                let text = mcp["content"][0]["text"].as_str().ok_or("MCP text")?;
                assert!(
                    !text.contains("[task"),
                    "task recovery belongs only in host metadata"
                );
                assert_eq!(reply.tasks.len(), 1);
                assert_eq!(
                    mcp["_meta"][RUN_META_KEY]["tasks"],
                    serde_json::to_value(&reply.tasks)?
                );
                assert_eq!(mcp["_meta"][RUN_META_KEY]["tasks"][0]["kind"], kind);
                assert!(mcp.get("structuredContent").is_none());
                if reply.error.is_none() {
                    assert_eq!(
                        text,
                        format!("### Result\n{}", reply.value),
                        "the Result section contains exactly the JSON value"
                    );
                    assert_eq!(
                        parse_text(text).ok_or("sections")?.value,
                        Ok(reply.value.clone())
                    );
                } else {
                    assert_eq!(
                        text,
                        format!(
                            "### Error\n{}",
                            super::error_line(reply.error.as_ref().ok_or("error")?)
                        )
                    );
                    assert!(parse_text(text).ok_or("sections")?.value.is_err());
                }
            }
            assert!(fetched.error.is_none(), "{fetched:?}");
            assert!(closed.error.is_none(), "{closed:?}");
            assert_eq!(
                failed.error.as_ref().ok_or("script error")?.code,
                "script_error"
            );
            rig.sink.flush().await?;
            let replay = rig
                .restarted_service(Duration::from_secs(30))?
                .run(request(code.to_owned(), "created")?)
                .await;
            assert_eq!(
                serde_json::to_value(RunTool::result(&replay))?,
                serde_json::to_value(RunTool::result(&created))?
            );
            Ok(())
        })
    }
}
