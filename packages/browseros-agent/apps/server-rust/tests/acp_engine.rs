//! `AcpEngine` and `POST /acpx/probe` over real stdio with acp-agents' scripted fake agent: 409 on a
//! busy conversation, one `error` chunk on preparation failure, lease re-mint only on `{agentId,
//! readOnly}` change, the transcript seed built from display history, and the probe route's TS
//! JSON (validation bodies compared against captured zod output). The live test drives real
//! Claude and Codex adapters (`BROWSEROS_ACP_LIVE=1 … -- --ignored`).

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use acp_agents::{AcpRuntimeConfig, FileSessionStore, HostEnvironment};
use axum::response::Response;
use futures_util::StreamExt;
use serde_json::{Value, json};
use server_rust::{
    chat::{
        AcpEngine, ChatEngine, ChatRejection, ChatTurnRequest, LeaseIssuer, RunHandle,
        SessionLease, Turn, TurnTarget,
        request::{Attachment, BrowserContext, ChatMode},
    },
    db::{AcpAgentRow, AgentKind},
    http::acpx_probe,
    mcp::{LeaseError, LeaseSpec},
};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use ui_message_stream::{ChunkStream, UiMessage};
use uuid::Uuid;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const FAKE_AGENT: &str = env!("CARGO_BIN_EXE_server-rust-fake-acp-agent");

#[derive(Debug, Clone, PartialEq)]
enum LeaseEvent {
    Mint { token: String, read_only: bool },
    Update { token: String },
    Revoke { token: String },
}

/// In-memory lease registry: records every mint, refresh and revocation.
#[derive(Default)]
struct FakeLeases {
    events: Arc<Mutex<Vec<LeaseEvent>>>,
}

impl FakeLeases {
    fn events(&self) -> Vec<LeaseEvent> {
        self.events
            .lock()
            .map(|events| events.clone())
            .unwrap_or_default()
    }
}

struct FakeLease {
    token: String,
    events: Arc<Mutex<Vec<LeaseEvent>>>,
}

struct NoGrants;

impl browseros_agent::tools::OutputGrants for NoGrants {
    fn allows(&self, _: &std::path::Path) -> bool {
        false
    }
}

impl SessionLease for FakeLease {
    fn token(&self) -> &str {
        &self.token
    }
    fn output_grants(&self) -> Arc<dyn browseros_agent::tools::OutputGrants> {
        Arc::new(NoGrants)
    }
    fn update_browser_context(&self, _: Option<&BrowserContext>) {
        record(
            &self.events,
            LeaseEvent::Update {
                token: self.token.clone(),
            },
        );
    }
    fn revoke(&self) {
        record(
            &self.events,
            LeaseEvent::Revoke {
                token: self.token.clone(),
            },
        );
    }
}

fn record(events: &Mutex<Vec<LeaseEvent>>, event: LeaseEvent) {
    if let Ok(mut events) = events.lock() {
        events.push(event);
    }
}

#[async_trait::async_trait]
impl LeaseIssuer for FakeLeases {
    async fn mint(&self, spec: LeaseSpec) -> Result<Box<dyn SessionLease>, LeaseError> {
        let token = format!(
            "lease-{}",
            self.events()
                .iter()
                .filter(|event| matches!(event, LeaseEvent::Mint { .. }))
                .count()
                + 1
        );
        record(
            &self.events,
            LeaseEvent::Mint {
                token: token.clone(),
                read_only: spec.read_only,
            },
        );
        Ok(Box::new(FakeLease {
            token,
            events: self.events.clone(),
        }))
    }
}

struct Fixture {
    dir: TempDir,
    log: PathBuf,
    leases: Arc<FakeLeases>,
}

impl Fixture {
    fn new() -> Result<Self, std::io::Error> {
        let dir = TempDir::new()?;
        let log = dir.path().join("agent.log");
        Ok(Self {
            dir,
            log,
            leases: Arc::default(),
        })
    }

