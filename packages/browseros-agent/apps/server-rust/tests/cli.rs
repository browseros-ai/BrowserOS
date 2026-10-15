//! Sidecar and executable protocol compatibility; process tests own only temporary state and ports.
use serde_json::{Value, json};
use server_rust::{
    VERSION,
    config::{CliAction, Config, EnvConfig, parse_cli},
};
use std::{ffi::OsString, process::Command};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn executable_version_and_general_errors() -> Result {
    let output = Command::new(env!("CARGO_BIN_EXE_browseros-server-rs"))
        .arg("--version")
        .output()?;
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout)?.trim(), VERSION);
    assert!(output.stderr.is_empty());
    for args in [
        vec![],
        vec!["--config"],
        vec!["--config", "/nonexistent/browseros-config.json"],
        vec!["--unknown"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_browseros-server-rs"))
            .args(args)
            .output()?;
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8(output.stderr)?.contains("[FATAL]"));
    }
    assert_eq!(
        parse_cli([OsString::from("--config=")]),
        Err(server_rust::config::CliError::ConfigPathMissing)
    );
    assert_eq!(
        parse_cli([OsString::from("--version")]),
        Ok(CliAction::Version)
    );
    Ok(())
}

#[test]
fn sidecar_normalization_and_required_validation() -> Result {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("config.json");
    let env = EnvConfig {
        home: Some(dir.path().to_path_buf()),
        node_env: Some("development".into()),
        ..Default::default()
    };
    let valid = json!({"ports":{"server":" 09200 ","cdp":9100,"proxy":9000},"directories":{"resources":" ./resources/../resources ","execution":" ./logs "},"flags":{"allow_remote_in_mcp":true},"replay":{"retentionDays":7},"extra":true});
    std::fs::write(&path, valid.to_string())?;
    let config = Config::load(&path, &env)?;
    assert_eq!(config.server_port, 9200);
    assert_eq!(config.proxy_port, Some(9000));
    assert_eq!(config.resources_dir, dir.path().join("resources"));
    assert_eq!(config.browseros_dir, dir.path().join(".browseros-dev"));
    assert!(config.allow_remote_in_mcp);
    for invalid in [
        json!(0),
        json!(65536),
        json!(1.5),
        json!("+9200"),
        json!("9e3"),
        json!(null),
        json!(true),
    ] {
        let mut value = valid.clone();
        value["ports"]["server"] = invalid;
        std::fs::write(&path, value.to_string())?;
        assert!(Config::load(&path, &env).is_err(), "{value}");
    }
    for invalid in [json!(" "), json!(null), json!(7)] {
        let mut value = valid.clone();
        value["directories"]["resources"] = invalid;
        std::fs::write(&path, value.to_string())?;
        assert!(Config::load(&path, &env).is_err(), "{value}");
    }
    std::fs::write(&path, Value::Object(Default::default()).to_string())?;
    assert!(Config::load(&path, &env).is_err());
    Ok(())
}

/// Reap every child even when an assertion fails so a red test cannot leak a listener.
struct ChildProcess(std::process::Child);
impl Drop for ChildProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn free_port() -> Result<u16> {
    Ok(
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?
            .local_addr()?
            .port(),
    )
}
fn process_config(dir: &std::path::Path, port: u16) -> Result<std::path::PathBuf> {
    let path = dir.join("config.json");
    std::fs::write(&path,json!({"ports":{"server":port,"cdp":free_port()?},"directories":{"resources":"resources","execution":"logs"}}).to_string())?;
    Ok(path)
}
fn spawn(dir: &std::path::Path, config: &std::path::Path) -> Result<ChildProcess> {
    use std::process::Stdio;
    Ok(ChildProcess(
        Command::new(env!("CARGO_BIN_EXE_browseros-server-rs"))
            .arg("--config")
            .arg(config)
            .env("BROWSEROS_DIR", dir.join("data"))
            .env("NODE_ENV", "test")
            .env("LOG_LEVEL", "info")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?,
    ))
}
fn wait_exit(child: &mut ChildProcess) -> Result<std::process::ExitStatus> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some(status) = child.0.try_wait()? {
            return Ok(status);
        }
        if std::time::Instant::now() > deadline {
            return Err("child exceeded Chromium's five-second grace".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
async fn wait_ready(child: &mut ChildProcess, dir: &std::path::Path, port: u16) -> Result {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_millis(100))
        .build()?;
    loop {
        if let Some(status) = child.0.try_wait()? {
            return Err(format!("child exited before ready: {status}").into());
        }
        if dir.join("data/server.json").exists()
            && client
                .get(format!("http://127.0.0.1:{port}/health"))
                .send()
                .await
                .is_ok_and(|response| response.status() == 200)
        {
            return Ok(());
        }
        if std::time::Instant::now() > deadline {
            return Err("server did not become ready".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[test]
fn prebind_port_conflict_exits_two_without_discovery() -> Result {
    let dir = tempfile::tempdir()?;
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    let path = process_config(dir.path(), listener.local_addr()?.port())?;
    let mut child = spawn(dir.path(), &path)?;
    assert_eq!(wait_exit(&mut child)?.code(), Some(2));
    assert!(!dir.path().join("data/server.json").exists());
    Ok(())
}

#[tokio::test]
async fn shutdown_response_flushes_before_exit_zero_and_rotates_json_logs() -> Result {
    use std::io::Read;
    let dir = tempfile::tempdir()?;
    let port = free_port()?;
    let config = process_config(dir.path(), port)?;
    std::fs::create_dir_all(dir.path().join("logs"))?;
    let log = dir.path().join("logs/browseros-server.log");
    std::fs::write(&log, "previous log")?;
    std::fs::File::options().write(true).open(&log)?.set_times(
        std::fs::FileTimes::new().set_modified(
            std::time::SystemTime::now() - std::time::Duration::from_secs(25 * 60 * 60),
        ),
    )?;
    let mut child = spawn(dir.path(), &config)?;
    wait_ready(&mut child, dir.path(), port).await?;
    let client = reqwest::Client::builder().no_proxy().build()?;
    let response = client
        .post(format!("http://127.0.0.1:{port}/shutdown"))
        .send()
        .await?;
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().await?, r#"{"status":"ok"}"#);
    assert_eq!(wait_exit(&mut child)?.code(), Some(0));
    assert!(!dir.path().join("data/server.json").exists());
    assert_eq!(
        std::fs::read_to_string(log.with_file_name("browseros-server.log.old"))?,
        "previous log"
    );
    let lines = std::fs::read_to_string(&log)?;
    assert!(lines.lines().any(|line| {
        serde_json::from_str::<Value>(line)
            .ok()
            .is_some_and(|value| value["fields"]["message"] == "http_server.started")
    }));
    let mut stdout = String::new();
    child
        .0
        .stdout
        .take()
        .ok_or("missing stdout")?
        .read_to_string(&mut stdout)?;
    assert!(stdout.is_empty());
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn sigterm_and_sigint_exit_three_and_remove_discovery() -> Result {
    for signal in ["-TERM", "-INT"] {
        let dir = tempfile::tempdir()?;
        let port = free_port()?;
        let config = process_config(dir.path(), port)?;
        let mut child = spawn(dir.path(), &config)?;
        wait_ready(&mut child, dir.path(), port).await?;
        assert!(
            Command::new("/bin/kill")
                .args([signal, &child.0.id().to_string()])
                .status()?
                .success()
        );
        assert_eq!(wait_exit(&mut child)?.code(), Some(3));
        assert!(!dir.path().join("data/server.json").exists());
    }
    Ok(())
}
