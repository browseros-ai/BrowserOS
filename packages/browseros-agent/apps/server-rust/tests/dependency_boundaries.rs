//! Crate-graph rules of the server-rust design, checked against
//! `cargo metadata` so a stray `Cargo.toml` line fails CI rather than eroding a seam:
//! - `ui-message-stream` is pure vocabulary: no agent, MCP, browser or app crate.
//! - `browseros-agent` reaches browser tools over loopback MCP, never through host/browser crates.
//! - `acp-client` knows nothing about BrowserOS or UI chunks; only it imports the ACP SDK.
//! - rig stays behind `browseros-agent`; apps are leaves; server-rust never uses claw crates.
//!
//! Rules naming crates that do not exist yet are vacuous until those crates land.

use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf, process::Command};

/// Workspace package name → names of everything it depends on (normal, build and dev).
fn workspace_dependencies() -> Result<BTreeMap<String, Vec<String>>, Box<dyn std::error::Error>> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
            "--manifest-path",
        ])
        .arg(&manifest)
        .output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
    }
    let metadata: Value = serde_json::from_slice(&output.stdout)?;
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("cargo metadata has no packages")?;
    Ok(packages
        .iter()
        .filter_map(|package| {
            let name = package.get("name")?.as_str()?.to_owned();
            let dependencies = package
                .get("dependencies")?
                .as_array()?
                .iter()
                .filter_map(|dependency| Some(dependency.get("name")?.as_str()?.to_owned()))
                .collect();
            Some((name, dependencies))
        })
        .collect())
}

const AGENT_CRATES: &[&str] = &["browseros-agent", "acp-client", "acp-agents"];
const BROWSER_CRATES: &[&str] = &[
    "browseros-mcp-host",
    "browseros-mcp",
    "browseros-core",
    "browseros-cdp",
];
const APPS: &[&str] = &["server-rust", "claw-server"];
const RIG: &[&str] = &["rig-core", "rig-reqwest", "rig-bedrock"];

struct Rule {
    krate: &'static str,
    forbidden: Vec<&'static str>,
    why: &'static str,
}

fn rules() -> Vec<Rule> {
    vec![
        Rule {
            krate: "ui-message-stream",
            forbidden: [AGENT_CRATES, BROWSER_CRATES, APPS, RIG].concat(),
            why: "the wire vocabulary knows nothing about agents, browsers or apps",
        },
        Rule {
            krate: "browseros-agent",
            forbidden: [
                BROWSER_CRATES,
                APPS,
                &["acp-client", "acp-agents", "claw-api"],
            ]
            .concat(),
            why: "the built-in agent reaches browser tools over loopback /mcp, not Rust calls",
        },
        Rule {
            krate: "acp-client",
            forbidden: [
                &["ui-message-stream", "browseros-agent", "acp-agents"][..],
                BROWSER_CRATES,
                APPS,
                RIG,
            ]
            .concat(),
            why: "the generic ACP client knows nothing about BrowserOS or UI chunks",
        },
        Rule {
            krate: "acp-agents",
            forbidden: [&["browseros-agent", "browseros-mcp-host"][..], APPS, RIG].concat(),
            why: "ACP policy is independent of the built-in agent and the MCP host",
        },
        Rule {
            krate: "server-rust",
            forbidden: vec!["claw-api", "claw-server"],
            why: "server-rust shares claw's MCP logic only through browseros-mcp-host",
        },
    ]
}

#[test]
fn crate_graph_respects_design_boundaries() -> Result<(), Box<dyn std::error::Error>> {
    let graph = workspace_dependencies()?;
    assert!(
        graph.contains_key("server-rust") && graph.contains_key("ui-message-stream"),
        "cargo metadata did not list the server-rust crates: {:?}",
        graph.keys().collect::<Vec<_>>()
    );
    let mut violations = Vec::new();
    for rule in rules() {
        let Some(dependencies) = graph.get(rule.krate) else {
            continue;
        };
        for dependency in dependencies {
            if rule.forbidden.contains(&dependency.as_str()) {
                violations.push(format!("{} → {dependency}: {}", rule.krate, rule.why));
            }
        }
    }
    for (krate, dependencies) in &graph {
        for dependency in dependencies {
            if APPS.contains(&dependency.as_str()) {
                violations.push(format!("{krate} → {dependency}: apps are leaves"));
            }
            if RIG.contains(&dependency.as_str()) && krate != "browseros-agent" {
                violations.push(format!(
                    "{krate} → {dependency}: rig stays behind browseros-agent"
                ));
            }
            if dependency == "agent-client-protocol" && krate != "acp-client" {
                violations.push(format!(
                    "{krate} → {dependency}: only acp-client imports the ACP SDK"
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "crate boundary violations:\n{}",
        violations.join("\n")
    );
    Ok(())
}