    fn host(&self) -> HostEnvironment {
        HostEnvironment {
            resources_dir: None,
            browseros_dir: self.dir.path().join("browseros"),
            inherited_env: BTreeMap::from([
                ("PATH".into(), std::env::var("PATH").unwrap_or_default()),
                ("SHELL".into(), "/nonexistent/shell".into()),
            ]),
        }
    }

    fn engine(&self) -> AcpEngine {
        let store = Arc::new(FileSessionStore::new(self.dir.path().join("acp-sessions")));
        let mut config = AcpRuntimeConfig::new(self.host(), store);
        config.control_timeout = Duration::from_secs(5);
        AcpEngine::with_runtime(config, 9, self.leases.clone())
    }

    fn agent(&self, scenario: &str, id: u128) -> AcpAgentRow {
        let config = json!({
            "command": format!("'{FAKE_AGENT}' {scenario}"),
            "env": {"FAKE_AGENT_LOG": self.log.display().to_string()},
            "fullAccessModes": ["bypassPermissions"],
        });
        AcpAgentRow {
            id: Uuid::from_u128(id),
            kind: AgentKind::Custom,
            name: "Fake".into(),
            model_id: None,
            reasoning_effort: None,
            working_directory: None,
            custom_config: Some(config.to_string()),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn spawns(&self) -> usize {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .filter(|line| line.contains(" spawn "))
            .count()
    }
}

fn user(text: &str) -> UiMessage {
    UiMessage::user(Uuid::new_v4().to_string(), Some(text.into()), vec![])
}

fn turn(
    conversation: u128,
    agent: AcpAgentRow,
    history: Vec<UiMessage>,
    request: ChatTurnRequest,
) -> (Turn, CancellationToken) {
    let cancel = CancellationToken::new();
    let turn = Turn {
        conversation_id: Uuid::from_u128(conversation),
        run: RunHandle::new(Uuid::new_v4(), cancel.clone()),
        target: TurnTarget::Acp(agent),
        request: Arc::new(request),
        history,
        browser_context: None,
    };
    (turn, cancel)
}

async fn collect(stream: ChunkStream) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let chunks: Vec<_> = tokio::time::timeout(Duration::from_secs(15), stream.collect()).await?;
    Ok(chunks
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<_, _>>()?)
}

async fn run(engine: &AcpEngine, turn: Turn) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let stream = engine
        .start_turn(turn)
        .await
        .map_err(|error| error.to_string())?;
    collect(stream).await
}

fn reply(chunks: &[Value]) -> String {
    chunks
        .iter()
        .filter(|chunk| chunk["type"] == "text-delta")
        .filter_map(|chunk| chunk["delta"].as_str())
        .collect()
}

