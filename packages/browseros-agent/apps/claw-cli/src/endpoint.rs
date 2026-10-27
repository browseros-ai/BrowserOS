//! Finding the server, and never trusting the file that claims to know.
//!
//! `services/runtime_file.rs` writes `{"url": "..."}` to
//! `<browserclaw_dir>/runtime.json` at startup, best effort. It is not cleaned
//! up on exit, so it outlives the process that wrote it. Observed on a
//! developer machine on 2026-10-27: both the release and the dev file pointed
//! at dead ports at the same moment. Every candidate is therefore health
//! checked before it is used.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::client::transport::Transport;

/// `config.rs: DEFAULT_SERVER_PORT`. Not 9100, which is a sidecar launch
/// config value rather than the server's own default.
pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:9200";
const HEALTH_TIMEOUT: Duration = Duration::from_millis(400);

/// Where a candidate came from, so an ambiguous machine can be explained once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Flag,
    Env,
    DirOverride,
    Release,
    Dev,
    Default,
    Cached,
}

impl Origin {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Flag => "--endpoint",
            Self::Env => "BROWSEROS_URL",
            Self::DirOverride => "BROWSERCLAW_DIR/runtime.json",
            Self::Release => "~/.browserclaw/runtime.json",
            Self::Dev => "~/.browserclaw-dev/runtime.json",
            Self::Default => "the compiled default",
            Self::Cached => "the scope file",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub url: String,
    pub origin: Origin,
}

/// The release state directory, matching the server's own `resolve_browserclaw_dir`.
#[must_use]
pub fn release_dir() -> Option<PathBuf> {
    if let Some(raw) = std::env::var_os("BROWSERCLAW_DIR").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(raw));
    }
    home().map(|home| home.join(".browserclaw"))
}

#[must_use]
pub fn dev_dir() -> Option<PathBuf> {
    home().map(|home| home.join(".browserclaw-dev"))
}

#[must_use]
fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Every place a server URL could come from, best first.
///
/// Pure so the order is testable without touching the network. Nothing here
/// decides whether a URL works; [`resolve`] does that.
#[must_use]
pub fn candidates(flag: Option<&str>, cached: Option<&str>) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    let mut push = |url: Option<String>, origin: Origin| {
        if let Some(url) = url.map(|u| u.trim().trim_end_matches('/').to_string())
            && !url.is_empty()
            && !out.iter().any(|existing| existing.url == url)
        {
            out.push(Candidate { url, origin });
        }
    };

    push(flag.map(ToString::to_string), Origin::Flag);
    push(std::env::var("BROWSEROS_URL").ok(), Origin::Env);
    push(cached.map(ToString::to_string), Origin::Cached);
    if std::env::var_os("BROWSERCLAW_DIR").is_some_and(|v| !v.is_empty()) {
        push(
            release_dir().as_deref().and_then(read_runtime_url),
            Origin::DirOverride,
        );
    } else {
        push(
            release_dir().as_deref().and_then(read_runtime_url),
            Origin::Release,
        );
        push(dev_dir().as_deref().and_then(read_runtime_url), Origin::Dev);
    }
    push(Some(DEFAULT_ENDPOINT.to_string()), Origin::Default);
    out
}

/// Reads the `url` field out of a `runtime.json`, if the file is there and sane.
#[must_use]
pub fn read_runtime_url(dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(dir.join("runtime.json")).ok()?;
    let parsed: Value = serde_json::from_str(&raw).ok()?;
    parsed
        .get("url")
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

/// What resolution settled on, and whether it had to choose.
pub struct Resolved {
    pub url: String,
    pub origin: Origin,
    /// Set when both a release and a dev server answered. A dogfooding machine
    /// running both is the one case where silence costs an hour.
    pub ambiguous_with: Option<String>,
}

/// Probes each candidate in order and returns the first that answers.
///
/// # Errors
/// Returns [`crate::exit::Code::NoBrowser`] listing everything that was tried,
/// because "connection refused" with no list is the least useful thing a CLI
/// can say here.
pub async fn resolve(flag: Option<&str>, cached: Option<&str>) -> anyhow::Result<Resolved> {
    let candidates = candidates(flag, cached);
    let mut live: Option<Candidate> = None;
    let mut also_live: Option<String> = None;

    for candidate in &candidates {
        let Ok(transport) = Transport::new(&candidate.url, HEALTH_TIMEOUT) else {
            continue;
        };
        if transport.health(HEALTH_TIMEOUT).await.is_none() {
            continue;
        }
        match (&live, candidate.origin) {
            (None, _) => live = Some(candidate.clone()),
            // Only the release-versus-dev pair is worth reporting: every other
            // ordering is an explicit instruction the caller already gave.
            (Some(first), Origin::Dev) if first.origin == Origin::Release => {
                also_live = Some(candidate.url.clone());
                break;
            }
            _ => break,
        }
    }

    let Some(found) = live else {
        let tried = candidates
            .iter()
            .map(|candidate| format!("  {:<28} {}", candidate.origin.label(), candidate.url))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(crate::exit::Failure::no_browser(format!(
            "BrowserOS neo is not answering. Tried:\n{tried}\n\n  Start it, or pass --endpoint <url>.\n  A runtime.json is written at startup and is not cleaned up, so a stale one is normal."
        ))
        .into());
    };

    Ok(Resolved {
        url: found.url,
        origin: found.origin,
        ambiguous_with: also_live,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flag_outranks_everything_and_the_default_is_last() {
        let list = candidates(Some("http://127.0.0.1:1234"), Some("http://127.0.0.1:5678"));
        assert_eq!(list[0].url, "http://127.0.0.1:1234");
        assert_eq!(list[0].origin, Origin::Flag);
        let last = list.last().map(|c| c.url.clone()).unwrap_or_default();
        assert_eq!(last, DEFAULT_ENDPOINT);
    }

    #[test]
    fn a_trailing_slash_never_produces_a_duplicate_candidate() {
        let list = candidates(
            Some("http://127.0.0.1:9200/"),
            Some("http://127.0.0.1:9200"),
        );
        let count = list
            .iter()
            .filter(|c| c.url == "http://127.0.0.1:9200")
            .count();
        assert_eq!(count, 1, "{list:?}");
    }

    #[test]
    fn the_cached_endpoint_is_tried_before_any_runtime_file() {
        let list = candidates(None, Some("http://127.0.0.1:4321"));
        let cached = list
            .iter()
            .position(|c| c.origin == Origin::Cached)
            .unwrap_or(usize::MAX);
        let default = list
            .iter()
            .position(|c| c.origin == Origin::Default)
            .unwrap_or(0);
        assert!(cached < default, "{list:?}");
    }

    #[test]
    fn a_runtime_file_with_no_url_field_yields_nothing() {
        let Ok(dir) = tempfile::tempdir() else { return };
        let _ = std::fs::write(dir.path().join("runtime.json"), "{\"port\":9200}\n");
        assert_eq!(read_runtime_url(dir.path()), None);
    }

    #[test]
    fn a_runtime_file_is_read_byte_for_byte_as_the_server_writes_it() {
        let Ok(dir) = tempfile::tempdir() else { return };
        let _ = std::fs::write(
            dir.path().join("runtime.json"),
            "{\n  \"url\": \"http://127.0.0.1:9201\"\n}\n",
        );
        assert_eq!(
            read_runtime_url(dir.path()),
            Some("http://127.0.0.1:9201".to_string())
        );
    }
}
