//! The filesystem tool source (port of `apps/server/src/tools/filesystem`): the seven
//! `filesystem_*` tools rooted at the user's workspace, or — in chat mode or without a workspace
//! — only `filesystem_read`, limited to BrowserOS-generated tool output the lease was granted.
//!
//! Every tool returns TS's `FilesystemToolResult` (`{text, isError?, images?}`) as its wire output
//! and fails the call only when it is cancelled: errors, including path-boundary refusals, become
//! `isError` results the model can read (`executeWithMetrics`). The disk work runs on the blocking
//! pool; a cancelled call returns at once and abandons it (bash kills its child instead).

mod bash;
mod edit;
mod paths;
mod read;
mod search;
mod util;

use super::{ToolCallError, ToolKind, ToolOutput, ToolSource, ToolSourceError, ToolSpec};
use crate::messages::{ToolContentItem, ToolResultOutput};
use serde_json::{Map, Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

/// Which tool-output files a lease may read back; implemented by `apps/server-rust`'s lease.
/// `allows` receives the canonical path of an existing file under the tool-output directory.
pub trait OutputGrants: Send + Sync {
    fn allows(&self, path: &Path) -> bool;
}

pub enum FsScope {
    /// All seven tools, rooted at this directory (TS `buildFilesystemToolSet(workingDir)`).
    Workspace(PathBuf),
    /// Only `filesystem_read`, for absolute tool-output paths the grants allow.
    OutputOnly(Arc<dyn OutputGrants>),
}

pub fn filesystem_tools(scope: FsScope) -> Arc<dyn ToolSource> {
    Arc::new(FilesystemTools {
        scope: Arc::new(scope),
    })
}

struct FilesystemTools {
    scope: Arc<FsScope>,
}

/// What `filesystem_read` resolves against.
enum ReadScope<'a> {
    Workspace(&'a Path),
    OutputOnly(&'a dyn OutputGrants),
}

/// TS `FilesystemToolResult`.
struct FsResult {
    text: String,
    is_error: bool,
    images: Vec<Image>,
}

struct Image {
    data: String,
    mime_type: String,
}

impl FsResult {
    fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: false,
            images: Vec::new(),
        }
    }

    fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: true,
            images: Vec::new(),
        }
    }

    /// The wire output (the raw execute result) and `toModelOutput` of `utils.ts`.
    fn into_output(self) -> ToolOutput {
        let mut ui = Map::new();
        ui.insert("text".to_owned(), Value::from(self.text.clone()));
        if self.is_error {
            ui.insert("isError".to_owned(), Value::Bool(true));
        }
        if !self.images.is_empty() {
            let images = self
                .images
                .iter()
                .map(|image| json!({ "data": image.data, "mimeType": image.mime_type }))
                .collect();
            ui.insert("images".to_owned(), Value::Array(images));
        }
        let model = if self.is_error {
            ToolResultOutput::ErrorText { value: self.text }
        } else if !self.images.is_empty() {
            let mut items = vec![ToolContentItem::Text {
                text: self.text,
                extra: Map::new(),
            }];
            items.extend(
                self.images
                    .into_iter()
                    .map(|image| ToolContentItem::ImageData {
                        data: Value::String(image.data),
                        media_type: image.mime_type,
                        extra: Map::new(),
                    }),
            );
            ToolResultOutput::Content { value: items }
        } else if self.text.is_empty() {
            ToolResultOutput::Text {
                value: "Success".to_owned(),
            }
        } else {
            ToolResultOutput::Text { value: self.text }
        };
        ToolOutput {
            ui: Value::Object(ui),
            model,
        }
    }
}

/// Typed access to a validated input object.
struct Input(Value);

impl Input {
    fn str(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(Value::as_str)
    }

    fn number(&self, key: &str) -> Option<f64> {
        self.0.get(key).and_then(Value::as_f64)
    }

    /// `params.x` used as a boolean condition.
    fn flag(&self, key: &str) -> bool {
        self.0.get(key).and_then(Value::as_bool).unwrap_or(false)
    }
}

