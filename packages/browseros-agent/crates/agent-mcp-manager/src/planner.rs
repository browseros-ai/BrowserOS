use std::{collections::BTreeMap, path::Path};

use crate::{
    AgentId, AgentScope, DisconnectInput, DisconnectSummary, Error, LinkInput, LinkSummary,
    ListedLink, ManifestLinkEntry, ManifestServerEntry, McpServerSpec, McpTransport, RescanEntry,
    RescanReport, ServerManifest, UnlinkInput, UnlinkSummary,
    catalog::resolve_agent_surface,
    emitter::Emitter,
    io::{AgentFileState, FsOp, Plan, State, serialize_manifest},
};

pub(crate) struct PlannedLink {
    pub(crate) plan: Plan,
    pub(crate) summary: LinkSummary,
}

pub(crate) struct PlannedUnlink {
    pub(crate) plan: Plan,
    pub(crate) summary: UnlinkSummary,
}

pub(crate) struct PlannedDisconnect {
    pub(crate) plan: Plan,
    pub(crate) summary: DisconnectSummary,
}

/// Computes a link plan without touching the filesystem or mutating the state snapshot.
pub(crate) fn plan_link(state: &State, input: &LinkInput, now: &str) -> Result<PlannedLink, Error> {
    let name = input.server.name.trim();
    if name.is_empty() {
        return Err(Error::InvalidServerSpec {
            reason: "server name is required".to_string(),
        });
    }
    validate_spec(&input.server.spec)?;
    let surface =
        ensure_transport_supported(input.agent, input.scope, input.server.spec.transport())?;
    let agent_file = require_agent_file(state, input.agent, input.scope)?;
    ensure_agent_installed(input.agent, agent_file)?;
    let emitter = Emitter::new(surface);

    let existing = state.manifest.servers.get(name);
    let is_known_to_manifest =
        existing.is_some_and(|server| server.links.contains_key(&input.agent));
    let existing_keys = emitter.read(&agent_file.raw_content)?;
    let is_foreign = existing_keys.iter().any(|key| key == name) && !is_known_to_manifest;
    if is_foreign && !input.allow_overwrite {
        return Err(Error::ForeignEntry {
            server_name: name.to_string(),
            agent: input.agent,
            config_path: agent_file.config_path.clone(),
        });
    }

    let next_raw = emitter.add(&agent_file.raw_content, name, &input.server.spec)?;
    let mut links = existing.map_or_else(BTreeMap::new, |server| server.links.clone());
    let created_at = links
        .get(&input.agent)
        .map_or_else(|| now.to_string(), |link| link.created_at.clone());
    links.insert(
        input.agent,
        ManifestLinkEntry {
            config_path: agent_file.config_path.clone(),
            created_at,
        },
    );
    let next_entry = ManifestServerEntry {
        name: name.to_string(),
        spec: input.server.spec.clone(),
        added_at: existing.map_or_else(|| now.to_string(), |server| server.added_at.clone()),
        links,
    };
    let mut next_manifest = state.manifest.clone();
    next_manifest.servers.insert(name.to_string(), next_entry);
    let mut ops = Vec::new();
    if next_raw != agent_file.raw_content {
        ops.push(FsOp::WriteFile {
            path: agent_file.config_path.clone(),
            content: next_raw,
        });
    }
    ops.push(manifest_write_op(state, &next_manifest)?);
    Ok(PlannedLink {
        plan: Plan { ops, next_manifest },
        summary: LinkSummary {
            server_name: name.to_string(),
            agent: input.agent,
            scope: input.scope,
            created: !is_known_to_manifest,
            overwrote_foreign: is_foreign,
        },
    })
}

