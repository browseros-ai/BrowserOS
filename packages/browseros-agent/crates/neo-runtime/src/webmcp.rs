//! Playwright: `server/webmcp.ts` `WebMCP`
//!
//! (G) `page.webmcp` and `frame.webmcp`: the tools a document registered with `WebMCP`, read and
//! called by Playwright's injected `WebMCPScript` in the frame's utility world, Neo's isolated
//! world (`glue.ts` `webmcpTools`, `webmcpCallTool`), never the page's own. A tool's `input`
//! arrives as the protocol's `SerializedValue` and goes to the page in the page serializer's
//! form ([`page_value`]), as Playwright's server parses it before it evaluates; the answer comes
//! back in the page serializer's form, which Neo's SDK reads as it reads an evaluation's result.

use neo_runtime_spec::injected::{
    InjectedWebmcpCallToolParams, InjectedWebmcpToolsParams, WebMCPToolDescription,
};
use neo_runtime_spec::serve::{
    FrameWebmcpCallToolParams, FrameWebmcpToolsResult, FrameWebmcpToolsResultToolsItem,
    FrameWebmcpToolsResultToolsItemAnnotations,
};

use serde_json::{Map, Value, json};

use crate::click::PageError;
use crate::page::FrameTab;
use crate::progress::{CallError, PlaywrightError, Progress};

/// Playwright: `server/webmcp.ts` `WebMCP.tools`
///
/// # Errors
///
/// The page's error, and the call's stop. Playwright's check of the browser's launch options
/// has nothing to read for a browser it connected to (`assertBrowserSupport`), so none fails.
pub(crate) async fn tools(
    tab: &FrameTab,
    progress: &Progress,
) -> Result<FrameWebmcpToolsResult, CallError> {
    let context = progress.race(tab.context()).await?.map_err(page_error)?;
    let described = progress
        .race(context.call(None, &InjectedWebmcpToolsParams {}))
        .await?
        .map_err(page_error)?;
    Ok(FrameWebmcpToolsResult {
        tools: described.tools.into_iter().map(normalize).collect(),
    })
}

/// Playwright: `server/webmcp.ts` `WebMCP.callTool`
///
/// # Errors
///
/// Playwright's error for a tool the frame does not register, the tool's own failure, and the
/// call's stop.
pub(crate) async fn call_tool(
    tab: &FrameTab,
    params: FrameWebmcpCallToolParams,
    progress: &Progress,
) -> Result<Value, CallError> {
    let FrameWebmcpCallToolParams { name, input } = params;
    if !input.handles.is_empty() {
        // Deviation: `server/webmcp.ts` `WebMCP.callTool`: `{ isFunction: true, world }, { name, input: input ?? {} });` Playwright hands a handle in `input` to its utility world, where a page's handle cannot be read; Neo refuses one before the call.
        return Err(PlaywrightError::error(
            "neo-runtime: a WebMCP tool's input cannot hold a JSHandle",
        )
        .into());
    }
    let tools = tools(tab, progress).await?.tools;
    if !tools.iter().any(|tool| tool.name == name) {
        let available: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
        // Playwright: `server/webmcp.ts` `WebMCP.callTool`: `if (!tools.some(tool => tool.name === name)) {`
        let message = if available.is_empty() {
            format!(
                "No WebMCP tool named \"{name}\". The frame does not register any WebMCP tools."
            )
        } else {
            format!(
                "No WebMCP tool named \"{name}\". Available tools: {}.",
                available.join(", ")
            )
        };
        return Err(PlaywrightError::error(message).into());
    }
    let context = progress.race(tab.context()).await?.map_err(page_error)?;
    let input = page_value(&serde_json::to_value(&input.value).map_err(|error| internal(&error))?)?;
    let answered = progress
        .race(context.call(None, &InjectedWebmcpCallToolParams { name, input }))
        .await?
        .map_err(page_error)?;
    // Neo: the answer stays in the page serializer's form, as an evaluation's result crosses to
    // the SDK (`evaluate.rs`), whose `parseResult` reads it.
    Ok(json!({ "result": answered.result }))
}

