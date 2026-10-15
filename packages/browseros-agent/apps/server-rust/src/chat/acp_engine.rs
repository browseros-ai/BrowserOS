//! `AcpEngine`: the [`ChatEngine`] adapter for external ACP agents (Claude, Codex, custom). It owns
//! only what the TS `ChatService` kept per ACP conversation: the browser-tool lease (re-minted only
//! when `{agentId, readOnly}` changes) and the agent behind the conversation; everything about
//! processes, sessions and translation lives in `acp_agents::AcpRuntime`.
//!
//! ```text
//! ChatService ─ Turn{history, request, run} ─► AcpEngine::start_turn
//!   lease: reuse (refresh browser context) | revoke + mint        (mcp::ToolLeases)
//!   prompt: latest user = system prompt + format_user_message + files ("User: …" line)
//!           transcript seed = earlier display history in convert-prompt line form
//!   AcpRuntime::stream ── Busy → 409 · other preparation failure → one `error` chunk
//! ```

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex, MutexGuard},
};

use acp_agents::{
    AcpRuntime, AcpRuntimeConfig, AgentDefinition, AgentKind, CustomAgentConfig, CustomMcpServer,
    FileSessionStore, HostEnvironment, RuntimeError, TurnContext, TurnEnd, TurnPrompt, TurnRequest,
    UiTranslator,
    schema::{AudioContent, ContentBlock, ImageContent, TextContent},
};
use browseros_agent::{
    messages::{AssistantPart, ModelMessage, ToolMessagePart, ToolResultOutput, UserPart},
    prompt::format_user_message,
    tools::ToolCatalog,
};
use serde_json::Value;
use ui_message_stream::{ChunkStream, Role, UiChunk, UiMessage, UiPart};
use uuid::Uuid;

use super::{
    browseros_engine::{LeaseIssuer, RegistryLeases, SessionLease},
    engine::{ChatEngine, ChatRejection, Turn, TurnTarget},
    request::{BrowserContext, ChatMode, ChatTurnRequest, ResolvedBrowserContext},
};
use crate::{
    config::Config,
    db::{self, AcpAgentRow},
    http::errors::thrown_error,
    mcp::{LeaseSpec, ToolLeases},
    paths::BrowserosPaths,
};

/// TS `AcpAgentPreparationError` text: the whole stream when the agent cannot be started.
pub const PREPARATION_ERROR: &str = "Unable to start the ACP agent.";

/// What one conversation holds between turns (TS `acpConversationAgents` + `acpToolLeases`).
struct Held {
    agent_id: Uuid,
    read_only: bool,
    lease: Box<dyn SessionLease>,
}

pub struct AcpEngine {
    runtime: Arc<AcpRuntime>,
    host: HostEnvironment,
    /// The same lease seam as the built-in engine (`RegistryLeases` in production).
    leases: Arc<dyn LeaseIssuer>,
    server_port: u16,
    held: Mutex<HashMap<Uuid, Held>>,
}

impl AcpEngine {
    /// Production wiring: bundled resources, `<browserosDir>/agents/acp-sessions` records, the
    /// server's own port for the loopback `/mcp`, and the shared lease registry.
    pub fn new(config: &Config, paths: &BrowserosPaths, leases: Arc<ToolLeases>) -> Self {
        let host = HostEnvironment::from_process(
            Some(config.resources_dir.clone()),
            config.browseros_dir.clone(),
        );
        let store = Arc::new(FileSessionStore::new(paths.acp_sessions_dir.clone()));
        Self::with_runtime(
            AcpRuntimeConfig::new(host, store),
            config.server_port,
            Arc::new(RegistryLeases(leases)),
        )
    }

    pub fn with_runtime(
        runtime: AcpRuntimeConfig,
        server_port: u16,
        leases: Arc<dyn LeaseIssuer>,
    ) -> Self {
        Self {
            host: runtime.host.clone(),
            runtime: AcpRuntime::new(runtime),
            leases,
            server_port,
            held: Mutex::default(),
        }
    }

    /// The launch environment `/acpx/probe` resolves agents against, identical to chat's.
    pub fn host(&self) -> &HostEnvironment {
        &self.host
    }

