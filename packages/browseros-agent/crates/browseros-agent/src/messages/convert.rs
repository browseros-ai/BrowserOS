//! UI history becomes SDK-shaped model messages here, including step boundaries and tool
//! output codecs. The catalog selects the same MCP/filesystem/nudge codecs used by TS;
//! serialization through the typed transcript prevents SDK-specific objects leaking to rig.
use super::{ConvertError, ModelMessage};
use crate::tools::{ToolCatalog, ToolKind};
use serde_json::{Map, Value, json};
use ui_message_stream::{Role, UiMessage};

pub(super) fn convert(
    history: &[UiMessage],
    catalog: &ToolCatalog,
) -> Result<Vec<ModelMessage>, ConvertError> {
    let mut result = Vec::new();
    for message in history {
        let wire = serde_json::to_value(message).map_err(error)?;
        let parts = wire["parts"]
            .as_array()
            .ok_or_else(|| ConvertError("Missing UI parts".into()))?;
        match message.role {
            Role::System => {
                let mut metadata = Map::new();
                let mut text = String::new();
                for part in parts.iter().filter(|p| p["type"] == "text") {
                    text.push_str(part["text"].as_str().unwrap_or_default());
                    if let Some(options) = part["providerMetadata"].as_object() {
                        metadata.extend(options.clone());
                    }
                }
                let mut output = json!({"role": "system", "content": text});
                if !metadata.is_empty() {
                    output["providerOptions"] = Value::Object(metadata);
                }
                push(&mut result, output)?;
            }
            Role::User => {
                let mut content = Vec::new();
                for part in parts {
                    match part["type"].as_str() {
                        Some("text") => content.push(text_part(part, "text")),
                        Some("file") => content.push(file_part(part, false)?),
                        _ => {}
                    }
                }
                push(&mut result, json!({"role": "user", "content": content}))?;
            }
            Role::Assistant => {
                let mut block = Vec::new();
                for part in parts {
                    let kind = part["type"].as_str().unwrap_or_default();
                    if kind == "step-start" {
                        process_block(&mut result, &block, catalog)?;
                        block.clear();
                    } else if is_tool(part) {
                        // BrowserOS always sets ignoreIncompleteToolCalls=true. A preliminary result
                        // is still incomplete, while an answered approval is useful without a result.
                        let completed = matches!(
                            part["state"].as_str(),
                            Some("approval-responded" | "output-error" | "output-denied")
                        ) || (part["state"] == "output-available"
                            && part["preliminary"] != true);
                        if completed {
                            block.push(part);
                        }
                    } else if matches!(
                        kind,
                        "text" | "reasoning" | "reasoning-file" | "file" | "custom"
                    ) || kind.starts_with("data-")
                    {
                        block.push(part);
                    }
                }
                process_block(&mut result, &block, catalog)?;
            }
        }
    }
    Ok(result)
}

fn error(error: impl std::fmt::Display) -> ConvertError {
    ConvertError(error.to_string())
}
fn push(result: &mut Vec<ModelMessage>, value: Value) -> Result<(), ConvertError> {
    result.push(serde_json::from_value(value).map_err(error)?);
    Ok(())
}
fn is_tool(part: &Value) -> bool {
    part["type"]
        .as_str()
        .is_some_and(|kind| kind == "dynamic-tool" || kind.starts_with("tool-"))
}
fn tool_name(part: &Value) -> &str {
    if part["type"] == "dynamic-tool" {
        part["toolName"].as_str().unwrap_or_default()
    } else {
        part["type"]
            .as_str()
            .and_then(|kind| kind.strip_prefix("tool-"))
            .unwrap_or_default()
    }
}
fn copy(target: &mut Value, key: &str, source: &Value, source_key: &str) {
    if let Some(value) = source.get(source_key) {
        target[key] = value.clone();
    }
}
fn metadata(target: &mut Value, part: &Value, key: &str) {
    if !part[key].is_null() {
        target["providerOptions"] = part[key].clone();
    }
}
fn text_part(part: &Value, kind: &str) -> Value {
    let mut output = json!({"type": kind, "text": part["text"]});
    metadata(&mut output, part, "providerMetadata");
    output
}
fn file_part(part: &Value, reasoning: bool) -> Result<Value, ConvertError> {
    let data = if !reasoning && !part["providerReference"].is_null() {
        json!({"type": "reference", "reference": part["providerReference"]})
    } else {
        let raw = part["url"]
            .as_str()
            .ok_or_else(|| ConvertError("File part without URL".into()))?;
        let url = url::Url::parse(raw).map_err(error)?;
        json!({"type": "url", "url": url.as_str()})
    };
    let mut output = json!({"type": if reasoning { "reasoning-file" } else { "file" }, "mediaType": part["mediaType"], "data": data});
    if !reasoning {
        copy(&mut output, "filename", part, "filename");
    }
    metadata(&mut output, part, "providerMetadata");
    Ok(output)
}

