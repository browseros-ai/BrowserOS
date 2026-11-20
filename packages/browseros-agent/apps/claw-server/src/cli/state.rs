//! The scope file: what makes `browseros click @e3` carry no arguments.
//!
//! Two things live here because a CLI process cannot hold them between
//! invocations: the current page, and the tab group the task works in. The
//! first is the ergonomic delta. The second is the task itself: the server
//! resolves the task from it, so it is the only identity a command carries.
//! There is no session. A scope file written before that change still loads;
//! its old session id is simply dropped on the next save.

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
    /// A cache, re-resolvable. A change discards `page` and the tool cache,
    /// which mean nothing on another server.
    pub endpoint: Option<String>,
    /// The task. Presented on every call, named back by the server in every
    /// result, and cleared when the server says the task has no group open.
    pub group_id: Option<String>,
    /// The task's name, once the server has it.
    pub task: Option<String>,
    /// A name given before the task's first page. Applied once its group exists,
    /// which is when the server has a task to name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_task: Option<PendingTask>,
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
    /// The server's run wall-clock budget in ms, from the run tool's schema.
    /// Configurable per deployment, so it is discovered, never assumed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_budget_ms: Option<u64>,
    /// The scope's human-readable path, for `task list`.
    pub scope_path: Option<String>,
    pub started_at: Option<String>,
    /// The kinds of tip this task has been shown, so each prints once per task. An agent
    /// reads stderr along with stdout, so a tip on every call is noise however it is routed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tips_shown: Vec<String>,
    /// The task `tips_shown` belongs to. A new task is shown its tips again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tips_task: Option<String>,
}

/// A task named with `task start` before it has a page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PendingTask {
    pub name: String,
    pub category: Option<String>,
    pub summary: Option<String>,
}

impl State {
    /// Drops everything that is meaningless on a different server.
    pub fn rebind(&mut self, endpoint: &str) {
        if self.endpoint.as_deref() != Some(endpoint) {
            self.page = None;
            self.page_url = None;
            self.tools.clear();
            self.run_budget_ms = None;
            self.server_version = None;
            self.endpoint = Some(endpoint.to_string());
        }
    }

    /// The tips this task has not been shown yet, now recorded as shown.
    pub fn first_showing(&mut self, tips: &[String]) -> Vec<String> {
        if self.tips_task != self.group_id {
            self.tips_shown.clear();
            self.tips_task.clone_from(&self.group_id);
        }
        let mut fresh = Vec::new();
        for tip in tips {
            let kind = tip_kind(tip);
            if !self.tips_shown.contains(&kind) {
                self.tips_shown.push(kind);
                fresh.push(tip.clone());
            }
        }
        fresh
    }

    /// Whether this server is known to offer a tool. An empty cache means
    /// "not yet asked", which must read as offered rather than as absent.
    #[must_use]
    pub fn offers(&self, tool: &str) -> bool {
        self.tools.is_empty() || self.tools.iter().any(|name| name == tool)
    }
}

/// What makes two tips the same tip: their text without what they quote, since the rename tip
/// names the task it is about.
#[must_use]
pub fn tip_kind(tip: &str) -> String {
    tip.split('"').step_by(2).collect::<Vec<_>>().join("\"\"")
}

/// Whether `BROWSEROS_QUIET` asks for what `-q` does. Any value but empty, `0` or `false`.
#[must_use]
pub fn quiet_from_env(value: Option<&str>) -> bool {
    value.map(str::trim).is_some_and(|value| {
        !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("false")
    })
}

/// Where the state for one working directory lives.
pub struct Scope {
    pub path: PathBuf,
    pub label: String,
}

/// Resolves the scope: the git worktree root, else the working directory.
///
/// Worktree root rather than cwd is deliberate. An agent that cds into a
/// subdirectory mid-task must keep the same task; the alternative silently
/// starts a second one in a second tab group.
///
/// `BROWSEROS_TASK=<name>` pins an explicit scope and overrides both, so two
/// agents in one checkout can each work on a task of their own.
#[must_use]
pub fn scope(dir: &Path, worktree_root: Option<&Path>) -> Scope {
    if let Some(name) = std::env::var("BROWSEROS_TASK")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return Scope {
            path: PathBuf::from(format!("name:{name}")),
            label: format!("task {name}"),
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
/// so a dev server and a release server never share a task.
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
            crate::cli::exit::Failure::usage("cannot locate a home directory for the scope file")
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
                    "[scope file was unreadable and has been reset: {error}]"
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
                    "[another browseros command is holding the scope file; continuing without the lock]",
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

/// Every scope file on this machine, for `task list`.
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
    fn a_changed_endpoint_discards_the_page_and_keeps_the_task() {
        let mut state = State {
            endpoint: Some("http://127.0.0.1:9200".into()),
            page: Some(3),
            group_id: Some("G".into()),
            task: Some("price check".into()),
            ..State::default()
        };
        state.rebind("http://127.0.0.1:9739");
        assert_eq!(state.page, None);
        // The group is the task, and the browser holds it whatever server
        // answers, so it survives and the next command resolves the task.
        assert_eq!(state.group_id.as_deref(), Some("G"));
        assert_eq!(state.task.as_deref(), Some("price check"));
    }

    #[test]
    fn the_same_endpoint_keeps_everything() {
        let mut state = State {
            endpoint: Some("http://127.0.0.1:9200".into()),
            page: Some(3),
            group_id: Some("G".into()),
            ..State::default()
        };
        state.rebind("http://127.0.0.1:9200");
        assert_eq!(state.page, Some(3));
        assert_eq!(state.group_id.as_deref(), Some("G"));
    }

    #[test]
    fn a_scope_file_from_the_session_model_loads_with_its_task_intact()
    -> Result<(), serde_json::Error> {
        // Written by a CLI that still paired sessions. The session id means
        // nothing now and is dropped; the group, which is the task, is kept.
        let old =
            r#"{"endpoint":"http://127.0.0.1:9200","mcpSessionId":"abc","groupId":"G","page":3}"#;
        let state: State = serde_json::from_str(old)?;
        assert_eq!(state.group_id.as_deref(), Some("G"));
        assert_eq!(state.page, Some(3));
        assert!(!serde_json::to_string(&state)?.contains("mcpSessionId"));
        Ok(())
    }

    #[test]
    fn a_tip_prints_once_per_task_even_when_it_names_the_task() {
        let mut state = State {
            group_id: Some("G1".into()),
            ..State::default()
        };
        let rename = |name: &str| {
            format!("Tip: this task is \"{name}\". Name it with: browseros task name \"<label>\"")
        };
        let done =
            "Tip: when the user says this task is done, run: browseros task done".to_string();
        assert_eq!(
            state.first_showing(&[rename("a/breezy"), done.clone()]),
            vec![rename("a/breezy"), done.clone()]
        );
        assert!(
            state
                .first_showing(&[rename("a/breezy"), done.clone()])
                .is_empty()
        );
        assert!(state.first_showing(&[rename("a/other")]).is_empty());

        state.group_id = Some("G2".into());
        assert_eq!(state.first_showing(std::slice::from_ref(&done)), vec![done]);
    }

    #[test]
    fn browseros_quiet_is_on_for_any_value_but_off_ones() {
        for on in ["1", "true", "yes"] {
            assert!(quiet_from_env(Some(on)), "{on}");
        }
        for off in [None, Some(""), Some("0"), Some("false"), Some("FALSE")] {
            assert!(!quiet_from_env(off), "{off:?}");
        }
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
