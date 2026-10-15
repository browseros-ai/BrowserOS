//! Renders `ProviderOptions` (the TS `AgentReasoningConfig` object) into request-body fields,
//! exactly as each AI SDK provider renders the same options: `@ai-sdk/anthropic` 4.0.63
//! (`getModelCapabilities`, `resolveAnthropicReasoningConfig`), `@ai-sdk/openai` 4.0.75 Responses
//! (`getOpenAILanguageModelCapabilities`), `@ai-sdk/google` and `@ai-sdk/amazon-bedrock`.
//!
//! The result rides in rig's `additional_params`, the last layer of rig's body merge, so these
//! fields win over rig's own mapping and rig's catalog refusals never see a typed option. Golden
//! bodies captured from the TS server (`tests/fixtures/llm/ts-requests.json`) pin the output.

use crate::llm::{ProviderKind, ProviderOptions};
use regex::Regex;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;

/// Body fields for one request: `params` merges into the provider body (for Bedrock, under
/// `additionalModelRequestFields`, where rig places raw params); `max_tokens` is the request's
/// output cap, which only some wires need set explicitly.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Overlay {
    pub params: Map<String, Value>,
    pub max_tokens: Option<u64>,
}

pub(crate) fn overlay(provider: ProviderKind, model: &str, options: &ProviderOptions) -> Overlay {
    let namespace = |name: &str| {
        options
            .0
            .get("providerOptions")
            .and_then(|namespaces| namespaces.get(name))
            .and_then(Value::as_object)
    };
    match provider {
        ProviderKind::Anthropic => {
            anthropic(model, options.0.get("reasoning").and_then(Value::as_str))
        }
        ProviderKind::OpenAi | ProviderKind::ChatGptPro => {
            openai_responses(model, namespace("openai"))
        }
        ProviderKind::Azure => openai_responses(model, namespace("azure")),
        ProviderKind::Google => google(namespace("google")),
        ProviderKind::Bedrock => bedrock(model, namespace("bedrock")),
        // `createOpenRouter({ extraBody: { reasoning: {} } })`: every OpenRouter body reasons.
        ProviderKind::OpenRouter => Overlay {
            params: Map::from_iter([("reasoning".to_owned(), json!({}))]),
            max_tokens: None,
        },
        ProviderKind::Ollama
        | ProviderKind::LmStudio
        | ProviderKind::OpenAiCompatible
        | ProviderKind::Moonshot
        | ProviderKind::GitHubCopilot
        | ProviderKind::QwenCode => Overlay::default(),
    }
}

// ───── Anthropic ─────

/// `getModelCapabilities` of `@ai-sdk/anthropic` 4.0.63, the fields the request needs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct AnthropicCapabilities {
    pub max_output_tokens: u64,
    pub adaptive_thinking: bool,
    pub xhigh_effort: bool,
    pub rejects_thinking_disabled_above_high_effort: bool,
    pub rejects_thinking_disabled: bool,
    pub known_model: bool,
}

static CLAUDE_SONNET_4: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"claude-sonnet-4(?:-|@)").ok());
static CLAUDE_OPUS_4: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"claude-opus-4(?:-|@)").ok());
/// JS `/claude-(?:instant(?:-|$)|v?2(?=$|[-.:])|3(?=$|[-.]))/`, rewritten without lookahead
/// (Rust's regex has none): the lookahead's alternatives become literal alternatives.
static CLAUDE_LEGACY: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"claude-(?:instant(?:-|$)|v?2(?:$|[-.:])|3(?:$|[-.]))").ok());

fn is_match(regex: &LazyLock<Option<Regex>>, text: &str) -> bool {
    regex.as_ref().is_some_and(|regex| regex.is_match(text))
}

pub(crate) fn anthropic_capabilities(model: &str) -> AnthropicCapabilities {
    let caps = |max, adaptive, xhigh, above_high, disabled, known| AnthropicCapabilities {
        max_output_tokens: max,
        adaptive_thinking: adaptive,
        xhigh_effort: xhigh,
        rejects_thinking_disabled_above_high_effort: above_high,
        rejects_thinking_disabled: disabled,
        known_model: known,
    };
    let has = |needle: &str| model.contains(needle);
    if has("claude-opus-5-5") {
        caps(128_000, true, true, true, true, true)
    } else if has("claude-opus-5") {
        caps(128_000, true, true, true, false, true)
    } else if has("claude-fable-5-1") || has("claude-fable-5") {
        caps(128_000, true, true, false, true, true)
    } else if has("claude-opus-4-8") || has("claude-opus-4-7") || has("claude-sonnet-5") {
        caps(128_000, true, true, false, false, true)
    } else if has("claude-sonnet-4-6") || has("claude-opus-4-6") {
        caps(128_000, true, false, false, false, true)
    } else if has("claude-sonnet-4-5") || has("claude-opus-4-5") || has("claude-haiku-4-5") {
        caps(64_000, false, false, false, false, true)
    } else if has("claude-opus-4-1") {
        caps(32_000, false, false, false, false, true)
    } else if is_match(&CLAUDE_SONNET_4, model) {
        caps(64_000, false, false, false, false, true)
    } else if is_match(&CLAUDE_OPUS_4, model) {
        caps(32_000, false, false, false, false, true)
    } else if has("claude-3-haiku") {
        caps(4096, false, false, false, false, true)
    } else if is_match(&CLAUDE_LEGACY, model) {
        caps(4096, false, false, false, false, false)
    } else if has("claude-") {
        caps(128_000, true, true, true, false, false)
    } else {
        caps(4096, false, false, false, false, false)
    }
}

