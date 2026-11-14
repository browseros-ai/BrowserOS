//! One-shot child supervision. The host keeps the sole stdin writer as a life
//! line; EOF also kills the SDK when the host dies without running cleanup.

#![cfg(unix)]

use std::{io, path::Path, process::Stdio, time::Duration};

use serde_json::{Value, json};
use tokio::{
    fs,
    process::{Child, ChildStdin, Command},
};

use super::{EngineLaunch, EngineState, EngineUnavailable, NeoEngine, RunTool, wire::Wire};

pub(super) async fn supervise(
    engine: &NeoEngine,
    launch: EngineLaunch,
) -> Result<(), EngineUnavailable> {
    let sdk_dir = launch.state_dir.join("claw-sdk");
    let saved_tool = sdk_dir.join("run-tool.json");
    if let Ok(saved) = fs::read(&saved_tool).await
        && let Ok(tool) = serde_json::from_slice::<RunTool>(&saved)
    {
        engine
            .inner
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .tool = Some(tool);
    }
    let (mut child, wire) = spawn(&launch).await.map_err(|error| {
        EngineUnavailable::new(format!("the Playwright engine could not start: {error}"))
    })?;
    // Child::wait closes child.stdin itself. Take the writer out first, or
    // simply monitoring an otherwise healthy child would tell it to exit.
    let mut lifeline = child.stdin.take();
    let tool = tokio::select! {
        () = engine.inner.stop.cancelled() => {
            stop_child(&mut child, &mut lifeline).await;
            engine.unavailable("the Playwright engine was shut down".to_owned());
            return Ok(());
        }
        status = child.wait() => return Err(exited(status)),
        result = wait_ready(&wire) => result?,
    };
    if let Err(error) = save_tool(&saved_tool, &tool).await {
        tracing::warn!(%error, "the Playwright run definition could not be cached");
    }
    {
        let mut connection = engine
            .inner
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        connection.wire = Some(wire);
        connection.tool = Some(tool);
    }
    engine.inner.state.send_replace(EngineState::Ready);
    tokio::select! {
        () = engine.inner.stop.cancelled() => {
            stop_child(&mut child, &mut lifeline).await;
            engine.unavailable("the Playwright engine was shut down".to_owned());
            Ok(())
        }
        status = child.wait() => Err(exited(status)),
    }
}

async fn spawn(launch: &EngineLaunch) -> io::Result<(Child, Wire)> {
    let state_dir = std::path::absolute(&launch.state_dir)?;
    let sdk_dir = state_dir.join("claw-sdk");
    fs::create_dir_all(&sdk_dir).await?;
    configure_settings(&sdk_dir.join("settings.json")).await?;
    // The SDK refuses port 0. Reserve a free port while preparing its files,
    // then release it immediately before spawn. A race is a startup failure.
    let reservation = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = reservation.local_addr()?.port();
    let original: Value = serde_json::from_slice(&fs::read(&launch.config_path).await?)?;
    let mut config = json!({
        "ports": {"cdp": launch.cdp_port, "server": port},
        "directories": {"resources": std::path::absolute(&launch.resources_dir)?},
        "flags": {"script_cdp": true}
    });
    if let Some(instance) = original.get("instance") {
        config["instance"] = instance.clone();
    }
    if let Some(execution) = original["directories"]["execution"]
        .as_str()
        .filter(|value| !value.is_empty())
    {
        let original_dir = launch.config_path.parent().unwrap_or_else(|| Path::new(""));
        config["directories"]["execution"] =
            json!(std::path::absolute(original_dir.join(execution))?);
    }
    let config_path = sdk_dir.join("launch.json");
    fs::write(&config_path, serde_json::to_vec_pretty(&config)?).await?;
    let wire = Wire::new(port).map_err(io::Error::other)?;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("__neo-sdk")
        .arg("--config")
        .arg(config_path)
        .env("BROWSERCLAW_DIR", &state_dir)
        .env("BROWSERCLAW_HOSTED_MODE", "0")
        .env("CLAW_XVNC_ADDR", "127.0.0.1:9")
        // Optional host listeners must not inherit another SDK's socket/token.
        // MCP needs none, and the browser owns its actual downloads location.
        .env_remove("CLAW_HOST_TCP")
        .env_remove("CLAW_HOST_SOCKET")
        .env_remove("CLAW_HOST_TOKEN_FILE")
        .env_remove("CLAW_DOWNLOADS_DIR")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    drop(reservation);
    let child = command.spawn()?;
    tracing::info!(pid = child.id(), "the Playwright engine was started");
    Ok((child, wire))
}

