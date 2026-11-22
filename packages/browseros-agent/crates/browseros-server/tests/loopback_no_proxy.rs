//! A child the server starts reaches loopback directly while the person's proxy stays set for
//! its outside calls. A test may not set its own environment, so each one runs its body again in
//! a copy of this binary whose environment points every proxy variable at a closed port.

#![cfg(unix)]

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use anyhow::{Result, ensure};
use browseros_server::browser::{Browser, BrowserConfig};
use tokio_util::sync::CancellationToken;

/// Set in the copy of this binary that `rerun` starts; a test body runs only there.
const RERUN: &str = "BROWSEROS_TEST_RERUN";
/// Nothing listens on the discard port, so a call sent to this proxy fails to connect.
const CLOSED_PROXY: &str = "http://127.0.0.1:9";
const PROXY_VARS: [&str; 6] = [
    "HTTP_PROXY",
    "http_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
];

/// Runs the test `name` in a copy of this binary with every proxy variable at `CLOSED_PROXY`,
/// neither `NO_PROXY` nor `no_proxy`, and then `env`.
fn rerun(name: &str, env: &[(&str, &str)]) -> Result<()> {
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command
        .args([name, "--exact", "--nocapture"])
        .env(RERUN, "1")
        .env_remove("NO_PROXY")
        .env_remove("no_proxy");
    for var in PROXY_VARS {
        command.env(var, CLOSED_PROXY);
    }
    let output = command.envs(env.iter().copied()).output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    ensure!(
        output.status.success() && stdout.contains("1 passed"),
        "{env:?}\n{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[tokio::test]
async fn the_engine_child_keeps_the_proxy_and_bypasses_it_for_loopback() -> Result<()> {
    if std::env::var_os(RERUN).is_none() {
        return rerun(
            "the_engine_child_keeps_the_proxy_and_bypasses_it_for_loopback",
            &[("NO_PROXY", "corp.example")],
        );
    }
    let state = tempfile::tempdir()?;
    // Stands in for the engine: writes down the environment it was started with, then exits.
    let engine = state.path().join("engine");
    std::fs::write(
        &engine,
        "#!/bin/sh\nenv > \"$BROWSERCLAW_DIR/env.part\" && mv \"$BROWSERCLAW_DIR/env.part\" \"$BROWSERCLAW_DIR/env\"\n",
    )?;
    std::fs::set_permissions(&engine, std::fs::Permissions::from_mode(0o700))?;
    let stop = CancellationToken::new();
    let _browser = Browser::start_with_engine_exe(
        BrowserConfig::Neo {
            cdp_port: 9,
            state_dir: state.path().to_owned(),
            execution_dir: state.path().join(".browseros"),
            browseros_version: None,
        },
        9200,
        stop.clone(),
        engine,
    )
    .await;
    let written = state.path().join("neo-engine/env");
    let env = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(env) = std::fs::read_to_string(&written) {
                return env;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    stop.cancel();

    let env: HashMap<&str, &str> = env.lines().filter_map(|l| l.split_once('=')).collect();
    let no_proxy = "corp.example,127.0.0.1,localhost,::1";
    assert_eq!(env.get("NO_PROXY"), Some(&no_proxy));
    assert_eq!(env.get("no_proxy"), Some(&no_proxy));
    for var in PROXY_VARS {
        assert_eq!(env.get(var), Some(&CLOSED_PROXY), "{var}");
    }
    Ok(())
}

/// The semantics the children's `NO_PROXY` relies on, with the client the engine's CDP discovery
/// builds (reqwest's defaults, which follow the proxy variables).
#[tokio::test]
async fn reqwest_skips_a_closed_proxy_for_loopback_only_when_no_proxy_names_it() -> Result<()> {
    const EXPECT: &str = "BROWSEROS_TEST_EXPECT";
    if std::env::var_os(RERUN).is_none() {
        let name = "reqwest_skips_a_closed_proxy_for_loopback_only_when_no_proxy_names_it";
        let loopback = "127.0.0.1,localhost,::1";
        rerun(
            name,
            &[
                ("NO_PROXY", loopback),
                ("no_proxy", loopback),
                (EXPECT, "direct"),
            ],
        )?;
        rerun(name, &[(EXPECT, "proxy")])?;
        // reqwest reads `NO_PROXY` first and stops there when it is set, even empty.
        rerun(
            name,
            &[("NO_PROXY", ""), ("no_proxy", loopback), (EXPECT, "proxy")],
        )?;
        return Ok(());
    }
    let direct = std::env::var(EXPECT)? == "direct";
    let v4 = serve("127.0.0.1:0").await?;
    let v6 = serve("[::1]:0").await?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    for url in [
        format!("http://127.0.0.1:{v4}/"),
        format!("http://localhost:{v4}/"),
        format!("http://[::1]:{v6}/"),
    ] {
        let reply = client.get(&url).send().await;
        if direct {
            ensure!(reply?.text().await? == "direct", "{url}");
        } else {
            ensure!(reply.is_err(), "{url} did not go to the closed proxy");
        }
    }
    Ok(())
}

async fn serve(address: &str) -> Result<u16> {
    let listener = tokio::net::TcpListener::bind(address).await?;
    let port = listener.local_addr()?.port();
    let app = axum::Router::new().route("/", axum::routing::get(|| async { "direct" }));
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(port)
}
