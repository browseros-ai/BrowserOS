//! BrowserOS's curated MCP integrations over harness-integrations. File planning
//! and atomic config writes stay in that crate; this module owns URL selection,
//! wire errors, serialization of manifest mutations, and conservative self-heal.

use harness_integrations::{
    AgentId, AgentScope, DisconnectInput, Error, InspectEntryInput, LinkInput, ListLinksFilter,
    McpManager, McpServer, McpServerSpec, McpTransport, UnlinkInput, detect_installed_agents,
    resolve_agent_surface,
};
use serde::Serialize;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

const NAMES: [&str; 2] = ["browseros", "browseros-stdio"];
const REPLACE_EDITED: &str = "Cannot replace a user-edited entry. Please remove BrowserOS from this agent's config manually and try again.";
const REMOVE_EDITED: &str = "Cannot remove a user-edited entry. Please remove BrowserOS from this agent manually and try again.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentStatus {
    pub id: String,
    #[serde(rename = "displayName")]
    pub name: String,
    #[serde(skip)]
    pub detected: bool,
    pub installed: bool,
    pub linked: bool,
    #[serde(rename = "configPath")]
    pub config_path: Option<PathBuf>,
}
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct McpManagerError(pub String);
impl McpManagerError {
    pub fn status(&self) -> http::StatusCode {
        if self.0.starts_with("Agent \"") && self.0.ends_with("\" is not supported.") {
            http::StatusCode::NOT_FOUND
        } else if self.0 == REPLACE_EDITED
            || self.0 == REMOVE_EDITED
            || self.0.starts_with("This agent is installed but")
        {
            http::StatusCode::CONFLICT
        } else if self.0.starts_with("This agent does not support") {
            http::StatusCode::BAD_REQUEST
        } else {
            http::StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
impl From<Error> for McpManagerError {
    fn from(error: Error) -> Self {
        Self(match error {
            Error::AgentNotSupported { agent } => format!("Agent \"{agent}\" is not supported."),
            Error::ForeignEntry { .. } => REPLACE_EDITED.into(),
            Error::AgentNotInstalled { .. } => "This agent is installed but its MCP config location is not writable yet. Open the app once so it creates its config, then try again.".into(),
            error @ Error::UnsupportedTransport { .. } => format!("This agent does not support BrowserOS's MCP transport. {error}"),
            error => error.to_string(),
        })
    }
}

/// Clones share the mutation lock. Holding it inside spawn_blocking is essential:
/// dropping an HTTP request must not unlock while its config write still runs.
#[derive(Debug, Clone)]
pub struct McpManagerService {
    manifest_dir: PathBuf,
    public_url: Option<String>,
    lock: Arc<Mutex<()>>,
}
impl McpManagerService {
    pub fn new(manifest_dir: PathBuf, public_url: Option<String>) -> Self {
        Self {
            manifest_dir,
            public_url,
            lock: Arc::new(Mutex::new(())),
        }
    }
    async fn run<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&McpManager) -> Result<T, McpManagerError> + Send + 'static,
    ) -> Result<T, McpManagerError> {
        let guard = self.lock.clone().lock_owned().await;
        let dir = self.manifest_dir.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            with_legacy_manifest(dir, operation)
        })
        .await
        .map_err(|e| McpManagerError(e.to_string()))?
    }
    pub async fn agents(&self) -> Result<Vec<AgentStatus>, McpManagerError> {
        self.run(|manager| {
            let detected = detect_installed_agents()?;
            let links = manager.list_links(filter())?;
            let mut rows = Vec::new();
            for id in AgentId::ALL {
                let detection = detected.iter().find(|a| a.id == id);
                let link = links.iter().rev().find(|l| l.agent == id);
                let installed = link.is_some() || detection.is_some_and(|a| a.installed);
                if !installed {
                    continue;
                }
                rows.push(AgentStatus {
                    id: id.to_string(),
                    name: detection
                        .map(|a| a.display_name.clone())
                        .unwrap_or_else(|| id.to_string()),
                    detected: detection.is_some_and(|a| a.installed),
                    installed,
                    linked: link.is_some(),
                    config_path: link
                        .map(|l| l.config_path.clone())
                        .or_else(|| detection.and_then(|a| a.config_path.clone())),
                });
            }
            Ok(rows)
        })
        .await
    }
    pub async fn install(&self, agent_id: &str) -> Result<(), McpManagerError> {
        let url = self.public_url.as_deref().ok_or_else(|| {
            McpManagerError(
                "MCP URL is not configured; supply the current URL with install_at".into(),
            )
        })?;
        self.install_at(agent_id, url).await
    }
    /// The route supplies the proxy URL, or its actual listening port as fallback.
    pub async fn install_at(&self, agent_id: &str, url: &str) -> Result<(), McpManagerError> {
        let agent: AgentId = agent_id.parse()?;
        let url = url.to_owned();
        self.run(move |manager| {
            let server = plan(agent, &url)?;
            for name in NAMES.into_iter().filter(|n| *n != server.name) {
                if is_edited(manager, name, agent)? {
                    continue;
                }
                match manager.unlink(UnlinkInput::new(name, agent)) {
                    Ok(_) | Err(Error::ForeignEntry { .. }) => {}
                    Err(e) => return Err(e.into()),
                }
            }
            let existing_path = manager
                .list()?
                .into_iter()
                .find(|s| s.name == server.name)
                .and_then(|s| s.links.get(&agent).map(|l| l.config_path.clone()));
            let mut input = LinkInput::new(server, agent);
            input.config_path = existing_path;
            input.allow_overwrite = true;
            manager.link(input)?;
            Ok(())
        })
        .await
    }
    pub async fn uninstall(&self, agent_id: &str) -> Result<(), McpManagerError> {
        let agent: AgentId = agent_id.parse()?;
        self.run(move |manager| {
            let mut edited = false;
            for name in NAMES {
                // The shared planner removes by name alone. Verify exact emitted
                // ownership first, so a user customization survives uninstall.
                if is_edited(manager, name, agent)? {
                    edited = true;
                    continue;
                }
                manager.disconnect(DisconnectInput::new(name, agent))?;
            }
            if edited {
                Err(McpManagerError(REMOVE_EDITED.into()))
            } else {
                Ok(())
            }
        })
        .await
    }
    /// Called once by main without delaying startup. A missing proxy URL skips URL
    /// repair; absent/edited configs never become new installs during self-heal.
    pub async fn self_heal(&self) {
        let Some(url) = self.public_url.clone() else {
            return;
        };
        if let Err(error) = self
            .run(move |manager| {
                let report = manager.rescan()?;
                for link in report
                    .verified
                    .into_iter()
                    .filter(|l| NAMES.contains(&l.server_name.as_str()))
                {
                    match is_edited(manager, &link.server_name, link.agent) {
                        Ok(false) => {}
                        Ok(true) => continue,
                        Err(error) => {
                            tracing::warn!(%error, "MCP ownership check failed for agent");
                            continue;
                        }
                    }
                    let mut input = LinkInput::new(
                        McpServer {
                            name: link.server_name.clone(),
                            spec: spec(&link.server_name, &url),
                        },
                        link.agent,
                    );
                    // Always use the recorded path, including imported custom paths.
                    input.config_path = Some(link.config_path);
                    input.allow_overwrite = true;
                    if let Err(error) = manager.link(input) {
                        tracing::warn!(%error, "MCP URL self-heal failed for agent");
                    }
                }
                Ok(())
            })
            .await
        {
            tracing::warn!(%error, "MCP self-heal failed");
        }
    }
}
fn filter() -> ListLinksFilter {
    ListLinksFilter {
        server_names: Some(NAMES.map(String::from).to_vec()),
        agents: None,
    }
}
fn spec(name: &str, url: &str) -> McpServerSpec {
    if name == "browseros-stdio" {
        McpServerSpec::Stdio {
            command: "npx".into(),
            args: vec!["mcp-remote".into(), url.into()],
            env: BTreeMap::new(),
        }
    } else {
        McpServerSpec::Http {
            url: url.into(),
            headers: BTreeMap::new(),
        }
    }
}
fn plan(agent: AgentId, url: &str) -> Result<McpServer, Error> {
    let surface = resolve_agent_surface(agent, AgentScope::System)?;
    let name = if surface.supported_transports.contains(&McpTransport::Http) {
        "browseros"
    } else {
        "browseros-stdio"
    };
    Ok(McpServer {
        name: name.into(),
        spec: spec(name, url),
    })
}
fn is_edited(manager: &McpManager, name: &str, agent: AgentId) -> Result<bool, Error> {
    let Some(server) = manager.list()?.into_iter().find(|s| s.name == name) else {
        return Ok(false);
    };
    let Some(link) = server.links.get(&agent) else {
        return Ok(false);
    };
    let input = InspectEntryInput::new(name, agent).at_path(&link.config_path);
    if !manager.entry_exists(input.clone())? {
        return Ok(false);
    }
    Ok(manager
        .inspect_entry(input)?
        .is_none_or(|entry| !same_owned_spec(&entry.spec, &server.spec)))
}