/// Desktop owns these two settings even when an earlier SDK launch saved overrides.
/// Merge atomically so a crash cannot restore hosted defaults or drop unrelated settings.
async fn configure_settings(path: &Path) -> io::Result<()> {
    let mut settings: serde_json::Map<String, Value> = match fs::read(path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            tracing::warn!(%error, "invalid SDK settings replaced with desktop policy");
            Default::default()
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Default::default(),
        Err(error) => return Err(error),
    };
    for (section, key, value) in [
        ("tabs", "idleCloseSeconds", json!(86400)),
        ("trace", "screenshots", json!("off")),
    ] {
        let section = settings.entry(section).or_insert_with(|| json!({}));
        if !section.is_object() {
            tracing::warn!(
                key,
                "invalid SDK settings section replaced with desktop policy"
            );
            *section = json!({});
        }
        section[key] = value;
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(&settings)?).await?;
    fs::rename(temporary, path).await
}

async fn wait_ready(wire: &Wire) -> Result<RunTool, EngineUnavailable> {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if wire.healthy().await {
                return wire.tool().await;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .map_err(|_| {
        EngineUnavailable::new("the Playwright engine did not become healthy within 30 s")
    })?
}

async fn stop_child(child: &mut Child, lifeline: &mut Option<ChildStdin>) {
    drop(lifeline.take());
    if tokio::time::timeout(Duration::from_secs(2), child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
    }
}

fn exited(status: io::Result<std::process::ExitStatus>) -> EngineUnavailable {
    match status {
        Ok(status) => EngineUnavailable::new(format!("the Playwright engine exited ({status})")),
        Err(error) => EngineUnavailable::new(format!(
            "the Playwright engine could not be observed: {error}"
        )),
    }
}

async fn save_tool(path: &Path, tool: &RunTool) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    let definition = json!({
        "name": "run", "description": tool.description, "inputSchema": tool.input_schema
    });
    fs::write(&temporary, serde_json::to_vec_pretty(&definition)?).await?;
    fs::rename(temporary, path).await
}

#[cfg(test)]
mod tests {
    use super::configure_settings;
    use serde_json::{Value, json};

    #[tokio::test]
    async fn desktop_settings_override_existing_defaults_and_preserve_other_values()
    -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("settings.json");
        for existing in [
            json!({}),
            json!({"tabs":null,"trace":[]}),
            json!({"tabs":{"idleCloseSeconds":1800},"trace":{"screenshots":"operations","valueBytes":42}}),
        ] {
            tokio::fs::write(&path, serde_json::to_vec(&existing)?).await?;
            configure_settings(&path).await?;
            let saved: Value = serde_json::from_slice(&tokio::fs::read(&path).await?)?;
            assert_eq!(saved["tabs"]["idleCloseSeconds"], 86400);
            assert_eq!(saved["trace"]["screenshots"], "off");
            assert_eq!(
                saved["trace"]["valueBytes"],
                existing["trace"]["valueBytes"]
            );
        }
        tokio::fs::write(&path, b"malformed JSON").await?;
        configure_settings(&path).await?;
        let saved: Value = serde_json::from_slice(&tokio::fs::read(&path).await?)?;
        assert_eq!(
            saved,
            json!({"tabs":{"idleCloseSeconds":86400},"trace":{"screenshots":"off"}})
        );
        Ok(())
    }
}
