//! The process contract Chromium relies on, checked against the built binary: `--version`, exit
//! codes 0/1/2, `GET /system/health` with the browser link's state, the start's log lines, a
//! clean exit on SIGTERM, and the packaging marker the release tooling reads from the binary. Also
//! `--check-config`, the image's preflight before it starts the browser.

use std::error::Error;
use std::fs;
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::time::{self, Instant};

#[path = "support/fork_lock.rs"]
mod fork_lock;

const BIN: &str = env!("CARGO_BIN_EXE_browseros-claw-sdk");
/// Generous for a debug binary on a loaded CI machine; a passing run takes well under a second.
const PATIENCE: Duration = Duration::from_secs(30);

type TestResult = Result<(), Box<dyn Error>>;

/// A scratch directory with a `HOME` and a `BROWSERCLAW_DIR` that do not exist yet, so a test can
/// tell whether the binary created either.
struct Scratch {
    dir: tempfile::TempDir,
}

impl Scratch {
    fn new() -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            dir: tempfile::tempdir()?,
        })
    }

    fn state_dir(&self) -> PathBuf {
        self.dir.path().join("state")
    }

    /// Writes a launch file the way Chromium does, with the agent port set to `port`.
    fn launch_file(&self, port: u16) -> Result<PathBuf, Box<dyn Error>> {
        let path = self.dir.path().join("server_config.json");
        let json = format!(
            r#"{{"ports":{{"server":{port},"cdp":1,"proxy":9010}},"directories":{{"resources":"r"}},"instance":{{"browseros_version":"0.51.1.3"}}}}"#
        );
        fs::write(&path, json)?;
        Ok(path)
    }

    /// The binary with only this scratch's env, so a developer's shell cannot change the outcome.
    fn neo(&self) -> Command {
        let mut command = Command::new(BIN);
        command
            .env_clear()
            .env("HOME", self.dir.path().join("home"))
            .env("BROWSERCLAW_DIR", self.state_dir())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command
    }
}

/// `GET path` on the agent port, retried until the binary is listening.
async fn get_when_up(port: u16, path: &str) -> Result<String, Box<dyn Error>> {
    let deadline = Instant::now() + PATIENCE;
    let mut stream = loop {
        match TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await {
            Ok(stream) => break stream,
            Err(error) if Instant::now() > deadline => return Err(error.into()),
            Err(_) => time::sleep(Duration::from_millis(50)).await,
        }
    };
    let request = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await?;
    let mut response = String::new();
    stream.read_to_string(&mut response).await?;
    Ok(response)
}

#[tokio::test]
async fn version_prints_alone_before_any_state() -> TestResult {
    let scratch = Scratch::new()?;
    let output = fork_lock::spawn(scratch.neo().arg("--version"))?
        .wait_with_output()
        .await?;

    assert_eq!(
        output.status.code(),
        Some(0),
        "Chromium's probe needs exit 0"
    );
    assert_eq!(
        String::from_utf8(output.stdout)?,
        format!("{}\n", env!("CARGO_PKG_VERSION")),
        "Chromium parses the trimmed stdout as a dotted version, so nothing else may be printed"
    );
    assert!(
        output.stderr.is_empty(),
        "--version runs before logging, so nothing reaches stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_dir(scratch.dir.path())?.count(),
        0,
        "--version runs before the state dir (or $HOME/.browserclaw) is created"
    );
    Ok(())
}

