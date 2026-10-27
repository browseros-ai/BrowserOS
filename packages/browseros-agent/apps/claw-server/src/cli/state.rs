//! The scope file: what makes `browseros click @e3` carry no arguments.
//!
//! Three things live here because a CLI process cannot hold them between
//! invocations: the current page, the MCP transport session id, and the tab
//! group id. The first is the ergonomic delta; the second and third are what
//! keep tab ownership intact across separate processes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cli::io;

/// How long to wait for the scope lock before giving up and proceeding
/// read-only. A stuck lock must never make the browser unusable.
const LOCK_WAIT: Duration = Duration::from_secs(2);
/// A lock file older than this belonged to a process that died holding it.
const LOCK_STALE: Duration = Duration::from_secs(5);
const LOCK_POLL: Duration = Duration::from_millis(25);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct State {
    /// A cache, re-resolvable. A change discards `mcp_session_id` and `page`,
    /// which mean nothing on another server.
    pub endpoint: Option<String>,
    /// The fast path. Dies with the server; a 404 is the signal.
    pub mcp_session_id: Option<String>,
    /// The durable identity. Survives a restart and is re-claimed on `tabs new`.
    pub group_id: Option<String>,
    /// So a recovered session can re-apply its name.
    pub task: Option<String>,
    /// The implicit current page.
    pub page: Option<u32>,
    /// A page id is only good while that tab is open and still ours, so the URL
    /// is recorded alongside it and a mismatch is reported rather than fixed.
    pub page_url: Option<String>,
    /// Passed as `agentName` so tabs from different harnesses are labelled apart.
    pub agent: Option<String>,
    /// Keys the tool cache below. Read from `GET /api/v1/system`.
    pub server_version: Option<String>,
    /// Which optional tools THIS server actually offers. The surface is
    /// conditional in five independent ways, so a static table would promise
    /// commands that get refused.
    pub tools: Vec<String>,
    /// The scope's human-readable path, for `session list`.
    pub scope_path: Option<String>,
    pub started_at: Option<String>,
}

impl State {
    /// Drops everything that is meaningless on a different server.
    pub fn rebind(&mut self, endpoint: &str) {
        if self.endpoint.as_deref() != Some(endpoint) {
            self.mcp_session_id = None;
            self.page = None;
            self.page_url = None;
            self.tools.clear();
            self.server_version = None;
            self.endpoint = Some(endpoint.to_string());
        }
    }

    /// Whether this server is known to offer a tool. An empty cache means
    /// "not yet asked", which must read as offered rather than as absent.
    #[must_use]
    pub fn offers(&self, tool: &str) -> bool {
        self.tools.is_empty() || self.tools.iter().any(|name| name == tool)
    }
}

/// Where the state for one working directory lives.
pub struct Scope {
    pub path: PathBuf,
    pub label: String,
}

/// Resolves the scope: the git worktree root, else the working directory.
///
/// Worktree root rather than cwd is deliberate. An agent that cds into a
/// subdirectory mid-task must keep the same browser session; the alternative
/// silently starts a second one and orphans the first one's tabs.
///
/// `BROWSEROS_SESSION=<name>` pins an explicit scope and overrides both.
#[must_use]
pub fn scope(dir: &Path, worktree_root: Option<&Path>) -> Scope {
    if let Some(name) = std::env::var("BROWSEROS_SESSION")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return Scope {
            path: PathBuf::from(format!("name:{name}")),
            label: format!("session {name}"),
        };
    }
    let root = worktree_root.unwrap_or(dir);
    Scope {
        path: root.to_path_buf(),
        label: root.display().to_string(),
    }
}

/// `git rev-parse --show-toplevel`, or nothing when not in a repository.
#[must_use]
pub fn worktree_root(dir: &Path) -> Option<PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if path.is_empty() {
        return None;
    }
    Some(PathBuf::from(path))
}

