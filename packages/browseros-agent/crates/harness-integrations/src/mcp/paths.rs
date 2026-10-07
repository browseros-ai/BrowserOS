use std::{
    collections::BTreeMap,
    env, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use crate::{
    catalog::{AgentId, PerOsPaths, get_catalog_entry, list_supported_agents},
    error::Error,
    skills::SkillEnvironment,
};

use super::types::{AgentInfo, AgentScope, AgentSurface};

/// Resolves the active configuration shape and transport set for an agent.
pub fn resolve_agent_surface(agent: AgentId, scope: AgentScope) -> Result<AgentSurface, Error> {
    ensure_system_scope(agent, scope)?;
    let harness = get_catalog_entry(agent);
    Ok(AgentSurface {
        harness,
        mcp: &harness.mcp,
        supported_transports: harness.mcp.system_transports,
        stdio: harness.mcp.stdio,
        http: harness.mcp.http,
    })
}

/// Resolves the first existing system config candidate, or the first resolvable candidate.
pub fn resolve_agent_mcp_config_path(agent: AgentId, scope: AgentScope) -> Result<PathBuf, Error> {
    ensure_system_scope(agent, scope)?;
    let candidates = selected_os_paths(&get_catalog_entry(agent).mcp.system_paths);
    if candidates.is_empty() {
        return Err(Error::UnresolvedConfigPath {
            agent,
            reason: format!(
                "no system config path configured for OS {}",
                std::env::consts::OS
            ),
        });
    }
    pick_config_path(candidates)?.ok_or_else(|| Error::UnresolvedConfigPath {
        agent,
        reason: "no system config path resolves (env vars unset?)".to_string(),
    })
}

pub(crate) fn has_install_fingerprint(agent: AgentId) -> Result<bool, Error> {
    let checks = selected_os_paths(&get_catalog_entry(agent).install_check_paths);
    any_installation_evidence(checks)
}

/// Reports catalog install checks separately from config-path writability.
pub fn detect_installed_agents() -> Result<Vec<AgentInfo>, Error> {
    list_supported_agents()
        .into_iter()
        .map(|agent| {
            let harness = get_catalog_entry(agent);
            let installed = has_install_fingerprint(agent)?;
            let config_path = resolve_agent_mcp_config_path(agent, AgentScope::System).ok();
            Ok(AgentInfo {
                id: agent,
                display_name: harness.display_name.to_string(),
                config_path,
                installed,
            })
        })
        .collect()
}

pub(crate) fn ensure_system_scope(agent: AgentId, scope: AgentScope) -> Result<(), Error> {
    if scope == AgentScope::System {
        return Ok(());
    }
    Err(Error::UnresolvedConfigPath {
        agent,
        reason: "project scope is not supported; only system scope is implemented".to_string(),
    })
}

pub(crate) fn selected_os_paths(paths: &PerOsPaths) -> &'static [&'static str] {
    match env::consts::OS {
        "macos" => paths.darwin,
        "windows" => paths.windows,
        _ => paths.linux,
    }
}

fn expand_path_with(raw: &str, mut lookup: impl FnMut(&str) -> Option<String>) -> Option<PathBuf> {
    let bytes = raw.as_bytes();
    let mut result = String::with_capacity(raw.len());
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] != b'$' {
            let character = raw[cursor..].chars().next()?;
            result.push(character);
            cursor += character.len_utf8();
            continue;
        }
        let name_start = cursor + 1;
        if name_start >= bytes.len()
            || !(bytes[name_start].is_ascii_alphabetic() || bytes[name_start] == b'_')
        {
            result.push('$');
            cursor += 1;
            continue;
        }
        let mut name_end = name_start + 1;
        while name_end < bytes.len()
            && (bytes[name_end].is_ascii_alphanumeric() || bytes[name_end] == b'_')
        {
            name_end += 1;
        }
        let value = lookup(&raw[name_start..name_end]).filter(|value| !value.is_empty())?;
        result.push_str(&value);
        cursor = name_end;
    }
    Some(PathBuf::from(result))
}

pub(crate) fn expand_paths(paths: &[&str]) -> Vec<PathBuf> {
    paths
        .iter()
        .filter_map(|raw| expand_path_with(raw, |name| env::var(name).ok()))
        .collect()
}

pub(crate) fn path_exists(path: &Path) -> Result<bool, Error> {
    match fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(Error::io("inspect", path, error)),
    }
}

