//! Per-provider reasoning options: an exact port of `agent/reasoning-config.ts:102-205`.
//! The output is the TS `AgentReasoningConfig` object verbatim — an optional
//! top-level `reasoning` level (Anthropic) and/or AI SDK `providerOptions` namespaces — so it
//! can be compared literally with the TS tests. The rig adapter (`rig::overlay`) renders it into
//! the request body exactly as each AI SDK provider would.

use super::{ProviderKind, ProviderOptions, ReasoningEffort, ReasoningSettings, ReasoningSummary};
use regex::Regex;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;

impl ReasoningEffort {
    /// The stored effort word (`LLMProvider.reasoningEffort`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "none" => Self::None,
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            "xhigh" => Self::XHigh,
            "max" => Self::Max,
            _ => return None,
        })
    }
}

impl ReasoningSummary {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Concise => "concise",
            Self::Detailed => "detailed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "auto" => Self::Auto,
            "concise" => Self::Concise,
            "detailed" => Self::Detailed,
            _ => return None,
        })
    }
}

static GEMINI_2_5: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)(^|/)gemini-2\.5(?:[.-]|$)").ok());
static GEMINI_3_PLUS: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)(^|/)gemini-(3|[4-9])").ok());

fn matches(regex: &LazyLock<Option<Regex>>, model: &str) -> bool {
    regex.as_ref().is_some_and(|regex| regex.is_match(model))
}

/// `toReasoningLevel`: the AI SDK top-level reasoning setting has no `max`.
fn reasoning_level(effort: Option<ReasoningEffort>) -> &'static str {
    match effort {
        None => "medium",
        Some(ReasoningEffort::Max) => "xhigh",
        Some(effort) => effort.as_str(),
    }
}

/// `toThinkingLevel`: Google's four-value thinking level.
fn thinking_level(effort: Option<ReasoningEffort>) -> &'static str {
    match effort {
        Some(ReasoningEffort::None) => "minimal",
        Some(ReasoningEffort::Low) => "low",
        Some(ReasoningEffort::High | ReasoningEffort::XHigh | ReasoningEffort::Max) => "high",
        Some(ReasoningEffort::Medium) | None => "medium",
    }
}

/// `toThinkingBudget`: `-1` lets the model decide.
fn thinking_budget(effort: Option<ReasoningEffort>) -> i64 {
    match effort {
        Some(ReasoningEffort::None) => 0,
        Some(ReasoningEffort::Low) => 4096,
        Some(ReasoningEffort::Medium) => 8192,
        Some(ReasoningEffort::High) => 24576,
        Some(ReasoningEffort::XHigh | ReasoningEffort::Max) => 32000,
        None => -1,
    }
}

fn namespaced(namespace: &str, options: Value) -> ProviderOptions {
    let mut out = Map::new();
    out.insert("providerOptions".to_owned(), json!({ namespace: options }));
    ProviderOptions(out)
}

/// Port of `buildAgentReasoningConfig`. `model_id` is lower-cased here, as in TS.
pub fn provider_options(
    provider: ProviderKind,
    model_id: &str,
    settings: &ReasoningSettings,
) -> ProviderOptions {
    // The catalog says this model does not reason: never request reasoning.
    if !settings.supports_reasoning {
        return ProviderOptions::default();
    }
    let effort = settings.effort;
    let summary = settings.summary.as_str();
    let model = model_id.to_lowercase();
    match provider {
        ProviderKind::OpenAi => {
            let mut openai = Map::new();
            openai.insert("reasoningSummary".to_owned(), summary.into());
            if let Some(effort) = effort {
                openai.insert("reasoningEffort".to_owned(), effort.as_str().into());
            }
            namespaced("openai", Value::Object(openai))
        }
        // Independent of OPENAI despite sharing the Responses adapter: the Codex backend needs
        // stateless reasoning continuity that plain OpenAI must not inherit.
        ProviderKind::ChatGptPro => namespaced(
            "openai",
            json!({
                "store": false,
                "reasoningEffort": effort.map_or("medium", ReasoningEffort::as_str),
                "reasoningSummary": summary,
                "include": ["reasoning.encrypted_content"],
            }),
        ),
        ProviderKind::Azure => {
            let mut azure = Map::new();
            azure.insert(
                "reasoningEffort".to_owned(),
                effort.map_or("medium", ReasoningEffort::as_str).into(),
            );
            azure.insert("reasoningSummary".to_owned(), summary.into());
            // TS forces reasoning only when the catalog confirms it (`supportsReasoning === true`);
            // the chat schema defaults the field to true, so reaching here means confirmed.
            azure.insert("forceReasoning".to_owned(), true.into());
            namespaced("azure", Value::Object(azure))
        }
        ProviderKind::Google => {
            if matches(&GEMINI_3_PLUS, &model) {
                namespaced(
                    "google",
                    json!({ "thinkingConfig": {
                        "includeThoughts": true,
                        "thinkingLevel": thinking_level(effort),
                    }}),
                )
            } else if matches(&GEMINI_2_5, &model) {
                namespaced(
                    "google",
                    json!({ "thinkingConfig": {
                        "includeThoughts": true,
                        "thinkingBudget": thinking_budget(effort),
                    }}),
                )
            } else {
                // Non-thinking Gemini (2.0, gemma): sending thinkingConfig is a 400.
                ProviderOptions::default()
            }
        }
        ProviderKind::Anthropic => {
            let mut out = Map::new();
            out.insert("reasoning".to_owned(), reasoning_level(effort).into());
            ProviderOptions(out)
        }
        ProviderKind::Bedrock => {
            // Extended thinking on Bedrock is Anthropic-only.
            if !model.contains("anthropic") {
                return ProviderOptions::default();
            }
            namespaced(
                "bedrock",
                json!({ "reasoningConfig": {
                    "type": "enabled",
                    "budgetTokens": thinking_budget(effort).max(1024),
                }}),
            )
        }
        // OpenRouter enables reasoning at model construction (`reasoning: {}` on every body);
        // the compatible providers surface `reasoning_content` on their own.
        ProviderKind::OpenRouter
        | ProviderKind::OpenAiCompatible
        | ProviderKind::LmStudio
        | ProviderKind::Ollama
        | ProviderKind::Moonshot
        | ProviderKind::GitHubCopilot
        | ProviderKind::QwenCode => ProviderOptions::default(),
    }
}
