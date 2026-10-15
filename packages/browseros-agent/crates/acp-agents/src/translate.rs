//! ACP `session/update` → AI SDK v7 UI message stream chunks. Replaces three TS layers
//! in one pass: acpx's event normalization (`runtime.js` tool summaries, plan status text), the
//! acpx-ai-provider `EventTranslator` (block/tool state), and AI SDK `streamText` +
//! `toUIMessageStream` (step envelope, finish mapping). Deliberate change vs TS: completed tools
//! emit `tool-input-available` instead of the spurious `tool-input-error`.

use std::collections::HashMap;

use acp_client::schema::{ContentBlock, ContentChunk, SessionUpdate, StopReason, ToolCallStatus};
use serde_json::Value;

use ui_message_stream::{FinishReason, UiChunk};

/// Generic UI error text for agent failures without a better message (TS `acpUiErrorMessage`).
pub const ACP_FAILED_TO_RESPOND: &str = "The ACP agent failed to respond.";
/// acpx truncates tool summaries to 500 UTF-16 units (`TOOL_OUTPUT_SUMMARY_MAX_CHARS`).
const TOOL_SUMMARY_MAX: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnEnd {
    /// The prompt response arrived.
    Completed(StopReason),
    /// Host stop (`POST /chat/:id/stop`): emits `abort`, like AI SDK on an aborted stream.
    CancelledByHost { reason: String },
    /// JSON-RPC error, process exit, or turn timeout.
    Failed { message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Text,
    Reasoning,
}

struct Block {
    kind: BlockKind,
    id: String,
    message_id: Option<String>,
}

#[derive(Default)]
struct ToolState {
    tool_name: String,
    last_summary: String,
    raw_input: Option<Value>,
    raw_output: Option<Value>,
    content: Option<Value>,
}

/// One turn's translation state: the open text/reasoning block, tool calls by id, the lazily
/// emitted `start-step`, and Codex's silent `systemError` flag (a failure it reports only in
/// `_meta`).
#[derive(Default)]
pub struct UiTranslator {
    next_id: u64,
    step_started: bool,
    block: Option<Block>,
    tools: HashMap<String, ToolState>,
    visible_output: bool,
    system_error: bool,
    last_usage: Option<(u64, u64)>,
}

impl UiTranslator {
    pub fn new() -> Self {
        Self::default()
    }

    /// True once any update produced a chunk. The runtime uses it to decide whether a failed
    /// attempt can still be retried invisibly.
    pub fn has_output(&self) -> bool {
        self.visible_output
    }

    /// Emits `start-step` lazily before the first chunk, so a turn that fails before any output
    /// shows no empty step (TS `forwardAttempt`).
    pub fn on_update(&mut self, update: &SessionUpdate) -> Vec<UiChunk> {
        let chunks = match update {
            SessionUpdate::AgentMessageChunk(chunk) => self.text(BlockKind::Text, chunk),
            SessionUpdate::AgentThoughtChunk(chunk) => self.text(BlockKind::Reasoning, chunk),
            SessionUpdate::ToolCall(call) => self.tool(ToolEvent {
                update: false,
                id: call.tool_call_id.0.trim(),
                title: Some(call.title.as_str()),
                status: Some(call.status),
                raw_input: call.raw_input.as_ref(),
                raw_output: call.raw_output.as_ref(),
                content: Some(json_or_null(&call.content))
                    .filter(|content| !is_empty_array(content)),
            }),
            SessionUpdate::ToolCallUpdate(update) => self.tool(ToolEvent {
                update: true,
                id: update.tool_call_id.0.trim(),
                title: update.fields.title.as_deref(),
                status: update.fields.status,
                raw_input: update.fields.raw_input.as_ref(),
                raw_output: update.fields.raw_output.as_ref(),
                content: update.fields.content.as_ref().map(json_or_null),
            }),
            SessionUpdate::Plan(plan) => {
                // acpx turns a plan into status text "plan: <first entry>", then the provider
                // prefixes "[Plan] "; both quirks are kept so history reads the same.
                let first = plan.entries.first().map(|entry| entry.content.trim());
                match first.filter(|content| !content.is_empty()) {
                    Some(content) => {
                        let id = self.next_block_id();
                        vec![
                            UiChunk::ReasoningStart {
                                id: id.clone(),
                                provider_metadata: None,
                            },
                            UiChunk::ReasoningDelta {
                                id: id.clone(),
                                delta: format!("[Plan] plan: {content}"),
                                provider_metadata: None,
                            },
                            UiChunk::ReasoningEnd {
                                id,
                                provider_metadata: None,
                            },
                        ]
                    }
                    None => vec![],
                }
            }
            SessionUpdate::UsageUpdate(usage) => {
                self.last_usage = Some((usage.used, usage.size));
                vec![]
            }
            SessionUpdate::SessionInfoUpdate(info) => {
                let status = info
                    .meta
                    .as_ref()
                    .and_then(|meta| meta.get("codex"))
                    .and_then(|codex| codex.pointer("/threadStatus/type"))
                    .and_then(Value::as_str);
                if let Some(status) = status {
                    tracing::debug!(status, "ACP agent thread status changed");
                    if status == "systemError" {
                        self.system_error = true;
                    }
                }
                vec![]
            }
            _ => vec![],
        };
        if chunks.is_empty() {
            return chunks;
        }
        self.visible_output = true;
        self.with_step(chunks)
    }

    /// Terminal chunks. `Completed` and `Failed` close the open block and end with `finish-step` +
    /// `finish`; host cancel ends with `abort` alone, exactly as AI SDK's aborted stream did.
    pub fn finish(&mut self, end: TurnEnd) -> Vec<UiChunk> {
        if let Some((used, size)) = self.last_usage {
            tracing::debug!(used, size, "ACP turn usage");
        }
        let chunks = match end {
            TurnEnd::CancelledByHost { reason } => vec![UiChunk::Abort {
                reason: Some(reason),
            }],
            TurnEnd::Completed(reason) => {
                let mut chunks = self.close_block();
                // Codex can end a turn `end_turn` with no output after a backend failure; the only
                // signal is `threadStatus: systemError`. Rendering that as an empty reply hides it.
                if !self.visible_output && self.system_error {
                    chunks.extend(failure(ACP_FAILED_TO_RESPOND.into()));
                } else {
                    chunks.push(UiChunk::FinishStep);
                    chunks.push(UiChunk::Finish {
                        finish_reason: Some(finish_reason(reason)),
                        message_metadata: None,
                    });
                }
                chunks
            }
            TurnEnd::Failed { message } => {
                let mut chunks = self.close_block();
                chunks.extend(failure(message));
                chunks
            }
        };
        self.with_step(chunks)
    }

    fn with_step(&mut self, chunks: Vec<UiChunk>) -> Vec<UiChunk> {
        if self.step_started {
            return chunks;
        }
        self.step_started = true;
        std::iter::once(UiChunk::StartStep).chain(chunks).collect()
    }

    fn next_block_id(&mut self) -> String {
        self.next_id += 1;
        format!("acpx-{}", self.next_id)
    }

    fn close_block(&mut self) -> Vec<UiChunk> {
        match self.block.take() {
            Some(Block {
                kind: BlockKind::Text,
                id,
                ..
            }) => vec![UiChunk::TextEnd {
                id,
                provider_metadata: None,
            }],
            Some(Block {
                kind: BlockKind::Reasoning,
                id,
                ..
            }) => vec![UiChunk::ReasoningEnd {
                id,
                provider_metadata: None,
            }],
            None => vec![],
        }
    }

    /// ACP chunks are token fragments: only a kind change or a *new* ACP messageId opens a block.
    /// A missing messageId continues the current block. Non-text and empty chunks are dropped.
    fn text(&mut self, kind: BlockKind, chunk: &ContentChunk) -> Vec<UiChunk> {
        let ContentBlock::Text(text) = &chunk.content else {
            return vec![];
        };
        if text.text.is_empty() {
            return vec![];
        }
        let message_id = chunk.message_id.as_ref().map(|id| id.0.to_string());
        let changed = match &self.block {
            None => true,
            Some(block) => {
                block.kind != kind || (message_id.is_some() && block.message_id != message_id)
            }
        };
        let mut chunks = Vec::new();
        if changed {
            chunks.extend(self.close_block());
            let id = self.next_block_id();
            chunks.push(match kind {
                BlockKind::Text => UiChunk::TextStart {
                    id: id.clone(),
                    provider_metadata: None,
                },
                BlockKind::Reasoning => UiChunk::ReasoningStart {
                    id: id.clone(),
                    provider_metadata: None,
                },
            });
            self.block = Some(Block {
                kind,
                id,
                message_id,
            });
        }
        if let Some(block) = &self.block {
            let id = block.id.clone();
            let delta = text.text.clone();
            chunks.push(match kind {
                BlockKind::Text => UiChunk::TextDelta {
                    id,
                    delta,
                    provider_metadata: None,
                },
                BlockKind::Reasoning => UiChunk::ReasoningDelta {
                    id,
                    delta,
                    provider_metadata: None,
                },
            });
        }
        chunks
    }

    fn tool(&mut self, event: ToolEvent<'_>) -> Vec<UiChunk> {
        if event.id.is_empty() {
            return vec![];
        }
        let summary = event.summary();
        let mut chunks = self.close_block();
        let id = event.id.to_owned();
        let state = match self.tools.get_mut(&id) {
            Some(state) => state,
            None => {
                // The tool name is fixed at first sight (Claude: "Terminal", later retitled to the
                // command); it names the static `tool-<title>` part the UI renders.
                let tool_name = event.title_text().to_owned();
                chunks.push(UiChunk::ToolInputStart {
                    tool_call_id: id.clone(),
                    tool_name: tool_name.clone(),
                    provider_executed: Some(true),
                    provider_metadata: None,
                    tool_metadata: None,
                    dynamic: None,
                    title: None,
                });
                self.tools.entry(id.clone()).or_insert(ToolState {
                    tool_name,
                    ..ToolState::default()
                })
            }
        };
        if let Some(raw_input) = event.raw_input {
            state.raw_input = Some(raw_input.clone());
        }
        if let Some(raw_output) = event.raw_output {
            state.raw_output = Some(raw_output.clone());
        }
        if let Some(content) = &event.content {
            state.content = Some(content.clone());
        }
        let terminal = matches!(
            event.status,
            Some(ToolCallStatus::Completed | ToolCallStatus::Failed)
        );
        if !terminal {
            // The visible progress line, sent whole and only when it changes. TS sent
            // the same text but re-sent repeats, since it diffed against the concatenation.
            if summary != state.last_summary {
                state.last_summary = summary.clone();
                chunks.push(UiChunk::ToolInputDelta {
                    tool_call_id: id,
                    input_text_delta: summary,
                });
            }
            return chunks;
        }
        let Some(state) = self.tools.remove(&id) else {
            return chunks;
        };
        let input = state.raw_input.clone().unwrap_or_else(|| {
            Value::String(if state.last_summary.is_empty() {
                summary.clone()
            } else {
                state.last_summary.clone()
            })
        });
        chunks.push(UiChunk::ToolInputAvailable {
            tool_call_id: id.clone(),
            tool_name: state.tool_name.clone(),
            input,
            provider_executed: Some(true),
            provider_metadata: None,
            tool_metadata: None,
            dynamic: None,
            title: None,
        });
        let content_text = state.content.as_ref().and_then(summarize_tool_content);
        if event.status == Some(ToolCallStatus::Failed) {
            let error_text = content_text
                .or_else(|| state.raw_output.as_ref().and_then(summarize_tool_output))
                .unwrap_or(summary);
            chunks.push(UiChunk::ToolOutputError {
                tool_call_id: id,
                error_text,
                provider_executed: Some(true),
                provider_metadata: None,
                tool_metadata: None,
                dynamic: None,
            });
        } else {
            let output = content_text
                .map(Value::String)
                .or_else(|| state.raw_output.clone().filter(|raw| !raw.is_null()))
                .unwrap_or(Value::String(summary));
            chunks.push(UiChunk::ToolOutputAvailable {
                tool_call_id: id,
                output,
                provider_executed: Some(true),
                provider_metadata: None,
                tool_metadata: None,
                preliminary: None,
                dynamic: None,
            });
        }
        chunks
    }
}

fn failure(message: String) -> Vec<UiChunk> {
    vec![
        UiChunk::Error {
            error_text: message,
        },
        UiChunk::FinishStep,
        UiChunk::Finish {
            finish_reason: Some(FinishReason::Error),
            message_metadata: None,
        },
    ]
}

/// `max_turn_requests` → length and `refusal` → content-filter are new mappings (TS had neither).
fn finish_reason(reason: StopReason) -> FinishReason {
    match reason {
        StopReason::EndTurn => FinishReason::Stop,
        StopReason::MaxTokens | StopReason::MaxTurnRequests => FinishReason::Length,
        StopReason::Refusal => FinishReason::ContentFilter,
        _ => FinishReason::Other,
    }
}

/// One `tool_call` / `tool_call_update` in wire terms, as acpx's `createToolCallEvent` saw it.
struct ToolEvent<'a> {
    update: bool,
    id: &'a str,
    title: Option<&'a str>,
    status: Option<ToolCallStatus>,
    raw_input: Option<&'a Value>,
    raw_output: Option<&'a Value>,
    content: Option<Value>,
}

