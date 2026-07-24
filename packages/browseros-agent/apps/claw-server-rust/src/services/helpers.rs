//! Persistence seam for code-mode helpers: reusable `.js` a script saves for a
//! host and a later script loads by name. Layout is
//! `<browserclaw_dir>/helpers/<host>/<name>.js`. This module owns only the
//! traversal-safe storage; the saveHelper primitive and the hot-load into the
//! script runtime that build on it land with the self-healing work.

use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

const HELPERS_DIR: &str = "helpers";
const HELPER_EXTENSION: &str = "js";
const HEADER_PREFIX: &str = "// browserclaw-helper: ";

/// Upper bound on a helper's source so a script cannot fill the disk.
pub const MAX_HELPER_BYTES: usize = 64 * 1024;

/// Provenance and freshness a helper file carries in a leading comment line, so
/// a later reader can judge staleness and origin. A `//` line is inert to
/// `eval`, so the file still evals as a bare function expression.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelperMeta {
    pub name: String,
    pub host: String,
    #[serde(rename = "lastVerified")]
    pub last_verified: i64,
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub candidate: bool,
    #[serde(default)]
    pub deps: String,
}

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

/// Derives the helper host bucket from a page URL: the hostname minus a leading
/// `www.`, or `None` when there is no usable host. Subdomains stay distinct so
/// subdomain-scoped apps do not collide. No URL crate: bounded to http(s) shapes.
#[must_use]
pub fn host_bucket(url: &str) -> Option<String> {
    let (scheme, after_scheme) = url.split_once("://")?;
    if !matches!(scheme, "http" | "https") {
        return None;
    }
    let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = host_port.split(':').next().unwrap_or("");
    let bucket = host.strip_prefix("www.").unwrap_or(host);
    is_safe_segment(bucket).then(|| bucket.to_string())
}

/// Renders a helper file: the provenance header line, then the source.
#[must_use]
pub fn format_helper(meta: &HelperMeta, source: &str) -> String {
    let header = serde_json::to_string(meta).unwrap_or_else(|_| "{}".to_string());
    format!("{HEADER_PREFIX}{header}\n{source}")
}

/// Splits a helper file into its parsed header (if any) and its source body,
/// with the header line removed so the body evals as a bare function expression.
#[must_use]
pub fn parse_helper(content: &str) -> (Option<HelperMeta>, String) {
    if let Some(rest) = content.strip_prefix(HEADER_PREFIX) {
        let (line, body) = rest.split_once('\n').unwrap_or((rest, ""));
        return (
            serde_json::from_str::<HelperMeta>(line.trim()).ok(),
            body.to_string(),
        );
    }
    (None, content.to_string())
}

/// Writes a helper with its provenance header. Rejects an oversized source.
pub fn save_helper(browserclaw_dir: &Path, meta: &HelperMeta, source: &str) -> io::Result<()> {
    if source.len() > MAX_HELPER_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "helper source exceeds the size limit",
        ));
    }
    write_helper(
        browserclaw_dir,
        &meta.host,
        &meta.name,
        &format_helper(meta, source),
    )
}

/// Reads a helper's source body with its provenance header stripped, ready to
/// eval. `None` for an unsafe host/name or a missing file.
#[must_use]
pub fn read_helper_source(browserclaw_dir: &Path, host: &str, name: &str) -> Option<String> {
    read_helper(browserclaw_dir, host, name).map(|content| parse_helper(&content).1)
}

/// Lists helpers for a host with their parsed provenance, sorted by name. A file
/// missing a header still lists, with default provenance.
#[must_use]
pub fn list_helper_meta(browserclaw_dir: &Path, host: &str) -> Vec<HelperMeta> {
    list_helpers(browserclaw_dir, host)
        .into_iter()
        .filter_map(|name| {
            let content = read_helper(browserclaw_dir, host, &name)?;
            let (meta, _) = parse_helper(&content);
            Some(meta.unwrap_or_else(|| HelperMeta {
                name,
                host: host.to_string(),
                last_verified: 0,
                agent: String::new(),
                candidate: false,
                deps: String::new(),
            }))
        })
        .collect()
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

    #[test]
    fn host_bucket_strips_scheme_www_path_and_port() {
        assert_eq!(
            host_bucket("https://www.linkedin.com/feed").as_deref(),
            Some("linkedin.com")
        );
        assert_eq!(
            host_bucket("https://docs.google.com/document/1").as_deref(),
            Some("docs.google.com")
        );
        assert_eq!(
            host_bucket("http://localhost:3000/app").as_deref(),
            Some("localhost")
        );
        assert_eq!(
            host_bucket("https://user@example.com:8443/x").as_deref(),
            Some("example.com")
        );
        assert_eq!(host_bucket("about:blank"), None);
        assert_eq!(host_bucket("chrome://newtab"), None);
    }

    #[test]
    fn save_read_and_list_carry_provenance_and_strip_header() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        let meta = HelperMeta {
            name: "accept-invites".to_string(),
            host: "linkedin.com".to_string(),
            last_verified: 1_784_887_201_128,
            agent: "codex".to_string(),
            candidate: true,
            deps: "linkedin.com invitation list".to_string(),
        };
        save_helper(root, &meta, "async (browser, page) => { return 1; }")?;

        // The eval body is the source with the header comment stripped.
        assert_eq!(
            read_helper_source(root, "linkedin.com", "accept-invites").as_deref(),
            Some("async (browser, page) => { return 1; }")
        );
        // Provenance round-trips through the header.
        let listed = list_helper_meta(root, "linkedin.com");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "accept-invites");
        assert_eq!(listed[0].last_verified, 1_784_887_201_128);
        assert!(listed[0].candidate);
        assert_eq!(listed[0].agent, "codex");
        Ok(())
    }

    #[test]
    fn oversized_source_is_rejected() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let meta = HelperMeta {
            name: "big".to_string(),
            host: "example.com".to_string(),
            last_verified: 0,
            agent: String::new(),
            candidate: false,
            deps: String::new(),
        };
        let huge = "a".repeat(MAX_HELPER_BYTES + 1);
        assert!(save_helper(dir.path(), &meta, &huge).is_err());
        Ok(())
    }

    #[test]
    fn a_headerless_file_still_lists_with_defaults() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        write_helper(root, "example.com", "legacy", "async () => {}")?;
        let listed = list_helper_meta(root, "example.com");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "legacy");
        assert_eq!(listed[0].last_verified, 0);
        // A headerless file reads back verbatim.
        assert_eq!(
            read_helper_source(root, "example.com", "legacy").as_deref(),
            Some("async () => {}")
        );
        Ok(())
    }
}