/// Computes an unlink plan using the manifest-recorded config path.
pub(crate) fn plan_unlink(state: &State, input: &UnlinkInput) -> Result<PlannedUnlink, Error> {
    let no_op = || PlannedUnlink {
        plan: Plan {
            ops: Vec::new(),
            next_manifest: state.manifest.clone(),
        },
        summary: UnlinkSummary {
            server_name: input.server_name.clone(),
            agent: input.agent,
            scope: input.scope,
            removed: false,
        },
    };
    let Some(server) = state.manifest.servers.get(&input.server_name) else {
        return Ok(no_op());
    };
    let Some(link) = server.links.get(&input.agent) else {
        return Ok(no_op());
    };
    let mut ops = Vec::new();
    if let Some(agent_file) = find_agent_file(state, input.agent, input.scope, &link.config_path) {
        let emitter = Emitter::new(resolve_agent_surface(input.agent, input.scope)?);
        let next_raw = emitter.remove(&agent_file.raw_content, &input.server_name)?;
        if next_raw != agent_file.raw_content {
            ops.push(FsOp::WriteFile {
                path: agent_file.config_path.clone(),
                content: next_raw,
            });
        }
    }
    let mut next_manifest = state.manifest.clone();
    if let Some(next_server) = next_manifest.servers.get_mut(&input.server_name) {
        next_server.links.remove(&input.agent);
    }
    ops.push(manifest_write_op(state, &next_manifest)?);
    Ok(PlannedUnlink {
        plan: Plan { ops, next_manifest },
        summary: UnlinkSummary {
            server_name: input.server_name.clone(),
            agent: input.agent,
            scope: input.scope,
            removed: true,
        },
    })
}

/// Computes a single-agent disconnect without producing operations for any other agent.
pub(crate) fn plan_disconnect(
    state: &State,
    input: &DisconnectInput,
) -> Result<PlannedDisconnect, Error> {
    let has_link = state
        .manifest
        .servers
        .get(&input.server_name)
        .is_some_and(|server| server.links.contains_key(&input.agent));
    if !has_link {
        return Ok(PlannedDisconnect {
            plan: Plan {
                ops: Vec::new(),
                next_manifest: state.manifest.clone(),
            },
            summary: DisconnectSummary {
                server_name: input.server_name.clone(),
                agent: input.agent,
                scope: input.scope,
                unlinked: false,
                removed_manifest: false,
            },
        });
    }
    let unlink = plan_unlink(
        state,
        &UnlinkInput {
            server_name: input.server_name.clone(),
            agent: input.agent,
            scope: input.scope,
            config_path: None,
        },
    )?;
    let has_remaining_links = unlink
        .plan
        .next_manifest
        .servers
        .get(&input.server_name)
        .is_some_and(|server| !server.links.is_empty());
    if !input.remove_if_last || has_remaining_links {
        return Ok(PlannedDisconnect {
            plan: unlink.plan,
            summary: DisconnectSummary {
                server_name: input.server_name.clone(),
                agent: input.agent,
                scope: input.scope,
                unlinked: true,
                removed_manifest: false,
            },
        });
    }
    let mut next_manifest = unlink.plan.next_manifest;
    next_manifest.servers.remove(&input.server_name);
    let mut ops = unlink
        .plan
        .ops
        .into_iter()
        .filter(|op| !matches!(op, FsOp::WriteFile { path, .. } if path == &state.manifest_path))
        .collect::<Vec<_>>();
    ops.push(manifest_write_op(state, &next_manifest)?);
    Ok(PlannedDisconnect {
        plan: Plan { ops, next_manifest },
        summary: DisconnectSummary {
            server_name: input.server_name.clone(),
            agent: input.agent,
            scope: input.scope,
            unlinked: true,
            removed_manifest: true,
        },
    })
}

/// Classifies manifest links against their recorded-path file snapshots without writing.
pub(crate) fn plan_rescan(state: &State) -> Result<RescanReport, Error> {
    let mut report = RescanReport::default();
    for server in state.manifest.servers.values() {
        for (agent, link) in &server.links {
            let scope = AgentScope::System;
            let Some(file) = find_agent_file(state, *agent, scope, &link.config_path) else {
                report.missing.push(RescanEntry {
                    server_name: server.name.clone(),
                    agent: *agent,
                    scope,
                    config_path: link.config_path.clone(),
                    reason: "config file was not included in readState".to_string(),
                });
                continue;
            };
            if !file.exists {
                report.missing.push(RescanEntry {
                    server_name: server.name.clone(),
                    agent: *agent,
                    scope,
                    config_path: link.config_path.clone(),
                    reason: "config file does not exist on disk".to_string(),
                });
                continue;
            }
            let emitter = Emitter::new(resolve_agent_surface(*agent, scope)?);
            if emitter
                .read(&file.raw_content)?
                .iter()
                .any(|name| name == &server.name)
            {
                report.verified.push(ListedLink {
                    server_name: server.name.clone(),
                    agent: *agent,
                    config_path: file.config_path.clone(),
                });
            } else {
                report.drifted.push(RescanEntry {
                    server_name: server.name.clone(),
                    agent: *agent,
                    scope,
                    config_path: file.config_path.clone(),
                    reason: "manifest link exists but on-disk config has no matching entry"
                        .to_string(),
                });
            }
        }
    }
    Ok(report)
}