impl ToolEvent<'_> {
    fn title_text(&self) -> &str {
        self.title
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .unwrap_or("tool call")
    }

    /// `"<title> (<status>): <detail>"`; updates prefer output detail, calls prefer input detail.
    /// ASSUMPTION: the schema defaults a missing `tool_call` status to pending, so where acpx
    /// printed `"<title>: …"` for status-less calls this prints `"<title> (pending): …"`.
    fn summary(&self) -> String {
        let title = self.title_text();
        let head = match self
            .status
            .and_then(|status| json_or_null(&status).as_str().map(str::to_owned))
        {
            Some(status) => format!("{title} ({status})"),
            None => title.to_owned(),
        };
        let input = self.raw_input.and_then(summarize_tool_input);
        let output = self
            .content
            .as_ref()
            .and_then(summarize_tool_content)
            .or_else(|| self.raw_output.and_then(summarize_tool_output));
        let detail = if self.update {
            output.or(input)
        } else {
            input.or(output)
        };
        match detail {
            Some(detail) => format!("{head}: {detail}"),
            None => head,
        }
    }
}

fn json_or_null<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

fn is_empty_array(value: &Value) -> bool {
    value.as_array().is_some_and(Vec::is_empty)
}

/// acpx `asOptionalString`: a string that is non-blank after trimming, trimmed.
fn optional_string(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn first_string<'a>(record: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| optional_string(record.get(key)))
}

fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

/// acpx `summarizeToolInput`: the command line, else the first path-like field.
fn summarize_tool_input(raw: &Value) -> Option<String> {
    if let Some(text) = scalar_text(raw) {
        return Some(text);
    }
    if !raw.is_object() {
        return None;
    }
    if let Some(command) = first_string(raw, &["command", "cmd", "program"]) {
        let args = ["args", "arguments"].iter().find_map(|key| {
            let entries: Vec<&str> = raw
                .get(key)?
                .as_array()?
                .iter()
                .filter_map(|entry| optional_string(Some(entry)))
                .collect();
            (!entries.is_empty()).then_some(entries)
        });
        return Some(
            std::iter::once(command)
                .chain(args.unwrap_or_default())
                .collect::<Vec<_>>()
                .join(" "),
        );
    }
    first_string(
        raw,
        &[
            "path", "file", "filePath", "filepath", "target", "uri", "url", "query", "pattern",
            "text", "search",
        ],
    )
    .map(str::to_owned)
}

/// acpx `summarizeToolContent` over wire JSON: distinct non-blank fragments joined by newlines.
fn summarize_tool_content(content: &Value) -> Option<String> {
    let mut fragments: Vec<String> = Vec::new();
    for entry in content.as_array()? {
        if let Some(text) = tool_content_text(entry)
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty())
            && !fragments.contains(&text)
        {
            fragments.push(text);
        }
    }
    (!fragments.is_empty()).then(|| truncate_summary(&fragments.join("\n")))
}

