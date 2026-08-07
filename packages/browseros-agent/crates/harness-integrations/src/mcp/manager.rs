use std::{collections::BTreeMap, path::PathBuf};

use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::error::Error;

use super::{
    emitter::Emitter,
    io::{apply_plan, read_state, read_state_at_paths},
    paths::resolve_agent_surface,
    planner::{
        plan_disconnect, plan_link, plan_migration_cleanup, plan_migration_install, plan_rescan,
        plan_unlink, verify_migration_destination,
    },
    types::{
        AgentScope, DisconnectInput, DisconnectSummary, InspectEntryInput, InspectedEntry,
        LinkInput, LinkSummary, ListLinksFilter, ListedLink, ManifestServerEntry,
        MigrateServerInput, MigrateServerSummary, RescanReport, UnlinkInput, UnlinkSummary,
    },
};

#[derive(Debug, Clone)]
pub struct McpManager {
    workspace_dir: PathBuf,
}

impl McpManager {
    /// Binds manager operations to one manifest workspace directory.
    pub fn new(workspace_dir: impl Into<PathBuf>) -> Self {
        Self {
            workspace_dir: workspace_dir.into(),
        }
    }

    /// Links a server through the read, pure-plan, and atomic-apply pipeline.
    pub fn link(&self, input: LinkInput) -> Result<LinkSummary, Error> {
        let overrides = input
            .config_path
            .clone()
            .map(|path| BTreeMap::from([(input.agent, path)]))
            .unwrap_or_default();
        let state = read_state(&self.workspace_dir, &[input.agent], input.scope, &overrides)?;
        let now = current_timestamp()?;
        let planned = plan_link(&state, &input, &now)?;
        apply_plan(&planned.plan)?;
        Ok(planned.summary)
    }

    /// Inspects one entry only when it exactly matches an emitted MCP shape.
    pub fn inspect_entry(&self, input: InspectEntryInput) -> Result<Option<InspectedEntry>, Error> {
        let overrides = input
            .config_path
            .clone()
            .map(|path| BTreeMap::from([(input.agent, path)]))
            .unwrap_or_default();
        let state = read_state(&self.workspace_dir, &[input.agent], input.scope, &overrides)?;
        let file = state
            .agents
            .first()
            .ok_or_else(|| Error::InvalidServerSpec {
                reason: format!(
                    "agent {} was not included in the state snapshot",
                    input.agent
                ),
            })?;
        let emitter = Emitter::new(resolve_agent_surface(input.agent, input.scope)?);
        Ok(emitter
            .inspect(&file.raw_content, &input.server_name)?
            .map(|spec| InspectedEntry {
                server_name: input.server_name,
                agent: input.agent,
                scope: input.scope,
                config_path: file.config_path.clone(),
                spec,
            }))
    }

    /// Installs and verifies a destination before removing one eligible source link.
    pub fn migrate_server(&self, input: MigrateServerInput) -> Result<MigrateServerSummary, Error> {
        let no_op = || MigrateServerSummary {
            source_name: input.source_name.clone(),
            destination_name: input.destination.name.trim().to_string(),
            agent: input.agent,
            scope: input.scope,
            migrated: false,
            overwrote_foreign: false,
        };
        let manifest_state = read_state(
            &self.workspace_dir,
            &[],
            AgentScope::System,
            &BTreeMap::new(),
        )?;
        let recorded_path = manifest_state
            .manifest
            .servers
            .get(&input.source_name)
            .and_then(|server| server.links.get(&input.agent))
            .map(|link| link.config_path.clone());
        let destination_path = manifest_state
            .manifest
            .servers
            .get(input.destination.name.trim())
            .and_then(|server| server.links.get(&input.agent))
            .map(|link| link.config_path.clone());
        let selected_source_path = recorded_path.or_else(|| input.config_path.clone());
        let overrides = selected_source_path
            .map(|path| BTreeMap::from([(input.agent, path)]))
            .unwrap_or_default();
        let source_state =
            read_state(&self.workspace_dir, &[input.agent], input.scope, &overrides)?;
        let source_path = source_state
            .agents
            .first()
            .ok_or_else(|| Error::InvalidServerSpec {
                reason: format!(
                    "agent {} was not included in the state snapshot",
                    input.agent
                ),
            })?
            .config_path
            .clone();
        let destination_path = destination_path.unwrap_or_else(|| source_path.clone());
        let mut paths = vec![(input.agent, destination_path.clone())];
        if source_path != destination_path {
            paths.push((input.agent, source_path.clone()));
        }
        let state = read_state_at_paths(&self.workspace_dir, &paths, input.scope)?;
        let now = current_timestamp()?;
        let Some(install) =
            plan_migration_install(&state, &input, &source_path, &destination_path, &now)?
        else {
            return Ok(no_op());
        };
        let overwrote_foreign = install.overwrote_foreign;
        apply_plan(&install.plan)?;

        let installed = read_state_at_paths(&self.workspace_dir, &paths, input.scope)?;
        if !verify_migration_destination(&installed, &input, &destination_path)? {
            return Err(Error::MigrationVerification {
                reason: format!(
                    "{} was not readable after installation for {}",
                    input.destination.name, input.agent
                ),
            });
        }
        let cleanup = plan_migration_cleanup(&installed, &input, &source_path, &destination_path)?;
        apply_plan(&cleanup)?;
        Ok(MigrateServerSummary {
            source_name: input.source_name,
            destination_name: input.destination.name.trim().to_string(),
            agent: input.agent,
            scope: input.scope,
            migrated: true,
            overwrote_foreign,
        })
    }