/// Playwright: `server/webmcp.ts` `normalizeTool`
fn normalize(tool: WebMCPToolDescription) -> FrameWebmcpToolsResultToolsItem {
    let annotations = tool.annotations.and_then(|annotations| {
        // Playwright: `server/webmcp.ts` `normalizeTool`: `if (tool.annotations?.readOnly)`
        let flag = |set: Option<bool>| set.filter(|&set| set);
        let kept = FrameWebmcpToolsResultToolsItemAnnotations {
            read_only: flag(annotations.read_only),
            untrusted_content: flag(annotations.untrusted_content),
            consequential: flag(annotations.consequential),
        };
        // Playwright: `server/webmcp.ts` `normalizeTool`: `...(Object.keys(annotations).length ? { annotations } : {}),`
        (kept.read_only.is_some()
            || kept.untrusted_content.is_some()
            || kept.consequential.is_some())
        .then_some(kept)
    });
    FrameWebmcpToolsResultToolsItem {
        name: tool.name,
        description: tool.description,
        input_schema: tool.input_schema,
        annotations,
    }
}

/// Neo: a page call's failure as the call's error; a frame that is gone fails as closed.
fn page_error(error: PageError) -> CallError {
    match error {
        PageError::Threw(message) | PageError::Contract(message) => {
            PlaywrightError::error(message).into()
        }
        PageError::Closed | PageError::NotConnected => PlaywrightError::target_closed().into(),
    }
}

/// Neo: a value Neo's serde cannot carry between the protocol and the page.
fn internal(error: &serde_json::Error) -> CallError {
    PlaywrightError::error(format!("neo-runtime: {error}")).into()
}

/// Neo: a protocol `SerializedValue` (Playwright's `protocol/src/serializers.ts`, primitives as
/// `{n}`, `{b}`, `{s}`) in the page serializer's form (`isomorphic/utilityScriptSerializers.ts`,
/// primitives bare), as Playwright's server parses the client's value before it evaluates. A
/// tool's input is data: a map, a set, a typed array or a handle is refused.
fn page_value(value: &Value) -> Result<Value, CallError> {
    let Some(tagged) = value.as_object() else {
        return Err(unsupported());
    };
    if let Some(primitive) = ["n", "b", "s"].iter().find_map(|tag| tagged.get(*tag))
        && (primitive.is_number() || primitive.is_boolean() || primitive.is_string())
    {
        return Ok(primitive.clone());
    }
    convert(tagged, page_value)
}

/// Neo: the tags both forms share, with each nested value converted by `inner`: an array's
/// items, an object's property values, and the leaves both forms write alike.
fn convert(
    tagged: &Map<String, Value>,
    inner: fn(&Value) -> Result<Value, CallError>,
) -> Result<Value, CallError> {
    let mut out = tagged.clone();
    if let Some(items) = tagged.get("a").and_then(Value::as_array) {
        let items: Result<Vec<Value>, CallError> = items.iter().map(inner).collect();
        out.insert("a".to_string(), Value::Array(items?));
        return Ok(Value::Object(out));
    }
    if let Some(entries) = tagged.get("o").and_then(Value::as_array) {
        let mut converted = Vec::with_capacity(entries.len());
        for entry in entries {
            let (Some(key), Some(value)) = (entry.get("k"), entry.get("v")) else {
                return Err(unsupported());
            };
            converted.push(json!({ "k": key, "v": inner(value)? }));
        }
        out.insert("o".to_string(), Value::Array(converted));
        return Ok(Value::Object(out));
    }
    let shared = ["v", "d", "u", "bi", "e", "r", "ref"];
    if tagged.keys().any(|tag| shared.contains(&tag.as_str())) {
        return Ok(Value::Object(out));
    }
    Err(unsupported())
}

/// Neo: a value a `WebMCP` tool's input or answer cannot carry.
fn unsupported() -> CallError {
    PlaywrightError::error(
        "neo-runtime: a WebMCP tool's input and answer must be plain data: no map, set, typed \
         array or handle",
    )
    .into()
}
