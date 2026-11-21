//! The production Agent across its subprocess boundary. The fake launcher replays ACP
//! exactly as a child would, including a tool update received after a cancelled response.

#![cfg(unix)]

mod support;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::Result;
use axum::http::{Method, StatusCode};
use browseros_server::agent::{Agent, AgentRuntime};
use browseros_server::browser::Browser;
use browseros_server::store::{AgentPatch, Edit, NewAgent};
use browseros_server::turn::{
    AgentKind, Attachment, ChatErrorCode, EventSink, FinishReason, Mode, Target, TurnContext,
    TurnEnd, TurnEvent, TurnInput, TurnRunner, UiMessage,
};
use serde_json::{Value, json};
use support::{TestServer, body_json};
use tokio_util::sync::CancellationToken;

struct Fixture {
    server: TestServer,
    runtime: AgentRuntime,
}

impl Fixture {
    async fn new() -> Result<Self> {
        let server = TestServer::start().await?;
        let root = server.state_dir().join("fake adapter");
        std::fs::create_dir(&root)?;
        let launcher = root.join("launcher.py");
        for name in [
            "launcher.py",
            "success.ndjson",
            "cancel.ndjson",
            "auth.ndjson",
        ] {
            std::fs::copy(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/acp")
                    .join(name),
                root.join(name),
            )?;
        }
        std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o700))?;
        let runtime = AgentRuntime {
            bun: Some(launcher),
            state_dir: server.state_dir().join("agents"),
        };
        Ok(Self { server, runtime })
    }

    fn agent(&self) -> Agent {
        let (browser, _) = Browser::scripted(Vec::new());
        Agent::new(
            self.server.app.models.clone(),
            browser,
            self.server.app.store.clone(),
            self.runtime.clone(),
        )
    }

    async fn target(&self, kind: AgentKind) -> Result<Target> {
        let row = self
            .server
            .app
            .store
            .agents()
            .create(NewAgent {
                kind,
                name: "Fixture".into(),
                model_id: Some("first-model".into()),
                reasoning_effort: None,
                working_directory: None,
            })
            .await?;
        Ok(Target::Coding {
            kind,
            agent_id: row.id,
        })
    }

    fn session_dir(&self) -> PathBuf {
        self.runtime.state_dir.join("sessions")
    }

    fn calls(&self) -> Result<Vec<Value>> {
        let lines =
            std::fs::read_to_string(self.server.state_dir().join("fake adapter/calls.ndjson"))?;
        lines
            .lines()
            .map(|line| serde_json::from_str(line).map_err(Into::into))
            .collect()
    }
}

fn input(text: &str) -> Result<TurnInput> {
    let messages: Vec<UiMessage> = serde_json::from_value(json!([
        {"id":"1", "role":"assistant", "parts":[{"type":"text","text":"earlier reply"}]},
        {"id":"2", "role":"user", "parts":[{"type":"text","text":text}]}
    ]))?;
    Ok(TurnInput {
        conversation: uuid::Uuid::new_v4(),
        history: messages.into(),
        context: TurnContext {
            mode: Mode::Chat,
            window_id: None,
            active_tab: None,
            selected_tabs: Vec::new(),
            selected_text: None,
            user_system_prompt: None,
            working_dir: None,
            supports_reasoning: false,
        },
        attachments: Vec::new(),
        ephemeral: false,
    })
}

