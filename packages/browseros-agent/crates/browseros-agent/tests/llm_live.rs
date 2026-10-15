//! Live smoke tests against real providers, ignored by default:
//!
//! ```text
//! cargo test -p browseros-agent --test llm_live -- --ignored --nocapture
//! ```
//!
//! - ChatGPT subscription (`chatgpt-pro`): the Codex OAuth token and account id from
//!   `~/.codex/auth.json` (written by `codex login`; never printed). Models from
//!   `BROWSEROS_LIVE_CHATGPT_MODELS` (comma-separated), default `gpt-6-astra,gpt-6.1-sol`.
//! - Ollama (`ollama`, OpenAI-compatible chat at `http://127.0.0.1:11434/v1`): the model in
//!   `BROWSEROS_LIVE_OLLAMA_MODEL`, default `gemma4:cloud`.
//!
//! Each runs a two-step tool turn the way the loop does (step, tool result, step) with reasoning
//! on, so native reasoning replay is exercised against the real backend.

// The printed summary (never a credential) is this test's report.
#![allow(clippy::print_stdout)]

mod llm_support;

use browseros_agent::llm::{
    HttpStack, LanguageModel, LlmConfig, ModelEvent, ModelRequest, ProviderKind, ReasoningEffort,
    ReasoningSettings, ReasoningSummary, create_language_model, generate_text, provider_options,
};
use llm_support::{assistant_from, collect, events, get_time_tool, tool_text_result, user};
use std::time::Instant;
use tokio_util::sync::CancellationToken;

fn summary(events: &[ModelEvent]) -> String {
    let mut text = String::new();
    let mut reasoning = 0;
    let mut calls = Vec::new();
    let mut finish = String::new();
    for event in events {
        match event {
            ModelEvent::TextDelta { delta, .. } => text.push_str(delta),
            ModelEvent::ReasoningDelta { .. } => reasoning += 1,
            ModelEvent::ToolCall {
                tool_name,
                input_json,
                ..
            } => calls.push(format!("{tool_name}({input_json})")),
            ModelEvent::Finish { reason, usage } => finish = format!("{reason:?} {usage:?}"),
            _ => {}
        }
    }
    format!(
        "reasoning_deltas={reasoning} calls={calls:?} finish={finish} text={:?}",
        text.trim()
    )
}

async fn tool_turn(
    model: &dyn LanguageModel,
    settings: &ReasoningSettings,
) -> anyhow::Result<String> {
    let options = provider_options(model.provider(), model.model_id(), settings);
    let system =
        Some("You are BrowserOS, a browser agent. Use tools when asked. Be brief.".to_owned());
    let prompt = "Think about it briefly, then call get_time. After you have the time, answer in one sentence.";
    let started = Instant::now();
    let first = ModelRequest {
        system: system.clone(),
        messages: vec![user(prompt)],
        tools: vec![get_time_tool()],
        options: options.clone(),
    };
    let step1 = events(collect(model.stream(first, CancellationToken::new()).await).await)?;
    let call_id = step1
        .iter()
        .find_map(|event| match event {
            ModelEvent::ToolCall {
                provider_call_id, ..
            } => Some(provider_call_id.clone()),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("step 1 made no tool call: {}", summary(&step1)))?;
    let second = ModelRequest {
        system,
        messages: vec![
            user(prompt),
            assistant_from(&step1),
            tool_text_result(&call_id, r#"{"utc":"2026-10-15T04:47:54Z"}"#),
        ],
        tools: vec![get_time_tool()],
        options,
    };
    let step2 = events(collect(model.stream(second, CancellationToken::new()).await).await)?;
    Ok(format!(
        "step1: {}\nstep2: {}\nelapsed={:.1}s",
        summary(&step1),
        summary(&step2),
        started.elapsed().as_secs_f32()
    ))
}

fn codex_auth() -> anyhow::Result<(String, Option<String>)> {
    let home = std::env::var("HOME")?;
    let auth: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(format!(
        "{home}/.codex/auth.json"
    ))?)?;
    let token = auth
        .pointer("/tokens/access_token")
        .and_then(serde_json::Value::as_str)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| anyhow::anyhow!("~/.codex/auth.json has no tokens.access_token"))?;
    let account = auth
        .pointer("/tokens/account_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    Ok((token.to_owned(), account))
}

fn base(provider: ProviderKind, model: &str) -> LlmConfig {
    LlmConfig {
        provider,
        model: model.to_owned(),
        api_key: None,
        base_url: None,
        headers: None,
        resource_name: None,
        region: None,
        access_key_id: None,
        secret_access_key: None,
        session_token: None,
        account_id: None,
        upstream_provider: None,
        conversation_id: "live-smoke".to_owned(),
    }
}

#[tokio::test]
#[ignore = "live: needs `codex login` (~/.codex/auth.json)"]
async fn chatgpt_subscription_tool_turn() -> anyhow::Result<()> {
    let (token, account) = codex_auth()?;
    let models = std::env::var("BROWSEROS_LIVE_CHATGPT_MODELS")
        .unwrap_or_else(|_| "gpt-6-astra,gpt-6.1-sol".to_owned());
    for model_id in models
        .split(',')
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        let config = LlmConfig {
            api_key: Some(token.clone()),
            account_id: account.clone(),
            ..base(ProviderKind::ChatGptPro, model_id)
        };
        let model = create_language_model(&config, &HttpStack::new())?;
        let settings = ReasoningSettings {
            supports_reasoning: true,
            effort: Some(ReasoningEffort::Low),
            summary: ReasoningSummary::Auto,
        };
        println!(
            "=== chatgpt-pro {model_id}\n{}",
            tool_turn(model.as_ref(), &settings).await?
        );
        let text = generate_text(
            model.as_ref(),
            None,
            "Respond with exactly: 'ok'",
            CancellationToken::new(),
        )
        .await?;
        println!("generate_text: {text:?}");
    }
    Ok(())
}

#[tokio::test]
#[ignore = "live: needs a local Ollama with the model pulled"]
async fn ollama_tool_turn() -> anyhow::Result<()> {
    let model_id =
        std::env::var("BROWSEROS_LIVE_OLLAMA_MODEL").unwrap_or_else(|_| "gemma4:cloud".to_owned());
    let config = LlmConfig {
        base_url: Some("http://127.0.0.1:11434/v1".to_owned()),
        ..base(ProviderKind::Ollama, &model_id)
    };
    let model = create_language_model(&config, &HttpStack::new())?;
    let settings = ReasoningSettings {
        supports_reasoning: true,
        effort: None,
        summary: ReasoningSummary::Auto,
    };
    println!(
        "=== ollama {model_id}\n{}",
        tool_turn(model.as_ref(), &settings).await?
    );
    Ok(())
}
