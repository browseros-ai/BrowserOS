//! `UiTranslator` against the spike §4.3 table, with updates built from wire JSON exactly as agents
//! send them and chunks compared as wire JSON exactly as the extension receives them.

use acp_agents::{TurnEnd, UiChunk, UiTranslator};
use acp_client::schema::{SessionUpdate, StopReason};
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn feed(
    translator: &mut UiTranslator,
    updates: &[Value],
) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let mut out = Vec::new();
    for update in updates {
        let update: SessionUpdate = serde_json::from_value(update.clone())?;
        out.extend(wire(translator.on_update(&update))?);
    }
    Ok(out)
}

fn wire(chunks: Vec<UiChunk>) -> Result<Vec<Value>, serde_json::Error> {
    chunks.into_iter().map(serde_json::to_value).collect()
}

fn text(text: &str, message_id: Option<&str>) -> Value {
    let mut update =
        json!({"sessionUpdate":"agent_message_chunk", "content":{"type":"text", "text":text}});
    if let Some(id) = message_id {
        update["messageId"] = json!(id);
    }
    update
}

#[test]
fn text_blocks_split_on_kind_and_new_message_ids_only() -> TestResult {
    let mut translator = UiTranslator::new();
    let chunks = feed(
        &mut translator,
        &[
            text("[context] notice", Some("m0")),
            text("p", Some("m1")),
            text("ong", None),
            text("!", Some("m1")),
            json!({"sessionUpdate":"agent_thought_chunk", "content":{"type":"text", "text":"hmm"}}),
            text("", Some("m9")),
            json!({"sessionUpdate":"agent_message_chunk", "content":{"type":"image", "mimeType":"image/png", "data":"AA=="}}),
            text("done", None),
        ],
    )?;
    assert_eq!(
        chunks,
        [
            json!({"type":"start-step"}),
            json!({"type":"text-start", "id":"acpx-1"}),
            json!({"type":"text-delta", "id":"acpx-1", "delta":"[context] notice"}),
            json!({"type":"text-end", "id":"acpx-1"}),
            json!({"type":"text-start", "id":"acpx-2"}),
            json!({"type":"text-delta", "id":"acpx-2", "delta":"p"}),
            json!({"type":"text-delta", "id":"acpx-2", "delta":"ong"}),
            json!({"type":"text-delta", "id":"acpx-2", "delta":"!"}),
            json!({"type":"text-end", "id":"acpx-2"}),
            json!({"type":"reasoning-start", "id":"acpx-3"}),
            json!({"type":"reasoning-delta", "id":"acpx-3", "delta":"hmm"}),
            json!({"type":"reasoning-end", "id":"acpx-3"}),
            json!({"type":"text-start", "id":"acpx-4"}),
            json!({"type":"text-delta", "id":"acpx-4", "delta":"done"}),
        ]
    );
    assert_eq!(
        wire(translator.finish(TurnEnd::Completed(StopReason::EndTurn)))?,
        [
            json!({"type":"text-end", "id":"acpx-4"}),
            json!({"type":"finish-step"}),
            json!({"type":"finish", "finishReason":"stop"}),
        ]
    );
    Ok(())
}

#[test]
fn claude_style_tool_call_ends_available_without_the_spurious_input_error() -> TestResult {
    let mut translator = UiTranslator::new();
    let chunks = feed(
        &mut translator,
        &[
            text("Running", Some("m1")),
            json!({"sessionUpdate":"tool_call", "toolCallId":"toolu_1", "title":"Terminal", "kind":"execute", "status":"pending", "rawInput":{}}),
            json!({"sessionUpdate":"tool_call_update", "toolCallId":"toolu_1", "title":"touch probe.txt", "status":"in_progress",
                   "rawInput":{"command":"touch probe.txt"}}),
            json!({"sessionUpdate":"tool_call_update", "toolCallId":"toolu_1", "title":"touch probe.txt", "status":"in_progress",
                   "rawInput":{"command":"touch probe.txt"}}),
            json!({"sessionUpdate":"tool_call_update", "toolCallId":"toolu_1", "status":"completed",
                   "rawOutput":"(Bash completed with no output)"}),
        ],
    )?;
    assert_eq!(
        chunks,
        [
            json!({"type":"start-step"}),
            json!({"type":"text-start", "id":"acpx-1"}),
            json!({"type":"text-delta", "id":"acpx-1", "delta":"Running"}),
            json!({"type":"text-end", "id":"acpx-1"}),
            json!({"type":"tool-input-start", "toolCallId":"toolu_1", "toolName":"Terminal", "providerExecuted":true}),
            json!({"type":"tool-input-delta", "toolCallId":"toolu_1", "inputTextDelta":"Terminal (pending)"}),
            json!({"type":"tool-input-delta", "toolCallId":"toolu_1", "inputTextDelta":"touch probe.txt (in_progress): touch probe.txt"}),
            json!({"type":"tool-input-available", "toolCallId":"toolu_1", "toolName":"Terminal",
                   "input":{"command":"touch probe.txt"}, "providerExecuted":true}),
            json!({"type":"tool-output-available", "toolCallId":"toolu_1",
                   "output":"(Bash completed with no output)", "providerExecuted":true}),
        ]
    );
    Ok(())
}

