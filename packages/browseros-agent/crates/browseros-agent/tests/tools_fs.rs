//! The local tool sources: the filesystem tools (port of `apps/server/src/tools/filesystem`, path
//! boundary first) and the nudge tools. Tool definitions are checked against a capture of what the
//! TS server sends the model; behaviour against the TS texts the model reads.

use browseros_agent::{
    messages::{ToolContentItem, ToolResultOutput},
    tools::{FsScope, OutputGrants, ToolOutput, ToolSource, filesystem_tools, nudge_tools},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// `asSchema(tool.inputSchema).jsonSchema` and `description` of every TS local tool, captured
/// from `apps/server` (`buildFilesystemToolSet`, the no-workspace `createReadTool`,
/// `buildNudgeToolSet`) with Bun. Keys are `<scope>:<tool>`.
const TS_CAPTURE: &str = r#"
{
 "ws:filesystem_read": {
  "description": "Read a file from the filesystem. Returns text content with line numbers, or image data for image files. Text reads are limited to 500 lines and 15000 characters per call. Use offset and limit to paginate through large files.",
  "inputSchema": {
   "$schema": "http://json-schema.org/draft-07/schema#",
   "type": "object",
   "properties": {
    "path": {
     "type": "string",
     "description": "File path relative to the selected workspace"
    },
    "offset": {
     "description": "Starting line number (1-indexed)",
     "type": "number"
    },
    "limit": {
     "description": "Maximum number of lines to read",
     "type": "integer",
     "exclusiveMinimum": 0,
     "maximum": 9007199254740991
    }
   },
   "required": [
    "path"
   ],
   "additionalProperties": false
  }
 },
 "ws:filesystem_write": {
  "description": "Create or overwrite a file. Automatically creates parent directories if they don't exist. Use this to create new files or completely replace file contents.",
  "inputSchema": {
   "$schema": "http://json-schema.org/draft-07/schema#",
   "type": "object",
   "properties": {
    "path": {
     "type": "string",
     "description": "File path relative to the selected workspace"
    },
    "content": {
     "type": "string",
     "description": "Complete file content to write"
    }
   },
   "required": [
    "path",
    "content"
   ],
   "additionalProperties": false
  }
 },
 "ws:filesystem_edit": {
  "description": "Make a targeted edit to a file by replacing an exact string match. The old_string must match exactly one location in the file. If exact match fails, a whitespace-tolerant match is attempted.",
  "inputSchema": {
   "$schema": "http://json-schema.org/draft-07/schema#",
   "type": "object",
   "properties": {
    "path": {
     "type": "string",
     "description": "File path relative to the selected workspace"
    },
    "old_string": {
     "type": "string",
     "description": "Exact text to find in the file"
    },
    "new_string": {
     "type": "string",
     "description": "Replacement text"
    }
   },
   "required": [
    "path",
    "old_string",
    "new_string"
   ],
   "additionalProperties": false
  }
 },
 "ws:filesystem_bash": {
  "description": "Execute a shell command and return its output. Commands run in a shell (sh/bash on Unix, cmd on Windows). Output is truncated to the last 2000 lines if too large.",
  "inputSchema": {
   "$schema": "http://json-schema.org/draft-07/schema#",
   "type": "object",
   "properties": {
    "command": {
     "type": "string",
     "description": "Shell command to execute"
    },
    "timeout": {
     "description": "Timeout in seconds (default: 120)",
     "type": "number"
    }
   },
   "required": [
    "command"
   ],
   "additionalProperties": false
  }
 },
 "ws:filesystem_grep": {
  "description": "Search file contents using a regular expression. Returns matching lines with file paths and line numbers. Searches recursively, skipping binary files and common build directories (node_modules, .git, dist, etc.).",
  "inputSchema": {
   "$schema": "http://json-schema.org/draft-07/schema#",
   "type": "object",
   "properties": {
    "pattern": {
     "type": "string",
     "description": "Search pattern (regex by default, or literal string if literal=true)"
    },
    "path": {
     "description": "Directory or file to search relative to the selected workspace",
     "type": "string"
    },
    "glob": {
     "description": "Filter files by glob pattern (e.g., \"*.ts\", \"*.{js,jsx}\")",
     "type": "string"
    },
    "ignore_case": {
     "description": "Case-insensitive search",
     "type": "boolean"
    },
    "literal": {
     "description": "Treat pattern as a literal string, not regex",
     "type": "boolean"
    },
    "context": {
     "description": "Lines of context before and after each match",
     "type": "number"
    },
    "limit": {
     "description": "Maximum matches to return (default: 100)",
     "type": "number"
    }
   },
   "required": [
    "pattern"
   ],
   "additionalProperties": false
  }
 },
 "ws:filesystem_find": {
  "description": "Find files matching a glob pattern. Searches recursively, skipping common build directories (node_modules, .git, dist, etc.). Returns relative file paths.",
  "inputSchema": {
   "$schema": "http://json-schema.org/draft-07/schema#",
   "type": "object",
   "properties": {
    "pattern": {
     "type": "string",
     "description": "Glob pattern (e.g., \"*.ts\", \"**/*.json\", \"src/**/*.test.ts\")"
    },
    "path": {
     "description": "Directory to search relative to the selected workspace",
     "type": "string"
    },
    "limit": {
     "description": "Maximum results (default: 1000)",
     "type": "number"
    }
   },
   "required": [
    "pattern"
   ],
   "additionalProperties": false
  }
 },
 "ws:filesystem_ls": {
  "description": "List directory contents. Shows directories (with trailing /) first, then files with sizes. Entries are sorted alphabetically.",
  "inputSchema": {
   "$schema": "http://json-schema.org/draft-07/schema#",
   "type": "object",
   "properties": {
    "path": {
     "description": "Directory path relative to the selected workspace",
     "type": "string"
    },
    "limit": {
     "description": "Maximum entries to return (default: 500)",
     "type": "number"
    }
   },
   "additionalProperties": false
  }
 },
 "out:filesystem_read": {
  "description": "Read BrowserOS-generated tool output files by absolute path. Returns text content with line numbers, or image data for image files. Text reads are limited to 500 lines and 15000 characters per call. Use offset and limit to paginate through large files.",
  "inputSchema": {
   "$schema": "http://json-schema.org/draft-07/schema#",
   "type": "object",
   "properties": {
    "path": {
     "type": "string",
     "description": "Absolute BrowserOS-generated tool output path returned by a browser tool"
    },
    "offset": {
     "description": "Starting line number (1-indexed)",
     "type": "number"
    },
    "limit": {
     "description": "Maximum number of lines to read",
     "type": "integer",
     "exclusiveMinimum": 0,
     "maximum": 9007199254740991
    }
   },
   "required": [
    "path"
   ],
   "additionalProperties": false
  }
 },
 "nudge:suggest_schedule": {
  "description": "Call this to suggest scheduling a task. Use in two cases: (1) MANDATORY after completing a task that could run on a recurring schedule (news, monitoring, reports, price tracking, data gathering). (2) Immediately when the user explicitly asks to schedule, automate, or repeat the current task - do NOT ask for clarification, infer all parameters from context. Do NOT call if the task requires real-time user interaction.",
  "inputSchema": {
   "$schema": "http://json-schema.org/draft-07/schema#",
   "type": "object",
   "properties": {
    "query": {
     "type": "string",
     "description": "The original user query to schedule"
    },
    "suggestedName": {
     "type": "string",
     "description": "A short, descriptive name for the scheduled task (e.g. \"Morning News Briefing\")"
    },
    "scheduleType": {
     "type": "string",
     "enum": [
      "daily",
      "hourly"
     ],
     "description": "How often the task should run"
    },
    "scheduleTime": {
     "description": "Suggested time for daily tasks in HH:MM format (e.g. \"09:00\").",
     "type": "string"
    }
   },
   "required": [
    "query",
    "suggestedName",
    "scheduleType"
   ],
   "additionalProperties": false
  }
 },
 "nudge:suggest_app_connection": {
  "description": "BLOCKING DECISION - Call when the user's request relates to a Connect Apps service but you don't have MCP tools for it. Your response must contain ONLY this tool call with zero text. The appName must be one of: Gmail, Google Calendar, Google Docs, Google Drive, Google Sheets, Slack, LinkedIn, Notion, Airtable, Confluence, GitHub, GitLab, Linear, Jira, Figma, Salesforce, ClickUp, Asana, Monday, Microsoft Teams, Outlook Mail, Outlook Calendar, Supabase, Vercel, Postman, Stripe, Cloudflare, Brave Search, Mem0, Dropbox, OneDrive, WordPress, YouTube, Box, HubSpot, PostHog, Mixpanel, Discord, WhatsApp, Shopify, Cal.com, Resend, Google Forms, Zendesk, Intercom.",
  "inputSchema": {
   "$schema": "http://json-schema.org/draft-07/schema#",
   "type": "object",
   "properties": {
    "appName": {
     "type": "string",
     "description": "The name of the app to connect (must match a supported app name exactly)"
    },
    "reason": {
     "type": "string",
     "description": "A brief, user-friendly explanation of why connecting this app would help"
    }
   },
   "required": [
    "appName",
    "reason"
   ],
   "additionalProperties": false
  }
 }
}"#;

fn capture() -> Result<BTreeMap<String, Value>, serde_json::Error> {
    serde_json::from_str(TS_CAPTURE)
}

struct Workspace {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Workspace {
    fn new() -> Result<Self, std::io::Error> {
        let dir = tempfile::tempdir()?;
        let root = std::fs::canonicalize(dir.path().join("."))?.join("ws");
        std::fs::create_dir_all(&root)?;
        Ok(Self { _dir: dir, root })
    }

    fn write(&self, path: &str, content: impl AsRef<[u8]>) -> Result<(), std::io::Error> {
        let path = self.root.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)
    }

    fn tools(&self) -> Arc<dyn ToolSource> {
        filesystem_tools(FsScope::Workspace(self.root.clone()))
    }

    /// A sibling of the workspace, i.e. outside it.
    fn outside(&self) -> PathBuf {
        self.root.with_file_name("outside")
    }
}

async fn call(source: &Arc<dyn ToolSource>, name: &str, input: Value) -> Result<ToolOutput, Box<dyn std::error::Error>> {
    Ok(source.call(name, input, CancellationToken::new()).await?)
}

/// The `{text, isError?}` wire output as (text, is_error).
fn result(output: &ToolOutput) -> (&str, bool) {
    (
        output.ui.get("text").and_then(Value::as_str).unwrap_or_default(),
        output.ui.get("isError").and_then(Value::as_bool).unwrap_or(false),
    )
}

async fn text_of(source: &Arc<dyn ToolSource>, name: &str, input: Value) -> Result<(String, bool), Box<dyn std::error::Error>> {
    let output = call(source, name, input).await?;
    let (text, is_error) = result(&output);
    Ok((text.to_owned(), is_error))
}

#[tokio::test]
async fn definitions_match_the_ts_tools() -> TestResult {
    let capture = capture()?;
    let ws = Workspace::new()?;
    let specs = ws.tools().tools().await?;
    let names: Vec<&str> = specs.iter().map(|spec| spec.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "filesystem_read",
            "filesystem_write",
            "filesystem_edit",
            "filesystem_bash",
            "filesystem_grep",
            "filesystem_find",
            "filesystem_ls"
        ]
    );
    for spec in &specs {
        let expected = &capture[&format!("ws:{}", spec.name)];
        assert_eq!(spec.description, expected["description"], "{}", spec.name);
        assert_eq!(spec.input_schema, expected["inputSchema"], "{}", spec.name);
    }

    let output_only = filesystem_tools(FsScope::OutputOnly(Arc::new(NoGrants))).tools().await?;
    assert_eq!(output_only.len(), 1);
    assert_eq!(output_only[0].description, capture["out:filesystem_read"]["description"]);
    assert_eq!(output_only[0].input_schema, capture["out:filesystem_read"]["inputSchema"]);

    let app_description = capture["nudge:suggest_app_connection"]["description"]
        .as_str()
        .unwrap_or_default();
    let apps: Vec<String> = app_description
        .split("must be one of: ")
        .nth(1)
        .unwrap_or_default()
        .trim_end_matches('.')
        .split(", ")
        .map(str::to_owned)
        .collect();
    assert_eq!(apps.len(), 45);
    for spec in nudge_tools(apps).tools().await? {
        let expected = &capture[&format!("nudge:{}", spec.name)];
        assert_eq!(spec.description, expected["description"], "{}", spec.name);
        assert_eq!(spec.input_schema, expected["inputSchema"], "{}", spec.name);
    }
    Ok(())
}

