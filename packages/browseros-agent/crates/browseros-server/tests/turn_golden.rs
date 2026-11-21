//! The golden fixtures for the UI message stream (CORRECTIONS-1 B2).
//!
//! A golden is three things for one case: the scripted `TurnEvent` list below, the `.sse` the
//! Rust reducer emits for it (`tests/fixtures/ui-stream/<case>.sse`), and the `UIMessage` the
//! extension's installed AI SDK builds from that `.sse` (`<case>.message.json`). This test holds
//! the first two together byte for byte, and `sealed()` to the third, JSON-equal. The third is
//! written and checked by the Bun half, `contracts/server-api/tests/ui-stream.test.ts`, which
//! runs the SDK the extension ships over the same bytes:
//!
//! ```text
//!   events ──Transcript──▶ <case>.sse ──ai (apps/app's install)──▶ <case>.message.json
//!                │                                                        ▲
//!                └────────────── sealed() ── JSON-equal ──────────────────┘
//! ```
//!
//! After a deliberate change to the stream, regenerate in this order:
//! `BROWSEROS_UPDATE_GOLDENS=1 cargo test -p browseros-server --test turn_golden`, then
//! `BROWSEROS_UPDATE_GOLDENS=1 bun test contracts/server-api/tests/ui-stream.test.ts`, then review the diff.

use std::path::{Path, PathBuf};

use browseros_server::browser::ToolOutput;
use browseros_server::turn::{
    ChatError, ChatErrorCode, FinishReason, MessageId, ToolCallId, ToolOwner, Transcript, TurnEnd,
    TurnEvent,
};
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The assistant message id every golden uses.
const MESSAGE_ID: &str = "msg-golden";

struct Case {
    name: &'static str,
    events: Vec<TurnEvent>,
    end: TurnEnd,
}

fn text(delta: &str) -> TurnEvent {
    TurnEvent::Text(delta.into())
}

fn reasoning(delta: &str) -> TurnEvent {
    TurnEvent::Reasoning(delta.into())
}

fn started(id: &str, name: &str, owner: ToolOwner) -> TurnEvent {
    TurnEvent::ToolStarted {
        id: ToolCallId(id.into()),
        name: name.into(),
        owner,
    }
}

fn input(id: &str, name: &str, owner: ToolOwner, input: Value) -> TurnEvent {
    TurnEvent::ToolInput {
        id: ToolCallId(id.into()),
        name: name.into(),
        input,
        owner,
    }
}

fn done(id: &str, result: Result<Value, &str>) -> TurnEvent {
    TurnEvent::ToolDone {
        id: ToolCallId(id.into()),
        result: result.map_err(str::to_owned),
    }
}

fn browser_done(id: &str, text: &str) -> TurnEvent {
    let output = ToolOutput {
        text: text.into(),
        pictures: Vec::new(),
        is_error: false,
    };
    TurnEvent::ToolDone {
        id: ToolCallId(id.into()),
        result: output.for_panel(),
    }
}

fn provider_error() -> ChatError {
    let mut error = ChatError::new(
        ChatErrorCode::RateLimited,
        "Rate limit reached",
        "The provider is limiting requests. Try again in a minute.",
    );
    error.provider = Some("openai".into());
    error.status_code = Some(429);
    error.retry_after_seconds = Some(60);
    error
}

