//! Persistent ACP session records: which agent-side session a conversation can reattach to.
//! Replaces acpx's `acpx.session.v1` store with BrowserOS's own small record and reads TS-era acpx
//! records one way, so existing users keep their agent context across the TS → Rust swap.
//!
//! Layout under `<browserosDir>/agents/acp-sessions/`:
//! - `browseros-sessions/<encodeURIComponent(key)>.json`: ours (`browseros.acp-session.v1`).
//! - `sessions/<encodeURIComponent(key)>.json`: acpx's, never written; deleted only when the user
//!   discards the conversation's agent state, so a discarded session cannot be re-imported.

use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

use serde_json::Value;

use crate::policy::SessionKey;

pub const SESSION_RECORD_SCHEMA: &str = "browseros.acp-session.v1";
const ACPX_RECORD_SCHEMA: &str = "acpx.session.v1";
/// `launch_fingerprint` of an imported acpx record. acpx compared its own argv (with an `env K=V`
/// prefix) that we cannot reconstruct, so an import is reusable whenever the cwd matches.
pub const IMPORTED_LAUNCH_FINGERPRINT: &str = "imported:acpx.session.v1";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    /// [`SESSION_RECORD_SCHEMA`].
    pub schema: String,
    pub session_key: String,
    /// The ACP `sessionId` passed to session/resume or session/load.
    pub agent_session_id: String,
    /// Launch identity the session was created under; a different launch starts a new session.
    pub launch_fingerprint: String,
    pub cwd: PathBuf,
    /// The agent has received at least one prompt, so it holds real context and a fresh process
    /// can resume with only the latest user message (acpx: `messages` contains a `User` entry).
    pub has_user_turn: bool,
    /// Set after a failed reattach: the next attach must start a new session.
    pub reset_on_next_ensure: bool,
    pub updated_at_ms: i64,
}

impl SessionRecord {
    /// acpx `shouldReuseExistingRecord`: not reset, same cwd, same launch identity.
    pub(crate) fn reusable_for(&self, cwd: &Path, launch_fingerprint: &str) -> bool {
        !self.reset_on_next_ensure
            && self.cwd == cwd
            && (self.launch_fingerprint == launch_fingerprint
                || self.launch_fingerprint == IMPORTED_LAUNCH_FINGERPRINT)
    }
}

/// Synchronous by design: records are tiny and the runtime calls the store from blocking-safe
/// points. Implementations must make `save` atomic so a crash never leaves a torn record.
pub trait SessionStore: Send + Sync + 'static {
    /// `Ok(None)` for a missing or unreadable record (a corrupt record means a fresh session).
    fn load(&self, key: &SessionKey) -> std::io::Result<Option<SessionRecord>>;
    fn save(&self, record: &SessionRecord) -> std::io::Result<()>;
    fn delete(&self, key: &SessionKey) -> std::io::Result<()>;
}

pub struct FileSessionStore {
    dir: PathBuf,
}

impl FileSessionStore {
    /// `state_dir` is `<browserosDir>/agents/acp-sessions` (the TS runtime's `stateDir`).
    pub fn new(state_dir: PathBuf) -> Self {
        Self { dir: state_dir }
    }

    fn native_path(&self, key: &str) -> PathBuf {
        self.dir
            .join("browseros-sessions")
            .join(format!("{}.json", encode_uri_component(key)))
    }

    fn acpx_path(&self, key: &SessionKey) -> PathBuf {
        self.dir
            .join("sessions")
            .join(format!("{}.json", encode_uri_component(key.as_str())))
    }

    /// One-way read of a TS-era `acpx.session.v1` record (snake_case on disk, written by acpx
    /// 0.13.2 and 0.19.3 alike): `acp_session_id`, `cwd`, `acpx.reset_on_next_ensure`, and whether
    /// `messages` holds a `User` entry.
    pub fn import_acpx(&self, key: &SessionKey) -> std::io::Result<Option<SessionRecord>> {
        let Some(raw) = read_json(&self.acpx_path(key))? else {
            return Ok(None);
        };
        if raw.get("schema").and_then(Value::as_str) != Some(ACPX_RECORD_SCHEMA) {
            return Ok(None);
        }
        let (Some(session_id), Some(cwd)) = (
            raw.get("acp_session_id").and_then(Value::as_str),
            raw.get("cwd").and_then(Value::as_str),
        ) else {
            return Ok(None);
        };
        let has_user_turn = raw
            .get("messages")
            .and_then(Value::as_array)
            .is_some_and(|messages| messages.iter().any(|message| message.get("User").is_some()));
        let reset = raw
            .pointer("/acpx/reset_on_next_ensure")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Ok(Some(SessionRecord {
            schema: SESSION_RECORD_SCHEMA.into(),
            session_key: key.as_str().into(),
            agent_session_id: session_id.into(),
            launch_fingerprint: IMPORTED_LAUNCH_FINGERPRINT.into(),
            cwd: cwd.into(),
            has_user_turn,
            reset_on_next_ensure: reset,
            updated_at_ms: 0,
        }))
    }
}

impl SessionStore for FileSessionStore {
    /// Our record wins; otherwise fall back to importing the acpx record.
    fn load(&self, key: &SessionKey) -> std::io::Result<Option<SessionRecord>> {
        if let Some(raw) = read_json(&self.native_path(key.as_str()))? {
            match serde_json::from_value::<SessionRecord>(raw) {
                Ok(record)
                    if record.schema == SESSION_RECORD_SCHEMA
                        && record.session_key == key.as_str() =>
                {
                    return Ok(Some(record));
                }
                Ok(_) => tracing::warn!("Ignoring ACP session record for another key or schema"),
                Err(error) => tracing::warn!(%error, "Ignoring unreadable ACP session record"),
            }
            return Ok(None);
        }
        self.import_acpx(key)
    }

    fn save(&self, record: &SessionRecord) -> std::io::Result<()> {
        let path = self.native_path(&record.session_key);
        let dir = path.parent().unwrap_or(&self.dir);
        std::fs::create_dir_all(dir)?;
        let bytes = serde_json::to_vec_pretty(record).map_err(std::io::Error::other)?;
        // Temp file in the same directory, then rename: readers see the old or the new record.
        let temp = tempfile_path(&path);
        std::fs::write(&temp, bytes)?;
        std::fs::rename(&temp, &path).inspect_err(|_| {
            let _ = std::fs::remove_file(&temp);
        })
    }

    fn delete(&self, key: &SessionKey) -> std::io::Result<()> {
        for path in [self.native_path(key.as_str()), self.acpx_path(key)] {
            match std::fs::remove_file(path) {
                Err(error) if error.kind() != ErrorKind::NotFound => return Err(error),
                _ => {}
            }
        }
        Ok(())
    }
}

fn tempfile_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()))
}

fn read_json(path: &Path) -> std::io::Result<Option<Value>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    match serde_json::from_slice(&bytes) {
        Ok(value) => Ok(Some(value)),
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "Ignoring malformed ACP session file");
            Ok(None)
        }
    }
}

/// JavaScript `encodeURIComponent`: everything but `A-Z a-z 0-9 - _ . ! ~ * ' ( )` is %XX-escaped
/// per UTF-8 byte, matching acpx's file names.
fn encode_uri_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