/// The sentinel JSON is the app's card contract: exact text, exact key order.
#[tokio::test]
async fn nudges_return_ts_sentinels() -> TestResult {
    let nudges = nudge_tools(vec!["Gmail".to_owned()]);
    let output = call(
        &nudges,
        "suggest_schedule",
        json!({ "query": "news", "suggestedName": "Morning", "scheduleType": "daily" }),
    )
    .await?;
    let sentinel = r#"{"type":"schedule_suggestion","query":"news","suggestedName":"Morning","scheduleType":"daily","scheduleTime":"09:00"}"#;
    assert_eq!(
        output.ui,
        json!({ "content": [{ "type": "text", "text": sentinel }], "isError": false })
    );
    assert_eq!(output.model, ToolResultOutput::Text { value: sentinel.to_owned() });

    let output = call(
        &nudges,
        "suggest_app_connection",
        json!({ "appName": "Gmail", "reason": "read mail" }),
    )
    .await?;
    assert_eq!(
        output.model,
        ToolResultOutput::Text {
            value: r#"{"type":"app_connection","appName":"Gmail","reason":"read mail"}"#.to_owned()
        }
    );
    Ok(())
}

#[tokio::test]
async fn paths_cannot_leave_the_workspace() -> TestResult {
    let ws = Workspace::new()?;
    let tools = ws.tools();
    std::fs::write(ws.outside(), "secret")?;
    ws.write("inside.txt", "ok")?;

    for (path, error) in [
        ("../outside", "Path is outside the selected workspace."),
        ("sub/../../outside", "Path is outside the selected workspace."),
        ("/etc/hosts", "Path must be relative to the selected workspace."),
        ("C:\\Windows\\win.ini", "Path must be relative to the selected workspace."),
        ("\\\\server\\share", "Path must be relative to the selected workspace."),
    ] {
        assert_eq!(
            text_of(&tools, "filesystem_read", json!({ "path": path })).await?,
            (error.to_owned(), true),
            "read {path}"
        );
        assert_eq!(
            text_of(&tools, "filesystem_write", json!({ "path": path, "content": "x" })).await?,
            (error.to_owned(), true),
            "write {path}"
        );
    }
    let (missing, is_error) = text_of(&tools, "filesystem_read", json!({ "path": "nope.txt" })).await?;
    assert!(is_error);
    assert_eq!(
        missing,
        format!("ENOENT: no such file or directory, realpath '{}'", ws.root.join("nope.txt").display())
    );
    assert_eq!(std::fs::read_to_string(ws.outside())?, "secret");
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn symlinks_cannot_leave_the_workspace() -> TestResult {
    use std::os::unix::fs::symlink;
    let ws = Workspace::new()?;
    let tools = ws.tools();
    let outside_dir = ws.root.with_file_name("outside-dir");
    std::fs::create_dir_all(&outside_dir)?;
    std::fs::write(outside_dir.join("secret.txt"), "secret")?;
    symlink(outside_dir.join("secret.txt"), ws.root.join("link.txt"))?;
    symlink(&outside_dir, ws.root.join("linkdir"))?;
    symlink(ws.root.with_file_name("dangling-target"), ws.root.join("dangling"))?;
    ws.write("real.txt", "real")?;
    symlink(ws.root.join("real.txt"), ws.root.join("inner-link.txt"))?;

    let outside = ("Path is outside the selected workspace.".to_owned(), true);
    assert_eq!(text_of(&tools, "filesystem_read", json!({ "path": "link.txt" })).await?, outside);
    assert_eq!(text_of(&tools, "filesystem_read", json!({ "path": "linkdir/secret.txt" })).await?, outside);
    assert_eq!(
        text_of(&tools, "filesystem_write", json!({ "path": "linkdir/new.txt", "content": "x" })).await?,
        outside
    );
    assert_eq!(
        text_of(&tools, "filesystem_write", json!({ "path": "link.txt", "content": "x" })).await?,
        outside
    );
    // A dangling link would let `writeFile` create its target wherever it points.
    assert_eq!(
        text_of(&tools, "filesystem_write", json!({ "path": "dangling", "content": "x" })).await?,
        outside
    );
    assert!(!ws.root.with_file_name("dangling-target").exists());
    assert!(!outside_dir.join("new.txt").exists());
    assert_eq!(std::fs::read_to_string(outside_dir.join("secret.txt"))?, "secret");

    // Links that stay inside are followed.
    assert_eq!(text_of(&tools, "filesystem_read", json!({ "path": "inner-link.txt" })).await?, ("1 | real".to_owned(), false));

    // Listing and searching hide what escapes.
    let (listing, _) = text_of(&tools, "filesystem_ls", json!({})).await?;
    assert_eq!(listing, "inner-link.txt (4B)\nreal.txt (4B)");
    let (found, _) = text_of(&tools, "filesystem_find", json!({ "pattern": "*" })).await?;
    assert_eq!(found, "inner-link.txt\nreal.txt");
    let (grepped, _) = text_of(&tools, "filesystem_grep", json!({ "pattern": "secret" })).await?;
    assert_eq!(grepped, "No matches found.");
    Ok(())
}

#[tokio::test]
async fn read_numbers_lines_and_paginates() -> TestResult {
    let ws = Workspace::new()?;
    let tools = ws.tools();
    let lines: Vec<String> = (1..=10).map(|n| format!("line {n}")).collect();
    ws.write("ten.txt", lines.join("\n"))?;

    let (all, _) = text_of(&tools, "filesystem_read", json!({ "path": "ten.txt" })).await?;
    assert!(all.starts_with(" 1 | line 1\n 2 | line 2"), "{all}");
    assert!(all.ends_with("10 | line 10"));

    let (page, _) = text_of(&tools, "filesystem_read", json!({ "path": "ten.txt", "offset": 3, "limit": 2 })).await?;
    assert_eq!(
        page,
        "3 | line 3\n4 | line 4\n\n(6 more lines in file. Use offset=5 to continue reading.)"
    );
    let (tail, _) = text_of(&tools, "filesystem_read", json!({ "path": "ten.txt", "offset": 9 })).await?;
    assert_eq!(tail, " 9 | line 9\n10 | line 10\n\n(Showing lines 9-10 of 10)");
    let (beyond, is_error) = text_of(&tools, "filesystem_read", json!({ "path": "ten.txt", "offset": 11 })).await?;
    assert_eq!((beyond.as_str(), is_error), ("File has 10 lines. Offset 11 is beyond end of file.", false));
    let output = call(&tools, "filesystem_read", json!({ "path": "ten.txt", "limit": 1 })).await?;
    assert_eq!(
        output.model,
        ToolResultOutput::Text {
            value: "1 | line 1\n\n(9 more lines in file. Use offset=2 to continue reading.)".to_owned()
        }
    );
    Ok(())
}

#[tokio::test]
async fn read_enforces_line_and_character_limits() -> TestResult {
    let ws = Workspace::new()?;
    let tools = ws.tools();
    let many: Vec<String> = (1..=600).map(|n| n.to_string()).collect();
    ws.write("many.txt", many.join("\n"))?;
    ws.write("wide.txt", "x".repeat(20_000))?;

    let (error, is_error) = text_of(&tools, "filesystem_read", json!({ "path": "many.txt" })).await?;
    assert!(is_error);
    assert_eq!(
        error,
        "Requested lines 1-600 exceed the 500-line limit for filesystem_read. Retry with offset and limit=500 or smaller."
    );
    let (error, _) = text_of(&tools, "filesystem_read", json!({ "path": "many.txt", "limit": 501 })).await?;
    assert_eq!(error, "filesystem_read accepts at most 500 lines per call. Retry with a smaller limit.");
    let (error, _) = text_of(&tools, "filesystem_read", json!({ "path": "wide.txt" })).await?;
    assert_eq!(
        error,
        "Requested lines 1-1 produce 20004 characters in the response, above the 15000-character limit for filesystem_read. Retry with a smaller limit or a later offset."
    );
    let output = call(&tools, "filesystem_read", json!({ "path": "many.txt" })).await?;
    assert_eq!(output.model, ToolResultOutput::ErrorText { value: error_text(&output) });
    Ok(())
}

fn error_text(output: &ToolOutput) -> String {
    result(output).0.to_owned()
}

#[tokio::test]
async fn read_returns_images_as_media() -> TestResult {
    let ws = Workspace::new()?;
    ws.write("pic.PNG", [0x89, b'P', b'N', b'G'])?;
    let output = call(&ws.tools(), "filesystem_read", json!({ "path": "pic.PNG" })).await?;
    assert_eq!(
        output.ui,
        json!({
            "text": "Image: pic.PNG (4 bytes)",
            "images": [{ "data": "iVBORw==", "mimeType": "image/png" }],
        })
    );
    assert_eq!(
        output.model,
        ToolResultOutput::Content {
            value: vec![
                ToolContentItem::Text { text: "Image: pic.PNG (4 bytes)".to_owned() },
                ToolContentItem::Media { data: "iVBORw==".to_owned(), media_type: "image/png".to_owned() },
            ]
        }
    );
    Ok(())
}

#[tokio::test]
async fn write_creates_parent_directories() -> TestResult {
    let ws = Workspace::new()?;
    let tools = ws.tools();
    assert_eq!(
        text_of(&tools, "filesystem_write", json!({ "path": "a/b/c.txt", "content": "héllo" })).await?,
        ("Wrote 6 bytes to a/b/c.txt".to_owned(), false)
    );
    assert_eq!(std::fs::read_to_string(ws.root.join("a/b/c.txt"))?, "héllo");
    Ok(())
}

#[tokio::test]
async fn edit_replaces_one_match_and_keeps_bom_and_line_endings() -> TestResult {
    let ws = Workspace::new()?;
    let tools = ws.tools();
    ws.write("crlf.txt", "\u{feff}alpha\r\n  beta\r\ngamma\r\n")?;

    let (text, is_error) = text_of(
        &tools,
        "filesystem_edit",
        json!({ "path": "crlf.txt", "old_string": "beta", "new_string": "BETA\nmore" }),
    )
    .await?;
    assert!(!is_error, "{text}");
    assert_eq!(text, "Applied edit to crlf.txt\n\n- beta\n+ BETA\n+ more");
    assert_eq!(
        std::fs::read_to_string(ws.root.join("crlf.txt"))?,
        "\u{feff}alpha\r\n  BETA\r\nmore\r\ngamma\r\n"
    );

    // Whitespace-tolerant fallback: indentation differs line by line.
    let (_, is_error) = text_of(
        &tools,
        "filesystem_edit",
        json!({ "path": "crlf.txt", "old_string": "alpha\nBETA", "new_string": "one" }),
    )
    .await?;
    assert!(!is_error);
    assert_eq!(std::fs::read_to_string(ws.root.join("crlf.txt"))?, "\u{feff}one\r\nmore\r\ngamma\r\n");

    // `String#replace` substitution patterns apply, as in TS.
    ws.write("dollar.txt", "price")?;
    text_of(&tools, "filesystem_edit", json!({ "path": "dollar.txt", "old_string": "price", "new_string": "$& $$5" })).await?;
    assert_eq!(std::fs::read_to_string(ws.root.join("dollar.txt"))?, "price $5");

    ws.write("dup.txt", "x\nx\n")?;
    for (old, new, expected) in [
        ("x", "y", "Found 2 exact occurrences of old_string. Add more surrounding context to make the match unique."),
        ("zzz", "y", "old_string not found in file (exact and whitespace-tolerant match both failed)."),
        ("x", "x", "old_string and new_string are identical — no change needed."),
    ] {
        assert_eq!(
            text_of(&tools, "filesystem_edit", json!({ "path": "dup.txt", "old_string": old, "new_string": new })).await?,
            (expected.to_owned(), true)
        );
    }
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn bash_runs_in_the_workspace_with_exit_codes_and_timeouts() -> TestResult {
    let ws = Workspace::new()?;
    let tools = ws.tools();
    let (pwd, _) = text_of(&tools, "filesystem_bash", json!({ "command": "pwd" })).await?;
    assert_eq!(PathBuf::from(pwd.trim()), ws.root);

    assert_eq!(
        text_of(&tools, "filesystem_bash", json!({ "command": "echo out; echo err >&2; exit 3" })).await?,
        ("out\n\nerr\n\n\n[Exit code: 3]".to_owned(), true)
    );
    assert_eq!(
        text_of(&tools, "filesystem_bash", json!({ "command": "true" })).await?,
        ("(no output)".to_owned(), false)
    );
    let (timed_out, is_error) =
        text_of(&tools, "filesystem_bash", json!({ "command": "echo started; sleep 5", "timeout": 0.5 })).await?;
    assert!(is_error);
    assert!(timed_out.starts_with("Command timed out after 0.5s\n\n"), "{timed_out}");

    let cancel = CancellationToken::new();
    let stopper = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        stopper.cancel();
    });
    let started = std::time::Instant::now();
    let cancelled = tools.call("filesystem_bash", json!({ "command": "sleep 5" }), cancel).await;
    assert!(cancelled.is_err());
    assert!(started.elapsed() < Duration::from_secs(3));
    Ok(())
}

