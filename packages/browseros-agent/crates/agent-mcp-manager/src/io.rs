use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    process,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

use crate::{
    AgentId, AgentScope, Error, ServerManifest,
    catalog::{ensure_system_scope, resolve_agent_mcp_config_path},
    paths::path_exists,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct State {
    pub(crate) workspace_dir: PathBuf,
    pub(crate) manifest_path: PathBuf,
    pub(crate) manifest: ServerManifest,
    pub(crate) agents: Vec<AgentFileState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentFileState {
    pub(crate) agent: AgentId,
    pub(crate) scope: AgentScope,
    pub(crate) config_path: PathBuf,
    pub(crate) raw_content: String,
    pub(crate) exists: bool,
    pub(crate) parent_exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FsOp {
    WriteFile {
        path: PathBuf,
        content: String,
    },
    // Plans retain ordered removal support even though the current public verbs only rewrite files.
    #[allow(dead_code)]
    RemoveFile {
        path: PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) ops: Vec<FsOp>,
    pub(crate) next_manifest: ServerManifest,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ApplyPlanResult {
    pub(crate) written_paths: Vec<PathBuf>,
    pub(crate) removed_paths: Vec<PathBuf>,
}

/// Snapshots the manifest and one config file for every requested agent.
pub(crate) fn read_state(
    workspace_dir: &Path,
    agents: &[AgentId],
    scope: AgentScope,
    overrides: &BTreeMap<AgentId, PathBuf>,
) -> Result<State, Error> {
    let manifest = read_manifest(workspace_dir)?;
    let mut paths = Vec::with_capacity(agents.len());
    for agent in agents {
        ensure_system_scope(*agent, scope)?;
        let config_path = match overrides.get(agent) {
            Some(path) => path.clone(),
            None => resolve_agent_mcp_config_path(*agent, scope)?,
        };
        paths.push((*agent, config_path));
    }
    snapshot_state(workspace_dir, manifest, &paths, scope)
}

/// Snapshots every explicit agent and config-path pair, including repeated agents.
pub(crate) fn read_state_at_paths(
    workspace_dir: &Path,
    paths: &[(AgentId, PathBuf)],
    scope: AgentScope,
) -> Result<State, Error> {
    let manifest = read_manifest(workspace_dir)?;
    for (agent, _) in paths {
        ensure_system_scope(*agent, scope)?;
    }
    snapshot_state(workspace_dir, manifest, paths, scope)
}

fn snapshot_state(
    workspace_dir: &Path,
    manifest: ServerManifest,
    paths: &[(AgentId, PathBuf)],
    scope: AgentScope,
) -> Result<State, Error> {
    let mut agent_files = Vec::with_capacity(paths.len());
    for (agent, config_path) in paths {
        let (raw_content, exists) = read_file_with_existence(config_path)?;
        let parent_exists = if exists {
            true
        } else {
            match config_path.parent() {
                Some(parent) => path_exists(parent)?,
                None => false,
            }
        };
        agent_files.push(AgentFileState {
            agent: *agent,
            scope,
            config_path: config_path.clone(),
            raw_content,
            exists,
            parent_exists,
        });
    }
    Ok(State {
        workspace_dir: workspace_dir.to_path_buf(),
        manifest_path: workspace_dir.join("manifest.json"),
        manifest,
        agents: agent_files,
    })
}

/// Applies writes in plan order, then applies removals while ignoring missing files.
pub(crate) fn apply_plan(plan: &Plan) -> Result<ApplyPlanResult, Error> {
    let mut result = ApplyPlanResult::default();
    for op in &plan.ops {
        if let FsOp::WriteFile { path, content } = op {
            atomic_write_file(path, content)?;
            result.written_paths.push(path.clone());
        }
    }
    for op in &plan.ops {
        if let FsOp::RemoveFile { path } = op {
            match fs::remove_file(path) {
                Ok(()) => result.removed_paths.push(path.clone()),
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(Error::io("remove", path, error)),
            }
        }
    }
    Ok(result)
}

pub(crate) fn serialize_manifest(manifest: &ServerManifest) -> Result<String, Error> {
    serde_json::to_string_pretty(manifest)
        .map(|serialized| format!("{serialized}\n"))
        .map_err(|error| Error::Manifest {
            message: format!("Could not serialize manifest: {error}"),
        })
}

fn read_manifest(workspace_dir: &Path) -> Result<ServerManifest, Error> {
    let path = workspace_dir.join("manifest.json");
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(ServerManifest::default()),
        Err(error) => return Err(Error::io("read", path, error)),
    };
    if raw.trim().is_empty() {
        return Ok(ServerManifest::default());
    }
    let value: Value = serde_json::from_str(&raw).map_err(|_| Error::Manifest {
        message: format!(
            "Manifest at {} is not valid JSON. Inspect and repair or delete to start fresh.",
            path.display()
        ),
    })?;
    let Some(object) = value.as_object() else {
        return Err(Error::Manifest {
            message: format!("Manifest at {} is not an object.", path.display()),
        });
    };
    if object.get("version").and_then(Value::as_u64) != Some(1) {
        let version = object
            .get("version")
            .map_or_else(|| "undefined".to_string(), Value::to_string);
        return Err(Error::Manifest {
            message: format!(
                "Manifest at {} has unsupported version {version}; expected 1.",
                path.display()
            ),
        });
    }
    if !object.get("servers").is_some_and(Value::is_object) {
        return Err(Error::Manifest {
            message: format!(
                "Manifest at {} is missing a valid `servers` object.",
                path.display()
            ),
        });
    }
    serde_json::from_value(value).map_err(|error| Error::Manifest {
        message: format!(
            "Manifest at {} has invalid schema: {error}.",
            path.display()
        ),
    })
}

fn read_file_with_existence(path: &Path) -> Result<(String, bool), Error> {
    match fs::read_to_string(path) {
        Ok(content) => Ok((content, true)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok((String::new(), false)),
        Err(error) => Err(Error::io("read", path, error)),
    }
}

fn atomic_write_file(path: &Path, content: &str) -> Result<(), Error> {
    let parent = path.parent().ok_or_else(|| {
        Error::io(
            "create parent directory for",
            path,
            std::io::Error::new(ErrorKind::InvalidInput, "path has no parent directory"),
        )
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| Error::io("create parent directory for", path, error))?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut temporary_name = OsString::from(path.as_os_str());
    temporary_name.push(format!(".tmp-{}-{nanos}", process::id()));
    let temporary_path = PathBuf::from(temporary_name);
    let mut temporary_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)
        .map_err(|error| Error::io("create temporary file", &temporary_path, error))?;
    if let Err(error) = temporary_file.write_all(content.as_bytes()) {
        let _ = fs::remove_file(&temporary_path);
        return Err(Error::io("write temporary file", &temporary_path, error));
    }
    drop(temporary_file);
    let temporary = tempfile::TempPath::try_from_path(temporary_path.clone()).map_err(|error| {
        let _ = fs::remove_file(&temporary_path);
        Error::io("prepare temporary file", &temporary_path, error)
    })?;
    temporary
        .persist(path)
        .map_err(|error| Error::io("rename temporary file over", path, error.error))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{FsOp, Plan, apply_plan, atomic_write_file, read_state};
    use crate::{AgentId, AgentScope, ServerManifest};

    #[test]
    fn empty_existing_config_file_is_reported_as_existing() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = tempdir()?;
        let config = root.path().join("empty.json");
        fs::write(&config, "")?;
        let state = read_state(
            root.path(),
            &[AgentId::Cursor],
            AgentScope::System,
            &[(AgentId::Cursor, config)].into_iter().collect(),
        )?;
        assert!(state.agents[0].exists);
        assert!(state.agents[0].raw_content.is_empty());
        Ok(())
    }

    #[test]
    fn apply_plan_writes_atomically_and_ignores_missing_removals()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let target = root.path().join("nested/config.json");
        let plan = Plan {
            ops: vec![
                FsOp::WriteFile {
                    path: target.clone(),
                    content: "{}".to_string(),
                },
                FsOp::RemoveFile {
                    path: root.path().join("missing"),
                },
            ],
            next_manifest: ServerManifest::default(),
        };
        let result = apply_plan(&plan)?;
        assert_eq!(fs::read_to_string(target)?, "{}");
        assert_eq!(result.written_paths.len(), 1);
        assert!(result.removed_paths.is_empty());
        Ok(())
    }

    #[test]
    fn malformed_or_wrong_version_manifests_are_hard_errors()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        fs::write(root.path().join("manifest.json"), "{ nope")?;
        assert!(read_state(root.path(), &[], AgentScope::System, &Default::default()).is_err());
        fs::write(
            root.path().join("manifest.json"),
            r#"{"version":2,"servers":{}}"#,
        )?;
        assert!(read_state(root.path(), &[], AgentScope::System, &Default::default()).is_err());
        Ok(())
    }

    #[test]
    fn failed_atomic_replace_cleans_up_the_sibling_temp_file()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let target = root.path().join("target");
        fs::create_dir(&target)?;
        assert!(atomic_write_file(&target, "content").is_err());
        let siblings = fs::read_dir(root.path())?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(siblings, vec!["target"]);
        Ok(())
    }
}