fn tool_content_text(record: &Value) -> Option<String> {
    let kind = record.get("type")?.as_str()?;
    let string = |key: &str| record.get(key).and_then(Value::as_str);
    let optional = |key: &str| optional_string(record.get(key));
    match kind {
        "content" => tool_content_text(record.get("content")?),
        "text" => string("text").map(str::to_owned),
        "audio" => Some(format!(
            "[audio] {}",
            optional("mimeType").unwrap_or("audio")
        )),
        "resource_link" => optional("title")
            .or_else(|| optional("name"))
            .or_else(|| optional("uri"))
            .map(str::to_owned),
        "resource" => {
            let resource = record.get("resource");
            resource
                .and_then(|resource| resource.get("text"))
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .or_else(|| optional_string(resource.and_then(|resource| resource.get("uri"))))
                .map(str::to_owned)
        }
        "diff" => Some(format!("diff {}", optional("path").unwrap_or("file"))),
        "terminal" => Some(
            optional("terminalId")
                .or_else(|| optional("id"))
                .map_or_else(|| "[terminal]".to_owned(), |id| format!("[terminal] {id}")),
        ),
        _ => None,
    }
}

/// acpx `summarizeToolOutput`: scalar text, else the first message-like field.
fn summarize_tool_output(raw: &Value) -> Option<String> {
    if let Some(text) = scalar_text(raw) {
        return Some(truncate_summary(&text));
    }
    if !raw.is_object() {
        return None;
    }
    let text = first_string(
        raw,
        &["text", "message", "error", "stdout", "stderr", "content"],
    )?;
    Some(truncate_summary(text))
}

/// JS `value.length <= 500 ? value : value.slice(0, 499) + "…"` in UTF-16 units (never splitting
/// a surrogate pair, which JS would).
fn truncate_summary(value: &str) -> String {
    if value.encode_utf16().count() <= TOOL_SUMMARY_MAX {
        return value.to_owned();
    }
    let mut units = 0;
    let mut out = String::new();
    for ch in value.chars() {
        units += ch.len_utf16();
        if units > TOOL_SUMMARY_MAX - 1 {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}