#[tokio::test]
async fn grep_matches_with_context_filters_and_limits() -> TestResult {
    let ws = Workspace::new()?;
    let tools = ws.tools();
    ws.write("src/a.ts", "foo\nbar\nbaz\nqux\nFoo bar")?;
    ws.write("src/b.js", "nothing here")?;
    ws.write("node_modules/dep/x.ts", "foo")?;
    ws.write("logo.png", "foo")?;

    let grep = |input: Value| text_of(&tools, "filesystem_grep", input);
    assert_eq!(grep(json!({ "pattern": "foo" })).await?, ("src/a.ts:1:foo".to_owned(), false));
    assert_eq!(
        grep(json!({ "pattern": "FOO", "ignore_case": true })).await?.0,
        "src/a.ts:1:foo\nsrc/a.ts:5:Foo bar"
    );
    assert_eq!(
        grep(json!({ "pattern": "foo", "ignore_case": true, "context": 1 })).await?.0,
        "src/a.ts:1:foo\nsrc/a.ts-2-bar\n--\nsrc/a.ts-4-qux\nsrc/a.ts:5:Foo bar"
    );
    assert_eq!(
        grep(json!({ "pattern": "a", "limit": 1 })).await?.0,
        "src/a.ts:2:bar\n\n(Showing first 1 matches. Use limit=2 to see more.)"
    );
    assert_eq!(grep(json!({ "pattern": "foo", "glob": "*.js" })).await?.0, "No matches found.");
    assert_eq!(grep(json!({ "pattern": "Foo b", "literal": true })).await?.0, "src/a.ts:5:Foo bar");
    assert_eq!(grep(json!({ "pattern": "baz", "path": "src/a.ts" })).await?.0, "src/a.ts:3:baz");
    assert_eq!(
        grep(json!({ "pattern": "(" })).await?,
        ("Invalid regex pattern: (".to_owned(), true)
    );
    Ok(())
}