fn validate_spec(spec: &McpServerSpec) -> Result<(), Error> {
    match spec {
        McpServerSpec::Stdio { command, .. } if command.trim().is_empty() => {
            Err(Error::InvalidServerSpec {
                reason: "stdio spec requires a non-empty command".to_string(),
            })
        }
        McpServerSpec::Sse { url, .. } if url.trim().is_empty() => Err(Error::InvalidServerSpec {
            reason: "sse spec requires a non-empty url".to_string(),
        }),
        McpServerSpec::Http { url, .. } if url.trim().is_empty() => Err(Error::InvalidServerSpec {
            reason: "http spec requires a non-empty url".to_string(),
        }),
        _ => Ok(()),
    }
}

fn ensure_transport_supported(
    agent: AgentId,
    scope: AgentScope,
    transport: McpTransport,
) -> Result<crate::AgentSurface, Error> {
    let surface = resolve_agent_surface(agent, scope)?;
    if surface.supported_transports.contains(&transport) {
        return Ok(surface);
    }
    let supported = surface.supported_transports.to_vec();
    Err(Error::UnsupportedTransport {
        agent,
        transport,
        hint: format!(
            "Emit a {} spec for this agent or pick a different agent.",
            supported.first().copied().unwrap_or(McpTransport::Stdio)
        ),
        supported,
    })
}

fn require_agent_file(
    state: &State,
    agent: AgentId,
    scope: AgentScope,
) -> Result<&AgentFileState, Error> {
    state
        .agents
        .iter()
        .find(|file| file.agent == agent && file.scope == scope)
        .ok_or_else(|| Error::InvalidServerSpec {
            reason: format!(
                "agent {agent}@{scope} was not included in readState; add it to the agents list before planning"
            ),
        })
}

fn ensure_agent_installed(agent: AgentId, file: &AgentFileState) -> Result<(), Error> {
    if file.exists || file.parent_exists {
        return Ok(());
    }
    Err(Error::AgentNotInstalled {
        agent,
        config_path: file.config_path.clone(),
        parent_dir: file
            .config_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_path_buf(),
    })
}

fn find_agent_file<'a>(
    state: &'a State,
    agent: AgentId,
    scope: AgentScope,
    config_path: &Path,
) -> Option<&'a AgentFileState> {
    state
        .agents
        .iter()
        .find(|file| file.agent == agent && file.scope == scope && file.config_path == config_path)
        .or_else(|| {
            state
                .agents
                .iter()
                .find(|file| file.agent == agent && file.config_path == config_path)
        })
}

