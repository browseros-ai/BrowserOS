use crate::{
    AppState,
    api::mcp::dispatch::{ToolCall, ToolIdentity, linked_cancel_token},
    config::Config,
    identity::{ClientIdentity, ConversationIdentity},
    ids::SessionId,
    services::sessions::Session,
};
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

pub async fn tool_call(tool_name: &str, raw_args: Value) -> anyhow::Result<ToolCall> {
    let dir = tempfile::tempdir()?;
    let root = dir.path().join("browserclaw");
    let home = dir.path().join("home");
    let _persisted = dir.keep();
    let config = Arc::new(Config {
        server_port: 9200,
        cdp_port: 49337,
        proxy_port: None,
        resources_dir: root.join("resources"),
        browserclaw_dir: root,
        session_idle: Duration::from_secs(300),
        session_retention: Duration::from_secs(7_200),
        session_sweep_interval: Duration::from_secs(60),
        replay_retention_days: 7,
        dev_mode: false,
        auth_token: None,
    });
    let state = AppState::new_with_home(config, home).await?;
    let session = Session::new(
        SessionId::new("s1"),
        ClientIdentity::Ephemeral {
            slug: "codex".to_string(),
            label: "Codex".to_string(),
        },
        ConversationIdentity::new("codex", "agile-alpaca".to_string()),
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
    let ownership_key = session.convo_id().clone();
    Ok(ToolCall::new(
        catalog,
        tool_index,
        raw_args,
        session.id().clone(),
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
    ))
}