#[async_trait::async_trait]
impl ToolSource for FilesystemTools {
    fn label(&self) -> &str {
        "filesystem"
    }

    async fn tools(&self) -> Result<Vec<ToolSpec>, ToolSourceError> {
        Ok(match self.scope.as_ref() {
            FsScope::Workspace(_) => workspace_specs(),
            FsScope::OutputOnly(_) => vec![output_read_spec()],
        })
    }

    async fn call(
        &self,
        name: &str,
        input: Value,
        cancel: CancellationToken,
    ) -> Result<ToolOutput, ToolCallError> {
        let input = Input(input);
        let result = match (self.scope.as_ref(), name) {
            (FsScope::Workspace(cwd), "filesystem_bash") => {
                let result = bash::run(cwd, &input, cancel.clone()).await;
                if cancel.is_cancelled() {
                    return Err(ToolCallError("Tool call cancelled".to_owned()));
                }
                result
            }
            (FsScope::Workspace(_), "filesystem_read" | "filesystem_write" | "filesystem_edit")
            | (FsScope::Workspace(_), "filesystem_grep" | "filesystem_find" | "filesystem_ls")
            | (FsScope::OutputOnly(_), "filesystem_read") => {
                let scope = Arc::clone(&self.scope);
                let name = name.to_owned();
                let work = tokio::task::spawn_blocking(move || run_blocking(&scope, &name, &input));
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => return Err(ToolCallError("Tool call cancelled".to_owned())),
                    joined = work => joined.unwrap_or_else(|error| Err(error.to_string())),
                }
            }
            (_, other) => return Err(ToolCallError(format!("Unknown filesystem tool: {other}"))),
        };
        let result = result.unwrap_or_else(FsResult::error);
        if result.is_error {
            tracing::debug!(tool = name, error = %result.text, "filesystem tool returned an error");
        }
        Ok(result.into_output())
    }

    async fn close(&self) {}
}

fn run_blocking(scope: &FsScope, name: &str, input: &Input) -> Result<FsResult, String> {
    match scope {
        FsScope::Workspace(cwd) => match name {
            "filesystem_read" => read::run(&ReadScope::Workspace(cwd), input),
            "filesystem_write" => edit::write(cwd, input),
            "filesystem_edit" => edit::edit(cwd, input),
            "filesystem_grep" => search::grep(cwd, input),
            "filesystem_find" => search::find(cwd, input),
            "filesystem_ls" => search::ls(cwd, input),
            other => Err(format!("Unknown filesystem tool: {other}")),
        },
        FsScope::OutputOnly(grants) => read::run(&ReadScope::OutputOnly(grants.as_ref()), input),
    }
}

const SCHEMA_DRAFT: &str = "http://json-schema.org/draft-07/schema#";
const READ_DESCRIPTION_TAIL: &str = "Returns text content with line numbers, or image data for image files. Text reads are limited to 500 lines and 15000 characters per call. Use offset and limit to paginate through large files.";

fn spec(name: &str, description: String, input_schema: Value) -> ToolSpec {
    ToolSpec {
        name: name.to_owned(),
        description,
        input_schema,
        kind: ToolKind::Static,
    }
}

/// `read.ts` schema; only the `path` description differs between scopes.
fn read_schema(path_description: &str) -> Value {
    json!({
        "$schema": SCHEMA_DRAFT,
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": path_description },
            "offset": { "description": "Starting line number (1-indexed)", "type": "number" },
            "limit": {
                "description": "Maximum number of lines to read",
                "type": "integer",
                "exclusiveMinimum": 0,
                "maximum": 9007199254740991_i64,
            },
        },
        "required": ["path"],
        "additionalProperties": false,
    })
}

fn output_read_spec() -> ToolSpec {
    spec(
        "filesystem_read",
        format!(
            "Read BrowserOS-generated tool output files by absolute path. {READ_DESCRIPTION_TAIL}"
        ),
        read_schema("Absolute BrowserOS-generated tool output path returned by a browser tool"),
    )
}

