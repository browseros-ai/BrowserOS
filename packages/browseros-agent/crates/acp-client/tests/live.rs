//! Opt-in end-to-end smoke against installed/logged-in coding agents. Normal
//! cargo test never invokes a model or downloads an adapter; ACP_LIVE=1 plus
//! --ignored is an explicit request to run both live transports.

use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};

use acp_client::{
    AgentConnection, ClientOptions, LaunchSpec, RejectAll, SessionParams, TurnEvent,
    schema::{ContentBlock, Meta, SessionUpdate, StopReason, TextContent},
};
use tempfile::TempDir;

#[tokio::test]
#[ignore = "requires ACP_LIVE=1 and logged-in fx/Claude CLIs"]
async fn live_fx_and_claude() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("ACP_LIVE").as_deref() != Ok("1") {
        return Err("set ACP_LIVE=1 to authorize live smoke".into());
    }
    for (name, program, args, meta) in [
        (
            "fx",
            PathBuf::from(std::env::var("HOME")?).join(".local/bin/fx"),
            vec!["acp".into()],
            None,
        ),
        (
            "claude",
            PathBuf::from("npx"),
            vec![
                "-y".into(),
                "@agentclientprotocol/claude-agent-acp@0.75.1".into(),
            ],
            Some(serde_json::from_value::<Meta>(
                serde_json::json!({"claudeCode":{"options":{"model":"haiku", "settingSources":["project", "local"]}}}),
            )?),
        ),
    ] {
        let dir = TempDir::new()?;
        let spec = LaunchSpec {
            program,
            args,
            env: BTreeMap::new(),
            cwd: dir.path().into(),
        };
        let options = ClientOptions {
            client_name: "acp-client-live-smoke".into(),
            client_version: "0.1.0".into(),
            control_timeout: Duration::from_secs(90),
            stderr_tail_lines: 20,
            ..ClientOptions::default()
        };
        let conn =
            AgentConnection::connect(spec.clone(), options.clone(), Arc::new(RejectAll)).await?;
        let params = SessionParams {
            cwd: dir.path().into(),
            mcp_servers: vec![],
            meta,
        };
        let session = conn.new_session(params.clone()).await?;
        let id = session.id().clone();
        let outcome = smoke_prompt(&session).await;
        drop(session);
        let report = conn.shutdown(Duration::from_secs(3)).await;
        outcome?;
        assert!(
            report.status.is_some_and(|status| status.success()),
            "{name} did not exit cleanly: {report:?}"
        );
        // Reattachment must work in a fresh child, rather than succeeding only
        // because the original process retained the session in memory.
        let conn = AgentConnection::connect(spec, options, Arc::new(RejectAll)).await?;
        let session = if conn.features().resume_session {
            conn.resume_session(id, params).await?
        } else {
            conn.load_session(id, params).await?
        };
        let outcome = smoke_prompt(&session).await;
        session.close().await?;
        drop(session);
        let report = conn.shutdown(Duration::from_secs(3)).await;
        outcome?;
        assert!(
            report.status.is_some_and(|status| status.success()),
            "{name} reattached process did not exit cleanly: {report:?}"
        );
        // Test output is intentional evidence for the final commit transcript.
        use std::io::Write;
        writeln!(
            std::io::stdout(),
            "LIVE PASS: {name}: initialize/new/prompt/fresh-process-reattach/prompt/close/shutdown"
        )?;
    }
    Ok(())
}

async fn smoke_prompt(session: &acp_client::Session) -> Result<(), Box<dyn std::error::Error>> {
    let mut turn = session.prompt(vec![ContentBlock::Text(TextContent::new(
        "Reply with exactly pong. Do not use any tools.",
    ))])?;
    tokio::time::timeout(Duration::from_secs(180), async {
        let mut text = String::new();
        while let Some(event) = turn.next().await {
            match event {
                TurnEvent::Update(SessionUpdate::AgentMessageChunk(chunk)) => {
                    if let ContentBlock::Text(chunk) = chunk.content {
                        text.push_str(&chunk.text);
                    }
                }
                TurnEvent::Done(result) => {
                    assert_eq!(result?.stop_reason, StopReason::EndTurn);
                    assert!(
                        text.to_lowercase().contains("pong"),
                        "missing live pong: {text}"
                    );
                    return Ok(());
                }
                _ => {}
            }
        }
        Err("live turn closed without Done".into())
    })
    .await?
}
