//! Golden bytes and per-chunk persisted messages come from the real pinned TypeScript SDK.
//! Exercising the public reducer/framer also catches contract drift at both engine call sites.
use futures_util::StreamExt as _;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{error::Error, fs, path::PathBuf};
use ui_message_stream::{ChunkStream, UiChunk, UiMessage, UiMessageAssembler, sse};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[derive(Deserialize)]
struct Golden {
    snapshots: Vec<Value>,
    message: Value,
}

#[tokio::test]
async fn ts_bytes_and_every_persisted_snapshot_match() -> Result<(), Box<dyn Error>> {
    let mut count = 0;
    for entry in fs::read_dir(fixtures())? {
        let path = entry?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("sse") {
            continue;
        }
        let bytes = fs::read(&path)?;
        let chunks = sse::parse(&bytes)?;
        let golden: Golden = serde_json::from_slice(&fs::read(path.with_extension("json"))?)?;
        assert_eq!(chunks.len(), golden.snapshots.len(), "{}", path.display());
        let mut assembler = UiMessageAssembler::new();
        assert!(assembler.message().is_none());
        for (index, (chunk, expected)) in chunks.iter().zip(&golden.snapshots).enumerate() {
            assembler.push(chunk);
            assert!(
                assembler.error().is_none(),
                "{} at {index}: {:?}",
                path.display(),
                assembler.error()
            );
            assert_eq!(
                serde_json::to_value(assembler.message())?,
                *expected,
                "{} after chunk {index}: {chunk:?}",
                path.display()
            );
            // A persisted snapshot must also survive loading through the shared message model.
            let stored: UiMessage = serde_json::from_value(expected.clone())?;
            assert_eq!(serde_json::to_value(stored)?, *expected);
            let emitted = sse::frame(chunk);
            let payload = emitted
                .strip_prefix(b"data: ")
                .and_then(|bytes| bytes.strip_suffix(b"\n\n"))
                .ok_or("missing framing")?;
            let _: server_api::models::UiMessageChunk = serde_json::from_slice(payload)?;
        }
        assert_eq!(
            serde_json::to_value(assembler.into_message())?,
            golden.message,
            "{} final",
            path.display()
        );
        let input: ChunkStream = Box::pin(futures_util::stream::iter(chunks));
        let actual = sse::frames(input)
            .map(|result| match result {
                Ok(frame) => frame,
                Err(never) => match never {},
            })
            .collect::<Vec<_>>()
            .await
            .concat();
        assert_eq!(
            String::from_utf8(actual)?,
            String::from_utf8(bytes)?,
            "{} SSE bytes/key order",
            path.display()
        );
        count += 1;
    }
    assert!(count >= 8, "fixture inventory is incomplete: {count}");
    Ok(())
}

#[test]
fn protocol_violations_stop_reduction_without_fabricating_parts() -> Result<(), Box<dyn Error>> {
    for invalid in [
        json!({"type":"text-delta","id":"missing","delta":"bad"}),
        json!({"type":"text-end","id":"missing"}),
        json!({"type":"reasoning-delta","id":"missing","delta":"bad"}),
        json!({"type":"reasoning-end","id":"missing"}),
        json!({"type":"tool-input-delta","toolCallId":"missing","inputTextDelta":"{}"}),
        json!({"type":"tool-output-available","toolCallId":"missing","output":null}),
        json!({"type":"tool-approval-response","approvalId":"missing","approved":true}),
    ] {
        let mut assembler = UiMessageAssembler::new();
        assembler.push(&serde_json::from_value(json!({"type":"start"}))?);
        assembler.push(&serde_json::from_value(invalid)?);
        assert!(assembler.error().is_some());
        assembler.push(&UiChunk::StartStep);
        assert_eq!(
            serde_json::to_value(assembler.message())?,
            json!({"id":"","role":"assistant","parts":[]})
        );
    }
    Ok(())
}

#[test]
fn parser_reports_bad_frames_and_chunks() {
    assert!(matches!(
        sse::parse(b"event: message\n\n"),
        Err(sse::ParseError::NotData { index: 0, .. })
    ));
    assert!(matches!(
        sse::parse(b"data: {\"type\":\"text-delta\"}\n\n"),
        Err(sse::ParseError::Chunk { index: 0, .. })
    ));
    assert!(matches!(sse::parse(&[0xff]), Err(sse::ParseError::Utf8(_))));
}

#[test]
fn headers_match_sdk_capture() -> Result<(), Box<dyn Error>> {
    let headers: Value = serde_json::from_slice(&fs::read(fixtures().join("headers.json"))?)?;
    let actual: serde_json::Map<String, Value> = sse::UI_STREAM_HEADERS
        .into_iter()
        .map(|(key, value)| (key.to_owned(), Value::String(value.to_owned())))
        .collect();
    assert_eq!(Value::Object(actual), headers);
    Ok(())
}

#[test]
fn framing_orders_nested_integer_keys_like_javascript() -> Result<(), Box<dyn Error>> {
    // These keys were constructed by Rust, rather than parsed from an already ordered TS
    // fixture. Ordinary strings retain insertion order after the numeric array-index keys.
    let chunk: UiChunk = serde_json::from_value(json!({"type":"data-json", "data": {
        "z": 0, "10": "ten", "2": "two", "01": "one", "4294967295": "max", "a": 1,
        "nested": {"b": [1.0, -0.0, 0.000001, 1e-7, 1e21], "0": 0}
    }}))?;
    assert_eq!(sse::frame(&chunk).as_ref(), b"data: {\"type\":\"data-json\",\"data\":{\"2\":\"two\",\"10\":\"ten\",\"z\":0,\"01\":\"one\",\"4294967295\":\"max\",\"a\":1,\"nested\":{\"0\":0,\"b\":[1,0,0.000001,1e-7,1e+21]}}}\n\n");
    Ok(())
}

#[test]
fn required_json_fields_cannot_disappear() {
    for invalid in [
        json!({"type":"tool-input-available","toolCallId":"c","toolName":"t"}),
        json!({"type":"tool-input-error","toolCallId":"c","toolName":"t","errorText":"bad"}),
        json!({"type":"tool-output-available","toolCallId":"c"}),
    ] {
        assert!(serde_json::from_value::<UiChunk>(invalid).is_err());
    }
}