    /// Unlinks one manifest-recorded agent entry without touching other agents.
    pub fn unlink(&self, input: UnlinkInput) -> Result<UnlinkSummary, Error> {
        let initial = read_state(
            &self.workspace_dir,
            &[],
            AgentScope::System,
            &BTreeMap::new(),
        )?;
        let recorded_path = initial
            .manifest
            .servers
            .get(&input.server_name)
            .and_then(|server| server.links.get(&input.agent))
            .map(|link| link.config_path.clone());
        let Some(config_path) = input.config_path.clone().or(recorded_path) else {
            let planned = plan_unlink(&initial, &input)?;
            apply_plan(&planned.plan)?;
            return Ok(planned.summary);
        };
        let overrides = BTreeMap::from([(input.agent, config_path)]);
        let state = read_state(&self.workspace_dir, &[input.agent], input.scope, &overrides)?;
        let planned = plan_unlink(&state, &input)?;
        apply_plan(&planned.plan)?;
        Ok(planned.summary)
    }

    /// Disconnects one agent and optionally removes the last-link manifest entry.
    pub fn disconnect(&self, input: DisconnectInput) -> Result<DisconnectSummary, Error> {
        let initial = read_state(
            &self.workspace_dir,
            &[],
            AgentScope::System,
            &BTreeMap::new(),
        )?;
        let recorded_path = initial
            .manifest
            .servers
            .get(&input.server_name)
            .and_then(|server| server.links.get(&input.agent))
            .map(|link| link.config_path.clone());
        let Some(config_path) = recorded_path else {
            let planned = plan_disconnect(&initial, &input)?;
            apply_plan(&planned.plan)?;
            return Ok(planned.summary);
        };
        let state = read_state(
            &self.workspace_dir,
            &[input.agent],
            input.scope,
            &BTreeMap::from([(input.agent, config_path)]),
        )?;
        let planned = plan_disconnect(&state, &input)?;
        apply_plan(&planned.plan)?;
        Ok(planned.summary)
    }

    /// Lists all manifest server entries in deterministic name order.
    pub fn list(&self) -> Result<Vec<ManifestServerEntry>, Error> {
        let state = read_state(
            &self.workspace_dir,
            &[],
            AgentScope::System,
            &BTreeMap::new(),
        )?;
        Ok(state.manifest.servers.into_values().collect())
    }

    /// Lists manifest links after applying optional server and agent filters.
    pub fn list_links(&self, filter: ListLinksFilter) -> Result<Vec<ListedLink>, Error> {
        let state = read_state(
            &self.workspace_dir,
            &[],
            AgentScope::System,
            &BTreeMap::new(),
        )?;
        let mut links = Vec::new();
        for server in state.manifest.servers.values() {
            if filter
                .server_names
                .as_ref()
                .is_some_and(|names| !names.contains(&server.name))
            {
                continue;
            }
            for (agent, link) in &server.links {
                if filter
                    .agents
                    .as_ref()
                    .is_some_and(|agents| !agents.contains(agent))
                {
                    continue;
                }
                links.push(ListedLink {
                    server_name: server.name.clone(),
                    agent: *agent,
                    config_path: link.config_path.clone(),
                });
            }
        }
        Ok(links)
    }

    /// Rescans each manifest link against its recorded configuration path.
    pub fn rescan(&self) -> Result<RescanReport, Error> {
        let manifest_state = read_state(
            &self.workspace_dir,
            &[],
            AgentScope::System,
            &BTreeMap::new(),
        )?;
        let mut paths = manifest_state
            .manifest
            .servers
            .values()
            .flat_map(|server| {
                server
                    .links
                    .iter()
                    .map(|(agent, link)| (*agent, link.config_path.clone()))
            })
            .collect::<Vec<_>>();
        paths.sort_unstable();
        paths.dedup();
        let state = read_state_at_paths(&self.workspace_dir, &paths, AgentScope::System)?;
        plan_rescan(&state)
    }
}

fn current_timestamp() -> Result<String, Error> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| Error::Manifest {
            message: format!("Could not format manifest timestamp: {error}"),
        })
}
