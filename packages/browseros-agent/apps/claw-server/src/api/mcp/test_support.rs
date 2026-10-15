//! claw's MCP test fixtures: the crate's synthetic browser peer, plus a `ToolCall` over a
//! full claw `AppState` (audit database, cockpit, skills) for tests that assert on
//! claw's own adapters.

use crate::{AppState, api::mcp::dispatch::ToolCall, config::Config};
use serde_json::Value;
use std::{sync::Arc, time::Duration};

pub use browseros_mcp_host::test_support::{
    ProfileConnection, current_native, current_session, tool_call_on,
};

/// A claw `AppState` over a temp `.browserclaw` dir, modelling the current native.
pub async fn claw_state() -> anyhow::Result<AppState> {
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
        native: crate::config::NativeIdentity {
            capabilities: current_native(),
            asserted: false,
        },
    });
    Ok(AppState::new_with_home(config, home).await?)
}

/// A `ToolCall` on claw's composed host (session `s1`, no browser), and the state behind it.
pub async fn claw_tool_call(
    tool_name: &str,
    raw_args: Value,
) -> anyhow::Result<(ToolCall, AppState)> {
    let state = claw_state().await?;
    let call = tool_call_on(state.mcp.clone(), tool_name, raw_args).await?;
    Ok((call, state))
}