#[tokio::test]
async fn the_transcript_seed_comes_from_display_history() -> TestResult {
    let fx = Fixture::new()?;
    let engine = fx.engine();
    let assistant: UiMessage = serde_json::from_value(json!({
        "id": "a1", "role": "assistant", "parts": [
            {"type": "step-start"},
            {"type": "text", "text": "It shows a form.", "state": "done"},
            {"type": "tool-Terminal", "toolCallId": "t1", "state": "output-available",
             "input": {"command": "ls"}, "output": "file.txt", "providerExecuted": true},
            {"type": "tool-Pending", "toolCallId": "t2", "state": "input-streaming"},
        ],
    }))?;
    let mut history = vec![user("What is on the page?"), assistant, user("Fill it in")];
    let request = ChatTurnRequest {
        user_system_prompt: Some("  Be careful.  ".into()),
        ..ChatTurnRequest::default()
    };
    let (first, _) = turn(1, fx.agent("default", 0xa), history.clone(), request);
    let chunks = run(&engine, first).await?;
    assert_eq!(chunks[0]["type"], "start");
    assert_eq!(
        reply(&chunks),
        "echo:User: What is on the page?\n\
         Assistant: It shows a form. [Tool call: Terminal({\"command\":\"ls\"})] [Tool result (t1): file.txt]\n\
         User: Be careful.\n\n<USER_QUERY>\nFill it in\n</USER_QUERY>"
    );
    assert_eq!(
        chunks.last(),
        Some(&json!({"type":"finish", "finishReason":"stop"}))
    );

    history.push(serde_json::from_value(
        json!({"id":"a2", "role":"assistant", "parts":[{"type":"text", "text":"ok"}]}),
    )?);
    history.push(user("next"));
    let (second, _) = turn(
        1,
        fx.agent("default", 0xa),
        history,
        ChatTurnRequest::default(),
    );
    let chunks = run(&engine, second).await?;
    assert_eq!(
        reply(&chunks),
        "echo:User: <USER_QUERY>\nnext\n</USER_QUERY>",
        "the live agent has context"
    );
    assert_eq!(fx.spawns(), 1);
    engine.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn image_attachments_ride_as_prompt_blocks_and_unsupported_files_fail_the_turn() -> TestResult
{
    let fx = Fixture::new()?;
    let engine = fx.engine();
    let attachment = |media_type: &str, data: &str| -> Result<Attachment, serde_json::Error> {
        serde_json::from_value(json!({"mediaType": media_type, "data": data}))
    };
    let request = ChatTurnRequest {
        attachments: vec![
            attachment("image/png", "iVBORw0KGgo=")?,
            attachment("text/plain", "data:text/plain;base64,aGVsbG8=")?,
        ],
        ..ChatTurnRequest::default()
    };
    let (with_files, _) = turn(1, fx.agent("default", 0xa), vec![user("look")], request);
    let chunks = run(&engine, with_files).await?;
    assert_eq!(
        reply(&chunks),
        "echo:User: <USER_QUERY>\nlook\n</USER_QUERY> [File: text/plain]\nhello|<image>"
    );

    let request = ChatTurnRequest {
        attachments: vec![attachment("application/pdf", "JVBERi0=")?],
        ..ChatTurnRequest::default()
    };
    let (pdf, _) = turn(
        2,
        fx.agent("default", 0xa),
        vec![user("read this")],
        request,
    );
    let chunks = run(&engine, pdf).await?;
    assert_eq!(chunks[0]["type"], "start");
    assert_eq!(
        &chunks[1..],
        [
            json!({"type":"start-step"}),
            json!({"type":"error", "errorText":"The ACP agent failed to respond."}),
            json!({"type":"finish-step"}),
            json!({"type":"finish", "finishReason":"error"}),
        ]
    );
    engine.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn a_busy_conversation_is_a_409_rejection() -> TestResult {
    let fx = Fixture::new()?;
    let engine = fx.engine();
    let (first, cancel) = turn(
        1,
        fx.agent("default", 0xa),
        vec![user("hang")],
        ChatTurnRequest::default(),
    );
    let mut stream = engine
        .start_turn(first)
        .await
        .map_err(|error| error.to_string())?;
    while let Some(chunk) = stream.next().await {
        if serde_json::to_value(&chunk)?["delta"] == "started" {
            break;
        }
    }
    let (second, _) = turn(
        1,
        fx.agent("default", 0xa),
        vec![user("again")],
        ChatTurnRequest::default(),
    );
    assert!(matches!(
        engine.start_turn(second).await,
        Err(ChatRejection::Busy)
    ));
    cancel.cancel();
    let rest = collect(stream).await?;
    assert_eq!(
        rest,
        [json!({"type":"abort", "reason":"AbortError: Conversation stopped"})]
    );
    engine.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn a_preparation_failure_is_a_single_error_chunk() -> TestResult {
    let fx = Fixture::new()?;
    let engine = fx.engine();
    let (auth, _) = turn(
        1,
        fx.agent("auth", 0xa),
        vec![user("hi")],
        ChatTurnRequest::default(),
    );
    assert_eq!(
        run(&engine, auth).await?,
        [json!({"type":"error", "errorText":"Unable to start the ACP agent."})]
    );
    let mut broken = fx.agent("default", 0xb);
    broken.custom_config = Some("{\"command\": 7}".into());
    let (missing, _) = turn(2, broken, vec![user("hi")], ChatTurnRequest::default());
    assert_eq!(
        run(&engine, missing).await?,
        [json!({"type":"error", "errorText":"Unable to start the ACP agent."})]
    );
    engine.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn the_lease_is_reminted_only_when_agent_or_read_only_changes() -> TestResult {
    let fx = Fixture::new()?;
    let engine = fx.engine();
    let agent_mode = ChatTurnRequest::default;
    let chat_mode = || ChatTurnRequest {
        mode: ChatMode::Chat,
        ..ChatTurnRequest::default()
    };
    for (agent, request) in [
        (0xa, agent_mode()),
        (0xa, agent_mode()),
        (0xa, chat_mode()),
        (0xb, chat_mode()),
    ] {
        let (next, _) = turn(1, fx.agent("default", agent), vec![user("hi")], request);
        run(&engine, next).await?;
    }
    let token = |n: u32| format!("lease-{n}");
    assert_eq!(
        fx.leases.events(),
        [
            LeaseEvent::Mint {
                token: token(1),
                read_only: false
            },
            LeaseEvent::Update { token: token(1) },
            LeaseEvent::Revoke { token: token(1) },
            LeaseEvent::Mint {
                token: token(2),
                read_only: true
            },
            LeaseEvent::Revoke { token: token(2) },
            LeaseEvent::Mint {
                token: token(3),
                read_only: true
            },
        ]
    );
    // Same lease: same process. read_only changes the MCP URL, so the agent restarts.
    assert_eq!(fx.spawns(), 3);

    assert!(engine.forget(Uuid::from_u128(1)).await);
    assert_eq!(
        fx.leases.events().last(),
        Some(&LeaseEvent::Revoke { token: token(3) })
    );
    assert!(!engine.forget(Uuid::from_u128(1)).await);
    engine.shutdown().await;
    Ok(())
}

async fn body(response: Response) -> Result<(u16, Value), Box<dyn std::error::Error>> {
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await?;
    Ok((status, serde_json::from_slice(&bytes)?))
}

#[tokio::test]
async fn probe_validation_matches_the_captured_zod_bodies() -> TestResult {
    let fx = Fixture::new()?;
    let fixture: Value = serde_json::from_str(include_str!("fixtures/acpx-probe-validation.json"))?;
    let cases = fixture["cases"].as_array().ok_or("cases")?;
    assert!(cases.len() >= 15);
    for case in cases {
        let (status, response) =
            body(acpx_probe::respond(case["body"].clone(), &fx.host(), None).await).await?;
        assert_eq!(status, 400, "{}", case["body"]);
        // The issue list is a pretty-printed string: compare bytes, not parsed values.
        assert_eq!(response, case["response"], "{}", case["body"]);
    }
    Ok(())
}

#[tokio::test]
async fn probe_reports_the_agent_and_wraps_launch_failures() -> TestResult {
    let fx = Fixture::new()?;
    let cwd = fx.dir.path().join("probe-cwd");
    std::fs::create_dir_all(&cwd)?;
    let request = json!({
        "type": "custom",
        "command": format!("  '{FAKE_AGENT}' default  "),
        "env": {"FAKE_AGENT_LOG": fx.log.display().to_string()},
        "cwd": format!("  {}  ", cwd.display()),
        "timeoutMs": 30000,
    });
    let (status, response) = body(acpx_probe::respond(request, &fx.host(), None).await).await?;
    assert_eq!(status, 200);
    assert_eq!(
        response,
        json!({
            "models":[{"id":"fake-default", "name":"Fake default"}, {"id":"fake-pro", "name":"Fake Pro", "description":"Bigger"}],
            "reasoning":{"values":["low", "medium", "high"], "defaultValue":"medium"},
            "supportsConfigOption":true,
            "agentInfo":{"name":"fake-agent", "title":"Fake Agent", "version":"9.9.9"},
            "protocolVersion":1,
        })
    );
    let log = std::fs::read_to_string(&fx.log)?;
    assert!(
        log.contains(&format!("cwd={}", cwd.canonicalize()?.display())),
        "trimmed cwd: {log}"
    );

    let (status, response) = body(
        acpx_probe::respond(
            json!({"type":"custom", "command":"agent 'oops"}),
            &fx.host(),
            None,
        )
        .await,
    )
    .await?;
    assert_eq!(status, 500);
    assert_eq!(
        response,
        json!({"error":{"code":"wrapper_error", "message":"Command has an unterminated quote"}})
    );
    Ok(())
}

/// Real adapters, launched as production does (bundled Bun from the installed app when present).
/// One turn, a fresh engine (process restart), then a recall that only a resumed session answers.
#[tokio::test]
#[ignore = "requires BROWSEROS_ACP_LIVE=1 and logged-in Claude Code + Codex"]
#[allow(
    clippy::print_stderr,
    reason = "the live run reports what each real agent replied"
)]
async fn live_claude_and_codex_through_the_engine() -> TestResult {
    if std::env::var("BROWSEROS_ACP_LIVE").as_deref() != Ok("1") {
        return Err("set BROWSEROS_ACP_LIVE=1 to authorize live turns".into());
    }
    const APP_RESOURCES: &str =
        "/Applications/BrowserOS.app/Contents/Resources/BrowserOSServer/default/resources";
    for (kind, model, effort) in [
        (AgentKind::Claude, "haiku", None),
        (AgentKind::Codex, "gpt-6-sol", Some("low")),
    ] {
        let fx = Fixture::new()?;
        let resources = Some(PathBuf::from(APP_RESOURCES)).filter(|dir| dir.is_dir());
        let host =
            HostEnvironment::from_process(resources, std::env::temp_dir().join("acp-agents-live"));
        let store = || Arc::new(FileSessionStore::new(fx.dir.path().join("acp-sessions")));
        let engine = || {
            AcpEngine::with_runtime(
                AcpRuntimeConfig::new(host.clone(), store()),
                9,
                fx.leases.clone(),
            )
        };
        let agent = AcpAgentRow {
            id: Uuid::new_v4(),
            kind,
            name: "Live".into(),
            model_id: Some(model.into()),
            reasoning_effort: effort.map(String::from),
            working_directory: None,
            custom_config: None,
            created_at: 0,
            updated_at: 0,
        };
        let mut history = vec![user(
            "Reply with exactly the single word pong and nothing else.",
        )];
        let first = engine();
        let (one, _) = turn(
            1,
            agent.clone(),
            history.clone(),
            ChatTurnRequest::default(),
        );
        let chunks = run(&first, one).await?;
        first.shutdown().await;
        let answer = reply(&chunks);
        eprintln!(
            "[{kind:?}] first reply={answer:?} last={}",
            chunks.last().unwrap_or(&Value::Null)
        );
        assert!(answer.to_lowercase().contains("pong"), "{chunks:#?}");

        history.push(serde_json::from_value(
            json!({"id":"a1", "role":"assistant", "parts":[{"type":"text", "text":answer}]}),
        )?);
        history.push(user("What single word did you reply with to my previous message? Answer with only that word."));
        let second = engine();
        let (two, _) = turn(1, agent, history, ChatTurnRequest::default());
        let chunks = run(&second, two).await?;
        second.shutdown().await;
        let answer = reply(&chunks);
        eprintln!("[{kind:?}] resumed reply={answer:?}");
        assert!(answer.to_lowercase().contains("pong"), "{chunks:#?}");
    }
    Ok(())
}