// A single manifest spec is shared by every linked agent. A previous repair may
// have updated it before another agent's config was writable, so port drift alone
// cannot disown that still-managed entry on the next boot.
fn same_owned_spec(actual: &McpServerSpec, recorded: &McpServerSpec) -> bool {
    fn endpoint(a: &str, b: &str) -> bool {
        match (url::Url::parse(a), url::Url::parse(b)) {
            (Ok(mut a), Ok(mut b)) => {
                let _ = a.set_port(None);
                let _ = b.set_port(None);
                a == b
            }
            _ => a == b,
        }
    }
    match (actual, recorded) {
        (
            McpServerSpec::Http {
                url: a,
                headers: ah,
            },
            McpServerSpec::Http {
                url: b,
                headers: bh,
            },
        ) => ah == bh && endpoint(a, b),
        (
            McpServerSpec::Stdio {
                command: a,
                args: aa,
                env: ae,
            },
            McpServerSpec::Stdio {
                command: b,
                args: ba,
                env: be,
            },
        ) if a == "npx"
            && b == "npx"
            && aa.len() == 2
            && ba.len() == 2
            && aa[0] == "mcp-remote"
            && ba[0] == "mcp-remote" =>
        {
            ae == be && endpoint(&aa[1], &ba[1])
        }
        _ => actual == recorded,
    }
}