fn cases() -> Vec<Case> {
    use TurnEvent::{StepEnd, StepStart};
    let server = ToolOwner::Server;
    vec![
        Case {
            name: "text-only",
            events: vec![StepStart, text("Paris is "), text("in France."), StepEnd],
            end: TurnEnd::Done(FinishReason::Stop),
        },
        Case {
            name: "reasoning-then-text",
            events: vec![
                StepStart,
                reasoning("The user asks about Paris. "),
                reasoning("Answer directly."),
                text("Paris is in France."),
                StepEnd,
            ],
            end: TurnEnd::Done(FinishReason::Stop),
        },
        Case {
            name: "text-tool-text",
            events: vec![
                StepStart,
                text("Checking the page."),
                started("call_1", "run", server),
                input(
                    "call_1",
                    "run",
                    server,
                    json!({ "code": "return document.title" }),
                ),
                browser_done("call_1", "Example Domain"),
                StepEnd,
                StepStart,
                text("The page is titled "),
                text("Example Domain."),
                StepEnd,
            ],
            end: TurnEnd::Done(FinishReason::Stop),
        },
        Case {
            name: "tool-error-then-text",
            events: vec![
                StepStart,
                started("call_1", "run", server),
                input("call_1", "run", server, json!({ "code": "throw 1" })),
                done("call_1", Err("Script failed: 1")),
                StepEnd,
                StepStart,
                text("The script failed."),
                StepEnd,
            ],
            end: TurnEnd::Done(FinishReason::Stop),
        },
        Case {
            name: "two-calls-stop-after-first",
            events: vec![
                StepStart,
                reasoning("Two clicks."),
                started("call_1", "run", server),
                started("call_2", "run", server),
                input("call_1", "run", server, json!({ "code": "click('a')" })),
                browser_done("call_1", r#"{"clicked":true}"#),
            ],
            end: TurnEnd::Stopped,
        },
        Case {
            name: "stop-mid-call",
            events: vec![
                StepStart,
                started("call_1", "run", server),
                input("call_1", "run", server, json!({ "code": "click('a')" })),
            ],
            end: TurnEnd::Stopped,
        },
        Case {
            name: "coding-agent-tool",
            events: vec![
                StepStart,
                text("Reading the file."),
                started("toolu_01", "Read", ToolOwner::Agent),
                input(
                    "toolu_01",
                    "Read",
                    ToolOwner::Agent,
                    json!({ "file_path": "README.md" }),
                ),
                done("toolu_01", Ok(json!("# BrowserOS"))),
                text("It is the readme."),
                StepEnd,
            ],
            end: TurnEnd::Done(FinishReason::Stop),
        },
        Case {
            name: "failure-before-content",
            events: vec![StepStart],
            end: TurnEnd::Failed(ChatError::new(
                ChatErrorCode::ProviderConfig,
                "No provider selected",
                "Choose a provider in settings, then send again.",
            )),
        },
        Case {
            name: "failure-after-text",
            events: vec![StepStart, text("Let me check")],
            end: TurnEnd::Failed(provider_error()),
        },
        Case {
            name: "stop-mid-text",
            events: vec![StepStart, text("Partial ans")],
            end: TurnEnd::Stopped,
        },
        Case {
            name: "content-filter",
            events: vec![StepStart, StepEnd],
            end: TurnEnd::Done(FinishReason::ContentFilter),
        },
    ]
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ui-stream")
}

/// The response body `POST /chat` writes for these frames: axum's SSE framing, one
/// `data: <json>` event per frame, then `[DONE]`.
fn sse(transcript: &Transcript) -> String {
    let mut body = String::new();
    for frame in transcript.frames(0) {
        body.push_str("data: ");
        body.push_str(frame.data());
        body.push_str("\n\n");
    }
    body.push_str("data: [DONE]\n\n");
    body
}

fn run(case: &Case) -> Transcript {
    let mut transcript = Transcript::new(MessageId(MESSAGE_ID.into()));
    for event in &case.events {
        transcript.apply(event.clone());
    }
    transcript.finish(&case.end);
    transcript
}

#[test]
fn every_golden_stream_is_what_the_reducer_emits() -> TestResult {
    let update = std::env::var_os("BROWSEROS_UPDATE_GOLDENS").is_some();
    for case in cases() {
        let path = fixtures().join(format!("{}.sse", case.name));
        let emitted = sse(&run(&case));
        if update {
            std::fs::write(&path, &emitted)?;
            continue;
        }
        let committed = std::fs::read_to_string(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        assert_eq!(emitted, committed, "{}: the stream changed", case.name);
    }
    Ok(())
}

#[test]
fn the_stored_message_is_the_message_the_sdk_builds() -> TestResult {
    for case in cases() {
        let path = fixtures().join(format!("{}.message.json", case.name));
        let sdk: Value = serde_json::from_str(
            &std::fs::read_to_string(&path)
                .map_err(|error| format!("{}: {error}", path.display()))?,
        )?;
        let sealed = run(&case).sealed();
        match sealed {
            Some(message) => assert_eq!(
                serde_json::to_value(&message)?,
                sdk,
                "{}: sealed() differs from the SDK's message",
                case.name
            ),
            None => assert_eq!(
                sdk["parts"],
                json!([{ "type": "step-start" }]),
                "{}: nothing is stored only when the SDK's message has no display content",
                case.name
            ),
        }
    }
    Ok(())
}

#[test]
fn a_failure_before_any_content_stores_nothing() {
    let case = cases()
        .into_iter()
        .find(|case| case.name == "failure-before-content");
    assert!(case.is_some_and(|case| run(&case).sealed().is_none()));
}

#[test]
fn the_cases_cover_every_terminal_chunk_and_both_tool_owners() -> TestResult {
    let mut types = std::collections::BTreeSet::new();
    for case in cases() {
        for frame in run(&case).frames(0) {
            let chunk: Value = serde_json::from_str(frame.data())?;
            if let Some(kind) = chunk["type"].as_str() {
                types.insert(kind.to_owned());
            }
            if chunk.get("dynamic") == Some(&json!(true)) {
                types.insert("dynamic tool".into());
            }
        }
    }
    for expected in [
        "start",
        "start-step",
        "finish-step",
        "text-start",
        "text-delta",
        "text-end",
        "reasoning-start",
        "reasoning-delta",
        "reasoning-end",
        "tool-input-start",
        "tool-input-available",
        "tool-output-available",
        "tool-output-error",
        "finish",
        "error",
        "abort",
        "dynamic tool",
    ] {
        assert!(types.contains(expected), "no golden emits {expected}");
    }
    Ok(())
}
