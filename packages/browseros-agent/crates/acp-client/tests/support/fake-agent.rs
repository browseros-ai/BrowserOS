//! Scripted JSON-RPC peer for process-level tests. It deliberately writes wire
//! JSON rather than using the SDK, so unknown schema tags and broken peers are
//! tested independently of the client's parser and lifecycle implementation.

use std::collections::HashMap;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Stdout};

struct Pending {
    prompt_id: Value,
    session: String,
}

async fn send(out: &mut Stdout, value: Value) -> std::io::Result<()> {
    out.write_all(format!("{value}\n").as_bytes()).await?;
    out.flush().await
}
async fn reply(out: &mut Stdout, id: Value, result: Value) -> std::io::Result<()> {
    send(out, json!({"jsonrpc":"2.0", "id":id, "result":result})).await
}
async fn attachment_reply(
    out: &mut Stdout,
    id: Value,
    result: Value,
    session: &str,
    immediate: bool,
) -> std::io::Result<()> {
    if !immediate {
        return reply(out, id, result).await;
    }
    let mut config = session_info()["configOptions"].clone();
    config[0]["currentValue"] = json!("after-attachment");
    send(out, json!([
        {"jsonrpc":"2.0", "id":id, "result":result},
        {"jsonrpc":"2.0", "method":"session/update", "params":{
            "sessionId":session, "update":{"sessionUpdate":"current_mode_update", "currentModeId":"code"}
        }},
        {"jsonrpc":"2.0", "method":"session/update", "params":{
            "sessionId":session, "update":{"sessionUpdate":"config_option_update", "configOptions":config}
        }}
    ])).await
}

async fn update(out: &mut Stdout, session: &str, text: &str) -> std::io::Result<()> {
    send(out, json!({"jsonrpc":"2.0", "method":"session/update", "params":{
        "sessionId":session, "update":{"sessionUpdate":"agent_message_chunk", "content":{"type":"text", "text":text}}
    }})).await
}
fn session_info() -> Value {
    json!({"modes":{"currentModeId":"default", "availableModes":[{"id":"default", "name":"Default"}, {"id":"code", "name":"Code"}]},
        "configOptions":[{"id":"model", "name":"Model", "type":"select", "currentValue":"fake", "options":[{"value":"fake", "name":"Fake"}]}]})
}

