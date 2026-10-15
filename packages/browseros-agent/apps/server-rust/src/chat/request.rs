//! One normalized `/chat` body and the provider resolution that follows it.
//!
//! ```text
//! JSON body ─ parse_chat_request ─► ChatRequest ─ hydrate_provider ─► HydratedProvider ─ resolve_llm ─► ResolvedLlm
//!             (zod union, ACP        (target +       (stored row        (trust decision)   (OAuth tokens,
//!              branch first)          turn fields)    overrides inline)                     model defaults)
//! ```
//!
//! `parse_chat_request` reproduces `ChatRequestSchema` from `apps/server/src/api/types.ts` — zod
//! defaults, the browseros `target` transform, unknown keys stripped — and on failure returns the
//! exact zod 4.6.5 issue list (key order, nested unions, the "one non-aborted branch" rule), so a
//! 400 body is byte-identical to the TS server's. Hydration and resolution port
//! `chat-provider-config.ts` and `lib/clients/llm/config.ts`, error texts verbatim.

use crate::{
    db::{AgentKind, ProviderRow},
    oauth::{OAuthError, OAuthProvider, TokenManager},
};
use browseros_agent::{
    ModelInfo,
    llm::{
        ChatError, LlmConfig, ProviderKind, ReasoningEffort, ReasoningSettings, ReasoningSummary,
    },
    prompt::Origin,
};
use serde_json::{Map, Value};
use std::{collections::BTreeMap, path::PathBuf, str::FromStr};
use uuid::Uuid;

use super::engine::ResolvedLlm;

/// The browser context as the app sends it (contract `BrowserContext`), unknown keys stripped.
pub type BrowserContext = server_api::BrowserContext;
/// The browser context after tab ids are resolved to page ids against the live browser.
pub type ResolvedBrowserContext = browseros_agent::prompt::BrowserContextView;
pub type SelectedTextSource = server_api::SelectedTextSource;
pub type PreviousMessage = server_api::PreviousMessage;
pub type Attachment = server_api::ChatAttachment;

pub struct ChatRequest {
    pub conversation_id: Uuid,
    pub message: String,
    pub target: RequestTarget,
    pub input: ChatTurnRequest,
}

pub enum RequestTarget {
    /// `provider_id` is the transform's `target.providerId || providerId || provider` (empty
    /// strings skipped); `None` selects the default provider.
    BrowserOs {
        provider_id: Option<String>,
        inline: Box<InlineLlm>,
    },
    Acp {
        kind: AgentKind,
        agent_id: Uuid,
    },
}