/// Descriptions and schemas exactly as `zod/v4` + the AI SDK render them for the TS tools
/// (captured from `asSchema(tool.inputSchema).jsonSchema`; `tests/tools_fs.rs` holds the capture).
fn workspace_specs() -> Vec<ToolSpec> {
    let ws_path = "File path relative to the selected workspace";
    vec![
        spec(
            "filesystem_read",
            format!("Read a file from the filesystem. {READ_DESCRIPTION_TAIL}"),
            read_schema(ws_path),
        ),
        spec(
            "filesystem_write",
            "Create or overwrite a file. Automatically creates parent directories if they don't exist. Use this to create new files or completely replace file contents.".to_owned(),
            json!({
                "$schema": SCHEMA_DRAFT,
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": ws_path },
                    "content": { "type": "string", "description": "Complete file content to write" },
                },
                "required": ["path", "content"],
                "additionalProperties": false,
            }),
        ),
        spec(
            "filesystem_edit",
            "Make a targeted edit to a file by replacing an exact string match. The old_string must match exactly one location in the file. If exact match fails, a whitespace-tolerant match is attempted.".to_owned(),
            json!({
                "$schema": SCHEMA_DRAFT,
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": ws_path },
                    "old_string": { "type": "string", "description": "Exact text to find in the file" },
                    "new_string": { "type": "string", "description": "Replacement text" },
                },
                "required": ["path", "old_string", "new_string"],
                "additionalProperties": false,
            }),
        ),
        spec(
            "filesystem_bash",
            "Execute a shell command and return its output. Commands run in a shell (sh/bash on Unix, cmd on Windows). Output is truncated to the last 2000 lines if too large.".to_owned(),
            json!({
                "$schema": SCHEMA_DRAFT,
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Shell command to execute" },
                    "timeout": { "description": "Timeout in seconds (default: 120)", "type": "number" },
                },
                "required": ["command"],
                "additionalProperties": false,
            }),
        ),
        spec(
            "filesystem_grep",
            "Search file contents using a regular expression. Returns matching lines with file paths and line numbers. Searches recursively, skipping binary files and common build directories (node_modules, .git, dist, etc.).".to_owned(),
            json!({
                "$schema": SCHEMA_DRAFT,
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Search pattern (regex by default, or literal string if literal=true)",
                    },
                    "path": {
                        "description": "Directory or file to search relative to the selected workspace",
                        "type": "string",
                    },
                    "glob": {
                        "description": "Filter files by glob pattern (e.g., \"*.ts\", \"*.{js,jsx}\")",
                        "type": "string",
                    },
                    "ignore_case": { "description": "Case-insensitive search", "type": "boolean" },
                    "literal": {
                        "description": "Treat pattern as a literal string, not regex",
                        "type": "boolean",
                    },
                    "context": {
                        "description": "Lines of context before and after each match",
                        "type": "number",
                    },
                    "limit": {
                        "description": "Maximum matches to return (default: 100)",
                        "type": "number",
                    },
                },
                "required": ["pattern"],
                "additionalProperties": false,
            }),
        ),
        spec(
            "filesystem_find",
            "Find files matching a glob pattern. Searches recursively, skipping common build directories (node_modules, .git, dist, etc.). Returns relative file paths.".to_owned(),
            json!({
                "$schema": SCHEMA_DRAFT,
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Glob pattern (e.g., \"*.ts\", \"**/*.json\", \"src/**/*.test.ts\")",
                    },
                    "path": {
                        "description": "Directory to search relative to the selected workspace",
                        "type": "string",
                    },
                    "limit": { "description": "Maximum results (default: 1000)", "type": "number" },
                },
                "required": ["pattern"],
                "additionalProperties": false,
            }),
        ),
        spec(
            "filesystem_ls",
            "List directory contents. Shows directories (with trailing /) first, then files with sizes. Entries are sorted alphabetically.".to_owned(),
            json!({
                "$schema": SCHEMA_DRAFT,
                "type": "object",
                "properties": {
                    "path": {
                        "description": "Directory path relative to the selected workspace",
                        "type": "string",
                    },
                    "limit": {
                        "description": "Maximum entries to return (default: 500)",
                        "type": "number",
                    },
                },
                "additionalProperties": false,
            }),
        ),
    ]
}