/// Atomically publish fixture acknowledgments so an observed file always has
/// its complete contents, including PID handoffs on a heavily loaded machine.
fn signal(path: &str, value: &str) -> std::io::Result<()> {
    let pending = format!("{path}.tmp");
    std::fs::write(&pending, value)?;
    std::fs::rename(pending, path)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let scenario = std::env::args().nth(1).unwrap_or_default();
    if scenario == "stdout-holder" || scenario == "grandchild-hang" {
        #[cfg(unix)]
        {
            use tokio::io::AsyncReadExt;
            if scenario == "stdout-holder" {
                // Escape the supervised group while retaining its stdout pipe.
                rustix::process::setsid()?;
            }
            // The parent observes actual process exit through socket EOF, not a
            // quiet heartbeat interval. Closing its end also cleans up a failed test.
            let mut lifetime =
                tokio::net::UnixStream::connect(std::env::var("ACP_TEST_LIFETIME_SOCKET")?).await?;
            lifetime.write_all(b"r").await?;
            if scenario == "stdout-holder" {
                signal("stdout-holder-ready", &std::process::id().to_string())?;
            }
            let _ = lifetime.read(&mut [0]).await?;
        }
        #[cfg(not(unix))]
        std::future::pending::<()>().await;
        return Ok(());
    }
    // Retain child handles for the life of the wrapper; subprocesses inherit
    // the group created by acp-client and must be stopped by group cleanup.
    let mut descendants = Vec::new();
    let mut input = BufReader::new(tokio::io::stdin()).lines();
    let mut out = tokio::io::stdout();
    let mut sessions = 0;
    let mut pending = HashMap::<String, Pending>::new();
    let mut permissions = HashMap::<String, Pending>::new();
    let mut cancels = HashMap::<String, usize>::new();
    let mut controls = Vec::new();
    let mut pending_new = None;
    while let Some(line) = input.next_line().await? {
        let message: Value = serde_json::from_str(&line)?;
        let id = message["id"].clone();
        let params = &message["params"];
        let session = params["sessionId"].as_str().unwrap_or_default().to_owned();
        if message.get("method").is_none() {
            if id == "new-permission" {
                if let Some(control_id) = pending_new.take() {
                    if message["result"]["outcome"]["optionId"] != "allow" {
                        send(&mut out, json!({"jsonrpc":"2.0", "id":control_id, "error":{"code":-32602, "message":"setup permission denied"}})).await?;
                    } else {
                        let mut result = session_info();
                        result["sessionId"] = json!("setup-session");
                        reply(&mut out, control_id, result).await?;
                    }
                }
                continue;
            }
            if let Some(request) = permissions.remove(&id.to_string()) {
                let result = &message["result"]["outcome"];
                update(&mut out, &request.session, &format!("permission:{result}")).await?;
                reply(
                    &mut out,
                    request.prompt_id,
                    json!({"stopReason":"end_turn"}),
                )
                .await?;
            }
            continue;
        }
        match message["method"].as_str().unwrap_or_default() {
            "initialize" => {
                if scenario == "hang-init" {
                    signal("initialize-received", "ready")?;
                    continue;
                }
                if scenario == "auth-init" {
                    send(&mut out, json!({"jsonrpc":"2.0", "id":id, "error":{
                        "code":-32603, "message":"Internal error: Failed to authenticate: OAuth session expired and could not be refreshed",
                        "data":{"errorKind":"authentication_failed"}}})).await?;
                    continue;
                }
                if scenario == "crash-init" {
                    let mut stderr = tokio::io::stderr();
                    stderr.write_all(b"startup crash\n").await?;
                    stderr.flush().await?;
                    std::process::exit(18);
                }
                // Fail immediately if the host accidentally opens the fs/PTY surface.
                if params["clientCapabilities"]["fs"]["readTextFile"] != false
                    || params["clientCapabilities"]["fs"]["writeTextFile"] != false
                    || params["clientCapabilities"]["terminal"] != false
                {
                    reply(&mut out, id, json!({})).await?;
                    continue;
                }
                let caps = if scenario == "no-caps" {
                    json!({})
                } else {
                    json!({"loadSession":true,
                    "sessionCapabilities":{"resume":{}, "close":{}}, "mcpCapabilities":{"http":true, "sse":true},
                    "promptCapabilities":{"image":true, "audio":true}})
                };
                reply(&mut out, id, json!({"protocolVersion":if scenario == "v2" {2} else {1}, "agentCapabilities":caps,
                    "authMethods":[{"id":"fake-login", "name":"Fake login"}], "agentInfo":{"name":"fake", "version":"1"}})).await?;
            }
            "session/new" => {
                if scenario == "setup-permission" {
                    pending_new = Some(id);
                    send(&mut out, json!({"jsonrpc":"2.0", "id":"new-permission", "method":"session/request_permission", "params":{
                        "sessionId":"setup-session", "toolCall":{"toolCallId":"setup"},
                        "options":[{"optionId":"allow", "name":"Allow", "kind":"allow_once"}]}})).await?;
                    continue;
                }
                if matches!(
                    scenario.as_str(),
                    "auth" | "rpc" | "auth-invalid-params" | "auth-mcp" | "rpc-local-looking"
                ) {
                    let (code, message, data) = match scenario.as_str() {
                        "auth" => (-32000, "scripted refusal", json!({"detail":42})),
                        "auth-invalid-params" => (
                            -32602,
                            "authentication required: invalid login parameter",
                            json!({"errorKind":"authentication_failed"}),
                        ),
                        "auth-mcp" => (
                            -32603,
                            "Failed to authenticate MCP server github: login required",
                            json!({"errorKind":"authentication_failed"}),
                        ),
                        "rpc-local-looking" => {
                            (-32603, "Internal error", json!("oneshot canceled"))
                        }
                        _ => (-32602, "scripted refusal", json!({"detail":42})),
                    };
                    send(
                        &mut out,
                        json!({"jsonrpc":"2.0", "id":id, "error":{
                        "code":code, "message":message, "data":data}}),
                    )
                    .await?;
                } else {
                    sessions += 1;
                    let mut result = session_info();
                    if scenario == "immediate-controls" {
                        result["modes"] = Value::Null;
                    }
                    let session_id = format!("s{sessions}");
                    result["sessionId"] = json!(session_id);
                    attachment_reply(
                        &mut out,
                        id,
                        result,
                        &session_id,
                        scenario == "immediate-controls",
                    )
                    .await?;
                }
            }
            "session/load" | "session/resume" => {
                if scenario == "load-failure" {
                    send(&mut out, json!({"jsonrpc":"2.0", "id":id, "error":{"code":-32602, "message":"cannot reload"}})).await?;
                    continue;
                }
                update(&mut out, &session, "REPLAY MUST BE SWALLOWED").await?;
                attachment_reply(
                    &mut out,
                    id,
                    session_info(),
                    &session,
                    scenario == "immediate-controls",
                )
                .await?;
            }
            "session/set_mode" | "session/set_config_option" => {
                if scenario == "hang-control" {
                    signal("control-received", "ready")?;
                    continue;
                }
                if scenario == "crash-control" {
                    let mut stderr = tokio::io::stderr();
                    stderr.write_all(b"control crash\n").await?;
                    stderr.flush().await?;
                    std::process::exit(24);
                }
                if scenario == "idle-controls" {
                    // The next control reply is a dispatch barrier: both idle
                    // notifications must have arrived before the caller resumes.
                    send(&mut out, json!({"jsonrpc":"2.0", "method":"session/update", "params":{
                        "sessionId":session, "update":{"sessionUpdate":"current_mode_update", "currentModeId":"code"}
                    }})).await?;
                    let mut config = session_info()["configOptions"].clone();
                    config[0]["currentValue"] = json!("agent-selected");
                    send(&mut out, json!({"jsonrpc":"2.0", "method":"session/update", "params":{
                        "sessionId":session, "update":{"sessionUpdate":"config_option_update", "configOptions":config}
                    }})).await?;
                }
                controls.push(message["method"].as_str().unwrap_or_default().to_owned());
                if message["method"] == "session/set_config_option" {
                    if scenario == "post-config-update" {
                        let mut latest = session_info()["configOptions"].clone();
                        latest[0]["currentValue"] = json!("newer-notification");
                        // A single read contains the reply then a newer update.
                        // Setter continuation scheduling must not reverse them.
                        send(&mut out, json!([
                            {"jsonrpc":"2.0", "id":id, "result":{"configOptions":session_info()["configOptions"]}},
                            {"jsonrpc":"2.0", "method":"session/update", "params":{
                                "sessionId":session, "update":{"sessionUpdate":"config_option_update", "configOptions":latest}
                            }}
                        ])).await?;
                        continue;
                    }
                    reply(
                        &mut out,
                        id,
                        json!({"configOptions": if scenario == "returned-config" {
                            let mut config = session_info()["configOptions"].clone();
                            config[0]["currentValue"] = json!("agent-selected");
                            config
                        } else { session_info()["configOptions"].clone() }}),
                    )
                    .await?;
                } else {
                    reply(&mut out, id, json!({})).await?;
                }
            }
            "session/close" => {
                reply(&mut out, id, json!({})).await?;
            }
            "session/prompt" => {
                let prompt = params["prompt"][0]["text"].as_str().unwrap_or_default();
                match prompt {
                    "inherited-stdout" => {
                        let child = tokio::process::Command::new(std::env::current_exe()?)
                            .arg("stdout-holder")
                            .stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::inherit())
                            .stderr(std::process::Stdio::null())
                            .spawn()?;
                        descendants.push(child);
                        while !std::path::Path::new("stdout-holder-ready").exists() {
                            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                        }
                        let mut stderr = tokio::io::stderr();
                        stderr.write_all(b"inherited stdout crash\n").await?;
                        stderr.flush().await?;
                        std::process::exit(23);
                    }
                    "broken-transport" => {
                        out.write_all(b"\xff\n").await?;
                        out.flush().await?;
                    }
                    "rpc-error-then-exit" => {
                        send(&mut out, json!({"jsonrpc":"2.0", "id":id, "error":{
                            "code":-32602, "message":"scripted invalid prompt", "data":{"detail":42}
                        }})).await?;
                        std::process::exit(25);
                    }
                    "exit-after-result" => {
                        update(&mut out, &session, "final output before exit").await?;
                        reply(&mut out, id, json!({"stopReason":"end_turn"})).await?;
                        std::process::exit(0);
                    }
                    "grandchild" => {
                        let child = tokio::process::Command::new(std::env::current_exe()?)
                            .arg("grandchild-hang")
                            .stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .spawn()?;
                        descendants.push(child);
                        update(&mut out, &session, "grandchild spawned").await?;
                        reply(&mut out, id, json!({"stopReason":"end_turn"})).await?;
                    }
                    "long-stderr" => {
                        let mut stderr = tokio::io::stderr();
                        stderr.write_all(&vec![b'x'; 20000]).await?;
                        stderr.write_all(b"TAIL").await?;
                        stderr.flush().await?;
                        std::process::exit(19);
                    }
                    "crash" => {
                        // Tokio's stderr adapter uses blocking work internally;
                        // flush before immediate exit so the fixture guarantees
                        // these bytes actually reached the client's pipe.
                        let mut stderr = tokio::io::stderr();
                        stderr
                            .write_all(b"first diagnostic\nsecond diagnostic\ncrash detail\n")
                            .await?;
                        stderr.flush().await?;
                        std::process::exit(17);
                    }
                    "hang" => {
                        update(&mut out, &session, "started").await?;
                        pending.insert(
                            session.clone(),
                            Pending {
                                prompt_id: id,
                                session,
                            },
                        );
                    }
                    "permission" | "missing-option" | "always-options" | "reject-only"
                    | "allow-only" | "reversed-options" => {
                        send(&mut out, json!({"jsonrpc":"2.0", "method":"session/update", "params":{
                            "sessionId":session, "update":{"sessionUpdate":"tool_call", "toolCallId":"t1", "title":"Fixture command", "status":"pending"}}})).await?;
                        let permission_id = format!("p-{session}");
                        let options = if prompt == "missing-option" {
                            json!([])
                        } else if prompt == "always-options" {
                            json!([
                                {"optionId":"reject-always", "name":"Reject", "kind":"reject_always"},
                                {"optionId":"allow-always", "name":"Allow", "kind":"allow_always"}
                            ])
                        } else if prompt == "reject-only" {
                            json!([{"optionId":"reject", "name":"Reject", "kind":"reject_once"}])
                        } else if prompt == "allow-only" {
                            json!([{"optionId":"allow", "name":"Allow", "kind":"allow_once"}])
                        } else if prompt == "reversed-options" {
                            json!([
                                {"optionId":"reject-always", "name":"Reject", "kind":"reject_always"},
                                {"optionId":"allow-always", "name":"Allow", "kind":"allow_always"},
                                {"optionId":"reject", "name":"Reject once", "kind":"reject_once"},
                                {"optionId":"allow", "name":"Allow once", "kind":"allow_once"}
                            ])
                        } else {
                            json!([
                            {"optionId":"allow", "name":"Allow", "kind":"allow_once"},
                            {"optionId":"reject", "name":"Reject", "kind":"reject_once"},
                            {"optionId":"allow-always", "name":"Always allow", "kind":"allow_always"},
                            {"optionId":"reject-always", "name":"Always reject", "kind":"reject_always"}])
                        };
                        send(&mut out, json!({"jsonrpc":"2.0", "id":permission_id, "method":"session/request_permission", "params":{
                            "sessionId":session, "toolCall":{"toolCallId":"t1"}, "options":options}})).await?;
                        // Dispatch must keep moving while a host policy waits.
                        update(&mut out, &session, "permission pending").await?;
                        permissions.insert(
                            json!(permission_id).to_string(),
                            Pending {
                                prompt_id: id,
                                session,
                            },
                        );
                    }
                    _ => {
                        send(&mut out, json!({"jsonrpc":"2.0", "method":"session/update", "params":{
                            "sessionId":session, "update":{"sessionUpdate":"future_unknown_tag", "secret":"never log payload"}}})).await?;
                        send(
                            &mut out,
                            json!({"jsonrpc":"2.0", "method":"session/update", "params":{
                            "sessionId":session, "update":"never log payload"}}),
                        )
                        .await?;
                        update(&mut out, "unowned-session", "WRONG SESSION").await?;
                        let text = if prompt == "environment" {
                            format!(
                                "{}:{}",
                                std::env::var("ACP_TEST_OVERLAY").unwrap_or_default(),
                                std::env::current_dir()?.display()
                            )
                        } else {
                            format!("hello:{session}")
                        };
                        update(&mut out, &session, &text).await?;
                        update(&mut out, &session, " world").await?;
                        reply(
                            &mut out,
                            id,
                            json!({"stopReason":"end_turn", "_meta":{"controls":controls,
                            "cancels":cancels.get(&session).copied().unwrap_or_default()}}),
                        )
                        .await?;
                    }
                }
            }
            "session/cancel" => {
                if scenario == "ignore-cancel" {
                    continue;
                }
                *cancels.entry(session.clone()).or_default() += 1;
                if let Some(request) = pending.remove(&session) {
                    update(&mut out, &session, "after cancel 1").await?;
                    update(&mut out, &session, "after cancel 2").await?;
                    if scenario == "pause-cancel" && cancels.get(&session) == Some(&1) {
                        signal("cancel-received", "ready")?;
                        while !std::path::Path::new("release-cancel").exists() {
                            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                        }
                    }
                    reply(
                        &mut out,
                        request.prompt_id,
                        json!({"stopReason":"cancelled"}),
                    )
                    .await?;
                }
            }
            _ => {}
        }
    }
    if scenario == "ignore-eof" {
        std::future::pending::<()>().await;
    }
    Ok(())
}