/// `DEFAULT_REASONING_BUDGET_PERCENTAGES` of `@ai-sdk/provider-utils`.
fn budget_percentage(level: &str) -> Option<f64> {
    Some(match level {
        "minimal" => 0.02,
        "low" => 0.1,
        "medium" => 0.3,
        "high" => 0.6,
        "xhigh" => 0.9,
        _ => return None,
    })
}

/// `mapReasoningToProviderBudget`: `min(max, max(1024, round(max * pct)))`.
fn reasoning_budget(level: &str, max_output_tokens: u64) -> Option<u64> {
    let pct = budget_percentage(level)?;
    // JS Math.round on a positive value: half rounds up, as f64::round does.
    let scaled = (max_output_tokens as f64 * pct).round() as u64;
    Some(max_output_tokens.min(scaled.max(1024)))
}

/// The Anthropic request fields for the top-level AI SDK `reasoning` level, as `getArgs` builds
/// them with no `maxOutputTokens` and no explicit `providerOptions.anthropic`.
fn anthropic(model: &str, reasoning: Option<&str>) -> Overlay {
    let caps = anthropic_capabilities(model);
    let mut thinking: Option<Value> = None;
    let mut budget: Option<u64> = None;
    let mut effort: Option<&str> = None;
    // `isCustomReasoning`: unset and `provider-default` leave thinking to the provider.
    if let Some(level) = reasoning.filter(|level| *level != "provider-default") {
        if level == "none" {
            if caps.rejects_thinking_disabled {
                effort = Some("low");
            } else {
                thinking = Some(json!({ "type": "disabled" }));
            }
        } else if caps.adaptive_thinking {
            effort = match level {
                "minimal" | "low" => Some("low"),
                "medium" => Some("medium"),
                "high" => Some("high"),
                "xhigh" => Some(if caps.xhigh_effort { "xhigh" } else { "max" }),
                _ => None,
            };
            thinking = Some(json!({ "type": "adaptive", "display": "summarized" }));
        } else if let Some(tokens) = reasoning_budget(level, caps.max_output_tokens) {
            budget = Some(tokens);
            thinking = Some(json!({ "type": "enabled", "budget_tokens": tokens }));
        }
    }
    let thinking_type = |thinking: &Option<Value>| {
        thinking
            .as_ref()
            .and_then(|thinking| thinking.get("type"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    if caps.rejects_thinking_disabled {
        match thinking_type(&thinking).as_deref() {
            Some("disabled") => thinking = None,
            Some("enabled") => {
                thinking = Some(json!({ "type": "adaptive" }));
                budget = None;
            }
            _ => {}
        }
    }
    if caps.rejects_thinking_disabled_above_high_effort
        && thinking_type(&thinking).as_deref() == Some("disabled")
        && matches!(effort, Some("xhigh" | "max"))
    {
        effort = Some("high");
    }
    let is_thinking = matches!(
        thinking_type(&thinking).as_deref(),
        Some("enabled" | "adaptive")
    );
    let mut max_tokens = caps.max_output_tokens;
    if is_thinking {
        max_tokens += budget.unwrap_or(0);
    }
    if caps.known_model && max_tokens > caps.max_output_tokens {
        max_tokens = caps.max_output_tokens;
    }
    let mut params = Map::new();
    params.insert("max_tokens".to_owned(), max_tokens.into());
    if let Some(thinking) = thinking {
        params.insert("thinking".to_owned(), thinking);
    }
    if let Some(effort) = effort {
        params.insert("output_config".to_owned(), json!({ "effort": effort }));
    }
    Overlay {
        params,
        max_tokens: Some(max_tokens),
    }
}

// ───── OpenAI Responses (openai, azure, chatgpt-pro) ─────

struct GptVersion {
    major: u64,
    minor: Option<u64>,
    variant: Option<String>,
}

static O_SERIES: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"^o(\d+)(?:-|$)").ok());
static GPT: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^gpt-(\d+)(?:\.(\d+))?(?:-(.+))?$").ok());

fn gpt_version(model: &str) -> Option<GptVersion> {
    let captures = GPT.as_ref()?.captures(model)?;
    Some(GptVersion {
        major: captures.get(1)?.as_str().parse().ok()?,
        minor: captures
            .get(2)
            .and_then(|minor| minor.as_str().parse().ok()),
        variant: captures.get(3).map(|variant| variant.as_str().to_owned()),
    })
}

/// `getOpenAILanguageModelCapabilities`: whether the model reasons, and which efforts it takes.
fn openai_capabilities(model: &str) -> (bool, Option<&'static [&'static str]>) {
    let o_series = O_SERIES.as_ref().is_some_and(|regex| regex.is_match(model));
    let gpt = gpt_version(model);
    let gpt_chat = gpt.as_ref().is_some_and(|gpt| {
        gpt.minor.is_none()
            && gpt
                .variant
                .as_deref()
                .is_some_and(|variant| variant.starts_with("chat"))
    });
    let gpt6 = gpt.as_ref().is_some_and(|gpt| gpt.major >= 6);
    let reasoning = o_series || gpt.as_ref().is_some_and(|gpt| gpt.major >= 5 && !gpt_chat);
    const SOL_LUNA: &[&str] = &["none", "low", "medium", "high", "xhigh", "max"];
    const GPT6: &[&str] = &["low", "medium", "high", "xhigh", "max"];
    let efforts = if model == "gpt-6-sol" || model == "gpt-6-luna" {
        Some(SOL_LUNA)
    } else if gpt6 {
        Some(GPT6)
    } else {
        None
    };
    (reasoning, efforts)
}