    /// `/agents/{id}` update and delete close every live session of that agent. Returns how many
    /// sessions were closed.
    pub async fn close_all_for_agent(
        &self,
        agent_id: Uuid,
        discard_persistent_state: bool,
    ) -> usize {
        self.runtime
            .close_all_for_agent(agent_id, discard_persistent_state)
            .await
    }

    pub async fn shutdown(&self) {
        self.runtime.shutdown().await;
    }

    /// Reuses the conversation's lease while agent and read-only flag are unchanged, refreshing
    /// its browser context in place; otherwise revokes it and mints a new one.
    async fn lease(
        &self,
        conversation_id: Uuid,
        row: &AcpAgentRow,
        read_only: bool,
        request: &ChatTurnRequest,
        task_label: String,
    ) -> Result<String, ChatRejection> {
        let browser_context = request.browser_context.as_ref();
        let previous = {
            let mut held = lock(&self.held);
            match held.get(&conversation_id) {
                Some(entry) if entry.agent_id == row.id && entry.read_only == read_only => {
                    entry.lease.update_browser_context(browser_context);
                    return Ok(entry.lease.token().to_owned());
                }
                _ => held.remove(&conversation_id),
            }
        };
        if let Some(previous) = previous {
            previous.lease.revoke();
        }
        let lease = self
            .leases
            .mint(LeaseSpec {
                conversation_id,
                read_only,
                client_slug: client_slug(row.kind),
                task_label,
                auto_group: !request.is_scheduled_task,
                browser_context: request.browser_context.clone(),
            })
            .await
            .map_err(|error| ChatRejection::Internal(thrown_error(&error.to_string())))?;
        let token = lease.token().to_owned();
        lock(&self.held).insert(
            conversation_id,
            Held {
                agent_id: row.id,
                read_only,
                lease,
            },
        );
        Ok(token)
    }
}

#[async_trait::async_trait]
impl ChatEngine for AcpEngine {
    async fn start_turn(&self, turn: Turn) -> Result<ChunkStream, ChatRejection> {
        let Turn {
            conversation_id,
            run,
            target,
            request,
            history,
            browser_context,
        } = turn;
        let TurnTarget::Acp(row) = target else {
            return Err(ChatRejection::Internal(thrown_error(
                "AcpEngine received a non-ACP turn",
            )));
        };
        let read_only = request.mode == ChatMode::Chat;
        let message = latest_user_text(&history);
        let token = self
            .lease(
                conversation_id,
                &row,
                read_only,
                &request,
                task_label(&message),
            )
            .await?;
        let ctx = TurnContext {
            agent: definition(&row),
            conversation_id,
            server_port: self.server_port,
            browser_tool_lease_token: token,
            read_only,
            custom_mcp_servers: custom_mcp_servers(request.browser_context.as_ref()),
            working_dir_override: request.user_working_dir.clone(),
        };
        let response_message_id = Uuid::new_v4().to_string();
        let prompt = match build_prompt(&history, &message, &request, browser_context.as_ref()) {
            Ok(prompt) => prompt,
            Err(error) => {
                // TS failed these inside the provider's prompt conversion: a failed turn whose
                // text is the generic ACP message, not a preparation error.
                tracing::warn!(%error, %conversation_id, "Unable to convert ACP prompt");
                return Ok(failed_turn(response_message_id));
            }
        };
        let request = TurnRequest {
            ctx,
            prompt,
            response_message_id,
            cancel: run.cancel().clone(),
        };
        match self.runtime.stream(request).await {
            Ok(stream) => Ok(stream),
            Err(RuntimeError::Busy) => Err(ChatRejection::Busy),
            Err(error) => {
                tracing::error!(agent_id = %row.id, %conversation_id, %error, "ACP agent preparation failed");
                Ok(Box::pin(futures_util::stream::iter([UiChunk::Error {
                    error_text: PREPARATION_ERROR.to_owned(),
                }])))
            }
        }
    }