/// The filename for a scope, stable across runs and opaque in a shared directory.
#[must_use]
pub fn scope_key(scope: &Scope) -> String {
    let mut hasher = Sha256::new();
    hasher.update(scope.path.as_os_str().as_encoded_bytes());
    let digest = hasher.finalize();
    digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The CLI's own directory inside whichever state directory the server uses,
/// so a dev server and a release server never share a session.
#[must_use]
pub fn state_dir() -> Option<PathBuf> {
    crate::cli::endpoint::release_dir().map(|dir| dir.join("cli"))
}

pub struct Store {
    file: PathBuf,
    lock: PathBuf,
    label: String,
}

impl Store {
    /// # Errors
    /// Fails only when the state directory cannot be created.
    pub fn open(scope: &Scope) -> anyhow::Result<Self> {
        let dir = state_dir().ok_or_else(|| {
            crate::cli::exit::Failure::usage("cannot locate a home directory for the session file")
        })?;
        Self::open_in(&dir, scope)
    }

    /// The same store rooted somewhere explicit.
    ///
    /// Exists so a test can point it at a temporary directory. Setting an
    /// environment variable instead would be `unsafe` under the 2024 edition,
    /// which the workspace denies, and would leak between parallel tests.
    ///
    /// # Errors
    /// Fails only when the directory cannot be created.
    pub fn open_in(dir: &Path, scope: &Scope) -> anyhow::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let key = scope_key(scope);
        Ok(Self {
            file: dir.join(format!("{key}.json")),
            lock: dir.join(format!("{key}.lock")),
            label: scope.label.clone(),
        })
    }

    #[must_use]
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Reads the state, treating a corrupt file as an empty one.
    ///
    /// A malformed scope file must never be fatal: it is a cache of three
    /// recoverable facts, and refusing to run because of it would turn a
    /// cosmetic problem into an outage.
    #[must_use]
    pub fn load(&self) -> State {
        let Ok(raw) = std::fs::read_to_string(&self.file) else {
            return State::default();
        };
        match serde_json::from_str::<State>(&raw) {
            Ok(state) => state,
            Err(error) => {
                io::note(&format!(
                    "[session file was unreadable and has been reset: {error}]"
                ));
                State::default()
            }
        }
    }

    /// Read, mutate, write, under an advisory lock held for microseconds.
    ///
    /// The lock is an `O_EXCL` file rather than `flock`, because the workspace
    /// denies `unsafe_code` and this avoids a new dependency for a critical
    /// section this short. The cost is that a crashed process leaves the lock
    /// behind, which is why a lock older than [`LOCK_STALE`] is stolen.
    ///
    /// # Errors
    /// Propagates write failures; failing to take the lock is a warning, not an
    /// error, and the write proceeds.
    pub fn update<F>(&self, mutate: F) -> anyhow::Result<State>
    where
        F: FnOnce(&mut State),
    {
        let held = self.acquire();
        let mut state = self.load();
        if state.scope_path.is_none() {
            state.scope_path = Some(self.label.clone());
        }
        mutate(&mut state);
        let result = self.write(&state);
        if held {
            let _ = std::fs::remove_file(&self.lock);
        }
        result?;
        Ok(state)
    }

    fn write(&self, state: &State) -> anyhow::Result<()> {
        let mut body = serde_json::to_string_pretty(state)?;
        body.push('\n');
        let temp = self.file.with_extension("json.tmp");
        std::fs::write(&temp, body)?;
        // Safe here in a way it is not for the server's runtime.json: that file
        // is polled by other processes on Windows, where the destination can be
        // momentarily locked. A scope file is only ever touched by CLI
        // processes holding this lock.
        std::fs::rename(&temp, &self.file)?;
        Ok(())
    }

    fn acquire(&self) -> bool {
        let deadline = SystemTime::now() + LOCK_WAIT;
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&self.lock)
            {
                Ok(_) => return true,
                Err(_) if self.lock_is_stale() => {
                    let _ = std::fs::remove_file(&self.lock);
                }
                Err(_) => {}
            }
            if SystemTime::now() >= deadline {
                io::note(
                    "[another browseros command is holding the session file; continuing without the lock]",
                );
                return false;
            }
            std::thread::sleep(LOCK_POLL);
        }
    }

    fn lock_is_stale(&self) -> bool {
        std::fs::metadata(&self.lock)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age > LOCK_STALE)
    }

    /// Forgets this scope entirely.
    ///
    /// # Errors
    /// Propagates anything other than the file already being gone.
    pub fn clear(&self) -> anyhow::Result<()> {
        match std::fs::remove_file(&self.file) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

/// Every scope file on this machine, for `session list`.
#[must_use]
pub fn all_scopes() -> BTreeMap<String, State> {
    state_dir().map(|dir| scopes_in(&dir)).unwrap_or_default()
}

/// Every scope file under one directory.
#[must_use]
pub fn scopes_in(dir: &Path) -> BTreeMap<String, State> {
    let mut found = BTreeMap::new();
    let dir = dir.to_path_buf();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Ok(state) = serde_json::from_str::<State>(&raw) {
            let key = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("?")
                .to_string();
            found.insert(key, state);
        }
    }
    found
}

/// An RFC3339-ish stamp without pulling in a date library for one field.
#[must_use]
pub fn now_stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("epoch:{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_worktree_root_wins_over_a_subdirectory() {
        let root = PathBuf::from("/repo");
        let deep = PathBuf::from("/repo/apps/web");
        let a = scope(&deep, Some(&root));
        let b = scope(&root, Some(&root));
        assert_eq!(scope_key(&a), scope_key(&b));
    }

    #[test]
    fn two_different_roots_do_not_share_a_file() {
        let a = scope(Path::new("/one"), None);
        let b = scope(Path::new("/two"), None);
        assert_ne!(scope_key(&a), scope_key(&b));
    }

    #[test]
    fn a_changed_endpoint_discards_the_session_and_the_page() {
        let mut state = State {
            endpoint: Some("http://127.0.0.1:9200".into()),
            mcp_session_id: Some("abc".into()),
            page: Some(3),
            group_id: Some("G".into()),
            task: Some("price check".into()),
            ..State::default()
        };
        state.rebind("http://127.0.0.1:9739");
        assert_eq!(state.mcp_session_id, None);
        assert_eq!(state.page, None);
        // The group and the task are the server-independent half of the
        // identity and survive, so a reconnect can re-claim them.
        assert_eq!(state.group_id.as_deref(), Some("G"));
        assert_eq!(state.task.as_deref(), Some("price check"));
    }

    #[test]
    fn the_same_endpoint_keeps_everything() {
        let mut state = State {
            endpoint: Some("http://127.0.0.1:9200".into()),
            mcp_session_id: Some("abc".into()),
            page: Some(3),
            ..State::default()
        };
        state.rebind("http://127.0.0.1:9200");
        assert_eq!(state.mcp_session_id.as_deref(), Some("abc"));
        assert_eq!(state.page, Some(3));
    }

    #[test]
    fn an_unasked_tool_cache_reads_as_offered() {
        let mut state = State::default();
        assert!(state.offers("pursue"));
        state.tools = vec!["tabs".into()];
        assert!(!state.offers("pursue"));
        assert!(state.offers("tabs"));
    }
}
