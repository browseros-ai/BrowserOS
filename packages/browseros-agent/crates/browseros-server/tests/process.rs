//! The browser's launch contract, against the real binary: the version probe, usage errors,
//! health on the launch file's port, exit 2 only for a taken port, exit 3 after SIGTERM within
//! the launcher's 5 s, exit 0 after `POST /system/shutdown`, and a start-up that waits out a
//! locked database instead of failing or hanging. Every run uses a state folder of its own.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const BINARY: &str = env!("CARGO_BIN_EXE_browseros_server");

fn free_port() -> std::io::Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port())
}

/// A launch file with every key the browser writes, and an invented one.
fn launch_file(dir: &Path, port: u16) -> std::io::Result<PathBuf> {
    let path = dir.join("config.json");
    std::fs::write(
        &path,
        serde_json::json!({
            "ports": { "cdp": 1, "server": port, "proxy": 2 },
            "directories": { "resources": dir.join("resources"), "execution": dir.join("execution") },
            "flags": { "allow_remote_in_mcp": false },
            "instance": { "install_id": "x", "browseros_version": "0.53.0", "chromium_version": "146" },
            "invented": true
        })
        .to_string(),
    )?;
    Ok(path)
}

fn serve(config: &Path, state: &Path) -> std::io::Result<Child> {
    Command::new(BINARY)
        .arg(format!("--config={}", config.display()))
        .env("BROWSEROS_DIR", state)
        .env("BROWSEROS_BROWSER", "off")
        .env_remove("BROWSEROS_USE_MOCK_LLM")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

/// Polls `/system/health` until it answers 200, or the deadline passes.
async fn wait_healthy(port: u16, within: Duration) -> bool {
    let client = reqwest::Client::new();
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if let Ok(response) = client
            .get(format!("http://127.0.0.1:{port}/system/health"))
            .send()
            .await
            && response.status().is_success()
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// Waits for the child to exit; `None` if it outlives `within` (and it is then killed).
fn wait_exit(child: &mut Child, within: Duration) -> std::io::Result<Option<i32>> {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            return Ok(status.code());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    child.kill()?;
    child.wait()?;
    Ok(None)
}

fn sigterm(child: &Child) -> std::io::Result<()> {
    Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .map(|_| ())
}

#[test]
fn the_version_probe_prints_the_version_and_touches_nothing() -> TestResult {
    let dir = tempfile::tempdir()?;
    let output = Command::new(BINARY)
        .arg("--version")
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env("BROWSEROS_DIR", dir.path().join("state"))
        .output()?;
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stdout)?, "0.1.0\n");
    assert_eq!(std::fs::read_dir(dir.path())?.count(), 0, "no file written");
    Ok(())
}

#[test]
fn a_usage_error_exits_1_never_2() -> TestResult {
    for args in [&["--bogus"][..], &[][..], &["--config"][..]] {
        let status = Command::new(BINARY)
            .args(args)
            .stderr(Stdio::null())
            .status()?;
        assert_eq!(status.code(), Some(1), "{args:?}");
    }
    Ok(())
}

#[tokio::test]
async fn it_serves_health_refuses_a_taken_port_with_2_and_stops_on_sigterm_with_3() -> TestResult {
    let dir = tempfile::tempdir()?;
    let port = free_port()?;
    let config = launch_file(dir.path(), port)?;
    let mut first = serve(&config, &dir.path().join("state"))?;
    assert!(
        wait_healthy(port, Duration::from_secs(15)).await,
        "health within 15 s"
    );

    let status: serde_json::Value = reqwest::get(format!("http://127.0.0.1:{port}/status"))
        .await?
        .json()
        .await?;
    assert_eq!(status["can_update"], serde_json::json!(true));

    let mut second = serve(&config, &dir.path().join("state-2"))?;
    assert_eq!(
        wait_exit(&mut second, Duration::from_secs(15))?,
        Some(2),
        "a taken port is exit 2"
    );

    // An idle keep-alive connection must not hold the process past the launcher's kill.
    let _idle = std::net::TcpStream::connect(("127.0.0.1", port))?;
    let stopping = Instant::now();
    sigterm(&first)?;
    assert_eq!(
        wait_exit(&mut first, Duration::from_secs(5))?,
        Some(3),
        "SIGTERM is exit 3"
    );
    assert!(
        stopping.elapsed() < Duration::from_millis(4500),
        "stopped in {:?}",
        stopping.elapsed()
    );
    Ok(())
}