#[tokio::test]
async fn replies_follow_prompt_id_and_tools_are_sealed_after_input() -> Result<()> {
    let fixture = Fixture::new().await?;
    let agent = fixture.agent();
    let target = fixture.target(AgentKind::Codex).await?;
    let mut events = Vec::new();
    let end = agent
        .run_turn(
            &target,
            &input("read the fixture")?,
            &mut events,
            &CancellationToken::new(),
        )
        .await;
    assert_eq!(end, TurnEnd::Done(FinishReason::Stop));
    assert_eq!(events.first(), Some(&TurnEvent::StepStart));
    assert_eq!(events.last(), Some(&TurnEvent::StepEnd));

    let text: String = events
        .iter()
        .filter_map(|event| match event {
            TurnEvent::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(text.contains("☂ violet-otter-813"));
    let finished: Vec<_> = events
        .iter()
        .filter(|event| match event {
            TurnEvent::ToolStarted { id, .. }
            | TurnEvent::ToolInput { id, .. }
            | TurnEvent::ToolDone { id, .. } => id.0 == "already-finished",
            _ => false,
        })
        .collect();
    assert!(matches!(finished.as_slice(), [
        TurnEvent::ToolStarted { .. },
        TurnEvent::ToolInput { input: Value::Null, .. },
        TurnEvent::ToolDone { result: Ok(value), .. },
    ] if value == "done"));
    Ok(())
}

#[tokio::test]
async fn restarting_reattaches_model_edits_and_delete_closes_the_same_owner() -> Result<()> {
    let fixture = Fixture::new().await?;
    let target = fixture.target(AgentKind::Codex).await?;
    let Target::Coding { agent_id, .. } = &target else {
        anyhow::bail!("expected coding target");
    };
    let input = input("next message")?;
    let name = input.conversation.to_string();
    let locator = fixture.session_dir().join(format!("{name}.json"));
    {
        let end = fixture
            .agent()
            .run_turn(&target, &input, &mut Vec::new(), &CancellationToken::new())
            .await;
        assert_eq!(end, TurnEnd::Done(FinishReason::Stop));
    }
    let route: Value = serde_json::from_slice(&std::fs::read(&locator)?)?;
    assert_eq!(route["adapter"], "codex");
    assert_eq!(route["agent_id"], *agent_id);
    assert!(
        route["mcp_url"]
            .as_str()
            .is_some_and(|url| url.ends_with(&format!("/mcp/{name}")))
    );

    fixture
        .server
        .app
        .store
        .agents()
        .update(
            agent_id,
            AgentPatch {
                model_id: Edit::Set("edited-model".into()),
                ..AgentPatch::default()
            },
        )
        .await?;
    let restarted = fixture.agent();
    let end = restarted
        .run_turn(&target, &input, &mut Vec::new(), &CancellationToken::new())
        .await;
    assert_eq!(end, TurnEnd::Done(FinishReason::Stop));
    restarted.end_session(input.conversation).await;

    let calls = fixture.calls()?;
    let models: Vec<&Value> = calls
        .iter()
        .filter(|call| call["operation"] == "set")
        .collect();
    assert_eq!(models.len(), 2);
    assert_eq!(
        models[0]["args"],
        json!(["set", "-s", name, "--", "model", "first-model"])
    );
    assert_eq!(
        models[1]["args"],
        json!(["set", "-s", name, "--", "model", "edited-model"])
    );
    let prompts: Vec<&str> = calls
        .iter()
        .filter_map(|call| call["prompt"].as_str())
        .collect();
    assert!(prompts[0].contains("earlier reply"));
    assert!(!prompts[1].contains("earlier reply"));
    for call in calls.iter().filter(|call| call["operation"].is_string()) {
        assert_eq!(call["adapter"], route["command"]);
        assert_eq!(call["cwd"], route["cwd"]);
    }
    let close = calls
        .last()
        .ok_or_else(|| anyhow::anyhow!("acpx had no calls"))?;
    assert_eq!(close["args"], json!(["sessions", "close", name]));
    assert!(!locator.exists());
    assert!(!fixture.session_dir().join(name).exists());
    Ok(())
}

struct StopAtTool {
    cancel: CancellationToken,
    events: Vec<TurnEvent>,
}

#[async_trait::async_trait]
impl EventSink for StopAtTool {
    async fn emit(&mut self, event: TurnEvent) {
        if matches!(event, TurnEvent::ToolInput { .. }) {
            self.cancel.cancel();
        }
        self.events.push(event);
    }
}

#[tokio::test]
async fn cancelling_waits_for_the_owner_and_discards_the_late_completion() -> Result<()> {
    let fixture = Fixture::new().await?;
    let agent = fixture.agent();
    let target = fixture.target(AgentKind::Codex).await?;
    let input = input("cancel this turn")?;
    let cancel = CancellationToken::new();
    let mut sink = StopAtTool {
        cancel: cancel.clone(),
        events: Vec::new(),
    };
    let end = tokio::time::timeout(
        Duration::from_secs(6),
        agent.run_turn(&target, &input, &mut sink, &cancel),
    )
    .await?;
    assert_eq!(end, TurnEnd::Stopped);
    assert!(
        sink.events
            .iter()
            .any(|event| matches!(event, TurnEvent::ToolInput { .. }))
    );
    assert!(
        !sink
            .events
            .iter()
            .any(|event| matches!(event, TurnEvent::ToolDone { .. } | TurnEvent::StepEnd))
    );

    let calls = fixture.calls()?;
    let prompt = calls
        .iter()
        .find(|call| call["operation"] == "prompt")
        .ok_or_else(|| anyhow::anyhow!("no prompt"))?;
    let cancel = calls
        .iter()
        .find(|call| call["operation"] == "cancel")
        .ok_or_else(|| anyhow::anyhow!("no cancellation"))?;
    assert_eq!(prompt["adapter"], cancel["adapter"]);
    assert_eq!(prompt["mcp"], cancel["mcp"]);
    assert_eq!(cancel["args"], json!(["cancel", "-s", input.conversation]));
    let pid = prompt["pid"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("no child pid"))?;
    assert!(
        !Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()?
            .success()
    );
    Ok(())
}

#[tokio::test]
async fn incognito_seeds_history_and_cleans_its_session_and_attachments() -> Result<()> {
    let fixture = Fixture::new().await?;
    let agent = fixture.agent();
    let target = fixture.target(AgentKind::Codex).await?;
    let mut input = input("refuse this turn")?;
    input.ephemeral = true;
    input.attachments.push(Attachment {
        media_type: "text/plain".into(),
        data_url: "data:text/plain;base64,Zml4dHVyZQ==".into(),
    });
    let end = agent
        .run_turn(&target, &input, &mut Vec::new(), &CancellationToken::new())
        .await;
    assert_eq!(end, TurnEnd::Done(FinishReason::Other));
    let calls = fixture.calls()?;
    let ensure = &calls[0];
    let name = ensure["args"][3]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("no incognito name"))?;
    assert!(name.starts_with("incognito-"));
    assert!(
        calls
            .iter()
            .any(|call| call["args"] == json!(["sessions", "close", name]))
    );
    let text = calls
        .iter()
        .find_map(|call| call["prompt"].as_str())
        .ok_or_else(|| anyhow::anyhow!("no prompt"))?;
    assert!(text.contains("earlier reply") && text.contains("attachment-1.txt"));
    assert_eq!(std::fs::read_dir(fixture.session_dir())?.count(), 0);
    assert_eq!(
        std::fs::read_dir(fixture.runtime.state_dir.join("attachments"))?.count(),
        0
    );
    Ok(())
}

#[tokio::test]
async fn claude_authentication_required_becomes_an_auth_error_card() -> Result<()> {
    let fixture = Fixture::new().await?;
    let target = fixture.target(AgentKind::Claude).await?;
    let end = fixture
        .agent()
        .run_turn(
            &target,
            &input("hello")?,
            &mut Vec::new(),
            &CancellationToken::new(),
        )
        .await;
    assert!(
        matches!(end, TurnEnd::Failed(error) if error.code == ChatErrorCode::AuthFailed && error.message.contains("Sign in"))
    );
    Ok(())
}

#[tokio::test]
async fn the_probe_route_normalizes_codex_choices_and_keeps_failures_on_the_success_wire()
-> Result<()> {
    let mut fixture = Fixture::new().await?;
    fixture.server.app.agent = fixture.agent();
    fixture.server.router = browseros_server::http::router(fixture.server.app.clone());
    let probe = |body: Value| fixture.server.json(Method::POST, "/acpx/probe", &body);

    let result = fixture.server.send(probe(json!({"type":"codex"}))).await;
    assert_eq!(result.status(), StatusCode::OK);
    let body = body_json(result).await;
    assert_eq!(body["models"].as_array().map(Vec::len), Some(1));
    assert_eq!(body["models"][0]["id"], "fixture");
    assert_eq!(
        body["reasoning"],
        json!({"values":["low","high"], "defaultValue":"high"})
    );
    assert_eq!(body["protocolVersion"], 1);

    let result = fixture.server.send(probe(json!({"type":"claude"}))).await;
    assert_eq!(result.status(), StatusCode::OK);
    assert_eq!(
        body_json(result).await,
        json!({
            "models":[], "reasoning":null, "supportsConfigOption":false, "agentInfo":null,
            "protocolVersion":0, "error":{
                "code":"acp_error", "acpErrorCode":-32000,
                "message":"The coding agent rejected the session/new request."
            },
        })
    );
    let result = fixture.server.send(probe(json!({"type":"custom"}))).await;
    assert_eq!(result.status(), StatusCode::BAD_REQUEST);
    assert!(body_json(result).await["error"].is_string());
    let result = fixture
        .server
        .send(probe(json!({"type":"codex","timeoutMs":0})))
        .await;
    assert_eq!(result.status(), StatusCode::BAD_REQUEST);
    Ok(())
}
