//! The UI message stream chunk vocabulary, byte-compatible with `ai@7.0.114`.
//!
//! Field order inside each variant is the key order `toUIMessageChunk`
//! (`ai/src/ui-message-stream/to-ui-message-chunk.ts`) emits, because the stream-compat harness
//! byte-diffs the Rust server's SSE against the TS server's. Optional keys are omitted; required
//! JSON payloads keep explicit null, including `tool-output-available.output` for absent results.
//! Variants cover every `ai@7.0.114` `UIMessageChunk` type (the contract union is complete),
//! including `data-*` chunks, whose open-ended type is the one untagged variant.

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
    SourceDocument {
        source_id: String,
        media_type: String,
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },
    File {
        media_type: String,
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },
    /// Model-generated reasoning media (e.g. Gemini thought images); same shape as `file`.
    ReasoningFile {
        media_type: String,
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },
    /// `kind` is namespaced (`<provider>.<name>`).
    Custom {
        #[serde(deserialize_with = "namespaced_kind")]
        kind: String,
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
    ToolApprovalRequest {
        approval_id: String,
        tool_call_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        approval_descriptor: Option<Value>,
        #[serde(
            default,
            deserialize_with = "present",
            skip_serializing_if = "Option::is_none"
        )]
        input_schema_input: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        is_automatic: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    ToolApprovalResponse {
        approval_id: String,
        approved: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_executed: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },
    ToolOutputDenied {
        tool_call_id: String,
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
    ResetStep,
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
    /// `messageMetadata` is required but may be `null`.
    MessageMetadata {
        message_metadata: Value,
    },
    /// `data-<name>` chunks. Untagged, so it must stay the last variant: serde tries it only when
    /// no fixed `type` matched, and [`DataChunk`] rejects types without the `data-` prefix.
    #[serde(untagged)]
    Data(DataChunk),
}

/// A `data-<name>` chunk; `kind` is the full wire `type` (e.g. `data-notice`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataChunk {
    #[serde(rename = "type", deserialize_with = "data_type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Required but may be `null`.
    pub data: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transient: Option<bool>,
}

// Approval input is opaque JSON: a present null differs from an omitted key in the SDK's
// hasOwnProperty check, and must survive both SSE replay and persisted approval metadata.
fn present<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

fn namespaced_kind<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let kind = String::deserialize(deserializer)?;
    if kind.contains('.') {
        Ok(kind)
    } else {
        Err(serde::de::Error::custom(format!(
            "custom chunk kind `{kind}` is not `<namespace>.<name>`"
        )))
    }
}

fn data_type<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let kind = String::deserialize(deserializer)?;
    if kind.starts_with("data-") {
        Ok(kind)
    } else {
        Err(serde::de::Error::custom(format!(
            "unknown UI chunk type `{kind}`"
        )))
    }
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

    /// Every `chunk-*.json` fixture of the server-api contract parses and re-serializes to the
    /// same JSON.
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

    /// The contract's extra examples (`tests/ui-stream-examples.json`): every additional chunk
    /// round-trips, every malformed known chunk is rejected.
    #[test]
    fn contract_stream_examples() -> Result<(), Box<dyn std::error::Error>> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../contracts/server-api/tests/ui-stream-examples.json");
        let examples: Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
        let list = |key: &str| {
            examples
                .get(key)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        let additional = list("additionalChunks");
        assert!(
            !additional.is_empty(),
            "no additionalChunks in the examples"
        );
        for original in additional {
            let chunk: UiChunk = serde_json::from_value(original.clone())
                .map_err(|error| format!("{original}: {error}"))?;
            assert!(!matches!(chunk, UiChunk::Data(ref data) if !data.kind.starts_with("data-")));
            assert_eq!(serde_json::to_value(&chunk)?, original);
        }
        for malformed in list("malformedChunks") {
            assert!(
                serde_json::from_value::<UiChunk>(malformed.clone()).is_err(),
                "accepted malformed chunk {malformed}"
            );
        }
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
    fn data_chunks_round_trip_and_unknown_types_fail() -> Result<(), serde_json::Error> {
        let data = json!({"type": "data-notice", "id": "n1", "data": {"k": 1}, "transient": true});
        let chunk: UiChunk = serde_json::from_value(data.clone())?;
        assert!(matches!(&chunk, UiChunk::Data(DataChunk { kind, .. }) if kind == "data-notice"));
        assert_eq!(serde_json::to_value(&chunk)?, data);
        assert!(serde_json::from_value::<UiChunk>(json!({"type": "nope"})).is_err());
        let file: UiChunk = serde_json::from_value(
            json!({"type": "reasoning-file", "mediaType": "image/png", "url": "data:image/png;base64,AA=="}),
        )?;
        assert!(matches!(file, UiChunk::ReasoningFile { .. }));
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
