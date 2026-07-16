use std::{
    collections::BTreeMap,
    env, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use crate::{AgentId, AgentScope, Error, PerOsPaths, catalog::resolve_agent_mcp_config_path};

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

pub(crate) fn any_exists(paths: &[&str]) -> Result<bool, Error> {
    for path in expand_paths(paths) {
        if path_exists(&path)? {
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

fn is_config_path_installed(config_path: &Path) -> Result<bool, Error> {
    if path_exists(config_path)? {
        return Ok(true);
    }
    match config_path.parent() {
        Some(parent) => path_exists(parent),
        None => Ok(false),
    }
}

/// Checks whether each agent's config file or its parent directory exists.
pub fn is_installed(agents: &[AgentId]) -> Result<BTreeMap<AgentId, bool>, Error> {
    let mut result = BTreeMap::new();
    for agent in agents {
        if result.contains_key(agent) {
            continue;
        }
        let installed = match resolve_agent_mcp_config_path(*agent, AgentScope::System) {
            Ok(config_path) => is_config_path_installed(&config_path)?,
            Err(Error::UnresolvedConfigPath { .. }) => false,
            Err(error) => return Err(error),
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
