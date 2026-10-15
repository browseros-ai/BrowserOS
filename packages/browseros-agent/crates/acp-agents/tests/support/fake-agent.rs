//! Scripted ACP peer for acp-agents runtime and probe tests. It writes raw wire JSON (no SDK) and
//! appends every call to `$FAKE_AGENT_LOG`, so tests can assert the exact control sequence across
//! the several processes one conversation may use (reattach, recovery, idle close).
//!
//! argv[1] scenario: `default`, `resume-fails` (resume/load refused), `no-resume` (only
//! session/load advertised), `mode-fails-after-resume` (set_mode refused on reattached sessions),
//! `auth` (session/new needs login). Prompt keywords: `hang` (waits for cancel), `never` (ignores
//! cancel), `slow`, `tool`, `crash`, `silent-error` (Codex systemError with no output).

use std::{collections::HashSet, io::Write};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Stdout};

fn log(line: &str) {
    if let Ok(path) = std::env::var("FAKE_AGENT_LOG")
        && let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    {
        let _ = writeln!(file, "{} {line}", std::process::id());
    }
}

async fn send(out: &mut Stdout, value: Value) -> std::io::Result<()> {
    out.write_all(format!("{value}\n").as_bytes()).await?;
    out.flush().await
}
async fn reply(out: &mut Stdout, id: &Value, result: Value) -> std::io::Result<()> {
    send(out, json!({"jsonrpc":"2.0", "id":id, "result":result})).await
}
async fn fail(out: &mut Stdout, id: &Value, code: i64, message: &str) -> std::io::Result<()> {
    send(
        out,
        json!({"jsonrpc":"2.0", "id":id, "error":{"code":code, "message":message}}),
    )
    .await
}
async fn update(out: &mut Stdout, session: &str, update: Value) -> std::io::Result<()> {
    send(out, json!({"jsonrpc":"2.0", "method":"session/update", "params":{"sessionId":session, "update":update}})).await
}
async fn text(out: &mut Stdout, session: &str, text: &str) -> std::io::Result<()> {
    update(
        out,
        session,
        json!({"sessionUpdate":"agent_message_chunk", "content":{"type":"text", "text":text}}),
    )
    .await
}

fn config_options(model: &str, effort: &str) -> Value {
    json!([
        {"id":"model", "name":"Model", "category":"model", "type":"select", "currentValue":model,
         "options":[{"value":"fake-default", "name":"Fake default"}, {"value":"fake-pro", "name":"Fake Pro", "description":"Bigger"}]},
        {"id":"effort", "name":"Effort", "category":"thought_level", "type":"select", "currentValue":effort,
         "options":[{"value":"low", "name":"Low"}, {"value":"medium", "name":"Medium"}, {"value":"high", "name":"High"}]}
    ])
}

fn session_info(model: &str, effort: &str) -> Value {
    json!({
        "modes":{"currentModeId":"default", "availableModes":[{"id":"default", "name":"Default"}, {"id":"bypassPermissions", "name":"Bypass"}]},
        "configOptions":config_options(model, effort),
    })
}

