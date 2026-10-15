//! `llm::provider_options` against the TS reasoning table: every case of
//! `apps/server/src/agent/reasoning-config.test.ts`, plus the branches that file leaves out
//! (Gemini level/budget maps, Bedrock's 1024 floor, ChatGPT/Azure defaults).

use browseros_agent::llm::{
    ProviderKind, ProviderOptions, ReasoningEffort, ReasoningSettings, ReasoningSummary,
    provider_options,
};
use serde_json::{Value, json};

fn settings(effort: Option<ReasoningEffort>) -> ReasoningSettings {
    ReasoningSettings {
        supports_reasoning: true,
        effort,
        summary: ReasoningSummary::Auto,
    }
}

fn options(provider: ProviderKind, model: &str, effort: Option<ReasoningEffort>) -> Value {
    let ProviderOptions(map) = provider_options(provider, model, &settings(effort));
    Value::Object(map)
}

#[test]
fn openai_requests_a_summary_without_store_or_include() {
    assert_eq!(
        options(ProviderKind::OpenAi, "gpt-5", None),
        json!({ "providerOptions": { "openai": { "reasoningSummary": "auto" } } })
    );
}

#[test]
fn openai_forwards_a_configured_effort() {
    assert_eq!(
        options(ProviderKind::OpenAi, "gpt-5", Some(ReasoningEffort::High)),
        json!({ "providerOptions": { "openai": { "reasoningSummary": "auto", "reasoningEffort": "high" } } })
    );
}

#[test]
fn chatgpt_pro_keeps_its_own_block_with_store_false_and_encrypted_include() {
    assert_eq!(
        options(ProviderKind::ChatGptPro, "gpt-5", None),
        json!({ "providerOptions": { "openai": {
            "store": false,
            "reasoningEffort": "medium",
            "reasoningSummary": "auto",
            "include": ["reasoning.encrypted_content"],
        } } })
    );
    assert_eq!(
        options(
            ProviderKind::ChatGptPro,
            "gpt-6-astra",
            Some(ReasoningEffort::XHigh)
        )["providerOptions"]["openai"]["reasoningEffort"],
        "xhigh"
    );
}

#[test]
fn azure_uses_the_azure_namespace_and_forces_reasoning_when_confirmed() {
    // The chat schema defaults `supportsReasoning` to true, so a resolved request is confirmed.
    assert_eq!(
        options(ProviderKind::Azure, "o3", None),
        json!({ "providerOptions": { "azure": {
            "reasoningEffort": "medium",
            "reasoningSummary": "auto",
            "forceReasoning": true,
        } } })
    );
}

#[test]
fn google_gemini_2_5_sends_a_budget_and_gemini_3_plus_a_level() {
    assert_eq!(
        options(
            ProviderKind::Google,
            "gemini-2.5-pro",
            Some(ReasoningEffort::High)
        ),
        json!({ "providerOptions": { "google": { "thinkingConfig": { "includeThoughts": true, "thinkingBudget": 24576 } } } })
    );
    assert_eq!(
        options(
            ProviderKind::Google,
            "gemini-3.5-flash",
            Some(ReasoningEffort::Low)
        ),
        json!({ "providerOptions": { "google": { "thinkingConfig": { "includeThoughts": true, "thinkingLevel": "low" } } } })
    );
}

#[test]
fn google_thinking_maps_follow_the_ts_tables() {
    let level = |effort| {
        options(ProviderKind::Google, "models/GEMINI-3-pro", effort)["providerOptions"]["google"]
            ["thinkingConfig"]["thinkingLevel"]
            .clone()
    };
    assert_eq!(level(Some(ReasoningEffort::None)), "minimal");
    assert_eq!(level(Some(ReasoningEffort::Medium)), "medium");
    assert_eq!(level(None), "medium");
    assert_eq!(level(Some(ReasoningEffort::XHigh)), "high");
    assert_eq!(level(Some(ReasoningEffort::Max)), "high");

    let budget = |effort| {
        options(ProviderKind::Google, "gemini-2.5-flash-lite", effort)["providerOptions"]["google"]
            ["thinkingConfig"]["thinkingBudget"]
            .clone()
    };
    assert_eq!(budget(Some(ReasoningEffort::None)), 0);
    assert_eq!(budget(Some(ReasoningEffort::Low)), 4096);
    assert_eq!(budget(Some(ReasoningEffort::Medium)), 8192);
    assert_eq!(budget(Some(ReasoningEffort::Max)), 32000);
    assert_eq!(budget(None), -1);
    // `gemini-2.5` must end the version: 2.50 is not 2.5.
    assert_eq!(
        options(ProviderKind::Google, "gemini-2.50", None),
        json!({})
    );
}