/// Older TS catalogs knew more than the seven Rust agents. Operate on a private
/// manifest projection and merge opaque legacy links back, keeping their configs
/// untouched. Publishing only after the planner finishes also preserves the real
/// manifest while preserving bookkeeping from any writes completed before an error.
fn with_legacy_manifest<T>(
    dir: PathBuf,
    operation: impl FnOnce(&McpManager) -> Result<T, McpManagerError>,
) -> Result<T, McpManagerError> {
    use serde_json::{Value, json};
    use std::fs;
    let manifest_path = dir.join("manifest.json");
    let Some(raw) = fs::read_to_string(&manifest_path).ok() else {
        return operation(&McpManager::new(dir));
    };
    let Ok(mut value) = serde_json::from_str::<Value>(&raw) else {
        return operation(&McpManager::new(dir));
    };
    let mut legacy = BTreeMap::new();
    if let Some(servers) = value.get_mut("servers").and_then(Value::as_object_mut) {
        for (name, server) in servers {
            let original = server.clone();
            if let Some(links) = server.get_mut("links").and_then(Value::as_object_mut) {
                let unknown: serde_json::Map<String, Value> = links
                    .iter()
                    .filter(|(id, _)| !harness_integrations::is_agent_supported(id))
                    .map(|(id, v)| (id.clone(), v.clone()))
                    .collect();
                if !unknown.is_empty() {
                    links.retain(|id, _| harness_integrations::is_agent_supported(id));
                    legacy.insert(name.clone(), (original, unknown));
                }
            }
        }
    }
    if legacy.is_empty() {
        return operation(&McpManager::new(dir));
    }
    let stage = dir.join(format!(".legacy-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&stage).map_err(io_error)?;
    // RAII removes only this operation's private manifest, never owner state.
    struct Stage(PathBuf);
    impl Drop for Stage {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _stage = Stage(stage.clone());
    let stage_manifest = stage.join("manifest.json");
    fs::write(&stage_manifest, value.to_string()).map_err(io_error)?;
    let result = operation(&McpManager::new(&stage));
    let next_raw = fs::read_to_string(&stage_manifest).map_err(io_error)?;
    let mut next: Value =
        serde_json::from_str(&next_raw).map_err(|e| McpManagerError(e.to_string()))?;
    for (name, (original, links)) in legacy {
        if next["servers"].get(&name).is_none() {
            let mut legacy_server = original;
            // Removing the final curated link drops the projected server. Keep
            // only its opaque links; restoring the full original would revive
            // every curated link that this operation just disconnected.
            legacy_server["links"] = json!({});
            next["servers"][&name] = legacy_server;
        }
        for (id, link) in links {
            next["servers"][&name]["links"][id] = link;
        }
    }
    if next != serde_json::from_str::<Value>(&raw).unwrap_or(json!(null)) {
        let publish = stage.join("publish.json");
        fs::write(
            &publish,
            format!(
                "{}\n",
                serde_json::to_string_pretty(&next).map_err(|e| McpManagerError(e.to_string()))?
            ),
        )
        .map_err(io_error)?;
        fs::rename(publish, manifest_path).map_err(io_error)?;
    }
    result
}
fn io_error(error: std::io::Error) -> McpManagerError {
    McpManagerError(error.to_string())
}
