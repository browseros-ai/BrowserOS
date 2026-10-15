//! Opt-in end-to-end turns against the real Claude and Codex adapters, launched exactly as
//! production does (bundled Bun when the installed app provides one, else host npx). Each test runs
//! one turn, restarts the runtime, and asks a recall question that only a resumed session answers.
//!
//! `BROWSEROS_ACP_LIVE=1 cargo test -p acp-agents --test live -- --ignored --nocapture`
//! Optional: `BROWSEROS_ACP_LIVE_RESOURCES`, `ACP_LIVE_CLAUDE_MODEL`, `ACP_LIVE_CODEX_MODEL`.

#![allow(clippy::print_stderr)]

use std::{path::PathBuf, sync::Arc, time::Duration};

use acp_agents::{
    AcpRuntime, AcpRuntimeConfig, AgentDefinition, AgentKind, FileSessionStore, HostEnvironment,
    TurnContext, TurnPrompt, TurnRequest,
};
use acp_client::schema::{ContentBlock, TextContent};
use futures_util::StreamExt;
use serde_json::Value;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const APP_RESOURCES: &str =
    "/Applications/BrowserOS.app/Contents/Resources/BrowserOSServer/default/resources";

fn host() -> HostEnvironment {
    let resources = std::env::var("BROWSEROS_ACP_LIVE_RESOURCES")
        .map(PathBuf::from)
        .ok()
        .or_else(|| Some(PathBuf::from(APP_RESOURCES)).filter(|dir| dir.is_dir()));
    // A stable BrowserOS dir keeps Bun's adapter cache warm across runs.
    HostEnvironment::from_process(resources, std::env::temp_dir().join("acp-agents-live"))
}

fn runtime(store: &TempDir) -> Arc<AcpRuntime> {
    let mut config =
        AcpRuntimeConfig::new(host(), Arc::new(FileSessionStore::new(store.path().into())));
    config.turn_timeout = Duration::from_secs(240);
    AcpRuntime::new(config)
}

async fn turn(
    runtime: &Arc<AcpRuntime>,
    agent: &AgentDefinition,
    conversation: Uuid,
    text: &str,
) -> Result<(String, Vec<Value>), Box<dyn std::error::Error>> {
    let request = TurnRequest {
        ctx: TurnContext {
            agent: agent.clone(),
            conversation_id: conversation,
            // Nothing listens on the discard port: Claude and Codex accept an unreachable MCP
            // server, and no browser control is handed to a live model from a test.
            server_port: 9,
            browser_tool_lease_token: "live-test-lease".into(),
            read_only: true,
            custom_mcp_servers: vec![],
            working_dir_override: None,
        },
        prompt: TurnPrompt {
            latest_user: vec![ContentBlock::Text(TextContent::new(format!(
                "User: {text}"
            )))],
            transcript_seed: None,
        },
        response_message_id: Uuid::new_v4().to_string(),
        cancel: CancellationToken::new(),
    };
    let stream = runtime.stream(request).await?;
    let chunks: Vec<Value> = stream
        .map(|chunk| serde_json::to_value(chunk).unwrap_or(Value::Null))
        .collect()
        .await;
    let reply: String = chunks
        .iter()
        .filter(|chunk| chunk["type"] == "text-delta")
        .filter_map(|chunk| chunk["delta"].as_str())
        .collect();
    let types: Vec<&str> = chunks
        .iter()
        .filter_map(|chunk| chunk["type"].as_str())
        .collect();
    let tools: Vec<&Value> = chunks
        .iter()
        .filter(|chunk| {
            chunk["type"]
                .as_str()
                .is_some_and(|kind| kind.starts_with("tool-"))
        })
        .collect();
    eprintln!(
        "[{:?}] reply={reply:?}\n  chunk types={types:?}\n  tools={tools:?}\n  last={}",
        agent.kind,
        chunks.last().unwrap_or(&Value::Null)
    );
    Ok((reply, chunks))
}

/// Extra pause between the turn and the runtime shutdown (`ACP_LIVE_SETTLE_MS`, default 0): the
/// runtime itself must keep an immediately retired Claude session resumable.
fn settle() -> Duration {
    let millis = std::env::var("ACP_LIVE_SETTLE_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    Duration::from_millis(millis)
}

async fn pong_then_recall(agent: AgentDefinition) -> TestResult {
    if std::env::var("BROWSEROS_ACP_LIVE").as_deref() != Ok("1") {
        return Err("set BROWSEROS_ACP_LIVE=1 to authorize live turns".into());
    }
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
    let store = TempDir::new()?;
    let conversation = Uuid::new_v4();
    let first = runtime(&store);
    let (reply, chunks) = turn(
        &first,
        &agent,
        conversation,
        "Reply with exactly the single word pong and nothing else.",
    )
    .await?;
    tokio::time::sleep(settle()).await;
    first.shutdown().await;
    assert!(
        reply.to_lowercase().contains("pong"),
        "first turn: {chunks:#?}"
    );

    let second = runtime(&store);
    let (reply, chunks) = turn(
        &second,
        &agent,
        conversation,
        "What single word did you reply with to my previous message? Answer with only that word.",
    )
    .await?;
    second.shutdown().await;
    assert!(
        reply.to_lowercase().contains("pong"),
        "resumed turn: {chunks:#?}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires BROWSEROS_ACP_LIVE=1 and a logged-in Claude Code"]
async fn live_claude_turn_and_resume() -> TestResult {
    pong_then_recall(AgentDefinition {
        id: Uuid::new_v4(),
        kind: AgentKind::Claude,
        model_id: Some(std::env::var("ACP_LIVE_CLAUDE_MODEL").unwrap_or_else(|_| "haiku".into())),
        reasoning_effort: None,
        working_directory: None,
        custom: None,
    })
    .await
}

#[tokio::test]
#[ignore = "requires BROWSEROS_ACP_LIVE=1 and a logged-in Codex"]
async fn live_codex_turn_and_resume() -> TestResult {
    pong_then_recall(AgentDefinition {
        id: Uuid::new_v4(),
        kind: AgentKind::Codex,
        model_id: Some(
            std::env::var("ACP_LIVE_CODEX_MODEL").unwrap_or_else(|_| "gpt-6-sol".into()),
        ),
        reasoning_effort: Some("low".into()),
        working_directory: None,
        custom: None,
    })
    .await
}
