//! Persists explicit per-agent Disconnect choices independently of MCP links.
//! A missing link is repairable; a saved opt-out is not. Callers hold the harness
//! mutation lock across preferences and configuration writes, including failures.

use std::{collections::BTreeSet, fs, io::ErrorKind, path::Path};

use harness_integrations::{AgentId, Error, McpManager, is_installed};
use serde::{Deserialize, Serialize};

const FILE: &str = "connection-preferences.json";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Preferences {
    version: u8,
    pub(super) disconnected: BTreeSet<AgentId>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            disconnected: BTreeSet::new(),
        }
    }
}

impl Preferences {
    /// Upgrade the old one-shot policy before any writes. It did not store per-app
    /// intent, so an installed but unlinked app on an old profile stays opted out.
    /// Future app installations are not in this snapshot and can connect normally.
    pub(super) fn initialize(workspace: &Path, manager: &McpManager) -> Result<Self, Error> {
        if let Some(preferences) = Self::read_existing(workspace)? {
            return Ok(preferences);
        }
        let mut preferences = Self::default();
        let marker = workspace
            .parent()
            .unwrap_or(workspace)
            .join("first-run-connect.json");
        let upgrading = match fs::read_to_string(&marker) {
            Ok(raw) => serde_json::from_str::<serde_json::Value>(&raw)
                .ok()
                .and_then(|value| value.get("firstRunConnectDone")?.as_bool())
                .unwrap_or(true),
            Err(error) => error.kind() != ErrorKind::NotFound,
        };
        if upgrading {
            // Failure to discover existing ownership is not evidence of consent.
            // Leave preferences absent so the next pass can retry the upgrade.
            let links = super::recognized_browseros_links(manager, workspace)?;
            let agents = super::Harness::ALL.map(super::Harness::agent_id);
            for (agent, installed) in is_installed(&agents)? {
                if !installed || links.iter().any(|link| link.agent == agent) {
                    continue;
                }
                let mut present = false;
                for name in super::BROWSEROS_MCP_SERVER_NAMES {
                    match manager.entry_exists(super::InspectEntryInput::new(name, agent)) {
                        Ok(exists) => present |= exists,
                        Err(error) => {
                            // This old app's choice is unknown, not every app's.
                            // Conservatively require Connect here after repair.
                            tracing::warn!(%agent, %error, "could not infer old connection choice; preserving opt-out");
                            present = false;
                            break;
                        }
                    }
                }
                if !present {
                    preferences.disconnected.insert(agent);
                }
            }
        }
        preferences.save(workspace)?;
        Ok(preferences)
    }

    pub(super) fn read(workspace: &Path) -> Result<Self, Error> {
        Ok(Self::read_existing(workspace)?.unwrap_or_default())
    }

    pub(super) fn read_existing(workspace: &Path) -> Result<Option<Self>, Error> {
        let path = workspace.join(FILE);
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(failure(&path, error)),
        };
        let preferences: Self =
            serde_json::from_str(&raw).map_err(|error| failure(&path, error))?;
        if preferences.version != 1 {
            return Err(failure(&path, "unsupported preferences version"));
        }
        Ok(Some(preferences))
    }

    pub(super) fn save(&self, workspace: &Path) -> Result<(), Error> {
        let path = workspace.join(FILE);
        let raw = serde_json::to_string_pretty(self).map_err(|error| failure(&path, error))? + "\n";
        super::atomic_replace(&path, raw.as_bytes()).map_err(|error| failure(&path, error))
    }

    pub(super) fn set_disconnected(
        workspace: &Path,
        agent: AgentId,
        disabled: bool,
    ) -> Result<(), Error> {
        let mut preferences = Self::read(workspace)?;
        let changed = if disabled {
            preferences.disconnected.insert(agent)
        } else {
            preferences.disconnected.remove(&agent)
        };
        if changed {
            preferences.save(workspace)?;
        }
        Ok(())
    }
}

fn failure(path: &Path, error: impl std::fmt::Display) -> Error {
    Error::Manifest {
        message: format!(
            "Could not use connection preferences at {}: {error}",
            path.display()
        ),
    }
}
