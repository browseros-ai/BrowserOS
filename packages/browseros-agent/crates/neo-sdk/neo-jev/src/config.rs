//! Where Jev lives and the key that opens it, read from the environment at start.
//!
//! The key file is the seam to whoever issues the bearer: today a dev key, later a mounted licence
//! that the host refreshes in place. The client reads the file again after a 401 (`client.rs`), so
//! a refreshed bearer is picked up without a restart.

use std::fmt;
use std::path::{Path, PathBuf};

/// Names the key file. Unset, empty or unreadable: the Jev verbs are unlisted.
pub const KEY_FILE_ENV: &str = "NEO_JEV_KEY_FILE";
/// Overrides the endpoint (tests point it at a local stub; later, the licensed worker).
pub const URL_ENV: &str = "NEO_JEV_URL";
/// Overrides the model alias.
pub const MODEL_ENV: &str = "NEO_JEV_MODEL";

const DEFAULT_URL: &str = "https://api.typesafe.ai/v1/systemone";
const DEFAULT_MODEL: &str = "jev-latest";

/// A bearer for Jev. `Debug` never prints it, so no derived `Debug` up the chain can leak it.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Key(String);

impl Key {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// The endpoint, model and key one `Jev` client uses. `Debug` never prints the key.
#[derive(Debug, Clone)]
pub struct JevConfig {
    pub(crate) url: String,
    pub(crate) model: String,
    /// Where the key came from, read again after a 401. `None` for an explicit key (`new`), which
    /// has nowhere to refresh from.
    pub(crate) key_file: Option<PathBuf>,
    /// The key as read at start; the client keeps the current one after that.
    pub(crate) key: Key,
}

impl JevConfig {
    /// Reads `NEO_JEV_KEY_FILE` (and the optional URL and model). No usable key file -> `None`.
    #[must_use]
    pub fn from_env() -> Option<JevConfig> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// `from_env` over any lookup, so tests need not touch the process environment.
    #[must_use]
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Option<JevConfig> {
        let path = lookup(KEY_FILE_ENV)?;
        let path = path.trim();
        if path.is_empty() {
            return None;
        }
        let path = PathBuf::from(path);
        let Ok(contents) = std::fs::read_to_string(&path) else {
            tracing::warn!(target: "jev", variable = KEY_FILE_ENV, "Jev key file could not be read; Jev verbs disabled");
            return None;
        };
        let key = key_in(&contents)?;
        let nonempty = |name: &str| {
            lookup(name)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        Some(JevConfig {
            url: nonempty(URL_ENV).unwrap_or_else(|| DEFAULT_URL.to_owned()),
            model: nonempty(MODEL_ENV).unwrap_or_else(|| DEFAULT_MODEL.to_owned()),
            key_file: Some(path),
            key,
        })
    }

    /// A config with an explicit endpoint and key, for tests against a local stub. With no key
    /// file, a 401 is final.
    #[must_use]
    pub fn new(url: String, model: String, key: String) -> JevConfig {
        JevConfig {
            url,
            model,
            key_file: None,
            key: Key(key),
        }
    }
}

/// The key a key file holds: its trimmed text, `None` when blank.
fn key_in(contents: &str) -> Option<Key> {
    let key = contents.trim();
    (!key.is_empty()).then(|| Key(key.to_owned()))
}

/// Reads the key file again, off the async workers. `None` when it is gone, unreadable or blank;
/// the caller then keeps the 401 it has.
pub(crate) async fn reread_key(path: &Path) -> Option<Key> {
    match tokio::fs::read_to_string(path).await {
        Ok(contents) => key_in(&contents),
        Err(_) => {
            tracing::warn!(target: "jev", variable = KEY_FILE_ENV, "Jev key file could not be read again after a 401");
            None
        }
    }
}
