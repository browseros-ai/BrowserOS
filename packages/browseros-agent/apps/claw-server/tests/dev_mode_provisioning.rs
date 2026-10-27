//! A dev claw-server (tools/dev `dev:claw:watch`) shares the developer's home with their
//! installed BrowserOS neo, and both name their MCP entry `browseros-neo`. These tests launch
//! the real binary against a disposable home holding the installed app's entry, so only a
//! non-dev server may provision agent harnesses on its own.
use std::{
    fs,
    io::{BufRead, BufReader},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};

const INSTALLED_MCP_URL: &str = "http://127.0.0.1:9000/mcp";
/// `main.rs` logs one of these where the boot task decides about harness provisioning.
const PROVISIONING_SKIPPED: &str = "harness provisioning is off";
const PROVISIONING_RAN: &str = "reconciled harness integrations";
const BOOT_TIMEOUT: Duration = Duration::from_secs(60);

#[test]
fn dev_server_leaves_the_installed_apps_harness_configs_alone() -> anyhow::Result<()> {
    let home = Home::seeded()?;
    let server = Server::launch(&home, true)?;
    server.wait_for_any(&[PROVISIONING_SKIPPED, PROVISIONING_RAN])?;

    assert_eq!(home.claude_mcp_url()?, INSTALLED_MCP_URL);
    assert!(
        !home.path().join(".agents/skills/browseros-neo").exists(),
        "a dev server must not install its skill into the shared skill roots"
    );
    Ok(())
}

#[test]
fn installed_server_provisions_harnesses_on_boot() -> anyhow::Result<()> {
    let home = Home::seeded()?;
    let server = Server::launch(&home, false)?;
    server.wait_for_any(&[PROVISIONING_RAN])?;

    assert_eq!(
        home.claude_mcp_url()?,
        format!("http://127.0.0.1:{}/mcp", server.port)
    );
    Ok(())
}

/// A home where Claude Code is installed and already points at the installed neo.
struct Home {
    root: TempDir,
}

impl Home {
    fn seeded() -> anyhow::Result<Self> {
        let root = tempdir()?;
        let home = root.path().join("home");
        fs::create_dir_all(home.join(".claude"))?;
        fs::write(
            home.join(".claude.json"),
            json!({ "mcpServers": { "browseros-neo": { "type": "http", "url": INSTALLED_MCP_URL } } })
                .to_string(),
        )?;
        Ok(Self { root })
    }

    fn path(&self) -> PathBuf {
        self.root.path().join("home")
    }

    fn claude_mcp_url(&self) -> anyhow::Result<String> {
        let config: Value =
            serde_json::from_str(&fs::read_to_string(self.path().join(".claude.json"))?)?;
        Ok(config["mcpServers"]["browseros-neo"]["url"]
            .as_str()
            .unwrap_or_default()
            .to_string())
    }
}

struct Server {
    child: Child,
    port: u16,
    lines: mpsc::Receiver<String>,
}

impl Server {
    /// The environment is cleared so no developer variable (CLAUDE_CONFIG_DIR, XDG_*) can
    /// point harness discovery back at the real machine.
    fn launch(home: &Home, dev_mode: bool) -> anyhow::Result<Self> {
        let port = free_port()?;
        let sidecar = write_sidecar(home.root.path(), port, free_port()?, dev_mode)?;
        let mut child = Command::new(env!("CARGO_BIN_EXE_browseros-claw-server-rs"))
            .arg("--config")
            .arg(&sidecar)
            .env_clear()
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .env("PATH", "")
            .env("BROWSERCLAW_DIR", home.root.path().join("state"))
            .env("CLAW_ANALYTICS_ENABLED", "false")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("server stderr is not piped"))?;
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    return;
                }
            }
        });
        Ok(Self { child, port, lines })
    }

    fn wait_for_any(&self, needles: &[&str]) -> anyhow::Result<()> {
        let deadline = Instant::now() + BOOT_TIMEOUT;
        let mut seen = Vec::new();
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            match self.lines.recv_timeout(remaining) {
                Ok(line) if needles.iter().any(|needle| line.contains(needle)) => return Ok(()),
                Ok(line) => seen.push(line),
                Err(_) => break,
            }
        }
        anyhow::bail!(
            "server never logged any of {needles:?}:\n{}",
            seen.join("\n")
        )
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn write_sidecar(root: &Path, server: u16, cdp: u16, dev_mode: bool) -> anyhow::Result<PathBuf> {
    let resources = root.join("resources");
    fs::create_dir_all(&resources)?;
    let path = root.join("sidecar.json");
    fs::write(
        &path,
        json!({
            "ports": { "server": server, "cdp": cdp, "proxy": server },
            "directories": { "resources": resources },
            "flags": { "devMode": dev_mode },
        })
        .to_string(),
    )?;
    Ok(path)
}

fn free_port() -> anyhow::Result<u16> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}