fn any_installation_evidence(paths: &[&str]) -> Result<bool, Error> {
    for path in expand_paths(paths) {
        if has_installation_evidence(&path)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// A skills-only directory can be planted before its application is installed.
/// Keep accepting an app's empty initialization directory, but do not let skill
/// provisioning itself opt that app into automatic MCP configuration.
fn has_installation_evidence(path: &Path) -> Result<bool, Error> {
    let roots = expand_paths(&["$HOME", "$USERPROFILE"])
        .into_iter()
        .flat_map(|home| SkillEnvironment::current(home).common_skill_roots())
        .map(|root| fs::canonicalize(&root).unwrap_or(root))
        .collect::<Vec<_>>();
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    has_non_skill_state(&path, &roots, true)
}

fn has_non_skill_state(
    path: &Path,
    skill_roots: &[PathBuf],
    accept_empty: bool,
) -> Result<bool, Error> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(Error::io("inspect", path, error)),
    };
    if !metadata.is_dir() {
        return Ok(true);
    }
    let mut entries = fs::read_dir(path)
        .map_err(|error| Error::io("inspect install directory", path, error))?
        .peekable();
    if entries.peek().is_none() {
        return Ok(accept_empty);
    }
    for entry in entries {
        let entry = entry.map_err(|error| Error::io("inspect install directory", path, error))?;
        if entry.file_name() == "skills" || entry.file_name() == ".DS_Store" {
            continue;
        }
        let child = entry.path();
        if skill_roots.contains(&child) {
            continue;
        }
        // A nested override can create ~/.claude/profiles/work/skills. Inspect
        // only ancestors of known skill roots so those scaffolding directories
        // do not impersonate an installed app. Other app state qualifies at once;
        // this never recursively scans arbitrary application directories.
        if skill_roots.iter().any(|root| root.starts_with(&child)) {
            if has_non_skill_state(&child, skill_roots, false)? {
                return Ok(true);
            }
        } else {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn pick_config_path(candidates: &[&str]) -> Result<Option<PathBuf>, Error> {
    let expanded = expand_paths(candidates);
    pick_expanded_config_path(expanded)
}

fn pick_expanded_config_path(expanded: Vec<PathBuf>) -> Result<Option<PathBuf>, Error> {
    for path in &expanded {
        if path_exists(path)? {
            return Ok(Some(path.clone()));
        }
    }
    Ok(expanded.into_iter().next())
}

pub(crate) fn is_config_path_installed(config_path: &Path) -> Result<bool, Error> {
    if path_exists(config_path)? {
        return Ok(true);
    }
    match config_path.parent() {
        Some(parent) => {
            // A home-level config (notably ~/.claude.json) does not make every
            // user with a home directory an installed agent. Actual config files
            // and the agent's own install fingerprints still qualify above.
            if expand_paths(&["$HOME", "$USERPROFILE"])
                .iter()
                .any(|home| home == parent)
            {
                return Ok(false);
            }
            has_installation_evidence(parent)
        }
        None => Ok(false),
    }
}

/// Checks app/config evidence, excluding home and skills-only directories.
pub fn is_installed(agents: &[AgentId]) -> Result<BTreeMap<AgentId, bool>, Error> {
    let mut result = BTreeMap::new();
    for agent in agents {
        if result.contains_key(agent) {
            continue;
        }
        let installed = if has_install_fingerprint(*agent)? {
            true
        } else {
            match resolve_agent_mcp_config_path(*agent, AgentScope::System) {
                Ok(config_path) => is_config_path_installed(&config_path)?,
                Err(Error::UnresolvedConfigPath { .. }) => false,
                Err(error) => return Err(error),
            }
        };
        result.insert(*agent, installed);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs};

    use tempfile::tempdir;

    use super::{expand_path_with, is_config_path_installed, pick_expanded_config_path};

    #[test]
    fn expansion_discards_a_candidate_when_any_variable_is_missing_or_empty() {
        let vars = BTreeMap::from([("HOME", "/tmp/home"), ("EMPTY", "")]);
        let lookup = |name: &str| vars.get(name).map(ToString::to_string);
        assert_eq!(
            expand_path_with("$HOME/.cursor/mcp.json", lookup),
            Some("/tmp/home/.cursor/mcp.json".into())
        );
        assert_eq!(expand_path_with("$HOME/$MISSING/x", lookup), None);
        assert_eq!(expand_path_with("$EMPTY/x", lookup), None);
    }

    #[test]
    fn expansion_only_recognizes_dollar_variable_syntax() {
        assert_eq!(
            expand_path_with("~/x/${HOME}/$9", |_| None),
            Some("~/x/${HOME}/$9".into())
        );
    }

    #[test]
    fn path_selection_prefers_first_existing_then_first_resolvable()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let first = root.path().join("first");
        let second = root.path().join("second");
        fs::write(&second, "")?;
        assert_eq!(
            pick_expanded_config_path(vec![first.clone(), second.clone()])?,
            Some(second)
        );
        fs::remove_file(root.path().join("second"))?;
        assert_eq!(
            pick_expanded_config_path(vec![first.clone(), root.path().join("second")])?,
            Some(first)
        );
        Ok(())
    }

    #[test]
    fn install_signal_accepts_an_existing_file_or_parent() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = tempdir()?;
        let config = root.path().join("agent/config.json");
        assert!(!is_config_path_installed(&config)?);
        fs::create_dir_all(config.parent().ok_or("missing test parent")?)?;
        assert!(is_config_path_installed(&config)?);
        fs::write(&config, "")?;
        assert!(is_config_path_installed(&config)?);
        Ok(())
    }
}