/// The Responses fields `getArgs` derives from `providerOptions.openai|azure`.
fn openai_responses(model: &str, options: Option<&Map<String, Value>>) -> Overlay {
    let empty = Map::new();
    let options = options.unwrap_or(&empty);
    let (capable, efforts) = openai_capabilities(model);
    let effort = options
        .get("reasoningEffort")
        .and_then(Value::as_str)
        .filter(|effort| efforts.is_none_or(|efforts| efforts.contains(effort)));
    let summary = match options.get("reasoningSummary") {
        Some(Value::String(summary)) => Some(summary.as_str()),
        Some(_) => None,
        None => effort
            .filter(|effort| *effort != "none")
            .map(|_| "detailed"),
    };
    let reasoning_model = options
        .get("forceReasoning")
        .and_then(Value::as_bool)
        .unwrap_or(capable);
    let mut params = Map::new();
    let store = options.get("store").and_then(Value::as_bool);
    if let Some(store) = store {
        params.insert("store".to_owned(), store.into());
    }
    let mut include: Option<Vec<Value>> = options.get("include").and_then(Value::as_array).cloned();
    if store == Some(false) && reasoning_model {
        let key = Value::from("reasoning.encrypted_content");
        let list = include.get_or_insert_with(Vec::new);
        if !list.contains(&key) {
            list.push(key);
        }
    }
    if let Some(include) = include {
        params.insert("include".to_owned(), Value::Array(include));
    }
    if reasoning_model && (effort.is_some() || summary.is_some()) {
        let mut reasoning = Map::new();
        if let Some(effort) = effort {
            reasoning.insert("effort".to_owned(), effort.into());
        }
        if let Some(summary) = summary {
            reasoning.insert("summary".to_owned(), summary.into());
        }
        params.insert("reasoning".to_owned(), Value::Object(reasoning));
    }
    Overlay {
        params,
        max_tokens: None,
    }
}

// ───── Google ─────

/// `providerOptions.google.thinkingConfig` is sent as `generationConfig.thinkingConfig`.
fn google(options: Option<&Map<String, Value>>) -> Overlay {
    let mut params = Map::new();
    if let Some(thinking) = options.and_then(|options| options.get("thinkingConfig")) {
        params.insert(
            "generationConfig".to_owned(),
            json!({ "thinkingConfig": thinking }),
        );
    }
    Overlay {
        params,
        max_tokens: None,
    }
}

// ───── Bedrock ─────

/// Anthropic-on-Bedrock thinking: `additionalModelRequestFields.thinking` plus
/// `maxTokens = budget + 4096` (the SDK default output cap, raised by the budget).
fn bedrock(model: &str, options: Option<&Map<String, Value>>) -> Overlay {
    let config = options.and_then(|options| options.get("reasoningConfig"));
    let enabled = config
        .and_then(|config| config.get("type"))
        .and_then(Value::as_str)
        == Some("enabled");
    let budget = config
        .and_then(|config| config.get("budgetTokens"))
        .and_then(Value::as_u64);
    match (model.contains("anthropic") && enabled, budget) {
        (true, Some(budget)) => Overlay {
            params: Map::from_iter([(
                "thinking".to_owned(),
                json!({ "type": "enabled", "budget_tokens": budget }),
            )]),
            max_tokens: Some(budget + 4096),
        },
        _ => Overlay::default(),
    }
}
