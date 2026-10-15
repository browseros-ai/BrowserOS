//! The UI message stream chunk vocabulary, byte-compatible with `ai@7.0.114`.
//!
//! Field order inside each variant is the key order `toUIMessageChunk`
//! (`ai/src/ui-message-stream/to-ui-message-chunk.ts`) emits, because the stream-compat harness
//! byte-diffs the Rust server's SSE against the TS server's. Optional keys are omitted, never
//! `null`; the one exception is `tool-output-available.output`, which the SDK always sends.
//! Variants cover C1's `UiMessageChunk` union (`contracts/server-api/schemas/ui_stream.yaml`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum UiChunk {
    Start {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_metadata: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
    },
    StartStep,
    TextStart {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },
    TextDelta {
        id: String,
        delta: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },
    TextEnd {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },
    ReasoningStart {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },
    ReasoningDelta {
        id: String,
        delta: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },
    ReasoningEnd {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },
    /// Only emitted when the SDK's `sendSources` is on; kept so every contract chunk parses.
    SourceUrl {
        source_id: String,
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },
    File {
        media_type: String,
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },
    ToolInputStart {
        tool_call_id: String,
        tool_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_executed: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_metadata: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dynamic: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    ToolInputDelta {
        tool_call_id: String,
        input_text_delta: String,
    },
    ToolInputAvailable {
        tool_call_id: String,
        tool_name: String,
        #[serde(default)]
        input: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_executed: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_metadata: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dynamic: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    /// `errorText` sits between `dynamic` and `title`: that is the SDK's emission order.
    ToolInputError {
        tool_call_id: String,
        tool_name: String,
        #[serde(default)]
        input: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_executed: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_metadata: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dynamic: Option<bool>,
        error_text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    /// `output` serializes `null` when the tool returned nothing (the one non-omitted optional).
    ToolOutputAvailable {
        tool_call_id: String,
        #[serde(default)]
        output: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_executed: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_metadata: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preliminary: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dynamic: Option<bool>,
    },
    ToolOutputError {
        tool_call_id: String,
        error_text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_executed: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_metadata: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dynamic: Option<bool>,
    },
    Error {
        error_text: String,
    },
    FinishStep,
    Finish {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        finish_reason: Option<FinishReason>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_metadata: Option<Value>,
    },
    Abort {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FinishReason {
    Stop,
    Length,
    ContentFilter,
    ToolCalls,
    Error,
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn fixtures_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts/server-api/fixtures")
    }

    /// Every `chunk-*.json` fixture of C1's contract parses and re-serializes to the same JSON.
    #[test]
    fn contract_chunk_fixtures_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let mut seen = 0;
        for entry in std::fs::read_dir(fixtures_dir())? {
            let path = entry?.path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            if !(name.starts_with("chunk-") && name.ends_with(".json")) {
                continue;
            }
            let original: Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
            let chunk: UiChunk = serde_json::from_value(original.clone())
                .map_err(|error| format!("{name}: {error}"))?;
            assert_eq!(serde_json::to_value(&chunk)?, original, "{name}");
            seen += 1;
        }
        assert!(
            seen >= 18,
            "expected the contract's chunk fixtures, found {seen}"
        );
        Ok(())
    }

    #[test]
    fn keys_follow_ai_sdk_emission_order() -> Result<(), serde_json::Error> {
        let chunk = UiChunk::ToolInputStart {
            tool_call_id: "c1".into(),
            tool_name: "snapshot".into(),
            provider_executed: None,
            provider_metadata: None,
            tool_metadata: Some(json!({"k": 1})),
            dynamic: Some(true),
            title: Some("Snapshot".into()),
        };
        assert_eq!(
            serde_json::to_string(&chunk)?,
            r#"{"type":"tool-input-start","toolCallId":"c1","toolName":"snapshot","toolMetadata":{"k":1},"dynamic":true,"title":"Snapshot"}"#
        );
        let start = UiChunk::Start {
            message_metadata: None,
            message_id: Some("m1".into()),
        };
        assert_eq!(
            serde_json::to_string(&start)?,
            r#"{"type":"start","messageId":"m1"}"#
        );
        Ok(())
    }

    #[test]
    fn tool_output_keeps_null_output() -> Result<(), serde_json::Error> {
        let chunk = UiChunk::ToolOutputAvailable {
            tool_call_id: "c1".into(),
            output: Value::Null,
            provider_executed: None,
            provider_metadata: None,
            tool_metadata: None,
            preliminary: None,
            dynamic: None,
        };
        assert_eq!(
            serde_json::to_string(&chunk)?,
            r#"{"type":"tool-output-available","toolCallId":"c1","output":null}"#
        );
        Ok(())
    }
}