#[test]
fn tool_output_prefers_content_text_and_failures_become_output_errors() -> TestResult {
    let mut translator = UiTranslator::new();
    let long = "x".repeat(600);
    let chunks = feed(
        &mut translator,
        &[
            json!({"sessionUpdate":"tool_call", "toolCallId":"a", "title":"Read", "status":"completed",
                   "content":[{"type":"content", "content":{"type":"text", "text":long}},
                              {"type":"diff", "path":"/f.txt", "newText":"n"}]}),
            json!({"sessionUpdate":"tool_call", "toolCallId":"b", "title":"  ", "status":"pending"}),
            json!({"sessionUpdate":"tool_call_update", "toolCallId":"b", "status":"failed",
                   "_meta":{"claudeCode":{"nonExecutionKind":"permission-rule"}},
                   "rawOutput":{"error":"User refused permission"}}),
            json!({"sessionUpdate":"tool_call", "toolCallId":"  ", "title":"ignored"}),
        ],
    )?;
    let truncated = format!("{}…", "x".repeat(499));
    assert_eq!(
        chunks,
        [
            json!({"type":"start-step"}),
            json!({"type":"tool-input-start", "toolCallId":"a", "toolName":"Read", "providerExecuted":true}),
            json!({"type":"tool-input-available", "toolCallId":"a", "toolName":"Read",
                   "input":format!("Read (completed): {truncated}"), "providerExecuted":true}),
            json!({"type":"tool-output-available", "toolCallId":"a", "output":truncated, "providerExecuted":true}),
            json!({"type":"tool-input-start", "toolCallId":"b", "toolName":"tool call", "providerExecuted":true}),
            json!({"type":"tool-input-delta", "toolCallId":"b", "inputTextDelta":"tool call (pending)"}),
            json!({"type":"tool-input-available", "toolCallId":"b", "toolName":"tool call",
                   "input":"tool call (pending)", "providerExecuted":true}),
            json!({"type":"tool-output-error", "toolCallId":"b", "errorText":"User refused permission", "providerExecuted":true}),
        ]
    );
    Ok(())
}

#[test]
fn plans_are_self_contained_reasoning_and_bookkeeping_updates_are_silent() -> TestResult {
    let mut translator = UiTranslator::new();
    let chunks = feed(
        &mut translator,
        &[
            json!({"sessionUpdate":"usage_update", "used":11220, "size":272000, "cost":{"amount":0.0, "currency":"USD"}}),
            json!({"sessionUpdate":"available_commands_update", "availableCommands":[]}),
            json!({"sessionUpdate":"current_mode_update", "currentModeId":"code"}),
            json!({"sessionUpdate":"session_info_update", "title":"Hi"}),
            json!({"sessionUpdate":"user_message_chunk", "content":{"type":"text", "text":"echo"}}),
            json!({"sessionUpdate":"plan", "entries":[]}),
        ],
    )?;
    assert!(
        chunks.is_empty(),
        "no chunk and no step for bookkeeping: {chunks:?}"
    );
    assert!(!translator.has_output());
    let chunks = feed(
        &mut translator,
        &[
            text("a", None),
            json!({"sessionUpdate":"plan", "entries":[
                {"content":" 1. do thing ", "priority":"high", "status":"pending"},
                {"content":"2. later", "priority":"low", "status":"pending"}]}),
        ],
    )?;
    assert_eq!(
        chunks,
        [
            json!({"type":"start-step"}),
            json!({"type":"text-start", "id":"acpx-1"}),
            json!({"type":"text-delta", "id":"acpx-1", "delta":"a"}),
            json!({"type":"reasoning-start", "id":"acpx-2"}),
            json!({"type":"reasoning-delta", "id":"acpx-2", "delta":"[Plan] plan: 1. do thing"}),
            json!({"type":"reasoning-end", "id":"acpx-2"}),
        ]
    );
    Ok(())
}

#[test]
fn stop_reasons_map_to_finish_reasons() -> TestResult {
    for (reason, finish) in [
        (StopReason::EndTurn, "stop"),
        (StopReason::MaxTokens, "length"),
        (StopReason::MaxTurnRequests, "length"),
        (StopReason::Refusal, "content-filter"),
        (StopReason::Cancelled, "other"),
    ] {
        let mut translator = UiTranslator::new();
        assert_eq!(
            wire(translator.finish(TurnEnd::Completed(reason)))?,
            [
                json!({"type":"start-step"}),
                json!({"type":"finish-step"}),
                json!({"type":"finish", "finishReason":finish}),
            ],
            "{reason:?}"
        );
    }
    Ok(())
}