#[tokio::test]
async fn find_lists_sorted_relative_paths() -> TestResult {
    let ws = Workspace::new()?;
    let tools = ws.tools();
    ws.write("src/z.ts", "")?;
    ws.write("src/a.ts", "")?;
    ws.write("b.js", "")?;
    ws.write("dist/out.ts", "")?;

    let find = |input: Value| text_of(&tools, "filesystem_find", input);
    assert_eq!(find(json!({ "pattern": "*.ts" })).await?.0, "src/a.ts\nsrc/z.ts");
    assert_eq!(find(json!({ "pattern": "**/*.{ts,js}" })).await?.0, "b.js\nsrc/a.ts\nsrc/z.ts");
    assert_eq!(find(json!({ "pattern": "*.ts", "path": "src" })).await?.0, "a.ts\nz.ts");
    let (limited, _) = find(json!({ "pattern": "*.ts", "limit": 1 })).await?;
    assert!(limited.ends_with("\n\n(Showing first 1 results. Use limit=2 to see more.)"), "{limited}");
    assert_eq!(find(json!({ "pattern": "*.rs" })).await?.0, "No files matching \"*.rs\" found.");
    Ok(())
}

#[tokio::test]
async fn ls_lists_directories_first_with_sizes() -> TestResult {
    let ws = Workspace::new()?;
    let tools = ws.tools();
    std::fs::create_dir_all(ws.root.join("b"))?;
    std::fs::create_dir_all(ws.root.join("A"))?;
    std::fs::create_dir_all(ws.root.join("empty"))?;
    ws.write("z.txt", "x".repeat(1280))?;
    ws.write("a.txt", "0123456789")?;
    ws.write("big.bin", vec![0u8; 3 * 1024 * 1024 + 300_000])?;

    let ls = |input: Value| text_of(&tools, "filesystem_ls", input);
    assert_eq!(
        ls(json!({})).await?.0,
        "A/\nb/\nempty/\na.txt (10B)\nbig.bin (3.3MB)\nz.txt (1.3KB)"
    );
    assert_eq!(
        ls(json!({ "limit": 2 })).await?.0,
        "A/\nb/\n\n(Showing 2 of 6 entries. Use limit=4 to see more.)"
    );
    assert_eq!(ls(json!({ "path": "empty" })).await?.0, "(empty directory)");
    let (error, is_error) = ls(json!({ "path": "a.txt" })).await?;
    assert!(is_error);
    assert!(error.starts_with("ENOTDIR: not a directory, scandir"), "{error}");
    Ok(())
}