#[tokio::test]
async fn post_system_shutdown_answers_then_exits_0() -> TestResult {
    let dir = tempfile::tempdir()?;
    let port = free_port()?;
    let mut child = serve(&launch_file(dir.path(), port)?, &dir.path().join("state"))?;
    assert!(wait_healthy(port, Duration::from_secs(15)).await);
    let response = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/system/shutdown"))
        .send()
        .await?;
    assert_eq!(
        response.json::<serde_json::Value>().await?,
        serde_json::json!({ "status": "ok" })
    );
    assert_eq!(wait_exit(&mut child, Duration::from_secs(5))?, Some(0));
    Ok(())
}

#[test]
fn an_unwritable_state_folder_is_exit_1_not_2() -> TestResult {
    let dir = tempfile::tempdir()?;
    let blocker = dir.path().join("a-file");
    std::fs::write(&blocker, "not a folder")?;
    let mut child = serve(
        &launch_file(dir.path(), free_port()?)?,
        &blocker.join("state"),
    )?;
    assert_eq!(wait_exit(&mut child, Duration::from_secs(15))?, Some(1));
    Ok(())
}

/// Holds a write lock on `<state>/db/browseros.sqlite` for `hold`, in another connection.
async fn hold_lock(state: &Path, hold: Duration) -> Result<(), sea_orm::DbErr> {
    use sea_orm::{ConnectionTrait, TransactionTrait};
    std::fs::create_dir_all(state.join("db")).map_err(|e| sea_orm::DbErr::Custom(e.to_string()))?;
    let url = format!(
        "sqlite://{}?mode=rwc",
        state.join("db").join("browseros.sqlite").display()
    );
    let db = sea_orm::Database::connect(url).await?;
    let transaction = db.begin().await?;
    transaction
        .execute_unprepared("CREATE TABLE lock_probe (x integer)")
        .await?;
    tokio::time::sleep(hold).await;
    transaction.rollback().await?;
    db.close().await
}

#[tokio::test]
async fn start_up_waits_out_a_locked_database_and_then_serves() -> TestResult {
    let dir = tempfile::tempdir()?;
    let state = dir.path().join("state");
    let port = free_port()?;
    let holder = tokio::spawn({
        let state = state.clone();
        async move { hold_lock(&state, Duration::from_secs(3)).await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let started = Instant::now();
    let mut child = serve(&launch_file(dir.path(), port)?, &state)?;
    assert!(
        wait_healthy(port, Duration::from_secs(19)).await,
        "served once the lock was released"
    );
    assert!(
        started.elapsed() >= Duration::from_millis(2500),
        "it waited for the lock"
    );
    holder.await??;
    sigterm(&child)?;
    assert_eq!(wait_exit(&mut child, Duration::from_secs(5))?, Some(3));
    Ok(())
}

/// Slow (about 20 s): a lock held past the start budget is exit 1, never a hang.
#[tokio::test]
#[ignore = "takes the full 20 s start budget; run with --ignored"]
async fn a_lock_held_past_the_start_budget_is_exit_1() -> TestResult {
    let dir = tempfile::tempdir()?;
    let state = dir.path().join("state");
    let holder = tokio::spawn({
        let state = state.clone();
        async move { hold_lock(&state, Duration::from_secs(26)).await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let started = Instant::now();
    let mut child = serve(&launch_file(dir.path(), free_port()?)?, &state)?;
    let code = wait_exit(&mut child, Duration::from_secs(24))?;
    let elapsed = started.elapsed();
    assert_eq!(code, Some(1));
    assert!(
        elapsed >= Duration::from_secs(19) && elapsed < Duration::from_secs(23),
        "{elapsed:?}"
    );
    holder.await??;
    Ok(())
}