#[test]
fn host_cancel_aborts_and_failures_close_blocks_before_the_error() -> TestResult {
    let mut translator = UiTranslator::new();
    feed(&mut translator, &[text("partial", None)])?;
    assert_eq!(
        wire(translator.finish(TurnEnd::CancelledByHost {
            reason: "AbortError: Conversation stopped".into()
        }))?,
        [json!({"type":"abort", "reason":"AbortError: Conversation stopped"})]
    );

    let mut translator = UiTranslator::new();
    feed(&mut translator, &[text("partial", None)])?;
    assert_eq!(
        wire(translator.finish(TurnEnd::Failed {
            message: "quota exceeded".into()
        }))?,
        [
            json!({"type":"text-end", "id":"acpx-1"}),
            json!({"type":"error", "errorText":"quota exceeded"}),
            json!({"type":"finish-step"}),
            json!({"type":"finish", "finishReason":"error"}),
        ]
    );

    let mut translator = UiTranslator::new();
    assert_eq!(
        wire(translator.finish(TurnEnd::CancelledByHost { reason: "r".into() }))?,
        [
            json!({"type":"start-step"}),
            json!({"type":"abort", "reason":"r"})
        ]
    );
    Ok(())
}

#[test]
fn a_silent_codex_system_error_becomes_an_error() -> TestResult {
    let error_status = json!({"sessionUpdate":"session_info_update", "_meta":{"codex":{"threadStatus":{"type":"systemError"}}}});
    let mut translator = UiTranslator::new();
    feed(&mut translator, std::slice::from_ref(&error_status))?;
    assert_eq!(
        wire(translator.finish(TurnEnd::Completed(StopReason::EndTurn)))?,
        [
            json!({"type":"start-step"}),
            json!({"type":"error", "errorText":"The ACP agent failed to respond."}),
            json!({"type":"finish-step"}),
            json!({"type":"finish", "finishReason":"error"}),
        ]
    );
    // With visible output the reply stands, as before.
    let mut translator = UiTranslator::new();
    feed(&mut translator, &[error_status, text("hi", None)])?;
    let end = wire(translator.finish(TurnEnd::Completed(StopReason::EndTurn)))?;
    assert_eq!(
        end.last(),
        Some(&json!({"type":"finish", "finishReason":"stop"}))
    );
    Ok(())
}

/// The F6 change is safe for history: the reducer that persists the assistant message (P1's port
/// of `processUIMessageStream`) still ends with a static, provider-executed tool part.
#[test]
fn the_persisted_message_keeps_a_static_provider_executed_tool_part() -> TestResult {
    let mut translator = UiTranslator::new();
    let mut assembler = ui_message_stream::UiMessageAssembler::new();
    assembler.push(&UiChunk::Start {
        message_metadata: None,
        message_id: Some("m".into()),
    });
    let updates = [
        text("Running", Some("m1")),
        json!({"sessionUpdate":"tool_call", "toolCallId":"toolu_1", "title":"Terminal", "status":"pending", "rawInput":{}}),
        json!({"sessionUpdate":"tool_call_update", "toolCallId":"toolu_1", "title":"touch probe.txt",
               "status":"in_progress", "rawInput":{"command":"touch probe.txt"}}),
        json!({"sessionUpdate":"tool_call_update", "toolCallId":"toolu_1", "status":"completed",
               "rawOutput":"(Bash completed with no output)"}),
    ];
    for update in updates {
        let update: SessionUpdate = serde_json::from_value(update)?;
        for chunk in translator.on_update(&update) {
            assembler.push(&chunk);
        }
    }
    for chunk in translator.finish(TurnEnd::Completed(StopReason::EndTurn)) {
        assembler.push(&chunk);
    }
    let message = serde_json::to_value(assembler.into_message().ok_or("no message")?)?;
    assert_eq!(message["id"], "m");
    assert_eq!(message["parts"][0]["type"], "step-start");
    assert_eq!(message["parts"][1]["text"], "Running");
    let tool = &message["parts"][2];
    assert_eq!(tool["type"], "tool-Terminal");
    assert_eq!(tool["state"], "output-available");
    assert_eq!(tool["input"], json!({"command":"touch probe.txt"}));
    assert_eq!(tool["output"], "(Bash completed with no output)");
    assert_eq!(tool["providerExecuted"], true);
    assert!(
        tool.get("errorText").is_none(),
        "no F6 error residue: {tool}"
    );
    Ok(())
}
