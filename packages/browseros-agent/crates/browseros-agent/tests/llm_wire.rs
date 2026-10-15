//! Wire tests for every provider family: the request `RigModel` sends is compared with the
//! request the TS server sent for the same config and transcript (`fixtures/llm/ts-requests.json`,
//! captured by `fixtures/llm/capture-ts.ts`), and canned provider streams are decoded into
//! `ModelEvent`s, including native-item replay into the next step.
//!
//! Each test starts a local fake provider; `HttpStack::with_test_origin` sends every request
//! there while keeping the provider's own path and query.

mod llm_support;

use browseros_agent::llm::ProviderKind;
use llm_support::{Fake, Reply, collect, request_for, scenario, scenarios};
use serde_json::Value;

/// The request fields that must be identical to TS for each family. Everything else (field order,
/// rig's own defaults such as tool `strict` flags, `stream_options.include_usage`, an implicit
/// `tool_choice: auto`) is free to differ. Known, accepted differences are listed in
/// `accepted_difference`.
fn compared_pointers(provider: ProviderKind) -> &'static [&'static str] {
    match provider {
        ProviderKind::Anthropic => &[
            "/model",
            "/max_tokens",
            "/thinking",
            "/output_config",
            "/stream",
            "/messages",
        ],
        ProviderKind::OpenAi | ProviderKind::Azure => {
            &["/model", "/reasoning", "/store", "/include", "/stream"]
        }
        ProviderKind::ChatGptPro => &[
            "/model",
            "/reasoning",
            "/store",
            "/include",
            "/stream",
            "/instructions",
        ],
        ProviderKind::Google => &[
            "/generationConfig/thinkingConfig",
            "/contents/0",
            "/contents/1",
        ],
        ProviderKind::Bedrock => &[
            "/additionalModelRequestFields/thinking",
            "/inferenceConfig/maxTokens",
            "/messages",
        ],
        _ => &[
            "/model",
            "/reasoning",
            "/stream",
            "/messages/0",
            "/messages/1",
        ],
    }
}

/// Differences from TS that are deliberate (each recorded as a DECIDED in the commit):
/// - OpenAI/Azure Responses: rig asks for `reasoning.encrypted_content` whenever the body
///   reasons, so in-turn reasoning replays statelessly; TS relies on stored item ids.
/// - Gemini: rig marks user parts `thought: false` and sends no `id` on our call/response parts
///   (they correlate by name and order); both are optional fields of the same API.
fn accepted_difference(
    provider: ProviderKind,
    pointer: &str,
    ours: Option<&Value>,
    theirs: Option<&Value>,
) -> bool {
    match (provider, pointer) {
        (ProviderKind::OpenAi | ProviderKind::Azure, "/include") => {
            theirs.is_none() && ours == Some(&serde_json::json!(["reasoning.encrypted_content"]))
        }
        (ProviderKind::Google, pointer) if pointer.starts_with("/contents") => {
            ours.map(normalize_gemini) == theirs.map(normalize_gemini)
        }
        _ => false,
    }
}

fn normalize_gemini(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, value)| {
                    !(key.as_str() == "thought" && **value == Value::Bool(false))
                })
                .filter(|(key, _)| key.as_str() != "id")
                .map(|(key, value)| (key.clone(), normalize_gemini(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(normalize_gemini).collect()),
        other => other.clone(),
    }
}

/// Headers that must match TS by value (when TS sent them) or by presence (secrets).
const HEADERS_BY_VALUE: &[&str] = &[
    "x-trace",
    "anthropic-version",
    "openai-intent",
    "x-initiator",
    "openai-beta",
    "originator",
    "chatgpt-account-id",
];
const HEADERS_BY_PRESENCE: &[&str] = &["authorization", "x-api-key", "api-key", "x-goog-api-key"];

#[tokio::test]
async fn every_ts_scenario_sends_the_ts_request() -> anyhow::Result<()> {
    let mut mismatches = Vec::new();
    let mut dump = serde_json::Map::new();
    for (name, entry) in scenarios()? {
        let fake = Fake::start(vec![Reply::error(
            400,
            r#"{"error":{"message":"captured"}}"#,
        )])
        .await?;
        let scenario = scenario(&entry)?;
        let model = browseros_agent::llm::create_language_model(&scenario.config, &fake.stack()?)?;
        let request = request_for(&name, &scenario);
        let _ = collect(model.stream(request, Default::default()).await).await;
        let Some(sent) = fake.requests().into_iter().next() else {
            mismatches.push(format!("{name}: no request reached the fake provider"));
            continue;
        };
        let ts = &entry["request"];
        dump.insert(name.clone(), sent.to_json());

        let ts_url = ts["url"].as_str().unwrap_or_default();
        let ts_path = ts_url
            .splitn(4, '/')
            .nth(3)
            .map(|rest| format!("/{rest}"))
            .unwrap_or_default();
        if sent.path_and_query != ts_path {
            mismatches.push(format!(
                "{name}: url {} != TS {ts_path}",
                sent.path_and_query
            ));
        }
        for header in HEADERS_BY_VALUE {
            let ours = sent.headers.get(*header);
            let theirs = ts["headers"].get(*header).and_then(Value::as_str);
            if ours.map(String::as_str) != theirs {
                mismatches.push(format!("{name}: header {header} {ours:?} != TS {theirs:?}"));
            }
        }
        for header in HEADERS_BY_PRESENCE {
            let ours = sent.headers.contains_key(*header);
            let theirs = ts["headers"].get(*header).is_some();
            if ours != theirs {
                mismatches.push(format!(
                    "{name}: header {header} present={ours} != TS {theirs}"
                ));
            }
        }
        let provider = scenario.config.provider;
        if provider == ProviderKind::ChatGptPro {
            // TS: `{role:"developer", content: SYSTEM}` first; rig spells content as parts.
            let first = &sent.body["input"][0];
            let text = first["content"][0]["text"]
                .as_str()
                .or(first["content"].as_str());
            if first["role"] != "developer" || text != Some(llm_support::SYSTEM) {
                mismatches.push(format!(
                    "{name}: /input/0 is not the developer system prompt: {first}"
                ));
            }
        }
        for pointer in compared_pointers(provider) {
            let ours = sent.body.pointer(pointer);
            let theirs = ts["body"].pointer(pointer);
            if ours != theirs && !accepted_difference(provider, pointer, ours, theirs) {
                mismatches.push(format!(
                    "{name}: {pointer}\n    ours: {}\n    TS:   {}",
                    ours.map(Value::to_string).unwrap_or_default(),
                    theirs.map(Value::to_string).unwrap_or_default()
                ));
            }
        }
    }
    if let Ok(path) = std::env::var("BROWSEROS_DUMP_WIRE") {
        std::fs::write(path, serde_json::to_string_pretty(&Value::Object(dump))?)?;
    }
    assert!(
        mismatches.is_empty(),
        "{} mismatches:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
    Ok(())
}
