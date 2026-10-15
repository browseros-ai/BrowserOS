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
async fn update(out: &mut Stdout, session: &str, text: &str) -> std::io::Result<()> {
    send(out, json!({"jsonrpc":"2.0", "method":"session/update", "params":{
        "sessionId":session, "update":{"sessionUpdate":"agent_message_chunk", "content":{"type":"text", "text":text}}
    }})).await
}
fn session_info() -> Value {
    json!({"modes":{"currentModeId":"default", "availableModes":[{"id":"default", "name":"Default"}, {"id":"code", "name":"Code"}]},
        "configOptions":[{"id":"model", "name":"Model", "type":"select", "currentValue":"fake", "options":[{"value":"fake", "name":"Fake"}]}]})
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let scenario = std::env::args().nth(1).unwrap_or_default();
    if scenario == "grandchild-hang" {
        let mut count = 0_u64;
        loop {
            std::fs::write("grandchild-heartbeat", count.to_string())?;
            count += 1;
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
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
                if scenario == "auth" || scenario == "rpc" {
                    send(&mut out, json!({"jsonrpc":"2.0", "id":id, "error":{
                        "code":if scenario == "auth" {-32000} else {-32602}, "message":"scripted refusal", "data":{"detail":42}}})).await?;
                } else {
                    sessions += 1;
                    let mut result = session_info();
                    result["sessionId"] = json!(format!("s{sessions}"));
                    reply(&mut out, id, result).await?;
                }
            }
            "session/load" | "session/resume" => {
                if scenario == "load-failure" {
                    send(&mut out, json!({"jsonrpc":"2.0", "id":id, "error":{"code":-32602, "message":"cannot reload"}})).await?;
                    continue;
                }
                update(&mut out, &session, "REPLAY MUST BE SWALLOWED").await?;
                reply(&mut out, id, session_info()).await?;
            }
            "session/set_mode" | "session/set_config_option" => {
                if scenario == "hang-control" {
                    continue;
                }
                controls.push(message["method"].as_str().unwrap_or_default().to_owned());
                if message["method"] == "session/set_config_option" {
                    reply(
                        &mut out,
                        id,
                        json!({"configOptions":session_info()["configOptions"]}),
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
                    "permission" | "missing-option" => {
                        send(&mut out, json!({"jsonrpc":"2.0", "method":"session/update", "params":{
                            "sessionId":session, "update":{"sessionUpdate":"tool_call", "toolCallId":"t1", "title":"Fixture command", "status":"pending"}}})).await?;
                        let permission_id = format!("p-{session}");
                        let options = if prompt == "missing-option" {
                            json!([])
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
                *cancels.entry(session.clone()).or_default() += 1;
                if let Some(request) = pending.remove(&session) {
                    update(&mut out, &session, "after cancel 1").await?;
                    update(&mut out, &session, "after cancel 2").await?;
                    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
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