/// Inline provider fields of a BrowserOS-target body; a stored row (by `provider_id`) replaces
/// all of them except `upstream_provider` during hydration.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InlineLlm {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub headers: Option<BTreeMap<String, String>>,
    pub resource_name: Option<String>,
    pub region: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub session_token: Option<String>,
    pub upstream_provider: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChatMode {
    #[default]
    Agent,
    Chat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryMode {
    Local,
    Cloud,
}

#[derive(Debug, Clone, Default)]
pub struct ChatTurnRequest {
    pub context_window_size: Option<f64>,
    pub browser_context: Option<BrowserContext>,
    pub user_system_prompt: Option<String>,
    pub is_scheduled_task: bool,
    pub user_working_dir: Option<PathBuf>,
    pub supports_images: bool,
    pub supports_reasoning: bool,
    pub mode: ChatMode,
    pub origin: Origin,
    pub declined_apps: Vec<String>,
    pub selected_text: Option<String>,
    pub selected_text_source: Option<SelectedTextSource>,
    pub previous_conversation: Vec<PreviousMessage>,
    pub persist: Option<bool>,
    pub history_mode: Option<HistoryMode>,
    /// Used by ACP only; the built-in path ignores attachments, as TS does.
    pub attachments: Vec<Attachment>,
    pub reasoning_effort: Option<String>,
    pub reasoning_summary: Option<String>,
}

impl ChatTurnRequest {
    /// `shouldPersist` (`chat-persistence-mode.ts`): scheduled runs never persist; otherwise
    /// `persist` wins, then the deprecated `historyMode === 'local'`, else persist.
    pub fn should_persist(&self) -> bool {
        if self.is_scheduled_task {
            return false;
        }
        if let Some(persist) = self.persist {
            return persist;
        }
        match self.history_mode {
            Some(mode) => mode == HistoryMode::Local,
            None => true,
        }
    }
}

/// Parses a `/chat` body the way `ChatRequestSchema.safeParse` does. `Err` carries zod's issue
/// array, ready for the 400 body.
pub fn parse_chat_request(body: &Value) -> Result<ChatRequest, Vec<Value>> {
    let root: &[Value] = &[];
    let Some(object) = body.as_object() else {
        // Both branches reject a non-object identically, and both rejections abort.
        let mut issues = zod::Issues::default();
        issues.invalid_type(root, "object", Some(body));
        let list = issues.into_list();
        return Err(vec![zod::union_issue(root, vec![list.clone(), list])]);
    };

    let mut input_issues = zod::Issues::default();
    let input = chat_input(object, &mut input_issues);

    let mut acp_issues = input_issues.clone();
    let acp_target = acp_target(object.get("target"), &mut acp_issues);
    if acp_issues.is_empty()
        && let (Some(fields), Some((kind, agent_id))) = (input.clone(), acp_target)
    {
        return Ok(fields.into_request(RequestTarget::Acp { kind, agent_id }, None));
    }

    let mut browseros_issues = zod::Issues::default();
    let llm = llm_fields(object, &mut browseros_issues);
    browseros_issues.extend(input_issues);
    let target_provider_id = browseros_target(object.get("target"), &mut browseros_issues);
    if browseros_issues.is_empty()
        && let Some(fields) = input
    {
        let provider_id = [
            target_provider_id,
            llm.provider_id,
            llm.inline.provider.clone(),
        ]
        .into_iter()
        .flatten()
        .find(|id| !id.is_empty());
        return Ok(fields.into_request(
            RequestTarget::BrowserOs {
                provider_id,
                inline: Box::new(llm.inline),
            },
            Some((llm.reasoning_effort, llm.reasoning_summary)),
        ));
    }

    // zod 4 unions: a lone branch whose issues are all continuable checks wins outright. The
    // browseros branch ends in a transform, so a failed browseros parse always counts as aborted
    // and only the ACP branch can win this way.
    if !acp_issues.aborted() {
        return Err(acp_issues.into_list());
    }
    Err(vec![zod::union_issue(
        root,
        vec![acp_issues.into_list(), browseros_issues.into_list()],
    )])
}

/// `ChatInputSchema` after zod defaults, before the union decides the target.
#[derive(Clone)]
struct ChatInputFields {
    conversation_id: Uuid,
    message: String,
    turn: ChatTurnRequest,
}

impl ChatInputFields {
    fn into_request(
        mut self,
        target: RequestTarget,
        reasoning: Option<(Option<String>, Option<String>)>,
    ) -> ChatRequest {
        if let Some((effort, summary)) = reasoning {
            self.turn.reasoning_effort = effort;
            self.turn.reasoning_summary = summary;
        }
        ChatRequest {
            conversation_id: self.conversation_id,
            message: self.message,
            target,
            input: self.turn,
        }
    }
}

const PROVIDERS: &[&str] = &[
    "anthropic",
    "openai",
    "google",
    "openrouter",
    "azure",
    "ollama",
    "lmstudio",
    "bedrock",
    "openai-compatible",
    "moonshot",
    "chatgpt-pro",
    "github-copilot",
    "qwen-code",
];
const REASONING_EFFORTS: &[&str] = &["none", "low", "medium", "high", "xhigh", "max"];
const REASONING_SUMMARIES: &[&str] = &["auto", "concise", "detailed"];
const ACP_TYPES: &[&str] = &["claude", "codex", "custom"];

/// `ChatInputSchema` fields in shape order (zod reports issues in schema key order, not body
/// order). Returns `None` when any field failed.
fn chat_input(object: &Map<String, Value>, issues: &mut zod::Issues) -> Option<ChatInputFields> {
    let before = issues.len();
    let at = |key: &str| [Value::from(key)];
    let conversation_id = zod::uuid(object.get("conversationId"), &at("conversationId"), issues);
    let message = zod::string(object.get("message"), &at("message"), issues, true);
    let context_window_size = zod::number(
        object.get("contextWindowSize"),
        &at("contextWindowSize"),
        issues,
    );
    let browser_context =
        browser_context(object.get("browserContext"), &at("browserContext"), issues);
    let user_system_prompt = zod::string(
        object.get("userSystemPrompt"),
        &at("userSystemPrompt"),
        issues,
        true,
    );
    let is_scheduled_task = zod::boolean(
        object.get("isScheduledTask"),
        &at("isScheduledTask"),
        issues,
    );
    let user_working_dir = zod::string_min1(
        object.get("userWorkingDir"),
        &at("userWorkingDir"),
        issues,
        true,
    );
    let supports_images = zod::boolean(object.get("supportsImages"), &at("supportsImages"), issues);
    let supports_reasoning = zod::boolean(
        object.get("supportsReasoning"),
        &at("supportsReasoning"),
        issues,
    );
    let mode = zod::option(object.get("mode"), &at("mode"), &["chat", "agent"], issues);
    let origin = zod::option(
        object.get("origin"),
        &at("origin"),
        &["sidepanel", "newtab"],
        issues,
    );
    let declined_apps = zod::string_array(object.get("declinedApps"), &at("declinedApps"), issues);
    let selected_text = zod::string(
        object.get("selectedText"),
        &at("selectedText"),
        issues,
        true,
    );
    let selected_text_source = selected_text_source(
        object.get("selectedTextSource"),
        &at("selectedTextSource"),
        issues,
    );
    let previous_conversation = previous_conversation(
        object.get("previousConversation"),
        &at("previousConversation"),
        issues,
    );
    let persist = zod::boolean(object.get("persist"), &at("persist"), issues);
    let history_mode = zod::option(
        object.get("historyMode"),
        &at("historyMode"),
        &["local", "cloud"],
        issues,
    );
    let attachments = attachments(object.get("attachments"), &at("attachments"), issues);
    if issues.len() != before {
        return None;
    }

    Some(ChatInputFields {
        conversation_id: conversation_id?,
        message: message.unwrap_or_default(),
        turn: ChatTurnRequest {
            context_window_size,
            browser_context,
            user_system_prompt,
            is_scheduled_task: is_scheduled_task.unwrap_or(false),
            user_working_dir: user_working_dir.map(PathBuf::from),
            supports_images: supports_images.unwrap_or(true),
            supports_reasoning: supports_reasoning.unwrap_or(true),
            mode: if mode == Some("chat") {
                ChatMode::Chat
            } else {
                ChatMode::Agent
            },
            origin: if origin == Some("newtab") {
                Origin::Newtab
            } else {
                Origin::Sidepanel
            },
            declined_apps: declined_apps.unwrap_or_default(),
            selected_text,
            selected_text_source,
            previous_conversation: previous_conversation.unwrap_or_default(),
            persist,
            history_mode: history_mode.map(|mode| {
                if mode == "local" {
                    HistoryMode::Local
                } else {
                    HistoryMode::Cloud
                }
            }),
            attachments: attachments.unwrap_or_default(),
            reasoning_effort: None,
            reasoning_summary: None,
        },
    })
}

/// The browseros branch's own fields: `LLMConfigSchema.partial().extend({model: min(1),
/// upstreamProvider})`.
struct LlmFields {
    inline: InlineLlm,
    /// The body's `providerId` (the transform's second choice for `target.providerId`).
    provider_id: Option<String>,
    reasoning_effort: Option<String>,
    reasoning_summary: Option<String>,
}

/// Parses [`LlmFields`] in shape order (`model` keeps its `LLMConfigSchema` position through
/// `.extend`; `upstreamProvider` is appended).
fn llm_fields(object: &Map<String, Value>, issues: &mut zod::Issues) -> LlmFields {
    let at = |key: &str| [Value::from(key)];
    let text =
        |key: &str, issues: &mut zod::Issues| zod::string(object.get(key), &at(key), issues, true);
    let provider = zod::option(object.get("provider"), &at("provider"), PROVIDERS, issues);
    let provider_id = text("providerId", issues);
    let model = zod::string_min1(object.get("model"), &at("model"), issues, true);
    let api_key = text("apiKey", issues);
    let base_url = text("baseUrl", issues);
    let headers = headers(object.get("headers"), &at("headers"), issues);
    let resource_name = text("resourceName", issues);
    let region = text("region", issues);
    let access_key_id = text("accessKeyId", issues);
    let secret_access_key = text("secretAccessKey", issues);
    let session_token = text("sessionToken", issues);
    let reasoning_effort = zod::option(
        object.get("reasoningEffort"),
        &at("reasoningEffort"),
        REASONING_EFFORTS,
        issues,
    );
    let reasoning_summary = zod::option(
        object.get("reasoningSummary"),
        &at("reasoningSummary"),
        REASONING_SUMMARIES,
        issues,
    );
    let upstream_provider = text("upstreamProvider", issues);
    LlmFields {
        inline: InlineLlm {
            provider: provider.map(str::to_owned),
            model,
            api_key,
            base_url,
            headers,
            resource_name,
            region,
            access_key_id,
            secret_access_key,
            session_token,
            upstream_provider,
        },
        provider_id,
        reasoning_effort: reasoning_effort.map(str::to_owned),
        reasoning_summary: reasoning_summary.map(str::to_owned),
    }
}

/// `AcpAgentTargetSchema`: a discriminated union on `type`.
fn acp_target(value: Option<&Value>, issues: &mut zod::Issues) -> Option<(AgentKind, Uuid)> {
    let path = [Value::from("target")];
    let Some(object) = value.and_then(Value::as_object) else {
        issues.invalid_type_code_first(&path, "object", value);
        return None;
    };
    let kind = match object.get("type").and_then(Value::as_str) {
        Some("claude") => AgentKind::Claude,
        Some("codex") => AgentKind::Codex,
        Some("custom") => AgentKind::Custom,
        _ => {
            issues.no_matching_discriminator(&zod::child(&path, "type"), ACP_TYPES);
            return None;
        }
    };
    let agent_id = zod::uuid(object.get("agentId"), &zod::child(&path, "agentId"), issues)?;
    Some((kind, agent_id))
}

/// `BrowserOsAgentTargetSchema.partial({providerId: true}).optional()`; returns `providerId`.
fn browseros_target(value: Option<&Value>, issues: &mut zod::Issues) -> Option<String> {
    let path = [Value::from("target")];
    let value = value?;
    let Some(object) = value.as_object() else {
        issues.invalid_type(&path, "object", Some(value));
        return None;
    };
    if object.get("type").and_then(Value::as_str) != Some("browseros") {
        issues.invalid_literal(&zod::child(&path, "type"), "browseros");
    }
    zod::string_min1(
        object.get("providerId"),
        &zod::child(&path, "providerId"),
        issues,
        true,
    )
}

/// `BrowserContextSchema`, rebuilt without unknown keys (zod strips them).
fn browser_context(
    value: Option<&Value>,
    path: &[Value],
    issues: &mut zod::Issues,
) -> Option<BrowserContext> {
    let value = value?;
    let Some(object) = value.as_object() else {
        issues.invalid_type(path, "object", Some(value));
        return None;
    };
    let mut context = BrowserContext::new();
    context.window_id = zod::number(
        object.get("windowId"),
        &zod::child(path, "windowId"),
        issues,
    );
    context.active_tab = object
        .get("activeTab")
        .and_then(|tab| browser_tab(tab, &zod::child(path, "activeTab"), issues));
    context.selected_tabs = tab_list(
        object.get("selectedTabs"),
        &zod::child(path, "selectedTabs"),
        issues,
    );
    context.tabs = tab_list(object.get("tabs"), &zod::child(path, "tabs"), issues);
    context.enabled_mcp_servers = zod::string_array(
        object.get("enabledMcpServers"),
        &zod::child(path, "enabledMcpServers"),
        issues,
    );
    context.custom_mcp_servers = zod::array(
        object.get("customMcpServers"),
        &zod::child(path, "customMcpServers"),
        issues,
        |item, path, issues| {
            let Some(server) = item.as_object() else {
                issues.invalid_type(path, "object", Some(item));
                return None;
            };
            let name = zod::string(server.get("name"), &zod::child(path, "name"), issues, false);
            let url = zod::url(server.get("url"), &zod::child(path, "url"), issues);
            Some(server_api::CustomMcpServer::new(name?, url?))
        },
    );
    Some(context)
}

fn tab_list(
    value: Option<&Value>,
    path: &[Value],
    issues: &mut zod::Issues,
) -> Option<Vec<server_api::BrowserTab>> {
    zod::array(value, path, issues, browser_tab)
}

/// `TabSchema`: `{id: number, url?, title?, pageId?: number}`.
fn browser_tab(
    value: &Value,
    path: &[Value],
    issues: &mut zod::Issues,
) -> Option<server_api::BrowserTab> {
    let Some(object) = value.as_object() else {
        issues.invalid_type(path, "object", Some(value));
        return None;
    };
    let id = zod::required_number(object.get("id"), &zod::child(path, "id"), issues);
    let url = zod::string(object.get("url"), &zod::child(path, "url"), issues, true);
    let title = zod::string(
        object.get("title"),
        &zod::child(path, "title"),
        issues,
        true,
    );
    let page_id = zod::number(object.get("pageId"), &zod::child(path, "pageId"), issues);
    let mut tab = server_api::BrowserTab::new(id?);
    tab.url = url;
    tab.title = title;
    tab.page_id = page_id;
    Some(tab)
}

fn selected_text_source(
    value: Option<&Value>,
    path: &[Value],
    issues: &mut zod::Issues,
) -> Option<SelectedTextSource> {
    let value = value?;
    let Some(object) = value.as_object() else {
        issues.invalid_type(path, "object", Some(value));
        return None;
    };
    let url = zod::string(object.get("url"), &zod::child(path, "url"), issues, false);
    let title = zod::string(
        object.get("title"),
        &zod::child(path, "title"),
        issues,
        false,
    );
    Some(SelectedTextSource::new(url?, title?))
}

/// `PreviousConversationSchema`: an array of `{role, content}` or a string (non-blank → one user
/// entry, blank → absent). The union's nested issue paths are relative to the field.
fn previous_conversation(
    value: Option<&Value>,
    path: &[Value],
    issues: &mut zod::Issues,
) -> Option<Vec<PreviousMessage>> {
    let value = value?;
    if let Some(text) = value.as_str() {
        return (!text.trim().is_empty()).then(|| {
            vec![PreviousMessage::new(
                server_api::PreviousMessageRole::User,
                text.to_owned(),
            )]
        });
    }
    let mut array_issues = zod::Issues::default();
    let messages = zod::array(Some(value), &[], &mut array_issues, |item, path, issues| {
        let Some(object) = item.as_object() else {
            issues.invalid_type(path, "object", Some(item));
            return None;
        };
        let role = zod::required_option(
            object.get("role"),
            &zod::child(path, "role"),
            &["user", "assistant"],
            issues,
        )
        .map(|role| {
            if role == "assistant" {
                server_api::PreviousMessageRole::Assistant
            } else {
                server_api::PreviousMessageRole::User
            }
        });
        let content = zod::string(
            object.get("content"),
            &zod::child(path, "content"),
            issues,
            false,
        );
        Some(PreviousMessage::new(role?, content?))
    });
    if array_issues.is_empty() {
        return messages;
    }
    let mut string_issues = zod::Issues::default();
    string_issues.invalid_type(&[], "string", Some(value));
    issues.push_union(
        path,
        vec![array_issues.into_list(), string_issues.into_list()],
    );
    None
}

fn attachments(
    value: Option<&Value>,
    path: &[Value],
    issues: &mut zod::Issues,
) -> Option<Vec<Attachment>> {
    zod::array(value, path, issues, |item, path, issues| {
        let Some(object) = item.as_object() else {
            issues.invalid_type(path, "object", Some(item));
            return None;
        };
        let media_type = zod::string_min1(
            object.get("mediaType"),
            &zod::child(path, "mediaType"),
            issues,
            false,
        );
        let data = zod::string_min1(object.get("data"), &zod::child(path, "data"), issues, false);
        Some(Attachment::new(media_type?, data?))
    })
}

/// `LLMHeadersSchema`: a record of RFC 7230 token names to visible-ASCII/latin1 values, with
/// case-insensitive duplicates rejected by a refinement that runs only if no key failed.
fn headers(
    value: Option<&Value>,
    path: &[Value],
    issues: &mut zod::Issues,
) -> Option<BTreeMap<String, String>> {
    let value = value?;
    let Some(object) = value.as_object() else {
        issues.invalid_type(path, "record", Some(value));
        return None;
    };
    let mut record_issues = zod::Issues::default();
    let mut headers = BTreeMap::new();
    for (name, value) in object {
        let entry_path = zod::child(path, name.as_str());
        if !is_header_name(name) {
            record_issues.invalid_key(&entry_path, zod::HEADER_NAME_PATTERN);
            continue;
        }
        match value.as_str() {
            Some(text) if is_header_value(text) => {
                headers.insert(name.clone(), text.to_owned());
            }
            Some(_) => record_issues.invalid_regex(&entry_path, zod::HEADER_VALUE_PATTERN),
            None => record_issues.invalid_type(&entry_path, "string", Some(value)),
        }
    }
    if !record_issues.aborted() {
        let mut seen = std::collections::HashSet::new();
        for name in object.keys() {
            if !seen.insert(name.to_lowercase()) {
                record_issues.duplicate_header(&zod::child(path, name.as_str()));
            }
        }
    }
    let ok = record_issues.is_empty();
    issues.extend(record_issues);
    ok.then_some(headers)
}

fn is_header_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+.^_`|~-".contains(&byte))
}

/// `[\t\x20-\x7e\x80-\xff]*` over UTF-16 code units: anything above U+00FF fails.
fn is_header_value(value: &str) -> bool {
    value.chars().all(|ch| {
        ch == '\t' || ('\u{20}'..='\u{7e}').contains(&ch) || ('\u{80}'..='\u{ff}').contains(&ch)
    })
}

// ───── provider hydration (chat-provider-config.ts) and resolution (llm/config.ts) ─────

/// What a provider id names in the `providers` table.
#[derive(Debug, Clone, PartialEq)]
pub enum ProviderRecord {
    Llm(Box<ProviderRow>),
    /// A `kind = 'acp'` row: naming it on the browseros path is a 400.
    CodingAgent {
        id: String,
    },
}

/// A browseros request after the stored row (if any) was laid over its inline fields.
#[derive(Debug, Clone, PartialEq)]
pub struct HydratedProvider {
    pub provider: String,
    pub provider_id: String,
    pub inline: InlineLlm,
    /// The caller must be a trusted app when the server supplies the credentials.
    pub used_stored_provider: bool,
    pub context_window_size: Option<f64>,
    pub supports_images: bool,
    pub reasoning_effort: Option<String>,
    pub reasoning_summary: Option<String>,
}

/// `hydrateChatProvider`. `row` is the lookup of `named_id` (or of the default provider when
/// `named_id` is `None`). `Err` is the 400 `{"error": …}` text.
pub fn hydrate_provider(
    inline: &InlineLlm,
    named_id: Option<&str>,
    turn: &ChatTurnRequest,
    row: Option<ProviderRecord>,
) -> Result<HydratedProvider, String> {
    let row = match row {
        Some(ProviderRecord::CodingAgent { id }) => {
            return Err(format!(
                "Provider {id} is a coding agent and cannot serve a browseros chat request"
            ));
        }
        Some(ProviderRecord::Llm(row)) => Some(row),
        None => None,
    };
    let none_selected = || "No provider given and none is selected".to_owned();

    let Some(row) = row else {
        let Some(provider) = inline.provider.clone() else {
            return Err(match named_id {
                Some(id) => format!("Unknown provider {id}"),
                None => none_selected(),
            });
        };
        let provider_id = named_id.map(str::to_owned).ok_or_else(none_selected)?;
        return Ok(HydratedProvider {
            provider,
            provider_id,
            inline: inline.clone(),
            used_stored_provider: false,
            context_window_size: turn.context_window_size,
            supports_images: turn.supports_images,
            reasoning_effort: turn.reasoning_effort.clone(),
            reasoning_summary: turn.reasoning_summary.clone(),
        });
    };

    // `{...request, ...providerRowToLlmConfig(row)}`: every row-config key is present (possibly
    // undefined), so the row replaces each inline LLM field; `upstreamProvider` is not one.
    Ok(HydratedProvider {
        provider: row.provider_type.clone(),
        provider_id: row.id.clone(),
        inline: InlineLlm {
            provider: Some(row.provider_type.clone()),
            model: row.model_id.clone(),
            api_key: row.api_key.clone(),
            base_url: row.base_url.clone(),
            headers: row.headers.clone(),
            resource_name: row.resource_name.clone(),
            region: row.region.clone(),
            access_key_id: row.access_key_id.clone(),
            secret_access_key: row.secret_access_key.clone(),
            session_token: row.session_token.clone(),
            upstream_provider: inline.upstream_provider.clone(),
        },
        used_stored_provider: true,
        context_window_size: row
            .context_window
            .map(|window| window as f64)
            .or(turn.context_window_size),
        supports_images: row.supports_images,
        reasoning_effort: row.reasoning_effort.clone(),
        reasoning_summary: row.reasoning_summary.clone(),
    })
}

/// `SERVER_CREDENTIALED_PROVIDERS`: the request names a provider whose token the server holds.
pub fn is_server_credentialed(provider: &str) -> bool {
    OAuthProvider::from_str(provider).is_ok()
}

/// `MOCK_MODEL_ID` of `mock-language-model.ts`.
const MOCK_MODEL_ID: &str = "browseros-test-mock";

/// `resolveLLMConfig` + the agent config assembly of `processBrowserOsMessage`: OAuth tokens for
/// the three subscription providers (with their default models), `model is required` otherwise,
/// `{{conversationId}}` substituted into user headers. Errors are pre-stream 500 envelopes.
pub async fn resolve_llm(
    hydrated: &HydratedProvider,
    turn: &ChatTurnRequest,
    conversation_id: Uuid,
    oauth: &TokenManager,
    use_mock_llm: bool,
) -> Result<ResolvedLlm, ChatError> {
    let inline = &hydrated.inline;
    let mut model = inline.model.clone();
    let mut api_key = inline.api_key.clone();
    let mut upstream_provider = inline.upstream_provider.clone();
    let mut account_id = None;
    let oauth_provider = OAuthProvider::from_str(&hydrated.provider).ok();

    if use_mock_llm {
        // Ahead of every credential branch: the harness never needs a token it does not have.
        model = model.or_else(|| Some(MOCK_MODEL_ID.to_owned()));
    } else if let Some(provider) = oauth_provider {
        let (display_name, default_model) = match provider {
            OAuthProvider::ChatGptPro => ("ChatGPT", "gpt-5.5"),
            OAuthProvider::GitHubCopilot => ("GitHub Copilot", "gpt-5-mini"),
            OAuthProvider::QwenCode => ("Qwen Code", "coder-model"),
        };
        let credentials = oauth.credentials(provider).await.map_err(|error| {
            let message = match error {
                OAuthError::Expired(text) | OAuthError::Upstream(text) => text,
                OAuthError::NotAuthenticated(_) | OAuthError::Unavailable(_) => {
                    format!("Not authenticated with {display_name}. Please login first.")
                }
            };
            config_error(&message)
        })?;
        model = Some(
            model
                .filter(|model| !model.is_empty())
                .unwrap_or_else(|| default_model.to_owned()),
        );
        api_key = Some(credentials.access_token);
        if provider == OAuthProvider::ChatGptPro {
            upstream_provider = Some("openai".to_owned());
            account_id = credentials.account_id;
        }
    }

    let Some(model) = model.filter(|model| !model.is_empty()) else {
        return Err(config_error(&format!(
            "model is required for {} provider",
            hydrated.provider
        )));
    };
    let provider = ProviderKind::from_str(&hydrated.provider)
        .map_err(|_| config_error(&format!("Unknown provider: {}", hydrated.provider)))?;

    // Managed OAuth providers deliberately never forward user headers (provider-factory.ts).
    let headers = if oauth_provider.is_some() {
        None
    } else {
        inline.headers.as_ref().map(|headers| {
            headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.clone(),
                        value.replace("{{conversationId}}", &conversation_id.to_string()),
                    )
                })
                .collect()
        })
    };

    Ok(ResolvedLlm {
        config: LlmConfig {
            provider,
            model: model.clone(),
            api_key,
            base_url: inline.base_url.clone(),
            headers,
            resource_name: inline.resource_name.clone(),
            region: inline.region.clone(),
            access_key_id: inline.access_key_id.clone(),
            secret_access_key: inline.secret_access_key.clone(),
            session_token: inline.session_token.clone(),
            account_id,
            upstream_provider,
            conversation_id: conversation_id.to_string(),
        },
        info: ModelInfo {
            provider,
            model_id: model,
            context_window: hydrated.context_window_size,
            supports_images: hydrated.supports_images,
        },
        reasoning: ReasoningSettings {
            supports_reasoning: turn.supports_reasoning,
            effort: hydrated
                .reasoning_effort
                .as_deref()
                .and_then(parse_reasoning_effort),
            summary: match hydrated.reasoning_summary.as_deref() {
                Some("concise") => ReasoningSummary::Concise,
                Some("detailed") => ReasoningSummary::Detailed,
                _ => ReasoningSummary::Auto,
            },
        },
    })
}

fn parse_reasoning_effort(value: &str) -> Option<ReasoningEffort> {
    Some(match value {
        "none" => ReasoningEffort::None,
        "low" => ReasoningEffort::Low,
        "medium" => ReasoningEffort::Medium,
        "high" => ReasoningEffort::High,
        "xhigh" => ReasoningEffort::XHigh,
        "max" => ReasoningEffort::Max,
        _ => return None,
    })
}

/// A credential/config failure, enveloped as the TS server's `onError` does for a plain `Error`.
fn config_error(message: &str) -> ChatError {
    super::server_error(message)
}

/// zod 4.6.5's issue objects, built in its key order. Each issue records whether it aborts its
/// branch (`continue !== true`): type and enum failures do, string checks do not. That flag is
/// what zod's union uses to pick a lone surviving branch.
mod zod {
    use serde_json::{Value, json};

    pub(super) const HEADER_NAME_PATTERN: &str = "/^[!#$%&'*+.^_`|~0-9A-Za-z-]+(?![\\s\\S])/";
    pub(super) const HEADER_VALUE_PATTERN: &str = "/^[\\t\\x20-\\x7e\\x80-\\xff]*(?![\\s\\S])/";

    #[derive(Debug, Clone, Default)]
    pub(super) struct Issues {
        list: Vec<Value>,
        aborted: bool,
    }

    pub(super) fn child(path: &[Value], segment: impl Into<Value>) -> Vec<Value> {
        let mut path = path.to_vec();
        path.push(segment.into());
        path
    }

    /// zod's `received` word: `typeof`, with `null` and `array` split out.
    fn received(value: Option<&Value>) -> &'static str {
        match value {
            None => "undefined",
            Some(Value::Null) => "null",
            Some(Value::Bool(_)) => "boolean",
            Some(Value::Number(_)) => "number",
            Some(Value::String(_)) => "string",
            Some(Value::Array(_)) => "array",
            Some(Value::Object(_)) => "object",
        }
    }

    fn uuid_issue(path: &[Value]) -> Value {
        json!({
            "origin": "string",
            "code": "invalid_format",
            "format": "uuid",
            "pattern": format!("/{}/", crate::http::extract::UUID_PATTERN),
            "path": path,
            "message": "Invalid UUID",
        })
    }

    pub(super) fn union_issue(path: &[Value], branches: Vec<Vec<Value>>) -> Value {
        json!({ "code": "invalid_union", "errors": branches, "path": path, "message": "Invalid input" })
    }

    impl Issues {
        fn push(&mut self, issue: Value, aborts: bool) {
            self.list.push(issue);
            self.aborted |= aborts;
        }

        pub(super) fn len(&self) -> usize {
            self.list.len()
        }

        pub(super) fn is_empty(&self) -> bool {
            self.list.is_empty()
        }

        pub(super) fn aborted(&self) -> bool {
            self.aborted
        }

        pub(super) fn extend(&mut self, other: Issues) {
            self.list.extend(other.list);
            self.aborted |= other.aborted;
        }

        pub(super) fn into_list(self) -> Vec<Value> {
            self.list
        }

        /// A schema's own type check (`z.string()`, `z.object()`, …): `expected` comes first.
        pub(super) fn invalid_type(&mut self, path: &[Value], expected: &str, got: Option<&Value>) {
            let message = format!(
                "Invalid input: expected {expected}, received {}",
                received(got)
            );
            self.push(
                json!({ "expected": expected, "code": "invalid_type", "path": path, "message": message }),
                true,
            );
        }

        /// A discriminated union's object check: `code` comes first.
        pub(super) fn invalid_type_code_first(
            &mut self,
            path: &[Value],
            expected: &str,
            got: Option<&Value>,
        ) {
            let message = format!(
                "Invalid input: expected {expected}, received {}",
                received(got)
            );
            self.push(
                json!({ "code": "invalid_type", "expected": expected, "path": path, "message": message }),
                true,
            );
        }

        pub(super) fn invalid_option(&mut self, path: &[Value], values: &[&str]) {
            let quoted: Vec<String> = values.iter().map(|value| format!("\"{value}\"")).collect();
            let message = format!("Invalid option: expected one of {}", quoted.join("|"));
            self.push(
                json!({ "code": "invalid_value", "values": values, "path": path, "message": message }),
                true,
            );
        }

        pub(super) fn invalid_literal(&mut self, path: &[Value], literal: &str) {
            let message = format!("Invalid input: expected \"{literal}\"");
            self.push(
                json!({ "code": "invalid_value", "values": [literal], "path": path, "message": message }),
                true,
            );
        }

        pub(super) fn too_small(&mut self, path: &[Value]) {
            self.push(
                json!({
                    "origin": "string",
                    "code": "too_small",
                    "minimum": 1,
                    "inclusive": true,
                    "path": path,
                    "message": "Too small: expected string to have >=1 characters",
                }),
                false,
            );
        }

        pub(super) fn invalid_uuid(&mut self, path: &[Value]) {
            self.push(uuid_issue(path), false);
        }

        pub(super) fn invalid_url(&mut self, path: &[Value]) {
            self.push(
                json!({ "code": "invalid_format", "format": "url", "path": path, "message": "Invalid URL" }),
                false,
            );
        }

        pub(super) fn invalid_regex(&mut self, path: &[Value], pattern: &str) {
            self.push(regex_issue(path, pattern), false);
        }

        pub(super) fn invalid_key(&mut self, path: &[Value], pattern: &str) {
            self.push(
                json!({
                    "code": "invalid_key",
                    "origin": "record",
                    "issues": [regex_issue(&[], pattern)],
                    "path": path,
                    "message": "Invalid key in record",
                }),
                true,
            );
        }

        pub(super) fn duplicate_header(&mut self, path: &[Value]) {
            self.push(
                json!({ "code": "custom", "message": "Duplicate header name", "path": path }),
                false,
            );
        }

        pub(super) fn no_matching_discriminator(&mut self, path: &[Value], options: &[&str]) {
            let quoted: Vec<String> = options.iter().map(|option| format!("'{option}'")).collect();
            let message = format!(
                "Invalid discriminator value. Expected {}",
                quoted.join(" | ")
            );
            self.push(
                json!({
                    "code": "invalid_union",
                    "errors": [],
                    "note": "No matching discriminator",
                    "discriminator": "type",
                    "options": options,
                    "path": path,
                    "message": message,
                }),
                true,
            );
        }

        pub(super) fn push_union(&mut self, path: &[Value], branches: Vec<Vec<Value>>) {
            self.push(union_issue(path, branches), true);
        }
    }

    fn regex_issue(path: &[Value], pattern: &str) -> Value {
        json!({
            "origin": "string",
            "code": "invalid_format",
            "format": "regex",
            "pattern": pattern,
            "path": path,
            "message": format!("Invalid string: must match pattern {pattern}"),
        })
    }

    pub(super) fn string(
        value: Option<&Value>,
        path: &[Value],
        issues: &mut Issues,
        optional: bool,
    ) -> Option<String> {
        match value {
            None if optional => None,
            Some(Value::String(text)) => Some(text.clone()),
            other => {
                issues.invalid_type(path, "string", other);
                None
            }
        }
    }

    pub(super) fn string_min1(
        value: Option<&Value>,
        path: &[Value],
        issues: &mut Issues,
        optional: bool,
    ) -> Option<String> {
        let text = string(value, path, issues, optional)?;
        if text.is_empty() {
            issues.too_small(path);
            return None;
        }
        Some(text)
    }

    pub(super) fn uuid(
        value: Option<&Value>,
        path: &[Value],
        issues: &mut Issues,
    ) -> Option<uuid::Uuid> {
        let text = string(value, path, issues, false)?;
        let parsed = crate::http::extract::ZodUuid::parse(&text).map(|uuid| uuid.0);
        if parsed.is_none() {
            issues.invalid_uuid(path);
        }
        parsed
    }

    /// `z.string().url()`: the value is trimmed, then must parse as a WHATWG URL; the trimmed
    /// string is the output (zod 4 quirk).
    pub(super) fn url(
        value: Option<&Value>,
        path: &[Value],
        issues: &mut Issues,
    ) -> Option<String> {
        let text = string(value, path, issues, false)?;
        let trimmed = text.trim();
        if url::Url::parse(trimmed).is_err() {
            issues.invalid_url(path);
            return None;
        }
        Some(trimmed.to_owned())
    }

    pub(super) fn number(
        value: Option<&Value>,
        path: &[Value],
        issues: &mut Issues,
    ) -> Option<f64> {
        value.and_then(|value| required_number(Some(value), path, issues))
    }

    pub(super) fn required_number(
        value: Option<&Value>,
        path: &[Value],
        issues: &mut Issues,
    ) -> Option<f64> {
        match value.and_then(Value::as_f64) {
            Some(number) => Some(number),
            None => {
                issues.invalid_type(path, "number", value);
                None
            }
        }
    }

    pub(super) fn boolean(
        value: Option<&Value>,
        path: &[Value],
        issues: &mut Issues,
    ) -> Option<bool> {
        match value? {
            Value::Bool(flag) => Some(*flag),
            other => {
                issues.invalid_type(path, "boolean", Some(other));
                None
            }
        }
    }

    /// An optional `z.enum([...])`; returns the matched option.
    pub(super) fn option<'a>(
        value: Option<&Value>,
        path: &[Value],
        options: &[&'a str],
        issues: &mut Issues,
    ) -> Option<&'a str> {
        let value = value?;
        let matched = value
            .as_str()
            .and_then(|text| options.iter().copied().find(|option| *option == text));
        if matched.is_none() {
            issues.invalid_option(path, options);
        }
        matched
    }

    /// A required `z.enum([...])`: absent fails the same way a wrong value does.
    pub(super) fn required_option<'a>(
        value: Option<&Value>,
        path: &[Value],
        options: &[&'a str],
        issues: &mut Issues,
    ) -> Option<&'a str> {
        if value.is_none() {
            issues.invalid_option(path, options);
            return None;
        }
        option(value, path, options, issues)
    }

    /// An optional `z.array(item)`; element issues carry their index.
    pub(super) fn array<T>(
        value: Option<&Value>,
        path: &[Value],
        issues: &mut Issues,
        mut item: impl FnMut(&Value, &[Value], &mut Issues) -> Option<T>,
    ) -> Option<Vec<T>> {
        let value = value?;
        let Some(items) = value.as_array() else {
            issues.invalid_type(path, "array", Some(value));
            return None;
        };
        let mut parsed = Vec::with_capacity(items.len());
        let mut failed = false;
        for (index, element) in items.iter().enumerate() {
            match item(element, &child(path, index), issues) {
                Some(element) => parsed.push(element),
                None => failed = true,
            }
        }
        (!failed).then_some(parsed)
    }

    pub(super) fn string_array(
        value: Option<&Value>,
        path: &[Value],
        issues: &mut Issues,
    ) -> Option<Vec<String>> {
        array(value, path, issues, |item, path, issues| {
            string(Some(item), path, issues, false)
        })
    }
}