#[tokio::test]
async fn usage_and_config_errors_exit_1_and_help_exits_0() -> TestResult {
    let scratch = Scratch::new()?;
    let missing = scratch.dir.path().join("absent.json");
    let missing = missing.to_str().ok_or("temp path is not UTF-8")?;
    let cases: [&[&str]; 7] = [
        &["--no-such-flag"],
        &[],
        &["--config"],
        &["--version", "--config", "x.json"],
        &["--config", missing],
        &["--check-config", "--config", "x.json"],
        &["--check-config", "--version"],
    ];
    for args in cases {
        let output = fork_lock::spawn(scratch.neo().args(args))?
            .wait_with_output()
            .await?;
        assert_eq!(
            output.status.code(),
            Some(1),
            "{args:?}: every failure but a taken port exits 1, never clap's 2: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let help = fork_lock::spawn(scratch.neo().arg("--help"))?
        .wait_with_output()
        .await?;
    assert_eq!(help.status.code(), Some(0), "help is not a failure");
    Ok(())
}

#[tokio::test]
async fn a_taken_agent_port_exits_2() -> TestResult {
    let scratch = Scratch::new()?;
    let taken = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let launch = scratch.launch_file(taken.local_addr()?.port())?;

    let output = time::timeout(
        PATIENCE,
        fork_lock::spawn(scratch.neo().arg("--config").arg(&launch))?.wait_with_output(),
    )
    .await??;
    assert_eq!(
        output.status.code(),
        Some(2),
        "exit 2 is how Chromium learns to relaunch on the next port: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[tokio::test]
async fn serves_health_logs_the_start_and_exits_0_on_sigterm() -> TestResult {
    let scratch = Scratch::new()?;
    let port = fork_lock::free_port()?;
    let launch = scratch.launch_file(port)?;
    let child = fork_lock::spawn(
        scratch
            .neo()
            .arg("--config")
            .arg(&launch)
            .env("BROWSERCLAW_CONFIG_FILE", "/etc/neo.toml"),
    )?;

    let response = get_when_up(port, "/system/health").await?;
    assert!(
        response.starts_with("HTTP/1.1 200"),
        "Chromium kills a sidecar whose health is not 200: {response}"
    );
    assert!(
        response.contains(r#"{"status":"ok","browser":{"connected":false"#),
        "the body Chromium's supervisor reads, with the browser link not up (CDP port 1): {response}"
    );

    let pid = child.id().ok_or("the binary exited before SIGTERM")?;
    let kill = fork_lock::spawn(Command::new("kill").args(["-TERM", &pid.to_string()]))?
        .wait()
        .await?;
    assert!(kill.success(), "kill -TERM {pid}");
    let output = time::timeout(PATIENCE, child.wait_with_output()).await??;
    let stderr = String::from_utf8(output.stderr)?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "SIGTERM is a normal stop: {stderr}"
    );
    assert!(
        stderr.contains("claw-sdk reads no config file"),
        "the image still sets BROWSERCLAW_CONFIG_FILE, so Neo says it ignores it: {stderr}"
    );
    assert!(
        stderr.contains("BROWSERCLAW_HOSTED_MODE is not"),
        "hosted mode unset is one error line, not a failed start: {stderr}"
    );

    let log = log_file(&scratch.state_dir().join("logs"))?;
    assert!(
        fs::read_to_string(&log)?.contains("SIGTERM: stopping"),
        "the last line before exit reached the file, so main returned and the writer flushed"
    );
    Ok(())
}

/// Today's `claw-sdk.log.<date>` file.
fn log_file(logs: &Path) -> Result<PathBuf, Box<dyn Error>> {
    for entry in fs::read_dir(logs)? {
        let path = entry?.path();
        let is_log = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("claw-sdk.log"));
        if is_log {
            return Ok(path);
        }
    }
    Err(format!("no claw-sdk.log* in {}", logs.display()).into())
}

/// A telemetry value Neo cannot use is a config error like any other: one line naming the
/// variable, exit 1, and nothing bound or written first.
#[tokio::test]
async fn a_telemetry_value_neo_cannot_use_exits_1_naming_it() -> TestResult {
    let scratch = Scratch::new()?;
    let launch = scratch.launch_file(fork_lock::free_port()?)?;
    let output = time::timeout(
        PATIENCE,
        fork_lock::spawn(
            scratch
                .neo()
                .arg("--config")
                .arg(&launch)
                .env("CLAW_TELEMETRY_SITE_DETAIL", "everything"),
        )?
        .wait_with_output(),
    )
    .await??;
    let stderr = String::from_utf8(output.stderr)?;
    assert_eq!(
        output.status.code(),
        Some(1),
        "a config error, which Chromium must not read as a taken port: {stderr}"
    );
    assert!(
        stderr.contains("CLAW_TELEMETRY_SITE_DETAIL"),
        "the operator learns which variable to fix: {stderr}"
    );
    assert!(
        !scratch.state_dir().exists(),
        "the environment is read before logging starts or any state is written"
    );
    Ok(())
}

#[tokio::test]
async fn check_config_reads_the_environment_alone_and_names_what_it_refuses() -> TestResult {
    let scratch = Scratch::new()?;
    let output = fork_lock::spawn(scratch.neo().arg("--check-config"))?
        .wait_with_output()
        .await?;
    assert_eq!(
        (
            output.status.code(),
            output.stdout.as_slice(),
            output.stderr.as_slice()
        ),
        (Some(0), &b""[..], &b""[..]),
        "an environment Neo can start with passes silently: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    for (name, value) in [
        ("OTEL_EXPORTER_OTLP_PROTOCOL", "grpc"),
        ("CLAW_TELEMETRY_SITE_DETAIL", "everything"),
        ("CLAW_HOST_TCP", "not an address"),
    ] {
        let output = fork_lock::spawn(scratch.neo().arg("--check-config").env(name, value))?
            .wait_with_output()
            .await?;
        let stderr = String::from_utf8(output.stderr)?;
        assert_eq!(
            output.status.code(),
            Some(1),
            "{name}: the start would fail on it, so the check does: {stderr}"
        );
        assert!(
            stderr.starts_with(&format!("browseros-claw-sdk: {name}: ")),
            "the image stops with the variable's own error: {stderr}"
        );
        assert!(output.stdout.is_empty(), "{name}: stdout stays empty");
    }
    assert_eq!(
        fs::read_dir(scratch.dir.path())?.count(),
        0,
        "the check writes no state and no log, and reads no launch file"
    );
    Ok(())
}

/// The last telemetry flush is bounded: with push configured to a collector that is down or
/// hung, SIGTERM still exits 0 well inside the 5 s before Chromium's SIGKILL, and the
/// exporter's credential reaches neither stderr nor the log.
#[tokio::test]
async fn sigterm_exits_0_in_time_with_the_collector_down_or_hung() -> TestResult {
    const SECRET: &str = "s3cr3tCollectorToken";
    let hung = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    for collector in [fork_lock::free_port()?, hung.local_addr()?.port()] {
        let scratch = Scratch::new()?;
        let port = fork_lock::free_port()?;
        let launch = scratch.launch_file(port)?;
        let child = fork_lock::spawn(
            scratch
                .neo()
                .arg("--config")
                .arg(&launch)
                .env("CLAW_TELEMETRY_SITE_DETAIL", "url")
                .env(
                    "OTEL_EXPORTER_OTLP_ENDPOINT",
                    format!("http://127.0.0.1:{collector}"),
                )
                .env(
                    "OTEL_EXPORTER_OTLP_HEADERS",
                    format!("authorization=Bearer%20{SECRET}"),
                ),
        )?;
        let response = get_when_up(port, "/system/health").await?;
        assert!(
            response.starts_with("HTTP/1.1 200"),
            "a collector that is not there costs the start nothing: {response}"
        );

        let pid = child.id().ok_or("the binary exited before SIGTERM")?;
        let stopping = Instant::now();
        let kill = fork_lock::spawn(Command::new("kill").args(["-TERM", &pid.to_string()]))?
            .wait()
            .await?;
        assert!(kill.success(), "kill -TERM {pid}");
        let output = time::timeout(PATIENCE, child.wait_with_output()).await??;
        let stopped = stopping.elapsed();
        let stderr = String::from_utf8(output.stderr)?;
        assert_eq!(
            output.status.code(),
            Some(0),
            "collector on port {collector}: an unanswered flush is no failure: {stderr}"
        );
        assert!(
            stopped < Duration::from_secs(4),
            "collector on port {collector}: SIGTERM took {stopped:?}; Chromium kills at 5 s"
        );
        let log = fs::read_to_string(log_file(&scratch.state_dir().join("logs"))?)?;
        assert!(
            !stderr.contains(SECRET) && !log.contains(SECRET),
            "an exporter header's value is never logged"
        );
    }
    Ok(())
}

/// The image build reads the source digest back from the binary to prove the server was compiled
/// from its own checkout (docker/build-server.sh); a build outside the image says it has none.
#[test]
fn binary_embeds_a_source_marker() -> TestResult {
    let binary = fs::read(BIN)?;
    let marker = b"browseros-claw-sdk-source=none;";
    assert!(
        binary.windows(marker.len()).any(|window| window == marker),
        "{BIN} has no source marker; the image build cannot prove which sources it was compiled from"
    );
    Ok(())
}

#[test]
fn binary_embeds_the_packaging_version_marker() -> TestResult {
    let marker = format!("browseros-claw-sdk-version={};", env!("CARGO_PKG_VERSION"));
    let binary = fs::read(BIN)?;
    assert!(
        binary
            .windows(marker.len())
            .any(|window| window == marker.as_bytes()),
        "{BIN} does not contain {marker:?}; the release CLI refuses such a build"
    );
    Ok(())
}
