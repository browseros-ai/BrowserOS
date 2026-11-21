//! The scripted model, behind `BROWSEROS_USE_MOCK_LLM=true`: canned steps, so tests and the
//! end-to-end proof run with no key and no network.
//!
//! A script is a list of steps; each `Model::step` call of a turn plays the next one, and once
//! the list is spent every further step answers with the old server's canned text. A step can
//! think, answer, ask for tool calls (the browser's `run`, so the proof covers a real tool call;
//! the loop runs them), report token usage, fail with a `ChatError`, or wait first (so Stop can
//! be tested). Like every model, it emits `ToolStarted` for a call and never `ToolInput`. With `BROWSEROS_MOCK_LLM_SCRIPT`
//! pointing at a JSON file, `app` loads the script from it:
//!
//! ```json
//! { "steps": [
//!     { "toolCalls": [{ "name": "run", "input": { "code": "return document.title" } }] },
//!     { "delayMs": 200, "text": "Example Domain" }
//! ] }
//! ```

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde::Deserialize;

use super::{AssistantTurn, ToolCall, Usage};
use crate::turn::{
    ChatError, ChatErrorCode, EventSink, FinishReason, ToolCallId, ToolOwner, TurnEvent,
};

/// The old server's canned answer, kept so existing harness expectations still hold.
pub const DEFAULT_ANSWER: &str = "Mock BrowserOS test response.";

/// What the scripted model answers, one entry per `step` call of a turn.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Script {
    #[serde(default)]
    pub steps: Vec<ScriptedStep>,
}

/// One model call's canned response, played in this order: wait, fail or reason, answer, call.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScriptedStep {
    /// Waits before any output. A stop during the wait ends the step: the loop drops the future.
    #[serde(default)]
    pub delay_ms: u64,
    pub reasoning: Option<String>,
    pub text: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ScriptedCall>,
    /// Fails the step after the wait, before any output.
    pub error: Option<ScriptedError>,
    /// What `AssistantTurn::usage` reports for this step.
    pub usage: Option<ScriptedUsage>,
}

/// Token counts the step reports, as a provider would.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScriptedUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// A tool call the step makes. Without an `id`, one is made from the step and call index.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScriptedCall {
    pub name: String,
    #[serde(default)]
    pub input: serde_json::Value,
    pub id: Option<String>,
}

/// A step failure, by the envelope's code (`rate_limited`, `auth_failed`, …).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScriptedError {
    pub code: String,
    pub message: String,
}

impl Script {
    /// One step that answers with the canned text: the script when no file is named.
    pub fn default_answer() -> Self {
        Self {
            steps: vec![ScriptedStep::answer(DEFAULT_ANSWER)],
        }
    }

    /// Reads a script file (`BROWSEROS_MOCK_LLM_SCRIPT`).
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("reading {}: {error}", path.display()))?;
        serde_json::from_str(&text).map_err(|error| format!("parsing {}: {error}", path.display()))
    }
}

impl ScriptedStep {
    /// A step that only answers with `text`.
    pub fn answer(text: impl Into<String>) -> Self {
        Self {
            text: Some(text.into()),
            ..Self::default()
        }
    }

    /// A step that calls one tool and says nothing.
    pub fn call(name: impl Into<String>, input: serde_json::Value) -> Self {
        Self {
            tool_calls: vec![ScriptedCall {
                name: name.into(),
                input,
                id: None,
            }],
            ..Self::default()
        }
    }
}

/// Plays the next step of `script` into `sink`. `next` is the model's own counter, so one
/// `Model` (one turn) walks the script once from the top.
pub(super) async fn step(
    script: &Script,
    next: &AtomicUsize,
    sink: &mut dyn EventSink,
) -> Result<AssistantTurn, ChatError> {
    let index = next.fetch_add(1, Ordering::Relaxed);
    let step = script
        .steps
        .get(index)
        .cloned()
        .unwrap_or_else(|| ScriptedStep::answer(DEFAULT_ANSWER));

    if step.delay_ms > 0 {
        tokio::time::sleep(Duration::from_millis(step.delay_ms)).await;
    }
    if let Some(error) = step.error {
        let code = ChatErrorCode::parse(&error.code).unwrap_or(ChatErrorCode::Unknown);
        return Err(ChatError::new(code, "Scripted failure", error.message));
    }
    if let Some(reasoning) = step.reasoning {
        sink.emit(TurnEvent::Reasoning(reasoning)).await;
    }
    let text = step.text.unwrap_or_default();
    if !text.is_empty() {
        sink.emit(TurnEvent::Text(text.clone())).await;
    }

    let calls: Vec<ToolCall> = step
        .tool_calls
        .into_iter()
        .enumerate()
        .map(|(position, call)| ToolCall {
            id: ToolCallId(
                call.id
                    .unwrap_or_else(|| format!("call_scripted_{index}_{position}")),
            ),
            name: call.name,
            input: call.input,
        })
        .collect();
    for call in &calls {
        sink.emit(TurnEvent::ToolStarted {
            id: call.id.clone(),
            name: call.name.clone(),
            owner: ToolOwner::Server,
        })
        .await;
    }
    let finish = if calls.is_empty() {
        FinishReason::Stop
    } else {
        FinishReason::ToolCalls
    };
    let usage = step.usage.map(|usage| Usage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    });
    Ok(AssistantTurn::plain(text, calls, finish, usage))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Model, StepRequest};
    use serde_json::json;

    fn request() -> StepRequest<'static> {
        StepRequest {
            system: "",
            messages: &[],
            tools: &[],
            effort: None,
        }
    }

    #[tokio::test]
    async fn a_turn_plays_the_script_then_the_canned_answer() -> Result<(), ChatError> {
        let script: Script = serde_json::from_value(json!({
            "steps": [
                { "text": "Opening", "toolCalls": [{ "name": "run", "input": { "code": "1" } }] },
                { "text": "Done" }
            ]
        }))
        .map_err(|error| ChatError::new(ChatErrorCode::Unknown, "script", error.to_string()))?;
        let model = Model::scripted(script, 1000, true);
        let mut events = Vec::new();

        let first = model.step(&request(), &mut events).await?;
        assert_eq!(first.finish(), FinishReason::ToolCalls);
        assert_eq!(
            first.tool_calls()[0].id,
            ToolCallId("call_scripted_0_0".into())
        );
        assert_eq!(first.tool_calls()[0].input, json!({ "code": "1" }));
        let second = model.step(&request(), &mut events).await?;
        assert_eq!(second.finish(), FinishReason::Stop);
        model.step(&request(), &mut events).await?;

        let call = ToolCallId("call_scripted_0_0".into());
        assert_eq!(
            events,
            vec![
                TurnEvent::Text("Opening".into()),
                TurnEvent::ToolStarted {
                    id: call,
                    name: "run".into(),
                    owner: ToolOwner::Server
                },
                TurnEvent::Text("Done".into()),
                TurnEvent::Text(DEFAULT_ANSWER.into()),
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_scripted_error_fails_the_step_with_its_code() {
        let script = Script {
            steps: vec![ScriptedStep {
                error: Some(ScriptedError {
                    code: "rate_limited".into(),
                    message: "slow down".into(),
                }),
                ..ScriptedStep::default()
            }],
        };
        let result = Model::scripted(script, 1000, true)
            .step(&request(), &mut Vec::new())
            .await;
        assert!(matches!(result, Err(error) if error.code == ChatErrorCode::RateLimited));
    }
}
