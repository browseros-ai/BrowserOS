use crate::{
    AppState,
    config::Config,
    domain::{AgentId, AgentRef, Session, SessionId},
    mcp::dispatch::{ToolCall, ToolIdentity, linked_cancel_token},
};
use rmcp::model::RequestId;
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

pub async fn tool_call(tool_name: &str, raw_args: Value) -> anyhow::Result<ToolCall> {
    tool_call_with_fallback(tool_name, raw_args, true).await
}

pub async fn tool_call_with_fallback(
    tool_name: &str,
    raw_args: Value,
    screencast_screenshot_fallback: bool,
) -> anyhow::Result<ToolCall> {
    let dir = tempfile::tempdir()?;
    let root = dir.path().join("browserclaw");
    let home = dir.path().join("home");
    let _persisted = dir.keep();
    let config = Arc::new(Config {
        server_port: 9200,
        cdp_port: 49337,
        proxy_port: None,
        resources_dir: root.join("resources"),
        browserclaw_dir: root.clone(),
        claw_dir: root,
        session_idle: Duration::from_secs(300),
        session_sweep_interval: Duration::from_secs(60),
        screencast_screenshot_fallback,
        dev_mode: false,
        auth_token: None,
    });
    let state = AppState::new_with_home(config, None, home).await?;
    let session = Session::new(
        SessionId::new("s1"),
        AgentRef::Ephemeral {
            agent_id: AgentId::new("codex-a"),
            slug: "codex".to_string(),
            label: "Codex".to_string(),
        },
        tokio::time::Instant::now(),
    );
    state.sessions.insert_for_testing(session.clone()).await;
    let catalog = Arc::new(browseros_mcp::catalog());
    let tool_index = catalog
        .iter()
        .position(|tool| tool.name == tool_name)
        .ok_or_else(|| anyhow::anyhow!("tool {tool_name} missing from catalog"))?;
    let client_cancel = CancellationToken::new();
    let dispatch_cancel = CancellationToken::new();
    let cancel = linked_cancel_token(
        session.child_token(),
        client_cancel.clone(),
        dispatch_cancel.clone(),
    );
    let ownership_key = session.agent().ownership_key();
    Ok(ToolCall::new(
        catalog,
        tool_index,
        raw_args,
        session.id().clone(),
        RequestId::Number(1),
        Some(ToolIdentity {
            session: session.clone(),
            agent: session.agent().clone(),
            ownership_key,
            agent_label: "Codex".to_string(),
        }),
        None,
        cancel,
        client_cancel,
        dispatch_cancel,
        None,
        state,
        browseros_mcp::output_file::create_browser_output_file_access(),
        None,
        Arc::default(),
    ))
}