fn process_block(
    result: &mut Vec<ModelMessage>,
    block: &[&Value],
    catalog: &ToolCatalog,
) -> Result<(), ConvertError> {
    let mut assistant = Vec::new();
    for part in block {
        match part["type"].as_str() {
            Some("text" | "reasoning") => {
                assistant.push(text_part(part, part["type"].as_str().unwrap_or_default()))
            }
            Some("file") => assistant.push(file_part(part, false)?),
            Some("reasoning-file") => assistant.push(file_part(part, true)?),
            Some("custom") => {
                let mut custom = json!({"type": "custom", "kind": part["kind"]});
                metadata(&mut custom, part, "providerMetadata");
                assistant.push(custom);
            }
            _ if is_tool(part) => {
                let name = tool_name(part);
                let mut call = json!({"type": "tool-call", "toolCallId": part["toolCallId"], "toolName": name});
                if part["state"] == "output-error" && part["input"].is_null() {
                    copy(&mut call, "input", part, "rawInput");
                } else {
                    copy(&mut call, "input", part, "input");
                }
                copy(&mut call, "providerExecuted", part, "providerExecuted");
                let call_metadata =
                    if part["callProviderMetadata"].is_null() && part["state"] == "output-error" {
                        "resultProviderMetadata"
                    } else {
                        "callProviderMetadata"
                    };
                metadata(&mut call, part, call_metadata);
                assistant.push(call);
                if !part["approval"].is_null() {
                    let approval = &part["approval"];
                    let mut request = json!({"type": "tool-approval-request", "approvalId": approval["id"], "toolCallId": part["toolCallId"]});
                    copy(&mut request, "isAutomatic", approval, "isAutomatic");
                    if !approval["requestReason"].is_null() {
                        request["reason"] = approval["requestReason"].clone();
                    }
                    copy(
                        &mut request,
                        "inputSchemaInput",
                        approval,
                        "inputSchemaInput",
                    );
                    if !approval["signature"].is_null() {
                        request["signature"] = approval["signature"].clone();
                    }
                    assistant.push(request);
                }
                if part["providerExecuted"] == true
                    && matches!(
                        part["state"].as_str(),
                        Some("output-available" | "output-error")
                    )
                {
                    let output = tool_output(part, catalog, true);
                    let key = if part["resultProviderMetadata"].is_null() {
                        "callProviderMetadata"
                    } else {
                        "resultProviderMetadata"
                    };
                    assistant.push(tool_result(part, output, key));
                }
            }
            _ => {} // data-* has no converter in BrowserOS and disappears.
        }
    }
    if !assistant.is_empty() {
        push(result, json!({"role": "assistant", "content": assistant}))?;
    }
    let mut tool = Vec::new();
    for part in block.iter().filter(|part| is_tool(part)) {
        if part["providerExecuted"] == true && part["approval"]["approved"].is_null() {
            continue;
        }
        let approval = &part["approval"];
        if !approval["approved"].is_null() {
            let mut response = json!({"type": "tool-approval-response", "approvalId": approval["id"], "approved": approval["approved"]});
            copy(&mut response, "reason", approval, "reason");
            copy(&mut response, "providerExecuted", part, "providerExecuted");
            tool.push(response);
        }
        if part["state"] == "approval-responded" && approval["approved"] == false {
            let mut output = json!({"type": "execution-denied"});
            copy(&mut output, "reason", approval, "reason");
            tool.push(tool_result(part, output, "callProviderMetadata"));
        }
        // A provider result is already in assistant content. Duplicating it here creates
        // orphaned function_call_output entries in OpenAI Responses replay.
        if part["providerExecuted"] == true {
            continue;
        }
        match part["state"].as_str() {
            Some("output-denied") => tool.push(tool_result(part, json!({"type": "error-text", "value": approval["reason"].as_str().unwrap_or("Tool call execution denied.")}), "callProviderMetadata")),
            Some("output-error" | "output-available") => tool.push(tool_result(part, tool_output(part, catalog, false), "callProviderMetadata")),
            _ => {},
        }
    }
    if !tool.is_empty() {
        push(result, json!({"role": "tool", "content": tool}))?;
    }
    Ok(())
}
fn tool_result(part: &Value, output: Value, key: &str) -> Value {
    let mut result = json!({"type": "tool-result", "toolCallId": part["toolCallId"], "toolName": tool_name(part), "output": output});
    metadata(&mut result, part, key);
    result
}

fn tool_output(part: &Value, catalog: &ToolCatalog, provider_executed: bool) -> Value {
    if part["state"] == "output-error" {
        return json!({"type": if provider_executed { "error-json" } else { "error-text" }, "value": part.get("errorText").cloned().unwrap_or(Value::Null)});
    }
    let output = &part["output"];
    let name = tool_name(part);
    match catalog.kind(name) {
        Some(ToolKind::Dynamic { .. }) => {
            if let Some(content) = output["content"].as_array() {
                let value: Vec<Value> = content.iter().map(|item| {
                    if item["type"] == "text" && item.get("text").is_some() {
                        json!({"type": "text", "text": item["text"]})
                    } else if item["type"] == "image" && item.get("data").is_some() && item.get("mimeType").is_some() {
                        json!({"type": "file", "mediaType": item["mimeType"], "data": {"type": "data", "data": item["data"]}})
                    } else { json!({"type": "text", "text": super::js_stringify(item)}) }
                }).collect();
                json!({"type": "content", "value": value})
            } else {
                json!({"type": "json", "value": output})
            }
        }
        Some(ToolKind::Static) if matches!(name, "suggest_schedule" | "suggest_app_connection") => {
            json!({"type": "text", "value": output["content"][0]["text"].as_str().unwrap_or_default()})
        }
        Some(ToolKind::Static) if name.starts_with("filesystem_") => {
            if output["isError"] == true {
                json!({"type": "error-text", "value": output["text"]})
            } else if let Some(images) = output["images"]
                .as_array()
                .filter(|images| !images.is_empty())
            {
                let mut value = vec![json!({"type": "text", "text": output["text"]})];
                value.extend(images.iter().map(|image| json!({"type": "image-data", "data": image["data"], "mediaType": image["mimeType"]})));
                json!({"type": "content", "value": value})
            } else {
                json!({"type": "text", "value": output["text"].as_str().filter(|text| !text.is_empty()).unwrap_or("Success")})
            }
        }
        _ => json!({"type": if output.is_string() { "text" } else { "json" }, "value": output}),
    }
}
