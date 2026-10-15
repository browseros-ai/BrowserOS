//! `probe_agent` against the fake agent: the `/acpx/probe` JSON shape, config-option detection,
//! and the acp-probe error codes.

use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use acp_agents::{
    AgentKind, DEFAULT_PROBE_TIMEOUT, HostEnvironment, LaunchError, ProbeRequest, probe_agent,
};
use serde_json::json;
use tempfile::TempDir;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn host(dir: &TempDir) -> HostEnvironment {
    HostEnvironment {
        resources_dir: None,
        browseros_dir: dir.path().into(),
        inherited_env: BTreeMap::from([
            ("PATH".into(), std::env::var("PATH").unwrap_or_default()),
            ("SHELL".into(), "/nonexistent/shell".into()),
            ("HOME".into(), dir.path().display().to_string()),
        ]),
    }
}

fn request(command: Option<String>, timeout: Duration) -> ProbeRequest {
    ProbeRequest {
        kind: AgentKind::Custom,
        command,
        env: BTreeMap::new(),
        cwd: None,
        timeout,
    }
}

fn fake(scenario: &str) -> Option<String> {
    Some(format!(
        "'{}' {scenario}",
        env!("CARGO_BIN_EXE_acp-agents-fake-agent")
    ))
}

#[tokio::test]
async fn a_probe_reports_models_reasoning_and_config_support() -> TestResult {
    let dir = TempDir::new()?;
    let log = dir.path().join("probe.log");
    let mut probe = request(fake("default"), DEFAULT_PROBE_TIMEOUT);
    probe
        .env
        .insert("FAKE_AGENT_LOG".into(), log.display().to_string());
    let result = probe_agent(probe, &host(&dir)).await?;
    assert_eq!(
        serde_json::to_value(&result)?,
        json!({
            "models":[{"id":"fake-default", "name":"Fake default"}, {"id":"fake-pro", "name":"Fake Pro", "description":"Bigger"}],
            "reasoning":{"values":["low", "medium", "high"], "defaultValue":"medium"},
            "supportsConfigOption":true,
            "agentInfo":{"name":"fake-agent", "title":"Fake Agent", "version":"9.9.9"},
            "protocolVersion":1,
        })
    );
    let calls = std::fs::read_to_string(&log)?;
    // Windows canonical paths carry a verbatim `\\?\` prefix the agent's current_dir() does not report.
    let canonical_home = dir.path().canonicalize()?.display().to_string();
    let canonical_home = canonical_home
        .strip_prefix(r"\\?\")
        .unwrap_or(&canonical_home);
    assert!(calls.contains(&format!("cwd={canonical_home}")), "{calls}");
    assert!(
        calls.contains("set_config sess-"),
        "the first option is poked with its own value"
    );
    assert!(calls.contains(" model=fake-default"), "{calls}");
    assert!(
        calls.contains("close sess-") && calls.contains("eof"),
        "{calls}"
    );
    assert!(!calls.contains("prompt"), "probing never prompts");
    Ok(())
}

#[tokio::test]
async fn probe_failures_use_acp_probe_codes() -> TestResult {
    let dir = TempDir::new()?;
    let result = probe_agent(request(fake("auth"), DEFAULT_PROBE_TIMEOUT), &host(&dir)).await?;
    assert_eq!(
        serde_json::to_value(&result)?,
        json!({
            "models":[], "reasoning":null, "supportsConfigOption":false, "agentInfo":null, "protocolVersion":0,
            "error":{"code":"unknown", "message":"Authentication required"},
        })
    );

    let missing = dir.path().join("no-such-agent");
    let result = probe_agent(
        request(Some(missing.display().to_string()), DEFAULT_PROBE_TIMEOUT),
        &host(&dir),
    )
    .await?;
    assert_eq!(
        result.error.as_ref().map(|error| error.code.as_str()),
        Some("spawn_failed")
    );

    // Agent stand-ins below are POSIX shell scripts; on Windows they are not executable, so the
    // probe correctly reports spawn_failed (covered above) and these cases only run on Unix.
    #[cfg(unix)]
    {
        let crash = dir.path().join("crash.sh");
        std::fs::write(&crash, "#!/bin/sh\necho boom >&2\nexit 3\n")?;
        set_executable(&crash)?;
        let result = probe_agent(
            request(Some(crash.display().to_string()), DEFAULT_PROBE_TIMEOUT),
            &host(&dir),
        )
        .await?;
        let error = result.error.ok_or("crash error")?;
        assert_eq!(error.code, "agent_crashed");
        assert_eq!(
            error.message,
            "Agent exited (code=3, signal=null) before responding to initialize"
        );

        let hang = dir.path().join("hang.sh");
        std::fs::write(&hang, "#!/bin/sh\nexec sleep 30\n")?;
        set_executable(&hang)?;
        let result = probe_agent(
            request(Some(hang.display().to_string()), Duration::from_millis(300)),
            &host(&dir),
        )
        .await?;
        assert_eq!(
            result.error.map(|error| error.code),
            Some("initialize_timeout".into())
        );
    }
    Ok(())
}

#[tokio::test]
async fn an_unsplittable_command_is_a_launch_error() -> TestResult {
    let dir = TempDir::new()?;
    let result = probe_agent(
        request(Some("agent 'oops".into()), DEFAULT_PROBE_TIMEOUT),
        &host(&dir),
    )
    .await;
    assert_eq!(result.err(), Some(LaunchError::UnterminatedQuote));
    let mut probe = request(None, DEFAULT_PROBE_TIMEOUT);
    probe.cwd = Some(PathBuf::from("/tmp"));
    assert_eq!(
        probe_agent(probe, &host(&dir)).await.err(),
        Some(LaunchError::MissingCommand)
    );
    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}
