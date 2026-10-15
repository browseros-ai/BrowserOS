//! Compatibility fixtures captured from ai@7.0.114 and the real TS server helpers.
use browseros_agent::{
    messages::*,
    tools::{ToolCatalog, ToolKind, ToolSpec},
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use ui_message_stream::UiMessage;

fn fixtures() -> Result<Value, serde_json::Error> {
    serde_json::from_str(include_str!("../src/messages/golden.json"))
}

#[test]
fn converts_ts_ui_history() -> Result<(), Box<dyn std::error::Error>> {
    for case in fixtures()?["conversion"]
        .as_array()
        .ok_or("conversion cases")?
    {
        let specs = case["catalog"]
            .as_array()
            .ok_or("catalog")?
            .iter()
            .map(|spec| ToolSpec {
                name: spec["name"].as_str().unwrap_or_default().to_owned(),
                description: String::new(),
                input_schema: json!({}),
                kind: if spec["dynamic"] == true {
                    ToolKind::Dynamic {
                        title: None,
                        metadata: json!({}),
                    }
                } else {
                    ToolKind::Static
                },
            })
            .collect::<Vec<_>>();
        let history: Vec<UiMessage> = serde_json::from_value(case["history"].clone())?;
        let actual = ui_to_model(&history, &ToolCatalog::new(&specs))?;
        assert_eq!(
            serde_json::to_value(actual)?,
            case["expected"],
            "{}",
            case["name"]
        );
    }
    Ok(())
}

#[test]
fn history_hygiene_matches_ts_without_mutating_display_history()
-> Result<(), Box<dyn std::error::Error>> {
    for case in fixtures()?["hygiene"].as_array().ok_or("hygiene cases")? {
        let history: Vec<UiMessage> = serde_json::from_value(case["history"].clone())?;
        let names: BTreeSet<String> = serde_json::from_value(case["toolNames"].clone())?;
        assert_eq!(
            serde_json::to_value(filter_valid(history.clone()))?,
            case["filtered"]
        );
        assert_eq!(
            serde_json::to_value(strip_reasoning(history.clone()))?,
            case["stripped"]
        );
        assert_eq!(
            serde_json::to_value(sanitize_for_toolset(history.clone(), &names))?,
            case["sanitized"]
        );
        assert_eq!(serde_json::to_value(history)?, case["history"]);
    }
    Ok(())
}

#[test]
fn media_hoisting_matches_ts_for_each_provider_capability() -> Result<(), Box<dyn std::error::Error>>
{
    for case in fixtures()?["normalization"]
        .as_array()
        .ok_or("normalization cases")?
    {
        let messages = serde_json::from_value(case["messages"].clone())?;
        let actual = normalize_for_model(
            messages,
            case["options"]["supportsMediaInToolResults"] == true,
            case["options"]["supportsImages"] == true,
        );
        assert_eq!(serde_json::to_value(actual)?, case["expected"]);
    }
    Ok(())
}

#[test]
fn native_media_transport_retains_provider_fields() -> Result<(), Box<dyn std::error::Error>> {
    let wire = json!([{"role": "tool", "content": [{"type": "tool-result", "toolCallId": "c",
    "toolName": "file", "output": {"type": "content", "value": [
        {"type": "file-url", "url": "https://example.com/file", "mediaType": "application/pdf", "filename": "a.pdf"},
        {"type": "text", "text": "caption", "providerOptions": {"vendor": {"cache": true}}}
    ]}}]}]);
    let messages = serde_json::from_value(wire.clone())?;
    assert_eq!(
        serde_json::to_value(normalize_for_model(messages, true, true))?,
        wire
    );
    Ok(())
}
