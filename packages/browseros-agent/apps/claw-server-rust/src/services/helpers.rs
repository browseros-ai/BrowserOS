//! Persistence seam for code-mode helpers: reusable `.js` a script saves for a
//! host and a later script loads by name. Layout is
//! `<browserclaw_dir>/helpers/<host>/<name>.js`. This module owns only the
//! traversal-safe storage; the saveHelper primitive and the hot-load into the
//! script runtime that build on it land with the self-healing work.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

const HELPERS_DIR: &str = "helpers";
const HELPER_EXTENSION: &str = "js";

/// A single safe path segment: non-empty, not a traversal token, limited to an
/// unsurprising character set so a host or helper name cannot escape the
/// helpers root.
fn is_safe_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Resolves `<browserclaw_dir>/helpers/<host>/`, or `None` for an unsafe host.
#[must_use]
pub fn helpers_dir(browserclaw_dir: &Path, host: &str) -> Option<PathBuf> {
    is_safe_segment(host).then(|| browserclaw_dir.join(HELPERS_DIR).join(host))
}

fn helper_path(browserclaw_dir: &Path, host: &str, name: &str) -> Option<PathBuf> {
    if !is_safe_segment(name) {
        return None;
    }
    helpers_dir(browserclaw_dir, host).map(|dir| dir.join(format!("{name}.{HELPER_EXTENSION}")))
}

/// Lists helper base names (without the `.js` extension) available for a host,
/// sorted. Missing directory or unsafe host yields an empty list.
#[must_use]
pub fn list_helpers(browserclaw_dir: &Path, host: &str) -> Vec<String> {
    let Some(dir) = helpers_dir(browserclaw_dir, host) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some(HELPER_EXTENSION))
        .filter_map(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_owned)
        })
        .collect();
    names.sort();
    names
}

/// Reads a helper's source, or `None` for an unsafe host/name or a missing file.
#[must_use]
pub fn read_helper(browserclaw_dir: &Path, host: &str, name: &str) -> Option<String> {
    let path = helper_path(browserclaw_dir, host, name)?;
    fs::read_to_string(path).ok()
}

/// Writes a helper's source, creating the host directory. Errors on an unsafe
/// host or name.
pub fn write_helper(browserclaw_dir: &Path, host: &str, name: &str, code: &str) -> io::Result<()> {
    let path = helper_path(browserclaw_dir, host, name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe helper host or name"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn write_then_read_round_trips_and_lists() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        write_helper(
            root,
            "linkedin.com",
            "accept-invites",
            "export const x = 1;",
        )?;
        write_helper(root, "linkedin.com", "messages", "export const y = 2;")?;

        assert_eq!(
            read_helper(root, "linkedin.com", "accept-invites").as_deref(),
            Some("export const x = 1;")
        );
        assert_eq!(
            list_helpers(root, "linkedin.com"),
            vec!["accept-invites".to_string(), "messages".to_string()]
        );
        // Distinct hosts do not collide.
        assert!(list_helpers(root, "docs.google.com").is_empty());
        Ok(())
    }

    #[test]
    fn unsafe_host_or_name_is_rejected() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        assert!(helpers_dir(root, "..").is_none());
        assert!(helpers_dir(root, "a/b").is_none());
        assert!(read_helper(root, "linkedin.com", "../escape").is_none());
        assert!(write_helper(root, "..", "x", "code").is_err());
        assert!(write_helper(root, "linkedin.com", "a/b", "code").is_err());
        Ok(())
    }

    #[test]
    fn missing_host_reads_and_lists_empty() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        assert!(read_helper(root, "linkedin.com", "nope").is_none());
        assert!(list_helpers(root, "linkedin.com").is_empty());
        Ok(())
    }
}