struct Pending {
    id: Value,
    session: String,
    ignore_cancel: bool,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let scenario = std::env::args().nth(1).unwrap_or_else(|| "default".into());
    let pid = std::process::id();
    log(&format!(
        "spawn {scenario} cwd={}",
        std::env::current_dir()?.display()
    ));
    let mut input = BufReader::new(tokio::io::stdin()).lines();
    let mut out = tokio::io::stdout();
    let mut sessions = 0;
    let mut reattached = HashSet::new();
    let mut pending: Option<Pending> = None;
    let (mut model, mut effort) = ("fake-default".to_owned(), "medium".to_owned());
    while let Some(line) = input.next_line().await? {
        let message: Value = serde_json::from_str(&line)?;
        let id = message["id"].clone();
        let params = &message["params"];
        let session = params["sessionId"].as_str().unwrap_or_default().to_owned();
        let method = message["method"].as_str().unwrap_or_default().to_owned();
        if method.is_empty() {
            continue;
        }
        match method.as_str() {
            "initialize" => {
                log("initialize");
                let session_caps = if scenario == "no-resume" {
                    json!({"close":{}})
                } else {
                    json!({"resume":{}, "close":{}})
                };
                reply(&mut out, &id, json!({"protocolVersion":1, "agentCapabilities":{
                    "loadSession":true, "sessionCapabilities":session_caps,
                    "mcpCapabilities":{"http":true, "sse":false}, "promptCapabilities":{"image":true}},
                    "agentInfo":{"name":"fake-agent", "title":"Fake Agent", "version":"9.9.9"}})).await?;
            }
            "session/new" => {
                if scenario == "auth" {
                    fail(&mut out, &id, -32000, "Authentication required").await?;
                    continue;
                }
                sessions += 1;
                let new_id = format!("sess-{pid}-{sessions}");
                let servers: Vec<String> = params["mcpServers"]
                    .as_array()
                    .map(|servers| {
                        servers
                            .iter()
                            .map(|server| {
                                let headers = server["headers"].as_array().map_or(0, Vec::len);
                                format!(
                                    "{}@{}#{headers}",
                                    server["name"].as_str().unwrap_or_default(),
                                    server["url"].as_str().unwrap_or_default()
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                log(&format!(
                    "new {new_id} mcp={} meta={}",
                    servers.join(","),
                    params["_meta"]
                ));
                let mut result = session_info(&model, &effort);
                result["sessionId"] = json!(new_id);
                reply(&mut out, &id, result).await?;
            }
            "session/load" | "session/resume" => {
                log(&format!(
                    "{} {session} meta={}",
                    method.trim_start_matches("session/"),
                    params["_meta"]
                ));
                if scenario == "resume-fails" {
                    fail(&mut out, &id, -32002, "Resource not found").await?;
                    continue;
                }
                reattached.insert(session.clone());
                if method == "session/load" {
                    text(&mut out, &session, "REPLAY MUST BE SWALLOWED").await?;
                }
                reply(&mut out, &id, session_info(&model, &effort)).await?;
            }
            "session/set_mode" => {
                let mode = params["modeId"].as_str().unwrap_or_default();
                log(&format!("set_mode {session} {mode}"));
                if mode != "bypassPermissions"
                    || (scenario == "mode-fails-after-resume" && reattached.contains(&session))
                {
                    fail(&mut out, &id, -32602, "unknown mode").await?;
                } else {
                    reply(&mut out, &id, json!({})).await?;
                }
            }
            "session/set_config_option" => {
                let config = params["configId"].as_str().unwrap_or_default();
                let value = params["value"].as_str().unwrap_or_default();
                log(&format!("set_config {session} {config}={value}"));
                match config {
                    "model" => model = value.to_owned(),
                    "effort" => effort = value.to_owned(),
                    _ => {
                        fail(&mut out, &id, -32602, "unknown config option").await?;
                        continue;
                    }
                }
                reply(
                    &mut out,
                    &id,
                    json!({"configOptions":config_options(&model, &effort)}),
                )
                .await?;
            }
            "session/close" => {
                log(&format!("close {session}"));
                reply(&mut out, &id, json!({})).await?;
            }
            "session/cancel" => {
                log(&format!("cancel {session}"));
                if let Some(turn) = pending.take_if(|turn| !turn.ignore_cancel) {
                    text(&mut out, &turn.session, "after cancel").await?;
                    reply(&mut out, &turn.id, json!({"stopReason":"cancelled"})).await?;
                }
            }
            "session/prompt" => {
                let prompt: Vec<String> = params["prompt"]
                    .as_array()
                    .map(|blocks| {
                        blocks
                            .iter()
                            .map(|block| match block["type"].as_str() {
                                Some("text") => {
                                    block["text"].as_str().unwrap_or_default().to_owned()
                                }
                                Some(kind) => format!("<{kind}>"),
                                None => String::new(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let prompt = prompt.join("|");
                log(&format!("prompt {session} {}", prompt.replace('\n', "\\n")));
                if prompt.contains("crash") {
                    text(&mut out, &session, "about to crash").await?;
                    std::process::exit(17);
                }
                if prompt.contains("hang") || prompt.contains("never") {
                    text(&mut out, &session, "started").await?;
                    pending = Some(Pending {
                        id,
                        session,
                        ignore_cancel: prompt.contains("never"),
                    });
                    continue;
                }
                if prompt.contains("slow") {
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                }
                if prompt.contains("silent-error") {
                    update(
                        &mut out,
                        &session,
                        json!({"sessionUpdate":"session_info_update",
                        "_meta":{"codex":{"threadStatus":{"type":"systemError"}}}}),
                    )
                    .await?;
                    reply(&mut out, &id, json!({"stopReason":"end_turn"})).await?;
                    continue;
                }
                if prompt.contains("tool") {
                    update(
                        &mut out,
                        &session,
                        json!({"sessionUpdate":"tool_call", "toolCallId":"t1", "title":"Terminal",
                        "kind":"execute", "status":"pending", "rawInput":{}}),
                    )
                    .await?;
                    update(&mut out, &session, json!({"sessionUpdate":"tool_call_update", "toolCallId":"t1",
                        "title":"ls -la", "status":"in_progress", "rawInput":{"command":"ls", "args":["-la"]}})).await?;
                    update(&mut out, &session, json!({"sessionUpdate":"tool_call_update", "toolCallId":"t1",
                        "status":"completed", "content":[{"type":"content", "content":{"type":"text", "text":"file.txt"}}]})).await?;
                }
                update(
                    &mut out,
                    &session,
                    json!({"sessionUpdate":"agent_message_chunk", "messageId":"m1",
                    "content":{"type":"text", "text":format!("echo:{prompt}")}}),
                )
                .await?;
                update(
                    &mut out,
                    &session,
                    json!({"sessionUpdate":"usage_update", "used":10, "size":100}),
                )
                .await?;
                reply(&mut out, &id, json!({"stopReason":"end_turn"})).await?;
            }
            other => {
                log(&format!("unhandled {other}"));
                if !id.is_null() {
                    fail(&mut out, &id, -32601, "method not found").await?;
                }
            }
        }
    }
    log("eof");
    Ok(())
}