    /// TS `deleteSession`: close with persistent state discarded, revoke the lease, forget the agent.
    async fn forget(&self, conversation_id: Uuid) -> bool {
        let Some(held) = lock(&self.held).remove(&conversation_id) else {
            return false;
        };
        self.runtime
            .close(held.agent_id, conversation_id, true)
            .await;
        held.lease.revoke();
        true
    }
}

fn client_slug(kind: db::AgentKind) -> &'static str {
    match kind {
        db::AgentKind::Claude => "claude",
        db::AgentKind::Codex => "codex",
        db::AgentKind::Custom => "custom",
    }
}

/// First five words of the user's request: the agent's tab-group label.
fn task_label(message: &str) -> String {
    message
        .split_whitespace()
        .take(5)
        .collect::<Vec<_>>()
        .join(" ")
}

fn failed_turn(response_message_id: String) -> ChunkStream {
    let mut chunks = vec![UiChunk::Start {
        message_metadata: None,
        message_id: Some(response_message_id),
    }];
    chunks.extend(UiTranslator::new().finish(TurnEnd::Failed {
        message: acp_agents::ACP_FAILED_TO_RESPOND.to_owned(),
    }));
    Box::pin(futures_util::stream::iter(chunks))
}

/// `CustomAcpAgentConfigSchema` as the agents store wrote it (camelCase JSON; `icon` is UI-only).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredCustomConfig {
    command: String,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    full_access_modes: Vec<String>,
    reasoning_effort_key: Option<String>,
    system_prompt_append: Option<String>,
}

fn definition(row: &AcpAgentRow) -> AgentDefinition {
    let custom = row.custom_config.as_deref().and_then(|raw| {
        serde_json::from_str::<StoredCustomConfig>(raw)
            .inspect_err(|error| {
                tracing::warn!(agent_id = %row.id, %error, "Ignoring unreadable custom ACP config");
            })
            .ok()
    });
    AgentDefinition {
        id: row.id,
        kind: match row.kind {
            db::AgentKind::Claude => AgentKind::Claude,
            db::AgentKind::Codex => AgentKind::Codex,
            db::AgentKind::Custom => AgentKind::Custom,
        },
        model_id: row.model_id.clone(),
        reasoning_effort: row.reasoning_effort.clone(),
        working_directory: row.working_directory.clone(),
        custom: custom.map(|config| CustomAgentConfig {
            command: config.command,
            env: config.env,
            full_access_modes: config.full_access_modes,
            reasoning_effort_key: config.reasoning_effort_key,
            system_prompt_append: config.system_prompt_append,
        }),
    }
}

fn custom_mcp_servers(ctx: Option<&BrowserContext>) -> Vec<CustomMcpServer> {
    ctx.and_then(|ctx| ctx.custom_mcp_servers.as_ref())
        .into_iter()
        .flatten()
        .map(|server| CustomMcpServer {
            name: server.name.clone(),
            url: server.url.clone(),
        })
        .collect()
}

