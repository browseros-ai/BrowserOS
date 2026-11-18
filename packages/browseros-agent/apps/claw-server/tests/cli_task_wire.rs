//! The CLI's task model over real HTTP: a `browseros-cli` client on the stateless
//! 2026-07-28 path carries a tab group and nothing else, and the server resolves its task
//! from that. Each request is a separate HTTP call, as each CLI command is a separate
//! process, so continuity here can only come from the group.

use browseros_core::NativeCapabilities;
use claw_server::{
    AppState, build_router,
    config::Config,
    db::{NewBinding, TaskOrigin},
    ids::SessionId,
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tempfile::TempDir;
use tokio::{net::TcpListener, task::JoinHandle};

/// The CLI's own constant, so the binary and the server's gate are tested as one: if
/// either side renamed the client alone, these tests would stop resolving tasks.
const CLI: &str = claw_server::cli::client::transport::CLIENT_NAME;
const GROUP_META: &str = "com.browseros.neo/group";
const SESSION_META: &str = "com.browseros.neo/session";

struct Server {
    client: reqwest::Client,
    url: String,
    state: AppState,
    task: JoinHandle<Result<(), std::io::Error>>,
    _dir: TempDir,
}

impl Server {
    async fn start() -> anyhow::Result<Self> {
        let dir = tempfile::tempdir()?;
        let config = Arc::new(Config {
            server_port: 9200,
            cdp_port: 49337,
            proxy_port: None,
            resources_dir: dir.path().join("resources"),
            browserclaw_dir: dir.path().join("claw"),
            session_idle: Duration::from_secs(300),
            session_complete_idle: Duration::from_secs(300),
            session_retention: Duration::from_secs(7_200),
            session_sweep_interval: Duration::from_secs(60),
            replay_retention_days: 7,
            dev_mode: false,
            native: claw_server::config::NativeIdentity {
                capabilities: NativeCapabilities::from_browseros_version(Some("0.51.1.3")),
                asserted: false,
            },
            mode: Default::default(),
            behavior: Default::default(),
            behavior_ignored: Vec::new(),
        });
        let state = AppState::new_with_home(config, dir.path().join("home")).await?;
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/mcp", listener.local_addr()?);
        let router = build_router(state.clone());
        let task = tokio::spawn(async move { axum::serve(listener, router).await });
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()?,
            url,
            state,
            task,
            _dir: dir,
        })
    }

    /// One stateless `tools/call` from `client`, as one CLI command would send it.
    async fn call(&self, client: &str, name: &str, arguments: Value) -> anyhow::Result<Value> {
        let params = json!({
            "name": name,
            "arguments": arguments,
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientCapabilities": {},
                "io.modelcontextprotocol/clientInfo": {"name": client, "version": "0.0.1"}
            }
        });
        let response = self
            .client
            .post(&self.url)
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2026-07-28")
            .header("mcp-method", "tools/call")
            .header("mcp-name", name)
            .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": params}))
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        anyhow::ensure!(status == reqwest::StatusCode::OK, "{status}: {body}");
        let encoded = body
            .lines()
            .find_map(|line| line.strip_prefix("data:"))
            .unwrap_or(&body);
        let value: Value = serde_json::from_str(encoded.trim())?;
        anyhow::ensure!(value["error"].is_null(), "{value}");
        Ok(value["result"].clone())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

fn text(result: &Value) -> String {
    result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn the_cli_is_refused_without_the_ultrafast_plan() -> anyhow::Result<()> {
    let server = Server::start().await?;

    let refused = server
        .call(CLI, "name_session", json!({"name": "checkout"}))
        .await?;

    assert_eq!(refused["isError"], true, "{refused}");
    assert!(
        text(&refused).starts_with("ultrafast_required:"),
        "{refused}"
    );
    assert_eq!(
        server.state.sessions.count().await,
        0,
        "a refused CLI call leaves no task behind"
    );

    let mcp = server
        .call("Codex", "name_session", json!({"name": "checkout"}))
        .await?;
    assert!(
        !text(&mcp).contains("ultrafast_required"),
        "an MCP client is never refused by the CLI's gate: {mcp}"
    );
    Ok(())
}

#[tokio::test]
async fn a_cli_call_joins_the_task_its_group_names() -> anyhow::Result<()> {
    let server = Server::start().await?;
    server.state.ultrafast.set(true);
    let task = SessionId::new("cli-task-from-an-earlier-command");
    server
        .state
        .task_groups
        .bind(
            NewBinding {
                group_id: "G1",
                session_id: task.as_str(),
                agent_id: "browseros-cli-task",
                origin: TaskOrigin::Cli,
                window_id: None,
                profile_id: None,
            },
            now_ms(),
        )
        .await?;

    let result = server
        .call(
            CLI,
            "name_session",
            json!({"name": "checkout", "group": "G1"}),
        )
        .await?;

    assert_eq!(result["_meta"][GROUP_META], "G1", "{result}");
    assert!(
        result["_meta"][SESSION_META].is_null(),
        "the CLI is handed no session handle: {result}"
    );
    assert!(
        !text(&result).contains("browseros-neo session"),
        "no session bookkeeping reaches the agent's text: {result}"
    );
    let session = server
        .state
        .sessions
        .lookup(&task)
        .await
        .ok_or_else(|| anyhow::anyhow!("the group did not resolve to its task"))?;
    assert!(session.follows_group());
    Ok(())
}

#[tokio::test]
async fn an_mcp_client_keeps_its_handle_and_never_sees_a_group() -> anyhow::Result<()> {
    let server = Server::start().await?;
    server.state.ultrafast.set(true);

    let result = server
        .call("Codex", "name_session", json!({"name": "checkout"}))
        .await?;

    assert!(
        result["_meta"][SESSION_META].is_string(),
        "an MCP client still gets its handle: {result}"
    );
    assert!(result["_meta"][GROUP_META].is_null(), "{result}");
    Ok(())
}