fn manifest_write_op(state: &State, manifest: &ServerManifest) -> Result<FsOp, Error> {
    Ok(FsOp::WriteFile {
        path: state.manifest_path.clone(),
        content: serialize_manifest(manifest)?,
    })
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, path::PathBuf};

    use super::{plan_disconnect, plan_link, plan_rescan, plan_unlink};
    use crate::{
        AgentId, AgentScope, DisconnectInput, Error, LinkInput, ManifestLinkEntry,
        ManifestServerEntry, McpServer, McpServerSpec, ServerManifest,
        io::{AgentFileState, FsOp, State},
    };

    const NOW: &str = "2026-07-06T12:00:00Z";

    fn stdio_spec() -> McpServerSpec {
        McpServerSpec::Stdio {
            command: "gh-mcp".to_string(),
            args: Vec::new(),
            env: BTreeMap::new(),
        }
    }

    fn state(agent: AgentId, raw: &str, parent_exists: bool) -> State {
        State {
            workspace_dir: PathBuf::from("/tmp/ws"),
            manifest_path: PathBuf::from("/tmp/ws/manifest.json"),
            manifest: ServerManifest::default(),
            agents: vec![AgentFileState {
                agent,
                scope: AgentScope::System,
                config_path: PathBuf::from("/tmp/ws/config.json"),
                raw_content: raw.to_string(),
                exists: !raw.is_empty(),
                parent_exists,
            }],
        }
    }

    fn link_input(agent: AgentId) -> LinkInput {
        LinkInput::new(
            McpServer {
                name: "gh".to_string(),
                spec: stdio_spec(),
            },
            agent,
        )
    }

    #[test]
    fn validation_order_checks_transport_before_install_gate() {
        let state = state(AgentId::Codex, "", false);
        let mut input = link_input(AgentId::Codex);
        input.server.spec = McpServerSpec::Sse {
            url: "https://example.com".to_string(),
            headers: BTreeMap::new(),
        };
        assert!(matches!(
            plan_link(&state, &input, NOW),
            Err(Error::UnsupportedTransport { .. })
        ));
    }

    #[test]
    fn validation_order_matches_the_porting_contract() {
        let missing_file_state = State {
            workspace_dir: PathBuf::from("/tmp/ws"),
            manifest_path: PathBuf::from("/tmp/ws/manifest.json"),
            manifest: ServerManifest::default(),
            agents: Vec::new(),
        };
        let mut input = link_input(AgentId::Codex);
        input.server.name = " ".to_string();
        input.server.spec = McpServerSpec::Stdio {
            command: String::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
        };
        assert!(matches!(
            plan_link(&missing_file_state, &input, NOW),
            Err(Error::InvalidServerSpec { reason }) if reason == "server name is required"
        ));

        input.server.name = "gh".to_string();
        assert!(matches!(
            plan_link(&missing_file_state, &input, NOW),
            Err(Error::InvalidServerSpec { reason }) if reason.contains("non-empty command")
        ));

        input.server.spec = McpServerSpec::Sse {
            url: "https://example.com".to_string(),
            headers: BTreeMap::new(),
        };
        assert!(matches!(
            plan_link(&missing_file_state, &input, NOW),
            Err(Error::UnsupportedTransport { .. })
        ));

        input.server.spec = stdio_spec();
        assert!(matches!(
            plan_link(&missing_file_state, &input, NOW),
            Err(Error::InvalidServerSpec { reason }) if reason.contains("was not included in readState")
        ));

        let not_installed = state(
            AgentId::Cursor,
            r#"{"mcpServers":{"gh":{"command":"foreign"}}}"#,
            false,
        );
        let mut not_installed = not_installed;
        not_installed.agents[0].exists = false;
        assert!(matches!(
            plan_link(&not_installed, &link_input(AgentId::Cursor), NOW),
            Err(Error::AgentNotInstalled { .. })
        ));

        let foreign = state(
            AgentId::Cursor,
            r#"{"mcpServers":{"gh":{"command":"foreign"}}}"#,
            true,
        );
        assert!(matches!(
            plan_link(&foreign, &link_input(AgentId::Cursor), NOW),
            Err(Error::ForeignEntry { .. })
        ));
    }

    #[test]
    fn link_trims_names_and_allow_overwrite_adopts_foreign_entries() -> Result<(), Error> {
        let state = state(
            AgentId::Cursor,
            r#"{"mcpServers":{"gh":{"command":"foreign"}}}"#,
            true,
        );
        let mut input = link_input(AgentId::Cursor);
        input.server.name = "  gh  ".to_string();
        input.allow_overwrite = true;
        let planned = plan_link(&state, &input, NOW)?;
        assert_eq!(planned.summary.server_name, "gh");
        assert!(planned.summary.overwrote_foreign);
        assert!(planned.plan.next_manifest.servers.contains_key("gh"));
        Ok(())
    }

    #[test]
    fn relink_preserves_timestamps_and_skips_unchanged_config_write() -> Result<(), Error> {
        let mut state = state(
            AgentId::Cursor,
            "{\n  \"mcpServers\": {\n    \"gh\": {\n      \"command\": \"gh-mcp\"\n    }\n  }\n}",
            true,
        );
        state.manifest.servers.insert(
            "gh".to_string(),
            ManifestServerEntry {
                name: "gh".to_string(),
                spec: stdio_spec(),
                added_at: "old-added".to_string(),
                links: BTreeMap::from([(
                    AgentId::Cursor,
                    ManifestLinkEntry {
                        config_path: PathBuf::from("/tmp/ws/config.json"),
                        created_at: "old-created".to_string(),
                    },
                )]),
            },
        );
        let planned = plan_link(&state, &link_input(AgentId::Cursor), NOW)?;
        assert!(!planned.summary.created);
        let entry = &planned.plan.next_manifest.servers["gh"];
        assert_eq!(entry.added_at, "old-added");
        assert_eq!(entry.links[&AgentId::Cursor].created_at, "old-created");
        assert_eq!(planned.plan.ops.len(), 1);
        assert!(
            matches!(planned.plan.ops[0], FsOp::WriteFile { ref path, .. } if path.ends_with("manifest.json"))
        );
        Ok(())
    }

    #[test]
    fn disconnect_only_writes_the_selected_agent_and_manifest() -> Result<(), Error> {
        let mut state = state(
            AgentId::Cursor,
            r#"{"mcpServers":{"gh":{"command":"gh-mcp"}}}"#,
            true,
        );
        state.manifest.servers.insert(
            "gh".to_string(),
            ManifestServerEntry {
                name: "gh".to_string(),
                spec: stdio_spec(),
                added_at: NOW.to_string(),
                links: BTreeMap::from([
                    (
                        AgentId::Cursor,
                        ManifestLinkEntry {
                            config_path: PathBuf::from("/tmp/ws/config.json"),
                            created_at: NOW.to_string(),
                        },
                    ),
                    (
                        AgentId::VsCode,
                        ManifestLinkEntry {
                            config_path: PathBuf::from("/tmp/ws/vscode.json"),
                            created_at: NOW.to_string(),
                        },
                    ),
                ]),
            },
        );
        let planned = plan_disconnect(&state, &DisconnectInput::new("gh", AgentId::Cursor))?;
        assert!(planned.summary.unlinked);
        assert!(!planned.summary.removed_manifest);
        assert!(planned.plan.ops.iter().all(|op| match op {
            FsOp::WriteFile { path, .. } => !path.ends_with("vscode.json"),
            FsOp::RemoveFile { path } => !path.ends_with("vscode.json"),
        }));
        Ok(())
    }

    #[test]
    fn unlink_unknown_server_or_link_is_a_zero_op() -> Result<(), Error> {
        let state = state(AgentId::Cursor, "", true);
        let unknown = plan_unlink(&state, &crate::UnlinkInput::new("ghost", AgentId::Cursor))?;
        assert!(!unknown.summary.removed);
        assert!(unknown.plan.ops.is_empty());
        Ok(())
    }

    #[test]
    fn disconnect_remove_if_last_false_keeps_empty_links() -> Result<(), Error> {
        let mut state = state(
            AgentId::Cursor,
            r#"{"mcpServers":{"gh":{"command":"gh-mcp"}}}"#,
            true,
        );
        state.manifest.servers.insert(
            "gh".to_string(),
            ManifestServerEntry {
                name: "gh".to_string(),
                spec: stdio_spec(),
                added_at: NOW.to_string(),
                links: BTreeMap::from([(
                    AgentId::Cursor,
                    ManifestLinkEntry {
                        config_path: PathBuf::from("/tmp/ws/config.json"),
                        created_at: NOW.to_string(),
                    },
                )]),
            },
        );
        let mut input = DisconnectInput::new("gh", AgentId::Cursor);
        input.remove_if_last = false;
        let planned = plan_disconnect(&state, &input)?;
        assert!(!planned.summary.removed_manifest);
        assert!(planned.plan.next_manifest.servers["gh"].links.is_empty());
        Ok(())
    }

    #[test]
    fn rescan_reports_exact_reasons_without_ops() -> Result<(), Error> {
        let mut state = state(
            AgentId::Cursor,
            r#"{"mcpServers":{"gh":{"command":"gh-mcp"}}}"#,
            true,
        );
        state.manifest.servers.insert(
            "gh".to_string(),
            ManifestServerEntry {
                name: "gh".to_string(),
                spec: stdio_spec(),
                added_at: NOW.to_string(),
                links: BTreeMap::from([(
                    AgentId::Cursor,
                    ManifestLinkEntry {
                        config_path: PathBuf::from("/tmp/ws/config.json"),
                        created_at: NOW.to_string(),
                    },
                )]),
            },
        );
        assert_eq!(plan_rescan(&state)?.verified.len(), 1);

        state.agents[0].raw_content = r#"{"mcpServers":{}}"#.to_string();
        let drifted = plan_rescan(&state)?;
        assert_eq!(
            drifted.drifted[0].reason,
            "manifest link exists but on-disk config has no matching entry"
        );

        state.agents[0].exists = false;
        let missing = plan_rescan(&state)?;
        assert_eq!(
            missing.missing[0].reason,
            "config file does not exist on disk"
        );

        state.agents.clear();
        let omitted = plan_rescan(&state)?;
        assert_eq!(
            omitted.missing[0].reason,
            "config file was not included in readState"
        );
        Ok(())
    }
}
