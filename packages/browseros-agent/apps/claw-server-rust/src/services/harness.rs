use crate::{
    analytics::{AnalyticsSink, NoopAnalyticsSink, events},
    error::{AppError, AppResult},
};
use harness_integrations::{
    AgentId, AgentScope, DisconnectInput, Error as ManagerError, InspectEntryInput, LinkInput,
    ListLinksFilter, ListedLink, ManifestLinkEntry, ManifestServerEntry, McpManager, McpServer,
    McpServerSpec, MigrateServerInput, ServerManifest, SkillEnvironment, SkillReconcileOutcome,
    SkillReconciler, SkillSpec, is_installed, resolve_agent_surface,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fmt, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process,
    str::FromStr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;
use url::Url;

pub const BROWSEROS_MCP_SERVER_NAME: &str = "browseros-neo";
pub const BROWSEROS_NEO_LEGACY_MCP_SERVER_NAME: &str = "BrowserOS neo";
pub const BROWSERCLAW_LEGACY_MCP_SERVER_NAME: &str = "BrowserClaw";

const BROWSEROS_LEGACY_MCP_SERVER_NAMES: [&str; 2] = [
    BROWSEROS_NEO_LEGACY_MCP_SERVER_NAME,
    BROWSERCLAW_LEGACY_MCP_SERVER_NAME,
];
const BROWSEROS_MCP_SERVER_NAMES: [&str; 3] = [
    BROWSEROS_MCP_SERVER_NAME,
    BROWSEROS_NEO_LEGACY_MCP_SERVER_NAME,
    BROWSERCLAW_LEGACY_MCP_SERVER_NAME,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Harness {
    #[serde(rename = "Claude Code")]
    ClaudeCode,
    #[serde(rename = "Codex")]
    Codex,
    #[serde(rename = "Cursor")]
    Cursor,
    #[serde(rename = "OpenCode")]
    OpenCode,
    #[serde(rename = "Antigravity")]
    Antigravity,
    #[serde(rename = "VS Code")]
    VsCode,
    #[serde(rename = "Zed")]
    Zed,
}

impl Harness {
    pub const ALL: [Self; 7] = [
        Self::ClaudeCode,
        Self::Codex,
        Self::Cursor,
        Self::OpenCode,
        Self::Antigravity,
        Self::VsCode,
        Self::Zed,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
            Self::Cursor => "Cursor",
            Self::OpenCode => "OpenCode",
            Self::Antigravity => "Antigravity",
            Self::VsCode => "VS Code",
            Self::Zed => "Zed",
        }
    }

    #[must_use]
    pub const fn agent_id(self) -> AgentId {
        match self {
            Self::ClaudeCode => AgentId::ClaudeCode,
            Self::Codex => AgentId::Codex,
            Self::Cursor => AgentId::Cursor,
            Self::OpenCode => AgentId::OpenCode,
            Self::Antigravity => AgentId::Antigravity,
            Self::VsCode => AgentId::VsCode,
            Self::Zed => AgentId::Zed,
        }
    }
}

impl fmt::Display for Harness {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Harness {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.replace("%20", " ").as_str() {
            "Claude Code" => Ok(Self::ClaudeCode),
            "Codex" => Ok(Self::Codex),
            "Cursor" => Ok(Self::Cursor),
            "OpenCode" => Ok(Self::OpenCode),
            "Antigravity" => Ok(Self::Antigravity),
            "VS Code" => Ok(Self::VsCode),
            "Zed" => Ok(Self::Zed),
            _ => Err(AppError::bad_request("unsupported harness")),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionState {
    pub harness: Harness,
    pub installed: bool,
    pub agent_id: AgentId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_path: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntegrityScanOutcome {
    pub verified: usize,
    pub drifted: usize,
    pub missing: usize,
    pub healed: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UrlMigrationOutcome {
    pub migrated: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IdentityMigrationOutcome {
    pub migrated: usize,
    pub skipped: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FirstRunConnectOutcome {
    pub connected: usize,
    pub failed: usize,
    pub already_linked: usize,
    /// True when an existing link makes the first-run sweep unnecessary.
    pub seeded_existing: bool,
}

#[derive(Clone)]
pub struct HarnessService {
    manager: McpManager,
    workspace_dir: PathBuf,
    skill_workspace_dir: PathBuf,
    home_dir: PathBuf,
    managed_skill: Option<ManagedSkill>,
    mutex: Arc<Mutex<()>>,
    analytics: Arc<dyn AnalyticsSink>,
}

#[derive(Clone)]
struct ManagedSkill {
    reconciler: SkillReconciler,
    spec: SkillSpec,
    environment: SkillEnvironment,
}

impl HarnessService {
    #[must_use]
    pub fn new(workspace_dir: PathBuf, home_dir: PathBuf) -> Self {
        Self::new_with_analytics(workspace_dir, home_dir, Arc::new(NoopAnalyticsSink))
    }

    #[must_use]
    pub fn new_with_analytics(
        workspace_dir: PathBuf,
        home_dir: PathBuf,
        analytics: Arc<dyn AnalyticsSink>,
    ) -> Self {
        let skill_workspace_dir = default_skill_workspace_dir(&workspace_dir);
        Self::build(
            workspace_dir,
            skill_workspace_dir,
            home_dir,
            analytics,
            None,
        )
    }

    #[must_use]
    pub fn new_with_managed_skill(
        workspace_dir: PathBuf,
        skill_workspace_dir: PathBuf,
        home_dir: PathBuf,
        spec: SkillSpec,
        analytics: Arc<dyn AnalyticsSink>,
    ) -> Self {
        let managed_skill = ManagedSkill {
            reconciler: SkillReconciler::new(skill_workspace_dir.clone()),
            spec,
            environment: SkillEnvironment::current(&home_dir),
        };
        Self::build(
            workspace_dir,
            skill_workspace_dir,
            home_dir,
            analytics,
            Some(managed_skill),
        )
    }

    fn build(
        workspace_dir: PathBuf,
        skill_workspace_dir: PathBuf,
        home_dir: PathBuf,
        analytics: Arc<dyn AnalyticsSink>,
        managed_skill: Option<ManagedSkill>,
    ) -> Self {
        Self {
            manager: McpManager::new(&workspace_dir),
            workspace_dir,
            skill_workspace_dir,
            home_dir,
            managed_skill,
            mutex: Arc::new(Mutex::new(())),
            analytics,
        }
    }

    /// Reconcile a user-authored skill into every connected agent, returning
    /// the agents it was linked into. Serialized with all other skill and
    /// MCP-link mutations through the shared lock so the shared skills.json
    /// manifest stays consistent.
    pub async fn install_skill(&self, spec: SkillSpec) -> AppResult<BTreeSet<AgentId>> {
        let _guard = self.mutex.lock().await;
        let manager = self.manager.clone();
        let workspace_dir = self.workspace_dir.clone();
        let reconciler = SkillReconciler::new(self.skill_workspace_dir.clone());
        let environment = SkillEnvironment::current(&self.home_dir);
        tokio::task::spawn_blocking(move || {
            let consumers = recognized_browseros_links(&manager, &workspace_dir)
                .map_err(manager_app_error)?
                .into_iter()
                .map(|link| link.agent)
                .collect::<BTreeSet<_>>();
            reconciler
                .reconcile(&spec, &consumers, &environment)
                .map_err(manager_app_error)?;
            Ok(consumers)
        })
        .await?
    }

    /// Remove a skill from every agent directory it was linked into. Idempotent.
    pub async fn uninstall_skill(&self, name: &str) -> AppResult<()> {
        let _guard = self.mutex.lock().await;
        let spec = SkillSpec::new(name, String::new()).map_err(manager_app_error)?;
        let reconciler = SkillReconciler::new(self.skill_workspace_dir.clone());
        let environment = SkillEnvironment::current(&self.home_dir);
        tokio::task::spawn_blocking(move || {
            reconciler
                .reconcile(&spec, &BTreeSet::new(), &environment)
                .map(|_| ())
                .map_err(manager_app_error)
        })
        .await?
    }

    /// The agents currently linked to BrowserOS neo over MCP, i.e. the set a
    /// newly authored skill links into by default.
    pub async fn connected_agents(&self) -> AppResult<BTreeSet<AgentId>> {
        let _guard = self.mutex.lock().await;
        let manager = self.manager.clone();
        let workspace_dir = self.workspace_dir.clone();
        tokio::task::spawn_blocking(move || {
            recognized_browseros_links(&manager, &workspace_dir)
                .map(|links| links.into_iter().map(|link| link.agent).collect())
                .map_err(manager_app_error)
        })
        .await?
    }

    pub async fn connect_browseros(
        &self,
        harness: Harness,
        mcp_url: &str,
    ) -> AppResult<ConnectionState> {
        let _guard = self.mutex.lock().await;
        let agent = harness.agent_id();
        let spec = spec_for(agent, mcp_url).map_err(manager_app_error)?;
        let target_mcp_url = mcp_url.to_string();
        let manager = self.manager.clone();
        let workspace_dir = self.workspace_dir.clone();
        let managed_skill = self.managed_skill.clone();
        let result = tokio::task::spawn_blocking(move || {
            let was_connected = recognized_browseros_links(&manager, &workspace_dir)
                .map_err(HarnessOperationError::Manager)?
                .into_iter()
                .any(|link| link.agent == agent);
            let managed_links = managed_browseros_links(&manager, &workspace_dir)
                .map_err(HarnessOperationError::Manager)?;
            if let Err(error) = migrate_browseros_agent(
                &manager,
                &workspace_dir,
                agent,
                &target_mcp_url,
                &managed_links,
            ) {
                if matches!(&error, ManagerError::ForeignEntry { .. }) {
                    return Err(HarnessOperationError::Manager(error));
                }
                tracing::warn!(harness = %harness, agent = %agent, %error, "BrowserOS MCP identity migration before connect failed");
            }
            let canonical_path = managed_browseros_links(&manager, &workspace_dir)
                .map_err(HarnessOperationError::Manager)?
                .into_iter()
                .find(|link| {
                    link.agent == agent && link.server_name == BROWSEROS_MCP_SERVER_NAME
                })
                .map(|link| link.config_path);
            let allow_overwrite = allow_canonical_destination_overwrite(
                &manager,
                agent,
                canonical_path.as_deref(),
                canonical_path.is_some(),
            )
            .map_err(HarnessOperationError::Manager)?;
            let summary = relink_managed_server(
                &manager,
                &workspace_dir,
                BROWSEROS_MCP_SERVER_NAME,
                agent,
                spec,
                allow_overwrite,
                canonical_path.as_deref(),
            )?;
            let config_path = managed_browseros_links(&manager, &workspace_dir)
                .map_err(HarnessOperationError::Manager)?
                .into_iter()
                .find(|link| {
                    link.agent == agent && link.server_name == BROWSEROS_MCP_SERVER_NAME
                })
                .map(|link| link.config_path);
            let skill_warning = managed_skill.as_ref().and_then(|managed_skill| {
                reconcile_skill_warning(&manager, &workspace_dir, managed_skill)
            });
            Ok((
                summary.created && !was_connected,
                config_path,
                skill_warning,
            ))
        })
        .await?;

        match result {
            Ok((created, config_path, skill_warning)) => {
                tracing::info!(harness = %harness, agent = %agent, "connected BrowserOS neo to harness");
                if created {
                    self.analytics.capture(
                        events::HARNESS_CONNECTED,
                        json!({ "harness": harness.as_str() }),
                    );
                }
                Ok(ConnectionState {
                    harness,
                    installed: true,
                    agent_id: agent,
                    config_path: tildify_home_path(config_path.as_deref(), &self.home_dir),
                    message: with_skill_retry_message(
                        format!("BrowserOS registered as an MCP server in {harness}."),
                        skill_warning,
                    ),
                })
            }
            Err(error) => Ok(self.failure(harness, error, "connect")),
        }
    }

    pub async fn disconnect_browseros(&self, harness: Harness) -> AppResult<ConnectionState> {
        let _guard = self.mutex.lock().await;
        let agent = harness.agent_id();
        let manager = self.manager.clone();
        let workspace_dir = self.workspace_dir.clone();
        let managed_skill = self.managed_skill.clone();
        let result = tokio::task::spawn_blocking(move || {
            adopt_config_only_aliases_for_disconnect(&manager, &workspace_dir, agent)
                .map_err(HarnessOperationError::Manager)?;
            let mut unlinked = false;
            let mut removed_manifest = false;
            let mut first_error = None;
            for server_name in BROWSEROS_MCP_SERVER_NAMES {
                match with_legacy_manifest_migration(&workspace_dir, || {
                    manager.disconnect(DisconnectInput::new(server_name, agent))
                }) {
                    Ok(summary) => {
                        unlinked |= summary.unlinked;
                        removed_manifest |= summary.removed_manifest;
                    }
                    Err(error) if first_error.is_none() => first_error = Some(error),
                    Err(_) => {}
                }
            }
            if let Some(error) = first_error {
                return Err(HarnessOperationError::Manager(error));
            }
            let skill_warning = managed_skill.as_ref().and_then(|managed_skill| {
                reconcile_skill_warning(&manager, &workspace_dir, managed_skill)
            });
            Ok((unlinked, removed_manifest, skill_warning))
        })
        .await?;

        match result {
            Ok((unlinked, removed_manifest, skill_warning)) => {
                tracing::info!(
                    harness = %harness,
                    agent = %agent,
                    unlinked,
                    removed_manifest,
                    "disconnected BrowserOS neo from harness"
                );
                if unlinked {
                    self.analytics.capture(
                        events::HARNESS_DISCONNECTED,
                        json!({ "harness": harness.as_str() }),
                    );
                }
                Ok(ConnectionState {
                    harness,
                    installed: false,
                    agent_id: agent,
                    config_path: None,
                    message: with_skill_retry_message(
                        format!("BrowserOS unregistered from {harness}."),
                        skill_warning,
                    ),
                })
            }
            Err(error) => Ok(self.failure(harness, error, "disconnect")),
        }
    }

    pub async fn list_browseros_connections(&self) -> AppResult<Vec<ConnectionState>> {
        let _guard = self.mutex.lock().await;
        let manager = self.manager.clone();
        let workspace_dir = self.workspace_dir.clone();
        let agents = Harness::ALL.map(Harness::agent_id);
        let (links_result, installed_result) = tokio::task::spawn_blocking(move || {
            let links = recognized_browseros_links(&manager, &workspace_dir);
            (links, is_installed(&agents))
        })
        .await?;

        let links = match links_result {
            Ok(links) => links,
            Err(error) => {
                tracing::warn!(error = %error, "list BrowserOS MCP links failed");
                Vec::new()
            }
        };
        let installed = match installed_result {
            Ok(installed) => installed,
            Err(error) => {
                tracing::warn!(error = %error, "probe installed harnesses failed");
                agents.into_iter().map(|agent| (agent, true)).collect()
            }
        };
        let by_agent = deduplicate_browseros_links(links);

        Ok(Harness::ALL
            .into_iter()
            .filter_map(|harness| {
                let agent = harness.agent_id();
                let link = by_agent.get(&agent);
                if link.is_none() && !installed.get(&agent).copied().unwrap_or(false) {
                    return None;
                }
                Some(match link {
                    Some(link) => ConnectionState {
                        harness,
                        installed: true,
                        agent_id: agent,
                        config_path: tildify_home_path(
                            Some(link.config_path.as_path()),
                            &self.home_dir,
                        ),
                        message: format!("Configured in {harness}."),
                    },
                    None => ConnectionState {
                        harness,
                        installed: false,
                        agent_id: agent,
                        config_path: None,
                        message: format!("{harness} is not configured."),
                    },
                })
            })
            .collect())
    }

    pub async fn first_run_connect(&self, mcp_url: &str) -> AppResult<FirstRunConnectOutcome> {
        let connections = self.list_browseros_connections().await?;
        let already_linked = connections.iter().filter(|state| state.installed).count();
        let Some(targets) = first_run_connect_targets(&connections) else {
            return Ok(FirstRunConnectOutcome {
                already_linked,
                seeded_existing: true,
                ..FirstRunConnectOutcome::default()
            });
        };
        let mut outcome = FirstRunConnectOutcome::default();
        for harness in targets {
            match self.connect_browseros(harness, mcp_url).await {
                Ok(_) => outcome.connected += 1,
                Err(error) => {
                    outcome.failed += 1;
                    tracing::warn!(harness = %harness, %error, "first-run auto-connect failed for harness");
                }
            }
        }
        Ok(outcome)
    }

    pub async fn run_integrity_scan(&self) -> AppResult<IntegrityScanOutcome> {
        let _guard = self.mutex.lock().await;
        let manager = self.manager.clone();
        let workspace_dir = self.workspace_dir.clone();
        tokio::task::spawn_blocking(move || run_integrity_scan(&manager, &workspace_dir))
            .await?
            .map_err(manager_app_error)
    }

    pub async fn run_skill_reconciliation(&self) -> AppResult<SkillReconcileOutcome> {
        let _guard = self.mutex.lock().await;
        let Some(managed_skill) = self.managed_skill.clone() else {
            return Ok(SkillReconcileOutcome::default());
        };
        let manager = self.manager.clone();
        let workspace_dir = self.workspace_dir.clone();
        tokio::task::spawn_blocking(move || {
            reconcile_managed_skill(&manager, &workspace_dir, &managed_skill)
        })
        .await?
        .map_err(manager_app_error)
    }

    pub async fn migrate_browseros_identity(
        &self,
        target_mcp_url: &str,
    ) -> AppResult<IdentityMigrationOutcome> {
        let _guard = self.mutex.lock().await;
        let manager = self.manager.clone();
        let workspace_dir = self.workspace_dir.clone();
        let target = target_mcp_url.to_string();
        tokio::task::spawn_blocking(move || {
            migrate_browseros_identity(&manager, &workspace_dir, &target)
        })
        .await?
        .map_err(manager_app_error)
    }

    pub async fn migrate_connected_urls(
        &self,
        target_mcp_url: &str,
    ) -> AppResult<UrlMigrationOutcome> {
        let _guard = self.mutex.lock().await;
        let manager = self.manager.clone();
        let workspace_dir = self.workspace_dir.clone();
        let target = target_mcp_url.to_string();
        tokio::task::spawn_blocking(move || {
            migrate_connected_urls(&manager, &workspace_dir, &target)
        })
        .await?
        .map_err(manager_app_error)
    }

    fn failure(
        &self,
        harness: Harness,
        error: HarnessOperationError,
        operation: &'static str,
    ) -> ConnectionState {
        let agent_id = harness.agent_id();
        match error {
            HarnessOperationError::Manager(ManagerError::AgentNotInstalled { .. }) => {
                tracing::info!(harness = %harness, agent = %agent_id, "harness is not installed");
                ConnectionState {
                    harness,
                    installed: false,
                    agent_id,
                    config_path: None,
                    message: format!(
                        "{harness} is not installed on this machine. Launch it once so the MCP config directory exists, then try again."
                    ),
                }
            }
            HarnessOperationError::Manager(ManagerError::ForeignEntry {
                server_name,
                config_path,
                ..
            }) => {
                tracing::warn!(harness = %harness, %server_name, path = %config_path.display(), "foreign harness entry exists");
                ConnectionState {
                    harness,
                    installed: false,
                    agent_id,
                    config_path: tildify_home_path(Some(&config_path), &self.home_dir),
                    message: format!(
                        "{harness} already has an entry under \"{server_name}\" that we did not write. Remove it from the config and try again."
                    ),
                }
            }
            error => {
                tracing::warn!(harness = %harness, %operation, error = %error, "harness operation failed");
                ConnectionState {
                    harness,
                    installed: operation == "disconnect",
                    agent_id,
                    config_path: None,
                    message: format!("Could not {operation} {harness}: {error}"),
                }
            }
        }
    }
}

fn reconcile_managed_skill(
    manager: &McpManager,
    workspace_dir: &Path,
    managed_skill: &ManagedSkill,
) -> Result<SkillReconcileOutcome, ManagerError> {
    let consumers = recognized_browseros_links(manager, workspace_dir)?
        .into_iter()
        .map(|link| link.agent)
        .collect::<BTreeSet<_>>();
    managed_skill
        .reconciler
        .reconcile(&managed_skill.spec, &consumers, &managed_skill.environment)
}

fn reconcile_skill_warning(
    manager: &McpManager,
    workspace_dir: &Path,
    managed_skill: &ManagedSkill,
) -> Option<String> {
    match reconcile_managed_skill(manager, workspace_dir, managed_skill) {
        Ok(outcome) if outcome.warnings.is_empty() => None,
        Ok(outcome) => {
            let message = outcome
                .warnings
                .into_iter()
                .map(|warning| format!("{}: {}", warning.target.display(), warning.message))
                .collect::<Vec<_>>()
                .join("; ");
            tracing::warn!(warning = %message, "harness skill reconciliation needs a retry");
            Some(message)
        }
        Err(error) => {
            tracing::warn!(%error, "harness skill reconciliation needs a retry");
            Some(error.to_string())
        }
    }
}

fn with_skill_retry_message(message: String, warning: Option<String>) -> String {
    match warning {
        Some(warning) => format!(
            "{message} BrowserOS neo skill reconciliation needs a retry on restart or reconnect: {warning}"
        ),
        None => message,
    }
}

pub fn spec_for(agent: AgentId, mcp_url: &str) -> Result<McpServerSpec, ManagerError> {
    let surface = resolve_agent_surface(agent, AgentScope::System)?;
    if surface
        .supported_transports
        .contains(&harness_integrations::McpTransport::Http)
    {
        return Ok(McpServerSpec::Http {
            url: mcp_url.to_string(),
            headers: BTreeMap::new(),
        });
    }
    Ok(McpServerSpec::Stdio {
        command: "npx".to_string(),
        args: vec!["mcp-remote".to_string(), mcp_url.to_string()],
        env: BTreeMap::new(),
    })
}

fn managed_browseros_links(
    manager: &McpManager,
    workspace_dir: &Path,
) -> Result<Vec<ListedLink>, ManagerError> {
    with_legacy_manifest_migration(workspace_dir, || {
        manager.list_links(ListLinksFilter {
            server_names: Some(
                BROWSEROS_MCP_SERVER_NAMES
                    .into_iter()
                    .map(ToString::to_string)
                    .collect(),
            ),
            agents: None,
        })
    })
}

fn recognized_browseros_links(
    manager: &McpManager,
    workspace_dir: &Path,
) -> Result<Vec<ListedLink>, ManagerError> {
    let mut links = managed_browseros_links(manager, workspace_dir)?;
    let linked_agents = links.iter().map(|link| link.agent).collect::<BTreeSet<_>>();
    for harness in Harness::ALL {
        let agent = harness.agent_id();
        if linked_agents.contains(&agent) {
            continue;
        }
        for server_name in BROWSEROS_LEGACY_MCP_SERVER_NAMES {
            match manager.inspect_entry(InspectEntryInput::new(server_name, agent)) {
                Ok(Some(entry)) if is_historical_browseros_spec(&entry.spec) => {
                    links.push(ListedLink {
                        server_name: server_name.to_string(),
                        agent,
                        config_path: entry.config_path,
                    });
                    break;
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(agent = %agent, %server_name, %error, "legacy BrowserOS MCP entry inspection failed");
                    break;
                }
            }
        }
    }
    Ok(links)
}

fn deduplicate_browseros_links(links: Vec<ListedLink>) -> BTreeMap<AgentId, ListedLink> {
    let mut by_agent = BTreeMap::new();
    for link in links {
        let replace =
            !by_agent.contains_key(&link.agent) || link.server_name == BROWSEROS_MCP_SERVER_NAME;
        if replace {
            by_agent.insert(link.agent, link);
        }
    }
    by_agent
}

fn adopt_config_only_aliases_for_disconnect(
    manager: &McpManager,
    workspace_dir: &Path,
    agent: AgentId,
) -> Result<(), ManagerError> {
    let linked_names = managed_browseros_links(manager, workspace_dir)?
        .into_iter()
        .filter(|link| link.agent == agent)
        .map(|link| link.server_name)
        .collect::<BTreeSet<_>>();
    for source_name in BROWSEROS_LEGACY_MCP_SERVER_NAMES {
        if linked_names.contains(source_name) {
            continue;
        }
        let entry = match manager.inspect_entry(InspectEntryInput::new(source_name, agent)) {
            Ok(Some(entry)) if is_historical_browseros_spec(&entry.spec) => entry,
            Ok(_) => continue,
            Err(error) => {
                tracing::warn!(agent = %agent, %source_name, %error, "legacy BrowserOS MCP disconnect inspection failed");
                continue;
            }
        };
        let allow_destination_overwrite = allow_canonical_destination_overwrite(
            manager,
            agent,
            Some(&entry.config_path),
            linked_names.contains(BROWSEROS_MCP_SERVER_NAME),
        )?;
        let mut input = MigrateServerInput::new(
            source_name,
            McpServer {
                name: BROWSEROS_MCP_SERVER_NAME.to_string(),
                spec: entry.spec.clone(),
            },
            agent,
        );
        input.config_path = Some(entry.config_path);
        input.unmanaged_source_spec = Some(entry.spec);
        input.allow_destination_overwrite = allow_destination_overwrite;
        with_legacy_manifest_migration(workspace_dir, || manager.migrate_server(input.clone()))?;
    }
    Ok(())
}

fn migrate_browseros_identity(
    manager: &McpManager,
    workspace_dir: &Path,
    target_mcp_url: &str,
) -> Result<IdentityMigrationOutcome, ManagerError> {
    let managed_links = managed_browseros_links(manager, workspace_dir)?;
    let mut outcome = IdentityMigrationOutcome::default();
    for harness in Harness::ALL {
        let agent = harness.agent_id();
        match migrate_browseros_agent(
            manager,
            workspace_dir,
            agent,
            target_mcp_url,
            &managed_links,
        ) {
            Ok(true) => {
                outcome.migrated += 1;
                tracing::info!(harness = %harness, agent = %agent, "migrated BrowserOS neo MCP identity");
            }
            Ok(false) => outcome.skipped += 1,
            Err(error) => {
                outcome.failed += 1;
                tracing::warn!(harness = %harness, agent = %agent, %error, "BrowserOS MCP identity migration failed");
            }
        }
    }
    Ok(outcome)
}

fn migrate_browseros_agent(
    manager: &McpManager,
    workspace_dir: &Path,
    agent: AgentId,
    target_mcp_url: &str,
    managed_links: &[ListedLink],
) -> Result<bool, ManagerError> {
    let mut destination_managed = managed_links
        .iter()
        .any(|link| link.agent == agent && link.server_name == BROWSEROS_MCP_SERVER_NAME);
    let mut migrated = if destination_managed {
        false
    } else {
        adopt_config_only_canonical(manager, workspace_dir, agent, target_mcp_url)?
    };
    destination_managed |= migrated;
    for source_name in BROWSEROS_LEGACY_MCP_SERVER_NAMES {
        let legacy_link = managed_links
            .iter()
            .find(|link| link.agent == agent && link.server_name == source_name);
        let alias_migrated = migrate_browseros_alias(
            manager,
            workspace_dir,
            agent,
            target_mcp_url,
            source_name,
            legacy_link,
            destination_managed,
        )?;
        migrated |= alias_migrated;
        destination_managed |= alias_migrated;
    }
    Ok(migrated)
}

fn adopt_config_only_canonical(
    manager: &McpManager,
    workspace_dir: &Path,
    agent: AgentId,
    target_mcp_url: &str,
) -> Result<bool, ManagerError> {
    let entry =
        match manager.inspect_entry(InspectEntryInput::new(BROWSEROS_MCP_SERVER_NAME, agent))? {
            Some(entry) if is_historical_browseros_spec(&entry.spec) => entry,
            _ => return Ok(false),
        };
    let mut input = LinkInput::new(
        McpServer {
            name: BROWSEROS_MCP_SERVER_NAME.to_string(),
            spec: spec_for(agent, target_mcp_url)?,
        },
        agent,
    );
    input.config_path = Some(entry.config_path);
    input.allow_overwrite = true;
    with_legacy_manifest_migration(workspace_dir, || manager.link(input.clone()))
        .map(|summary| summary.created)
}

fn migrate_browseros_alias(
    manager: &McpManager,
    workspace_dir: &Path,
    agent: AgentId,
    target_mcp_url: &str,
    source_name: &str,
    legacy_link: Option<&ListedLink>,
    destination_managed: bool,
) -> Result<bool, ManagerError> {
    let unmanaged_source = if legacy_link.is_none() {
        match manager.inspect_entry(InspectEntryInput::new(source_name, agent))? {
            Some(entry) if is_historical_browseros_spec(&entry.spec) => Some(entry),
            _ => return Ok(false),
        }
    } else {
        None
    };
    let config_path = legacy_link
        .map(|link| link.config_path.clone())
        .or_else(|| {
            unmanaged_source
                .as_ref()
                .map(|entry| entry.config_path.clone())
        });
    let allow_destination_overwrite = allow_canonical_destination_overwrite(
        manager,
        agent,
        config_path.as_deref(),
        destination_managed,
    )?;
    let mut input = MigrateServerInput::new(
        source_name,
        McpServer {
            name: BROWSEROS_MCP_SERVER_NAME.to_string(),
            spec: spec_for(agent, target_mcp_url)?,
        },
        agent,
    );
    input.config_path = config_path;
    input.unmanaged_source_spec = unmanaged_source.map(|entry| entry.spec);
    input.allow_destination_overwrite = allow_destination_overwrite;
    with_legacy_manifest_migration(workspace_dir, || manager.migrate_server(input.clone()))
        .map(|summary| summary.migrated)
}

fn allow_canonical_destination_overwrite(
    manager: &McpManager,
    agent: AgentId,
    config_path: Option<&Path>,
    destination_managed: bool,
) -> Result<bool, ManagerError> {
    if destination_managed {
        return Ok(true);
    }
    let mut input = InspectEntryInput::new(BROWSEROS_MCP_SERVER_NAME, agent);
    if let Some(config_path) = config_path {
        input = input.at_path(config_path);
    }
    Ok(manager
        .inspect_entry(input)?
        .is_some_and(|entry| is_historical_browseros_spec(&entry.spec)))
}

fn is_historical_browseros_spec(spec: &McpServerSpec) -> bool {
    match spec {
        McpServerSpec::Http { url, headers } => {
            headers.is_empty() && is_historical_browseros_url(url)
        }
        McpServerSpec::Stdio { command, args, env } => {
            command == "npx"
                && env.is_empty()
                && args.len() == 2
                && args[0] == "mcp-remote"
                && is_historical_browseros_url(&args[1])
        }
        McpServerSpec::Sse { .. } => false,
    }
}

fn is_historical_browseros_url(raw: &str) -> bool {
    let Ok(url) = Url::parse(raw) else {
        return false;
    };
    url.scheme() == "http"
        && url.host_str() == Some("127.0.0.1")
        && url.port().is_some()
        && url.path() == "/mcp"
        && url.query().is_none()
        && url.fragment().is_none()
        && url.username().is_empty()
        && url.password().is_none()
}

fn relink_managed_server(
    manager: &McpManager,
    workspace_dir: &Path,
    server_name: &str,
    agent: AgentId,
    spec: McpServerSpec,
    allow_overwrite: bool,
    config_path: Option<&Path>,
) -> Result<harness_integrations::LinkSummary, HarnessOperationError> {
    let previous_spec = with_legacy_manifest_migration(workspace_dir, || manager.list())
        .map_err(HarnessOperationError::Manager)?
        .into_iter()
        .find(|server| server.name == server_name)
        .map(|server| server.spec);
    let link = |spec: McpServerSpec| {
        with_legacy_manifest_migration(workspace_dir, || {
            let mut input = LinkInput::new(
                McpServer {
                    name: server_name.to_string(),
                    spec: spec.clone(),
                },
                agent,
            );
            input.allow_overwrite = allow_overwrite;
            input.config_path = config_path.map(Path::to_path_buf);
            manager.link(input)
        })
    };
    match link(spec) {
        Ok(summary) => Ok(summary),
        Err(relink_error) => {
            let Some(previous_spec) = previous_spec else {
                return Err(HarnessOperationError::Manager(relink_error));
            };
            match link(previous_spec) {
                Ok(_) => Err(HarnessOperationError::Manager(relink_error)),
                Err(restore_error) => Err(HarnessOperationError::Relink(format!(
                    "Could not relink {server_name}: {relink_error}; also failed to restore previous link: {restore_error}"
                ))),
            }
        }
    }
}

fn run_integrity_scan(
    manager: &McpManager,
    workspace_dir: &Path,
) -> Result<IntegrityScanOutcome, ManagerError> {
    let report = with_legacy_manifest_migration(workspace_dir, || manager.rescan())?;
    let spec_by_name = with_legacy_manifest_migration(workspace_dir, || manager.list())?
        .into_iter()
        .map(|server| (server.name, server.spec))
        .collect::<BTreeMap<_, _>>();
    let outcome_counts = (
        report.verified.len(),
        report.drifted.len(),
        report.missing.len(),
    );
    let mut to_heal = report.drifted;
    to_heal.extend(report.missing);
    let mut healed = 0;
    let mut failed = 0;

    for entry in to_heal {
        let Some(spec) = spec_by_name.get(&entry.server_name).cloned() else {
            failed += 1;
            tracing::warn!(
                server_name = %entry.server_name,
                agent = %entry.agent,
                reason = %entry.reason,
                "integrity scan found no manifest spec"
            );
            continue;
        };
        let mut input = LinkInput::new(
            McpServer {
                name: entry.server_name.clone(),
                spec,
            },
            entry.agent,
        );
        input.scope = entry.scope;
        input.allow_overwrite = true;
        match with_legacy_manifest_migration(workspace_dir, || manager.link(input.clone())) {
            Ok(_) => {
                healed += 1;
                tracing::info!(
                    server_name = %entry.server_name,
                    agent = %entry.agent,
                    reason = %entry.reason,
                    "integrity scan healed entry"
                );
            }
            Err(error) => {
                failed += 1;
                tracing::warn!(server_name = %entry.server_name, agent = %entry.agent, %error, "integrity scan heal failed");
            }
        }
    }

    Ok(IntegrityScanOutcome {
        verified: outcome_counts.0,
        drifted: outcome_counts.1,
        missing: outcome_counts.2,
        healed,
        failed,
    })
}

/// Retries one failed manager operation after atomically upgrading the legacy Rust manifest.
fn with_legacy_manifest_migration<T>(
    workspace_dir: &Path,
    mut operation: impl FnMut() -> Result<T, ManagerError>,
) -> Result<T, ManagerError> {
    match operation() {
        Err(original @ ManagerError::Manifest { .. }) => {
            let migrated = match migrate_legacy_manifest(workspace_dir) {
                Ok(migrated) => migrated,
                Err(error) => {
                    tracing::warn!(%error, "legacy MCP manifest migration failed");
                    false
                }
            };
            if migrated { operation() } else { Err(original) }
        }
        result => result,
    }
}

fn migrate_legacy_manifest(workspace_dir: &Path) -> Result<bool, String> {
    let path = workspace_dir.join("manifest.json");
    let raw = fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let legacy = match serde_json::from_str::<LegacyManifest>(&raw) {
        Ok(legacy) if legacy.version == 1 => legacy,
        Ok(_) | Err(_) => return Ok(false),
    };
    let mut manifest = ServerManifest {
        version: 1,
        servers: legacy
            .servers
            .into_iter()
            .map(|(name, server)| {
                (
                    name.clone(),
                    ManifestServerEntry {
                        name,
                        spec: server.spec,
                        added_at: server.added_at,
                        links: BTreeMap::new(),
                    },
                )
            })
            .collect(),
    };
    for link in legacy.links {
        let Ok(agent) = AgentId::from_str(&link.agent) else {
            tracing::warn!(agent = %link.agent, "dropping unsupported legacy MCP manifest link");
            continue;
        };
        let server = manifest
            .servers
            .get_mut(&link.server_name)
            .ok_or_else(|| format!("legacy link references missing server {}", link.server_name))?;
        server.links.insert(
            agent,
            ManifestLinkEntry {
                config_path: link.config_path,
                created_at: server.added_at.clone(),
            },
        );
    }
    let mut serialized =
        serde_json::to_string_pretty(&manifest).map_err(|error| error.to_string())?;
    serialized.push('\n');
    atomic_replace(&path, serialized.as_bytes()).map_err(|error| error.to_string())?;
    tracing::info!(path = %path.display(), "migrated legacy MCP manifest");
    Ok(true)
}

fn atomic_replace(path: &Path, content: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "manifest has no parent"))?;
    fs::create_dir_all(parent)?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut temporary_name = OsString::from(path.as_os_str());
    temporary_name.push(format!(".tmp-{}-{nanos}", process::id()));
    let temporary_path = PathBuf::from(temporary_name);
    let write_result = (|| {
        let mut temporary = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)?;
        temporary.write_all(content)?;
        drop(temporary);
        fs::rename(&temporary_path, path)
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    write_result
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyManifest {
    version: u8,
    servers: BTreeMap<String, LegacyServer>,
    links: Vec<LegacyLink>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyServer {
    spec: McpServerSpec,
    added_at: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyLink {
    server_name: String,
    agent: String,
    config_path: PathBuf,
}

fn tildify_home_path(path: Option<&Path>, home_dir: &Path) -> Option<String> {
    let path = path?;
    if path == home_dir {
        return Some("~".to_string());
    }
    match path.strip_prefix(home_dir) {
        Ok(relative) => Some(format!("~/{}", relative.display())),
        Err(_) => Some(path.display().to_string()),
    }
}

fn migrate_connected_urls(
    manager: &McpManager,
    workspace_dir: &Path,
    target_mcp_url: &str,
) -> Result<UrlMigrationOutcome, ManagerError> {
    // Always re-link every connected agent rather than short-circuiting on the
    // manifest URL. A partial migration (crash after some agents relinked but
    // before the rest) leaves the manifest already pointing at the target while
    // stale agents keep the old port; a manifest-level skip would strand them,
    // and the integrity scan only checks server-name presence, not the URL.
    // relink is write-on-change, so agents already on the target aren't rewritten.
    let links =
        deduplicate_browseros_links(managed_browseros_links(manager, workspace_dir)?).into_values();

    let mut outcome = UrlMigrationOutcome::default();
    for link in links {
        let agent = link.agent;
        let spec = match spec_for(agent, target_mcp_url) {
            Ok(spec) => spec,
            Err(error) => {
                outcome.failed += 1;
                tracing::warn!(agent = %agent, error = %error, "URL migration: spec build failed");
                continue;
            }
        };
        match relink_managed_server(
            manager,
            workspace_dir,
            &link.server_name,
            agent,
            spec,
            true,
            Some(&link.config_path),
        ) {
            Ok(_) => outcome.migrated += 1,
            Err(error) => {
                outcome.failed += 1;
                tracing::warn!(agent = %agent, error = %error, "URL migration: relink failed");
            }
        }
    }
    Ok(outcome)
}

fn manager_app_error(error: ManagerError) -> AppError {
    AppError::Internal(error.to_string())
}

/// The reconciler workspace (holding `skills.json`) as a sibling of the MCP
/// manager workspace, matching how `new_with_managed_skill` is wired in
/// `AppState`. Keeps the two reconciler entry points pointed at one manifest.
fn default_skill_workspace_dir(mcp_workspace_dir: &Path) -> PathBuf {
    mcp_workspace_dir
        .parent()
        .map(|parent| parent.join("harness-integrations"))
        .unwrap_or_else(|| mcp_workspace_dir.join("harness-integrations"))
}

#[derive(Debug)]
enum HarnessOperationError {
    Manager(ManagerError),
    Relink(String),
}

impl fmt::Display for HarnessOperationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manager(error) => error.fmt(formatter),
            Self::Relink(message) => formatter.write_str(message),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(source: std::io::Error) -> Self {
        Self::Io { path: None, source }
    }
}

fn first_run_connect_targets(connections: &[ConnectionState]) -> Option<Vec<Harness>> {
    if connections.iter().any(|state| state.installed) {
        return None;
    }
    Some(connections.iter().map(|state| state.harness).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(harness: Harness, installed: bool) -> ConnectionState {
        ConnectionState {
            harness,
            installed,
            agent_id: harness.agent_id(),
            config_path: None,
            message: String::new(),
        }
    }

    #[test]
    fn sweeps_all_when_nothing_linked() {
        let connections = vec![
            conn(Harness::ClaudeCode, false),
            conn(Harness::Cursor, false),
        ];
        assert_eq!(
            first_run_connect_targets(&connections),
            Some(vec![Harness::ClaudeCode, Harness::Cursor])
        );
    }

    #[test]
    fn seeds_existing_install_without_sweeping() {
        let connections = vec![
            conn(Harness::ClaudeCode, true),
            conn(Harness::Cursor, false),
        ];
        assert_eq!(first_run_connect_targets(&connections), None);
    }

    #[test]
    fn empty_machine_sweeps_nothing() {
        assert_eq!(first_run_connect_targets(&[]), Some(Vec::new()));
    }
}