#[test]
fn google_non_thinking_model_gets_no_reasoning_options() {
    assert_eq!(
        options(ProviderKind::Google, "gemini-2.0-flash", None),
        json!({})
    );
    assert_eq!(
        options(ProviderKind::Google, "gemma-3-27b", None),
        json!({})
    );
}

#[test]
fn anthropic_uses_the_top_level_reasoning_setting_with_max_mapped_to_xhigh() {
    assert_eq!(
        options(ProviderKind::Anthropic, "claude-opus-4-8", None),
        json!({ "reasoning": "medium" })
    );
    assert_eq!(
        options(
            ProviderKind::Anthropic,
            "claude-opus-4-8",
            Some(ReasoningEffort::Max)
        ),
        json!({ "reasoning": "xhigh" })
    );
    assert_eq!(
        options(
            ProviderKind::Anthropic,
            "claude-opus-4-8",
            Some(ReasoningEffort::None)
        ),
        json!({ "reasoning": "none" })
    );
}

#[test]
fn bedrock_enables_thinking_only_for_anthropic_models() {
    assert_eq!(
        options(
            ProviderKind::Bedrock,
            "us.anthropic.claude-sonnet-4-5-20250101-v1:0",
            Some(ReasoningEffort::Medium)
        ),
        json!({ "providerOptions": { "bedrock": { "reasoningConfig": { "type": "enabled", "budgetTokens": 8192 } } } })
    );
    assert_eq!(
        options(ProviderKind::Bedrock, "amazon.nova-pro-v1:0", None),
        json!({})
    );
    // An unset effort is budget -1, floored to 1024.
    assert_eq!(
        options(ProviderKind::Bedrock, "anthropic.claude-3-haiku", None)["providerOptions"]["bedrock"]
            ["reasoningConfig"]["budgetTokens"],
        1024
    );
}

#[test]
fn openrouter_and_compatible_providers_get_no_agent_level_options() {
    for provider in [
        ProviderKind::OpenRouter,
        ProviderKind::OpenAiCompatible,
        ProviderKind::LmStudio,
        ProviderKind::Ollama,
        ProviderKind::Moonshot,
        ProviderKind::GitHubCopilot,
        ProviderKind::QwenCode,
    ] {
        assert_eq!(
            options(provider, "any-model", Some(ReasoningEffort::High)),
            json!({}),
            "{provider:?}"
        );
    }
}

#[test]
fn a_non_reasoning_model_is_skipped_for_every_provider() {
    let off = ReasoningSettings {
        supports_reasoning: false,
        effort: Some(ReasoningEffort::High),
        summary: ReasoningSummary::Detailed,
    };
    for provider in ProviderKind::ALL {
        assert_eq!(
            provider_options(provider, "gpt-5", &off),
            ProviderOptions::default(),
            "{provider:?}"
        );
    }
}

#[test]
fn summary_and_effort_words_round_trip() {
    for effort in [
        ReasoningEffort::None,
        ReasoningEffort::Low,
        ReasoningEffort::Medium,
        ReasoningEffort::High,
        ReasoningEffort::XHigh,
        ReasoningEffort::Max,
    ] {
        assert_eq!(ReasoningEffort::parse(effort.as_str()), Some(effort));
    }
    for summary in [
        ReasoningSummary::Auto,
        ReasoningSummary::Concise,
        ReasoningSummary::Detailed,
    ] {
        assert_eq!(ReasoningSummary::parse(summary.as_str()), Some(summary));
    }
    let detailed = ReasoningSettings {
        supports_reasoning: true,
        effort: None,
        summary: ReasoningSummary::Detailed,
    };
    assert_eq!(
        provider_options(ProviderKind::OpenAi, "gpt-5", &detailed).0["providerOptions"]["openai"]["reasoningSummary"],
        "detailed"
    );
}