// ───── tool-output reads (BrowserOS state directory) ─────

const CHILD_ENV: &str = "BROWSEROS_AGENT_TOOLS_FS_CHILD";

/// Re-runs one test in a child process with `BROWSEROS_DIR` set to a fresh directory: setting
/// the variable in-process needs `unsafe`, which the workspace denies. Returns `true` in the
/// parent (the child did the work).
fn rerun_with_state_dir(test: &str) -> Result<bool, Box<dyn std::error::Error>> {
    if std::env::var_os(CHILD_ENV).is_some() {
        return Ok(false);
    }
    let state = tempfile::tempdir()?;
    let output = std::process::Command::new(std::env::current_exe()?)
        .args([test, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, "1")
        .env("BROWSEROS_DIR", state.path())
        .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "child run of {test} failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(true)
}

fn state_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(std::fs::canonicalize(std::env::var("BROWSEROS_DIR")?)?)
}

struct Grants(Vec<PathBuf>);

impl OutputGrants for Grants {
    fn allows(&self, path: &Path) -> bool {
        self.0.iter().any(|granted| granted == path)
    }
}

struct NoGrants;

impl OutputGrants for NoGrants {
    fn allows(&self, _path: &Path) -> bool {
        false
    }
}

#[tokio::test]
async fn output_only_reads_are_granted_fenced_and_bounded() -> TestResult {
    if rerun_with_state_dir("output_only_reads_are_granted_fenced_and_bounded")? {
        return Ok(());
    }
    let state = state_dir()?;
    let outputs = state.join("tool-output");
    std::fs::create_dir_all(&outputs)?;
    std::fs::write(outputs.join("snapshot.txt"), "row one\nrow two")?;
    std::fs::write(outputs.join("other.txt"), "not granted")?;
    std::fs::write(state.join("browseros.db"), "app state")?;
    let granted = std::fs::canonicalize(outputs.join("snapshot.txt"))?;
    let tools = filesystem_tools(FsScope::OutputOnly(Arc::new(Grants(vec![granted.clone()]))));

    let (text, is_error) = text_of(&tools, "filesystem_read", json!({ "path": granted })).await?;
    assert!(!is_error, "{text}");
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("[UNTRUSTED_PAGE_CONTENT nonce="), "{text}");
    assert!(lines[0].contains(&format!("origin={}]", granted.display())));
    assert_eq!(&lines[1..3], ["1 | row one", "2 | row two"]);
    assert!(lines[3].starts_with("[END_UNTRUSTED_PAGE_CONTENT nonce="));

    let refuse = |path: PathBuf, error: &'static str| {
        let tools = Arc::clone(&tools);
        async move {
            assert_eq!(
                text_of(&tools, "filesystem_read", json!({ "path": path })).await?,
                (error.to_owned(), true),
                "{}",
                path.display()
            );
            Ok::<(), Box<dyn std::error::Error>>(())
        }
    };
    refuse(
        outputs.join("other.txt"),
        "filesystem_read can only read BrowserOS-generated tool output files returned in this session.",
    )
    .await?;
    refuse(state.join("browseros.db"), "Path is outside BrowserOS tool output.").await?;
    refuse(
        PathBuf::from("snapshot.txt"),
        "No workspace selected. filesystem_read can only read BrowserOS-generated tool output files by absolute path.",
    )
    .await?;
    refuse(
        PathBuf::from("/etc/hosts"),
        "No workspace selected. filesystem_read can only read BrowserOS-generated tool output files by absolute path.",
    )
    .await?;
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(state.join("browseros.db"), outputs.join("escape.txt"))?;
        refuse(outputs.join("escape.txt"), "Path is outside BrowserOS tool output.").await?;
    }
    Ok(())
}

/// With a workspace, absolute tool-output paths a browser tool returned stay readable (no grant
/// check, no fence), as TS does without `allowedOutputPaths`.
#[tokio::test]
async fn workspace_reads_fall_back_to_tool_output() -> TestResult {
    if rerun_with_state_dir("workspace_reads_fall_back_to_tool_output")? {
        return Ok(());
    }
    let state = state_dir()?;
    std::fs::create_dir_all(state.join("tool-output"))?;
    std::fs::write(state.join("tool-output/page.md"), "# Title")?;
    let ws = Workspace::new()?;
    let path = state.join("tool-output/page.md");
    assert_eq!(
        text_of(&ws.tools(), "filesystem_read", json!({ "path": path })).await?,
        ("1 | # Title".to_owned(), false)
    );
    Ok(())
}
