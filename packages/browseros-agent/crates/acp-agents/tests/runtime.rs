//! `AcpRuntime` over real stdio with the scripted fake agent: busy lock, transcript seeding,
//! process reuse, reattach across runtime restarts, resume recovery before output, idle close,
//! explicit close, crashes, deadlines and dropped streams. The fake agent logs every call per pid.

use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};

use acp_agents::{
    AcpRuntime, AcpRuntimeConfig, AgentDefinition, AgentKind, CustomAgentConfig, CustomMcpServer,
    FileSessionStore, HostEnvironment, LaunchError, RuntimeError, SessionKey, SessionStore,
    TurnContext, TurnPrompt, TurnRequest, UiChunkStream,
};
use acp_client::{
    AcpError,
    schema::{ContentBlock, TextContent},
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const AGENT: Uuid = Uuid::from_u128(0xa);

struct Fixture {
    dir: TempDir,
    log: PathBuf,
}

impl Fixture {
    fn new() -> Result<Self, std::io::Error> {
        let dir = TempDir::new()?;
        let log = dir.path().join("agent.log");
        Ok(Self { dir, log })
    }

    fn store(&self) -> FileSessionStore {
        FileSessionStore::new(self.dir.path().join("acp-sessions"))
    }

    fn runtime(&self, configure: impl FnOnce(&mut AcpRuntimeConfig)) -> Arc<AcpRuntime> {
        let host = HostEnvironment {
            resources_dir: None,
            browseros_dir: self.dir.path().join("browseros"),
            inherited_env: BTreeMap::from([
                ("PATH".into(), std::env::var("PATH").unwrap_or_default()),
                ("SHELL".into(), "/nonexistent/shell".into()),
            ]),
        };
        let mut config = AcpRuntimeConfig::new(host, Arc::new(self.store()));
        config.control_timeout = Duration::from_secs(5);
        configure(&mut config);
        AcpRuntime::new(config)
    }

    fn ctx(&self, scenario: &str, conversation: u128) -> TurnContext {
        let command = format!(
            "'{}' {scenario}",
            env!("CARGO_BIN_EXE_acp-agents-fake-agent")
        );
        TurnContext {
            agent: AgentDefinition {
                id: AGENT,
                kind: AgentKind::Custom,
                model_id: Some("fake-pro".into()),
                reasoning_effort: Some("high".into()),
                working_directory: None,
                custom: Some(CustomAgentConfig {
                    command,
                    env: BTreeMap::from([(
                        "FAKE_AGENT_LOG".into(),
                        self.log.display().to_string(),
                    )]),
                    full_access_modes: vec!["missing-mode".into(), "bypassPermissions".into()],
                    reasoning_effort_key: None,
                    system_prompt_append: None,
                }),
            },
            conversation_id: Uuid::from_u128(conversation),
            server_port: 9,
            browser_tool_lease_token: "lease-1".into(),
            read_only: false,
            custom_mcp_servers: vec![CustomMcpServer {
                name: "notes".into(),
                url: "http://127.0.0.1:9/notes".into(),
            }],
            working_dir_override: None,
        }
    }

    fn lines(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    /// Log lines with the pid replaced by its spawn order (`p1`, `p2`, …), so assertions can say
    /// "same process" or "new process" without knowing pids.
    fn calls(&self) -> Vec<String> {
        let lines = self.lines();
        let mut pids: Vec<&str> = Vec::new();
        for line in &lines {
            let pid = line.split(' ').next().unwrap_or_default();
            if !pids.contains(&pid) {
                pids.push(pid);
            }
        }
        lines
            .iter()
            .map(|line| {
                let mut line = line.clone();
                for (index, pid) in pids.iter().enumerate() {
                    line = line
                        .replacen(&format!("{pid} "), &format!("p{} ", index + 1), 1)
                        .replace(&format!("sess-{pid}-"), &format!("sess-p{}-", index + 1));
                }
                line
            })
            .collect()
    }

    /// The raw id of the most recently created session.
    fn newest_session(&self) -> Option<String> {
        self.lines().iter().rev().find_map(|line| {
            let (_, rest) = line.split_once(" new ")?;
            rest.split(' ').next().map(String::from)
        })
    }

    fn spawns(&self) -> usize {
        self.lines()
            .iter()
            .filter(|line| line.contains(" spawn "))
            .count()
    }

    async fn wait_for(&self, needle: &str) -> TestResult {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while !self.lines().iter().any(|line| line.contains(needle)) {
            if tokio::time::Instant::now() > deadline {
                return Err(format!("log never contained {needle:?}: {:#?}", self.calls()).into());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok(())
    }
}

fn request(ctx: TurnContext, text: &str, seed: Option<&str>) -> (TurnRequest, CancellationToken) {
    let cancel = CancellationToken::new();
    let request = TurnRequest {
        ctx,
        prompt: TurnPrompt {
            latest_user: vec![ContentBlock::Text(TextContent::new(format!(
                "User: {text}"
            )))],
            transcript_seed: seed.map(String::from),
        },
        response_message_id: "response-1".into(),
        cancel: cancel.clone(),
    };
    (request, cancel)
}

async fn collect(stream: UiChunkStream) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let chunks: Vec<_> = tokio::time::timeout(Duration::from_secs(15), stream.collect()).await?;
    Ok(chunks
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<_, _>>()?)
}

async fn turn(
    runtime: &Arc<AcpRuntime>,
    ctx: TurnContext,
    text: &str,
    seed: Option<&str>,
) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let (request, _cancel) = request(ctx, text, seed);
    collect(runtime.stream(request).await?).await
}

fn reply(chunks: &[Value]) -> String {
    chunks
        .iter()
        .filter(|chunk| chunk["type"] == "text-delta")
        .filter_map(|chunk| chunk["delta"].as_str())
        .collect()
}

const SEED: &str = "User: earlier\nAssistant: noted";

#[tokio::test]
async fn first_turn_seeds_and_applies_controls_then_reuses_the_process() -> TestResult {
    let fx = Fixture::new()?;
    let runtime = fx.runtime(|_| {});

    let chunks = turn(&runtime, fx.ctx("default", 1), "hi", Some(SEED)).await?;
    assert_eq!(
        chunks,
        [
            json!({"type":"start", "messageId":"response-1"}),
            json!({"type":"start-step"}),
            json!({"type":"text-start", "id":"acpx-1"}),
            json!({"type":"text-delta", "id":"acpx-1", "delta":"echo:User: earlier\nAssistant: noted\nUser: hi"}),
            json!({"type":"text-end", "id":"acpx-1"}),
            json!({"type":"finish-step"}),
            json!({"type":"finish", "finishReason":"stop"}),
        ]
    );
    let workspace = fx.dir.path().join("browseros/agents/acp-workspace");
    assert!(workspace.join("CLAUDE.md").is_file());
    assert_eq!(
        fx.calls(),
        [
            format!("p1 spawn default cwd={}", display_cwd(&workspace)?),
            "p1 initialize".into(),
            "p1 new sess-p1-1 mcp=browseros@http://127.0.0.1:9/mcp#1,notes@http://127.0.0.1:9/notes#0 meta={\"claudeCode\":{\"options\":{\"model\":\"fake-pro\"}}}".into(),
            "p1 set_config sess-p1-1 model=fake-pro".into(),
            "p1 set_config sess-p1-1 effort=high".into(),
            "p1 set_mode sess-p1-1 missing-mode".into(),
            "p1 set_mode sess-p1-1 bypassPermissions".into(),
            "p1 prompt sess-p1-1 User: earlier\\nAssistant: noted\\nUser: hi".into(),
        ]
    );

    let chunks = turn(&runtime, fx.ctx("default", 1), "again", Some(SEED)).await?;
    assert_eq!(
        reply(&chunks),
        "echo:User: again",
        "the live agent has context: no seed"
    );
    assert_eq!(fx.spawns(), 1, "same process");
    let record = fx
        .store()
        .load(&SessionKey::new(AGENT, Uuid::from_u128(1)))?
        .ok_or("record")?;
    assert_eq!(Some(record.agent_session_id), fx.newest_session());
    assert!(record.has_user_turn);
    runtime.shutdown().await;
    fx.wait_for("eof").await?;
    Ok(())
}

#[tokio::test]
async fn a_busy_conversation_is_rejected_until_its_stream_ends() -> TestResult {
    let fx = Fixture::new()?;
    let runtime = fx.runtime(|_| {});
    let (first, cancel) = request(fx.ctx("default", 1), "hang", None);
    let mut first = runtime.stream(first).await?;
    while let Some(chunk) = first.next().await {
        if serde_json::to_value(&chunk)?["delta"] == "started" {
            break;
        }
    }
    let (second, _) = request(fx.ctx("default", 1), "hi", None);
    assert!(matches!(
        runtime.stream(second).await,
        Err(RuntimeError::Busy)
    ));
    let other = turn(&runtime, fx.ctx("default", 2), "elsewhere", None).await?;
    assert_eq!(reply(&other), "echo:User: elsewhere");

    cancel.cancel();
    let rest = collect(first).await?;
    assert_eq!(
        rest,
        [json!({"type":"abort", "reason":"AbortError: Conversation stopped"})],
        "nothing after the host stop is forwarded"
    );
    fx.wait_for("cancel").await?;
    let after = turn(&runtime, fx.ctx("default", 1), "after", Some(SEED)).await?;
    assert_eq!(
        reply(&after),
        format!("echo:{SEED}\nUser: after"),
        "TS quirk kept: an aborted first turn leaves the live session unmarked, so it is seeded again"
    );
    runtime.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn a_restarted_runtime_resumes_the_recorded_session_without_seeding() -> TestResult {
    let fx = Fixture::new()?;
    let first = fx.runtime(|_| {});
    turn(&first, fx.ctx("default", 1), "hi", Some(SEED)).await?;
    first.shutdown().await;

    let second = fx.runtime(|_| {});
    let chunks = turn(&second, fx.ctx("default", 1), "back", Some(SEED)).await?;
    assert_eq!(reply(&chunks), "echo:User: back");
    let calls = fx.calls();
    assert!(
        calls.contains(
            &"p2 resume sess-p1-1 meta={\"claudeCode\":{\"options\":{\"model\":\"fake-pro\"}}}"
                .to_owned()
        ),
        "{calls:#?}"
    );
    assert!(
        calls.contains(&"p2 set_mode sess-p1-1 bypassPermissions".to_owned()),
        "controls re-applied after reattach"
    );
    assert!(
        !calls.iter().any(|call| call.starts_with("p2 new")),
        "{calls:#?}"
    );
    second.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn resume_recovery_starts_a_fresh_seeded_session_when_controls_fail_after_reattach()
-> TestResult {
    let fx = Fixture::new()?;
    let first = fx.runtime(|_| {});
    turn(&first, fx.ctx("mode-fails-after-resume", 1), "hi", None).await?;
    first.shutdown().await;

    let second = fx.runtime(|_| {});
    let chunks = turn(
        &second,
        fx.ctx("mode-fails-after-resume", 1),
        "back",
        Some(SEED),
    )
    .await?;
    assert_eq!(
        reply(&chunks),
        format!("echo:{SEED}\nUser: back"),
        "the failed attempt is invisible and the fresh session gets the transcript"
    );
    assert!(!chunks.iter().any(|chunk| chunk["type"] == "error"));
    let calls = fx.calls();
    assert!(
        calls.contains(&"p2 set_mode sess-p1-1 bypassPermissions".to_owned()),
        "{calls:#?}"
    );
    assert!(
        calls
            .iter()
            .any(|call| call.starts_with("p3 new sess-p3-1")),
        "fresh process + session: {calls:#?}"
    );
    let record = fx
        .store()
        .load(&SessionKey::new(AGENT, Uuid::from_u128(1)))?
        .ok_or("record")?;
    assert_eq!(Some(record.agent_session_id.clone()), fx.newest_session());
    assert!(record.has_user_turn && !record.reset_on_next_ensure);
    second.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn a_refused_resume_falls_back_to_a_new_seeded_session() -> TestResult {
    let fx = Fixture::new()?;
    let first = fx.runtime(|_| {});
    turn(&first, fx.ctx("resume-fails", 1), "hi", None).await?;
    first.shutdown().await;
    let second = fx.runtime(|_| {});
    let chunks = turn(&second, fx.ctx("resume-fails", 1), "back", Some(SEED)).await?;
    assert_eq!(reply(&chunks), format!("echo:{SEED}\nUser: back"));
    let calls = fx.calls();
    assert!(
        calls.contains(
            &"p2 resume sess-p1-1 meta={\"claudeCode\":{\"options\":{\"model\":\"fake-pro\"}}}"
                .to_owned()
        )
    );
    assert!(
        calls
            .iter()
            .any(|call| call.starts_with("p2 new sess-p2-1")),
        "same process, new session: {calls:#?}"
    );
    second.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn session_load_is_used_without_resume_and_its_replay_is_swallowed() -> TestResult {
    let fx = Fixture::new()?;
    let first = fx.runtime(|_| {});
    turn(&first, fx.ctx("no-resume", 1), "hi", None).await?;
    first.shutdown().await;
    let second = fx.runtime(|_| {});
    let chunks = turn(&second, fx.ctx("no-resume", 1), "back", Some(SEED)).await?;
    assert_eq!(reply(&chunks), "echo:User: back");
    assert!(
        fx.calls()
            .iter()
            .any(|call| call.starts_with("p2 load sess-p1-1"))
    );
    second.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn idle_sessions_close_and_the_next_turn_resumes() -> TestResult {
    let fx = Fixture::new()?;
    let runtime = fx.runtime(|config| config.idle_timeout = Duration::from_millis(200));
    turn(&runtime, fx.ctx("default", 1), "hi", None).await?;
    fx.wait_for("close sess-").await?;
    fx.wait_for("eof").await?;
    let chunks = turn(&runtime, fx.ctx("default", 1), "later", Some(SEED)).await?;
    assert_eq!(reply(&chunks), "echo:User: later");
    assert_eq!(fx.spawns(), 2);
    assert!(
        fx.calls()
            .iter()
            .any(|call| call.starts_with("p2 resume sess-p1-1"))
    );
    runtime.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn an_active_turn_holds_off_the_idle_close() -> TestResult {
    let fx = Fixture::new()?;
    let runtime = fx.runtime(|config| config.idle_timeout = Duration::from_millis(300));
    turn(&runtime, fx.ctx("default", 1), "hi", None).await?;
    // The second turn starts before the first idle deadline and runs past it.
    tokio::time::sleep(Duration::from_millis(150)).await;
    let chunks = turn(&runtime, fx.ctx("default", 1), "slow", None).await?;
    assert_eq!(reply(&chunks), "echo:User: slow");
    assert_eq!(fx.spawns(), 1);
    assert!(!fx.lines().iter().any(|line| line.contains("close sess-")));
    runtime.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn close_with_discard_forgets_the_agent_session() -> TestResult {
    let fx = Fixture::new()?;
    let runtime = fx.runtime(|_| {});
    turn(&runtime, fx.ctx("default", 1), "hi", None).await?;
    assert!(runtime.close(AGENT, Uuid::from_u128(1), true).await);
    fx.wait_for("close sess-").await?;
    assert_eq!(
        fx.store()
            .load(&SessionKey::new(AGENT, Uuid::from_u128(1)))?,
        None
    );
    assert!(!runtime.close(AGENT, Uuid::from_u128(1), true).await);
    let chunks = turn(&runtime, fx.ctx("default", 1), "fresh", Some(SEED)).await?;
    assert_eq!(reply(&chunks), format!("echo:{SEED}\nUser: fresh"));
    assert!(!fx.calls().iter().any(|call| call.contains(" resume ")));

    turn(&runtime, fx.ctx("default", 2), "two", None).await?;
    assert_eq!(runtime.close_all_for_agent(AGENT, false).await, 2);
    assert_eq!(runtime.close_all_for_agent(AGENT, false).await, 0);
    assert!(
        fx.store()
            .load(&SessionKey::new(AGENT, Uuid::from_u128(2)))?
            .is_some(),
        "no discard keeps records"
    );
    Ok(())
}

#[tokio::test]
async fn a_policy_change_restarts_the_process_but_lease_rotation_does_not() -> TestResult {
    let fx = Fixture::new()?;
    let runtime = fx.runtime(|_| {});
    turn(&runtime, fx.ctx("default", 1), "one", None).await?;
    let mut rotated = fx.ctx("default", 1);
    rotated.browser_tool_lease_token = "lease-2".into();
    turn(&runtime, rotated, "two", None).await?;
    assert_eq!(fx.spawns(), 1);

    let mut read_only = fx.ctx("default", 1);
    read_only.read_only = true;
    let chunks = turn(&runtime, read_only, "three", Some(SEED)).await?;
    assert_eq!(
        reply(&chunks),
        "echo:User: three",
        "same launch: the new process resumes"
    );
    assert_eq!(fx.spawns(), 2);
    let calls = fx.calls();
    assert!(
        calls
            .iter()
            .any(|call| call.starts_with("p2 resume sess-p1-1")),
        "{calls:#?}"
    );
    runtime.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn an_agent_crash_mid_turn_is_an_error_and_the_next_turn_reopens() -> TestResult {
    let fx = Fixture::new()?;
    let runtime = fx.runtime(|_| {});
    let chunks = turn(&runtime, fx.ctx("default", 1), "crash", None).await?;
    let tail: Vec<&Value> = chunks.iter().rev().take(3).collect();
    assert_eq!(tail[0], &json!({"type":"finish", "finishReason":"error"}));
    assert_eq!(tail[1], &json!({"type":"finish-step"}));
    assert_eq!(tail[2]["type"], "error");
    let message = tail[2]["errorText"].as_str().unwrap_or_default();
    assert!(
        message.starts_with("The ACP agent exited with code 17."),
        "{message}"
    );
    assert!(reply(&chunks).contains("about to crash"));

    let chunks = turn(&runtime, fx.ctx("default", 1), "after", Some(SEED)).await?;
    assert_eq!(
        reply(&chunks),
        "echo:User: after",
        "the crashed session had the prompt"
    );
    assert_eq!(fx.spawns(), 2);
    runtime.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn preparation_failures_are_errors_and_release_the_conversation() -> TestResult {
    let fx = Fixture::new()?;
    let runtime = fx.runtime(|_| {});
    let (auth, _) = request(fx.ctx("auth", 1), "hi", None);
    match runtime.stream(auth).await {
        Err(RuntimeError::Preparation(AcpError::AuthRequired { message, .. })) => {
            assert_eq!(message, "Authentication required");
        }
        Err(other) => return Err(format!("unexpected error {other:?}").into()),
        Ok(_) => return Err("auth failure must not stream".into()),
    }
    let mut missing = fx.ctx("default", 1);
    missing.agent.custom = None;
    let (missing, _) = request(missing, "hi", None);
    assert!(matches!(
        runtime.stream(missing).await,
        Err(RuntimeError::Launch(LaunchError::MissingCommand))
    ));
    let chunks = turn(&runtime, fx.ctx("default", 1), "works", None).await?;
    assert_eq!(reply(&chunks), "echo:User: works");
    runtime.shutdown().await;
    let (after, _) = request(fx.ctx("default", 1), "late", None);
    assert!(matches!(
        runtime.stream(after).await,
        Err(RuntimeError::Preparation(AcpError::Closed))
    ));
    Ok(())
}

#[tokio::test]
async fn the_turn_deadline_cancels_and_retires_an_unresponsive_agent() -> TestResult {
    let fx = Fixture::new()?;
    let runtime = fx.runtime(|config| {
        config.turn_timeout = Duration::from_millis(300);
        config.control_timeout = Duration::from_secs(1);
    });
    let chunks = turn(&runtime, fx.ctx("default", 1), "never", None).await?;
    assert_eq!(
        &chunks[chunks.len() - 3..],
        [
            json!({"type":"error", "errorText":"ACP turn timed out after 300ms"}),
            json!({"type":"finish-step"}),
            json!({"type":"finish", "finishReason":"error"}),
        ]
    );
    fx.wait_for("cancel sess-").await?;
    fx.wait_for("eof").await?;
    let chunks = turn(&runtime, fx.ctx("default", 1), "next", None).await?;
    assert_eq!(reply(&chunks), "echo:User: next");
    assert_eq!(fx.spawns(), 2);
    runtime.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn tool_calls_and_silent_failures_translate_end_to_end() -> TestResult {
    let fx = Fixture::new()?;
    let runtime = fx.runtime(|_| {});
    let chunks = turn(&runtime, fx.ctx("default", 1), "tool", None).await?;
    let tools: Vec<&Value> = chunks
        .iter()
        .filter(|chunk| {
            chunk["type"]
                .as_str()
                .is_some_and(|kind| kind.starts_with("tool-"))
        })
        .collect();
    assert_eq!(
        tools,
        [
            &json!({"type":"tool-input-start", "toolCallId":"t1", "toolName":"Terminal", "providerExecuted":true}),
            &json!({"type":"tool-input-delta", "toolCallId":"t1", "inputTextDelta":"Terminal (pending)"}),
            &json!({"type":"tool-input-delta", "toolCallId":"t1", "inputTextDelta":"ls -la (in_progress): ls -la"}),
            &json!({"type":"tool-input-available", "toolCallId":"t1", "toolName":"Terminal",
                    "input":{"command":"ls", "args":["-la"]}, "providerExecuted":true}),
            &json!({"type":"tool-output-available", "toolCallId":"t1", "output":"file.txt", "providerExecuted":true}),
        ]
    );
    let chunks = turn(&runtime, fx.ctx("default", 1), "silent-error", None).await?;
    assert!(
        chunks.contains(&json!({"type":"error", "errorText":"The ACP agent failed to respond."}))
    );
    runtime.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn dropping_the_stream_cancels_the_turn_and_frees_the_conversation() -> TestResult {
    let fx = Fixture::new()?;
    let runtime = fx.runtime(|_| {});
    let (hang, _cancel) = request(fx.ctx("default", 1), "hang", None);
    let mut stream = runtime.stream(hang).await?;
    while let Some(chunk) = stream.next().await {
        if serde_json::to_value(&chunk)?["delta"] == "started" {
            break;
        }
    }
    drop(stream);
    fx.wait_for("cancel sess-").await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let (next, _) = request(fx.ctx("default", 1), "next", None);
        match runtime.stream(next).await {
            Ok(stream) => {
                assert_eq!(reply(&collect(stream).await?), "echo:User: next");
                break;
            }
            Err(RuntimeError::Busy) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
    runtime.shutdown().await;
    Ok(())
}

/// The canonical path a child reports as its cwd: Windows `canonicalize` adds a verbatim `\\?\`
/// prefix that `current_dir()` in the agent does not.
fn display_cwd(path: &std::path::Path) -> std::io::Result<String> {
    let canonical = path.canonicalize()?.display().to_string();
    Ok(canonical
        .strip_prefix(r"\\?\")
        .unwrap_or(&canonical)
        .to_owned())
}