/// The raw message of this turn: the text of the history's last (user) message.
fn latest_user_text(history: &[UiMessage]) -> String {
    history
        .last()
        .filter(|message| message.role == Role::User)
        .map(|message| {
            message
                .parts
                .iter()
                .filter_map(|part| match part {
                    UiPart::Text(text) => Some(text.text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

/// TS builds the prompt copy of the newest user message from `userSystemPrompt` +
/// `formatUserMessage(..)` and the request's attachments, then acpx-ai-provider's `convertPrompt`
/// flattens messages into `Role: segments` lines. The runtime sends `transcript_seed` only to an
/// agent without context of its own, joined to the latest line with `\n`.
///
/// DECIDED: images and audio from *earlier* messages are not re-sent when seeding a fresh session
/// (TS re-attached them). WHY: the seed is text; re-attaching would resend them on every
/// continuation too. Earlier text files are still inlined.
fn build_prompt(
    history: &[UiMessage],
    message: &str,
    request: &ChatTurnRequest,
    browser_context: Option<&ResolvedBrowserContext>,
) -> Result<TurnPrompt, String> {
    let source = request
        .selected_text_source
        .as_ref()
        .map(|source| (source.url.as_str(), source.title.as_str()));
    let user_content = format_user_message(
        message,
        browser_context,
        request.selected_text.as_deref(),
        source,
    );
    let prompt_text = match request
        .user_system_prompt
        .as_deref()
        .map(crate::js::trim)
        .filter(|prompt| !prompt.is_empty())
    {
        Some(system) => format!("{system}\n\n{user_content}"),
        None => user_content,
    };
    let mut segments = vec![prompt_text];
    let mut attachments = Vec::new();
    for attachment in &request.attachments {
        let url = if attachment.data.starts_with("data:") {
            attachment.data.clone()
        } else {
            format!("data:{};base64,{}", attachment.media_type, attachment.data)
        };
        match render_file(&attachment.media_type, None, &url)? {
            FileRender::Inline(text) => segments.push(text),
            FileRender::Attachment(block) => attachments.push(*block),
        }
    }
    let mut latest_user = vec![ContentBlock::Text(TextContent::new(format!(
        "User: {}",
        segments.join(" ")
    )))];
    latest_user.extend(attachments);

    let earlier = history
        .split_last()
        .map(|(_, earlier)| earlier)
        .unwrap_or_default();
    let transcript_seed =
        match browseros_agent::messages::ui_to_model(earlier, &ToolCatalog::new(&[])) {
            Ok(messages) => {
                let lines = render_lines(&messages)?;
                (!lines.is_empty()).then(|| lines.join("\n"))
            }
            // TS `convertHistory`: legacy records that no longer convert still let the new request run.
            Err(error) => {
                tracing::warn!(%error, "Continuing ACP turn without incompatible saved history");
                None
            }
        };
    Ok(TurnPrompt {
        latest_user,
        transcript_seed,
    })
}

/// convert-prompt `appendMessage` for every message: `System: …`, `User: …`, `Assistant: …`,
/// `Tool: …`, segments joined by a space; a message without segments is skipped.
fn render_lines(messages: &[ModelMessage]) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    for message in messages {
        let (prefix, segments) = match message {
            ModelMessage::System { content, .. } => {
                lines.push(format!("System: {content}"));
                continue;
            }
            ModelMessage::User { content } => {
                let mut segments = Vec::new();
                for part in content {
                    match part {
                        UserPart::Text { text, .. } => segments.push(text.clone()),
                        UserPart::Image {
                            image, media_type, ..
                        } => {
                            let media_type = media_type.as_deref().unwrap_or("image");
                            segments.extend(seed_file(media_type, None, image)?);
                        }
                        UserPart::File {
                            data,
                            media_type,
                            filename,
                            ..
                        } => segments.extend(seed_file(media_type, filename.as_deref(), data)?),
                    }
                }
                ("User: ", segments)
            }
            ModelMessage::Assistant { content } => {
                let mut segments = Vec::new();
                for part in content {
                    match part {
                        AssistantPart::Text { text, .. } => segments.push(text.clone()),
                        AssistantPart::Reasoning { text, .. } => {
                            segments.push(format!("[Reasoning: {text}]"));
                        }
                        AssistantPart::ToolCall {
                            tool_name, input, ..
                        } => segments.push(format!(
                            "[Tool call: {tool_name}({})]",
                            input
                                .as_ref()
                                .map_or("undefined".to_owned(), Value::to_string)
                        )),
                        AssistantPart::ToolResult(result) => segments.push(format!(
                            "[Tool result ({}): {}]",
                            result.tool_call_id,
                            format_tool_output(&result.output)
                        )),
                        AssistantPart::File {
                            data,
                            media_type,
                            filename,
                            ..
                        } => segments.extend(seed_file(media_type, filename.as_deref(), data)?),
                        _ => {}
                    }
                }
                ("Assistant: ", segments)
            }
            ModelMessage::Tool { content } => {
                let segments = content
                    .iter()
                    .filter_map(|part| match part {
                        ToolMessagePart::ToolResult(result) => Some(format!(
                            "[Tool result ({}): {}]",
                            result.tool_call_id,
                            format_tool_output(&result.output)
                        )),
                        _ => None,
                    })
                    .collect();
                ("Tool: ", segments)
            }
        };
        if !segments.is_empty() {
            lines.push(format!("{prefix}{}", segments.join(" ")));
        }
    }
    Ok(lines)
}

/// convert-prompt `formatToolOutput` (an execution denial had no case and printed `undefined`).
fn format_tool_output(output: &ToolResultOutput) -> String {
    match output {
        ToolResultOutput::Text { value } | ToolResultOutput::ErrorText { value } => value.clone(),
        ToolResultOutput::Json { value } | ToolResultOutput::ErrorJson { value } => {
            value.to_string()
        }
        ToolResultOutput::Content { value } => value
            .iter()
            .map(|item| {
                let item = serde_json::to_value(item).unwrap_or(Value::Null);
                match item["type"].as_str() {
                    Some("text") => item["text"].as_str().unwrap_or_default().to_owned(),
                    _ => format!(
                        "<media:{}>",
                        item["mediaType"].as_str().unwrap_or("undefined")
                    ),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        ToolResultOutput::ExecutionDenied { .. } => "undefined".to_owned(),
    }
}

enum FileRender {
    Inline(String),
    Attachment(Box<ContentBlock>),
}

/// A file inside an earlier message: inlined when textual, dropped when media (see DECIDED).
fn seed_file(
    media_type: &str,
    filename: Option<&str>,
    data: &Value,
) -> Result<Option<String>, String> {
    let url = match data {
        Value::String(url) => url.clone(),
        Value::Object(tagged) if tagged.get("type").and_then(Value::as_str) == Some("url") => {
            tagged
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        }
        _ => return Err(FILE_URL_NOT_SUPPORTED.to_owned()),
    };
    match render_file(media_type, filename, &url)? {
        FileRender::Inline(text) => Ok(Some(text)),
        FileRender::Attachment(_) => Ok(None),
    }
}

const FILE_URL_NOT_SUPPORTED: &str = "convertPrompt does not support remote URL file parts; inline the file as base64 or Uint8Array before passing it to the provider.";

/// convert-prompt `renderFile`: image/audio become prompt attachments; text and JSON are inlined
/// as `[File: <name>]\n<decoded>`; anything else fails the turn.
fn render_file(media_type: &str, filename: Option<&str>, url: &str) -> Result<FileRender, String> {
    if media_type.starts_with("image/") || media_type.starts_with("audio/") {
        let Some(data) = url.strip_prefix("data:") else {
            return Err(FILE_URL_NOT_SUPPORTED.to_owned());
        };
        let base64 = data
            .split_once(',')
            .map_or(data, |(_, value)| value)
            .to_owned();
        let block = if media_type.starts_with("image/") {
            ContentBlock::Image(ImageContent::new(base64, media_type))
        } else {
            ContentBlock::Audio(AudioContent::new(base64, media_type))
        };
        return Ok(FileRender::Attachment(Box::new(block)));
    }
    if media_type.starts_with("text/") || media_type == "application/json" {
        let label = filename.unwrap_or(media_type);
        return Ok(FileRender::Inline(format!(
            "[File: {label}]\n{}",
            decode_text(url)?
        )));
    }
    Err(format!("Unsupported ACP file type: {media_type}"))
}

/// convert-prompt `decodeTextFile` for data URLs: base64 or percent-encoded, decoded as UTF-8.
fn decode_text(url: &str) -> Result<String, String> {
    use base64::Engine;
    let Some(data) = url.strip_prefix("data:") else {
        return Err(FILE_URL_NOT_SUPPORTED.to_owned());
    };
    let Some((metadata, value)) = data.split_once(',') else {
        return Ok(String::new());
    };
    let bytes = if metadata.ends_with(";base64") {
        base64::engine::general_purpose::STANDARD
            .decode(value.trim())
            .map_err(|error| error.to_string())?
    } else {
        percent_decode(value)?
    };
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// `decodeURIComponent` (throws, like JS, on a malformed escape).
fn percent_decode(value: &str) -> Result<Vec<u8>, String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = value
                .get(index + 1..index + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                .ok_or_else(|| "URI malformed".to_owned())?;
            out.push(hex);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    std::str::from_utf8(&out).map_err(|_| "URI malformed".to_owned())?;
    Ok(out)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
