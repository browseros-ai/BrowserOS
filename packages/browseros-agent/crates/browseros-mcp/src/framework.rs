//! Tool registry and execution framework for BrowserOS MCP tools.

use crate::{response::ToolResponse, tools};
use browseros_core::{BrowserSession, CoreError, PageId, WindowId};
use futures_util::future::BoxFuture;
use rmcp::model::{CallToolResult, ContentBlock, JsonObject, Tool, ToolAnnotations};
use schemars::{JsonSchema, generate::SchemaSettings};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::{collections::HashSet, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

pub type OutputFileAccess = Arc<Mutex<HashSet<PathBuf>>>;
pub type ToolHandler = for<'a> fn(
    Value,
    &'a ToolCtx,
    &'a mut ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>>;

#[derive(Debug, Clone, Default)]
pub struct BrowserToolDefaults {
    pub default_window_id: Option<WindowId>,
    pub default_tab_group_id: Option<String>,
}

#[derive(Clone)]
pub struct BrowserToolOptions {
    pub session: Arc<BrowserSession>,
    pub defaults: BrowserToolDefaults,
    pub cancel: CancellationToken,
    pub output_files: OutputFileAccess,
}

#[derive(Clone)]
pub struct ToolCtx {
    pub session: Arc<BrowserSession>,
    pub defaults: BrowserToolDefaults,
    pub cancel: CancellationToken,
    pub output_files: OutputFileAccess,
}

impl ToolCtx {
    #[must_use]
    pub fn new(options: BrowserToolOptions) -> Self {
        Self {
            session: options.session,
            defaults: options.defaults,
            cancel: options.cancel,
            output_files: options.output_files,
        }
    }

    pub fn throw_if_cancelled(&self) -> ToolExecResult<()> {
        if self.cancel.is_cancelled() {
            Err(ToolError::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone)]
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Arc<JsonObject>,
    pub output_schema: Option<Arc<JsonObject>>,
    pub annotations: Option<ToolAnnotations>,
    pub metadata: ToolMetadata,
    pub handler: ToolHandler,
}

impl ToolDef {
    #[must_use]
    pub fn to_mcp_tool(&self) -> Tool {
        let mut tool = Tool::new(self.name, self.description, self.input_schema.clone());
        if let Some(output_schema) = &self.output_schema {
            tool = tool.with_raw_output_schema(output_schema.clone());
        }
        if let Some(annotations) = &self.annotations {
            tool = tool.with_annotations(annotations.clone());
        }
        tool
    }

    #[must_use]
    pub fn call_hooks(&self, raw_args: &Value) -> ToolCallHooks {
        let (filter_tabs_list, capture_new_page, close_page) =
            tabs_action_flags(self.name, raw_args);
        ToolCallHooks {
            accepts_page_arg: self.metadata.accepts_page_arg,
            filter_tabs_list,
            capture_new_page,
            close_page,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ToolMetadata {
    pub accepts_page_arg: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ToolCallHooks {
    pub accepts_page_arg: bool,
    pub filter_tabs_list: bool,
    pub capture_new_page: bool,
    pub close_page: bool,
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: Vec<ContentBlock>,
    pub is_error: bool,
    pub structured_content: Option<Value>,
}

impl ToolResult {
    #[must_use]
    pub fn text(text: impl Into<String>, structured_content: Option<Value>) -> Self {
        Self {
            content: vec![ContentBlock::text(text)],
            is_error: false,
            structured_content,
        }
    }

    #[must_use]
    pub fn image(data: impl Into<String>, mime_type: impl Into<String>, structured: Value) -> Self {
        Self {
            content: vec![ContentBlock::image(data, mime_type)],
            is_error: false,
            structured_content: Some(structured),
        }
    }

    #[must_use]
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![ContentBlock::text(message)],
            is_error: true,
            structured_content: None,
        }
    }

    #[must_use]
    pub fn into_call_tool_result(self) -> CallToolResult {
        let mut result = if self.is_error {
            CallToolResult::error(self.content)
        } else {
            CallToolResult::success(self.content)
        };
        result.structured_content = self.structured_content;
        result.is_error = Some(self.is_error);
        result
    }
}

pub type ToolExecResult<T> = Result<T, ToolError>;

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("cancelled")]
    Cancelled,
    #[error("invalid arguments")]
    InvalidArguments(Vec<ArgIssue>),
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgIssue {
    pub path: String,
    pub message: String,
}

impl ToolError {
    #[must_use]
    pub fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

impl From<CoreError> for ToolError {
    fn from(value: CoreError) -> Self {
        Self::Message(value.to_string())
    }
}

impl From<serde_json::Error> for ToolError {
    fn from(value: serde_json::Error) -> Self {
        Self::Message(value.to_string())
    }
}

impl From<std::io::Error> for ToolError {
    fn from(value: std::io::Error) -> Self {
        Self::Message(value.to_string())
    }
}

pub async fn execute_tool(
    def: &ToolDef,
    raw_args: Value,
    ctx: &ToolCtx,
) -> ToolExecResult<ToolResult> {
    ctx.throw_if_cancelled()?;
    let mut response = ToolResponse::new();
    match (def.handler)(raw_args, ctx, &mut response).await {
        Ok(Some(result)) => response.append_result(result),
        Ok(None) => {}
        Err(ToolError::InvalidArguments(issues)) => {
            return Ok(ToolResult::error(format_invalid_arguments(
                def.name, &issues,
            )));
        }
        Err(ToolError::Cancelled) => return Err(ToolError::Cancelled),
        Err(err) => response.error(format!("{} failed: {err}", def.name)),
    }
    ctx.throw_if_cancelled()?;
    let mut result = response.build_for_session(ctx).await?;
    ctx.throw_if_cancelled()?;

    if let Some(page_id) = result_page_id(&result)
        && let Some(tab_id) = ctx.session.pages.get_tab_id(PageId(page_id)).await
    {
        result.metadata_tab_id = Some(tab_id.0);
    }
    Ok(result.into_tool_result())
}

#[must_use]
pub fn catalog() -> Vec<ToolDef> {
    tools::catalog()
}

pub fn parse_args<T>(raw_args: Value) -> ToolExecResult<T>
where
    T: DeserializeOwned,
{
    match serde_path_to_error::deserialize::<_, T>(raw_args) {
        Ok(value) => Ok(value),
        Err(err) => {
            let path = path_for_issue(err.path());
            Err(ToolError::InvalidArguments(vec![ArgIssue {
                path,
                message: err.inner().to_string(),
            }]))
        }
    }
}

/// Builds the normalized JSON object schema used for MCP tool arguments.
pub fn input_schema<T>() -> Arc<JsonObject>
where
    T: JsonSchema + std::any::Any,
{
    schema_for_mcp_tool::<T>("inputSchema")
}

/// Builds the normalized JSON object schema used for MCP structured output.
pub fn output_schema<T>() -> Arc<JsonObject>
where
    T: JsonSchema + std::any::Any,
{
    schema_for_mcp_tool::<T>("outputSchema")
}

/// Pins schema generation before applying compatibility rewrites for strict MCP clients.
fn schema_for_mcp_tool<T>(purpose: &str) -> Arc<JsonObject>
where
    T: JsonSchema + std::any::Any,
{
    let schema = browseros_schema_settings()
        .into_generator()
        .into_root_schema_for::<T>();
    let Value::Object(mut object) = serde_json::to_value(schema).unwrap_or_else(|err| {
        panic!("invalid BrowserOS MCP {purpose}: schema should serialize: {err}");
    }) else {
        panic!("invalid BrowserOS MCP {purpose}: schema root should be an object");
    };

    match object.get("type") {
        Some(Value::String(schema_type)) if schema_type == "object" => {}
        Some(Value::String(schema_type)) => {
            panic!(
                "invalid BrowserOS MCP {purpose}: root type should be object, got {schema_type}"
            );
        }
        Some(schema_type) => {
            panic!(
                "invalid BrowserOS MCP {purpose}: root type should be a string, got {schema_type}"
            );
        }
        None => {
            panic!("invalid BrowserOS MCP {purpose}: root type is missing");
        }
    }

    object.remove("title");
    object.remove("description");
    normalize_schema_object(object)
}

fn browseros_schema_settings() -> SchemaSettings {
    SchemaSettings::draft2020_12().with(|settings| {
        settings.inline_subschemas = true;
    })
}

/// Rewrites generated JSON Schema into the object-only form expected by strict MCP clients.
fn normalize_schema_object(schema: JsonObject) -> Arc<JsonObject> {
    let mut value = Value::Object(schema);
    normalize_schema_value(&mut value);
    if let Some(path) = first_boolean_path(&value) {
        panic!("unsupported boolean value in BrowserOS MCP schema at {path}");
    }
    match value {
        Value::Object(object) => Arc::new(object),
        _ => panic!("BrowserOS MCP schema root should remain an object"),
    }
}

/// Rewrites boolean schemas and removes boolean default annotations before MCP serialization.
fn normalize_schema_value(value: &mut Value) {
    match value {
        Value::Bool(true) => *value = json!({}),
        Value::Bool(false) => *value = json!({ "not": {} }),
        Value::Array(_) => {}
        Value::Object(object) => {
            if object.get("default").is_some_and(Value::is_boolean) {
                object.remove("default");
            }

            for key in [
                "additionalItems",
                "additionalProperties",
                "contains",
                "contentSchema",
                "else",
                "if",
                "items",
                "not",
                "propertyNames",
                "then",
                "unevaluatedItems",
                "unevaluatedProperties",
            ] {
                if let Some(value) = object.get_mut(key) {
                    normalize_schema_value(value);
                }
            }

            for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
                if let Some(Value::Array(items)) = object.get_mut(key) {
                    for item in items {
                        normalize_schema_value(item);
                    }
                }
            }

            for key in [
                "$defs",
                "definitions",
                "dependentSchemas",
                "patternProperties",
                "properties",
            ] {
                if let Some(Value::Object(schemas)) = object.get_mut(key) {
                    for value in schemas.values_mut() {
                        normalize_schema_value(value);
                    }
                }
            }
        }
        _ => {}
    }
}

fn first_boolean_path(value: &Value) -> Option<String> {
    first_boolean_path_inner(value, "$".to_string())
}

fn first_boolean_path_inner(value: &Value, path: String) -> Option<String> {
    match value {
        Value::Bool(_) => Some(path),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .find_map(|(index, item)| first_boolean_path_inner(item, format!("{path}[{index}]"))),
        Value::Object(object) => object
            .iter()
            .find_map(|(key, value)| first_boolean_path_inner(value, format!("{path}.{key}"))),
        _ => None,
    }
}

#[must_use]
pub fn error_result(message: impl Into<String>) -> ToolResult {
    ToolResult::error(message)
}

#[must_use]
pub fn text_result(text: impl Into<String>, structured: impl Into<Option<Value>>) -> ToolResult {
    ToolResult::text(text, structured.into())
}

#[must_use]
pub fn clamp_timeout(value: Option<f64>, default_ms: u64, max_ms: u64) -> u64 {
    let Some(value) = value else {
        return default_ms;
    };
    if !value.is_finite() || value <= 0.0 {
        return default_ms;
    }
    (value.round() as u64).min(max_ms)
}

pub async fn abortable_delay(ctx: &ToolCtx, duration: std::time::Duration) -> ToolExecResult<()> {
    ctx.throw_if_cancelled()?;
    tokio::select! {
        () = ctx.cancel.cancelled() => Err(ToolError::Cancelled),
        () = tokio::time::sleep(duration) => Ok(()),
    }
}

fn format_invalid_arguments(name: &str, issues: &[ArgIssue]) -> String {
    let detail = issues
        .iter()
        .map(|issue| format!("{}: {}", issue.path, issue.message))
        .collect::<Vec<_>>()
        .join("; ");
    format!("Invalid arguments for {name}: {detail}")
}

fn path_for_issue(path: &serde_path_to_error::Path) -> String {
    let rendered = path.to_string();
    if rendered == "." || rendered.is_empty() {
        "(root)".to_string()
    } else {
        rendered.trim_start_matches('.').to_string()
    }
}

fn result_page_id(result: &crate::response::BuiltToolResponse) -> Option<u32> {
    result
        .structured_content
        .as_ref()
        .and_then(|value| value.get("page"))
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

pub fn merge_structured(target: &mut Option<Value>, value: Value) {
    match (target.as_mut(), value) {
        (Some(Value::Object(target)), Value::Object(source)) => {
            target.extend(source);
        }
        (_, value) => *target = Some(value),
    }
}

fn tabs_action_flags(name: &str, raw_args: &Value) -> (bool, bool, bool) {
    if name != "tabs" {
        return (false, false, false);
    }
    let action = raw_args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("list");
    (action == "list", action == "new", action == "close")
}

#[must_use]
pub fn json_object(value: Value) -> Option<serde_json::Map<String, Value>> {
    match value {
        Value::Object(object) => Some(object),
        _ => None,
    }
}

#[must_use]
pub fn page_json(page: &browseros_core::pages::PageInfo) -> Value {
    let mut value = json!({
        "pageId": page.page_id.0,
        "targetId": page.target_id.as_str(),
        "tabId": page.tab_id.0,
        "url": page.url.as_str(),
        "title": page.title.as_str(),
        "isActive": page.is_active,
        "isLoading": page.is_loading,
        "loadProgress": page.load_progress,
        "isPinned": page.is_pinned,
        "isHidden": page.is_hidden,
    });
    if let Value::Object(object) = &mut value {
        if let Some(window_id) = &page.window_id {
            object.insert("windowId".to_string(), json!(window_id.0));
        }
        if let Some(index) = page.index {
            object.insert("index".to_string(), json!(index));
        }
        if let Some(group_id) = &page.group_id {
            object.insert("groupId".to_string(), json!(group_id));
        }
    }
    value
}
