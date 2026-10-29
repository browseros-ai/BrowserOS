//! A short-lived, local record of the scripts a session ran, so the agent can turn the session
//! into an ultrafast skill. The audit log stores `run` source redacted, so without this the code
//! that actually worked would exist only in the agent's own transcript.
//!
//! One JSON line per `run` call at `<browserclaw_dir>/journal/<session>.jsonl`. Written only while
//! ultrafast skills are enabled, never for a script that may hold secrets, deleted once the
//! session's skill is saved for good, and swept after a day either way.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

const JOURNAL_DIR: &str = "journal";
pub const RETENTION: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_VALUE_BYTES: usize = 8 * 1024;
const MAX_LOG_BYTES: usize = 2 * 1024;
const MAX_CODE_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEntry {
    pub dispatch_id: String,
    pub at: i64,
    pub duration_ms: i64,
    pub ok: bool,
    pub code: String,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub logs: Option<String>,
}

impl JournalEntry {
    /// Builds an entry from a finished `run` call, trimming what it keeps.
    #[must_use]
    pub fn from_run(
        dispatch_id: String,
        at: i64,
        duration_ms: i64,
        code: &str,
        structured: &Value,
        is_error: bool,
    ) -> Self {
        let value = structured
            .get("value")
            .filter(|value| !value.is_null())
            .map(|value| trim(&value.to_string(), MAX_VALUE_BYTES));
        let logs = structured
            .get("logs")
            .and_then(Value::as_array)
            .filter(|logs| !logs.is_empty())
            .map(|logs| {
                let joined = logs
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n");
                trim(&joined, MAX_LOG_BYTES)
            });
        let error = structured
            .get("error")
            .and_then(Value::as_str)
            .map(|error| trim(error, MAX_LOG_BYTES));
        Self {
            dispatch_id,
            at,
            duration_ms,
            ok: !is_error,
            code: trim(code, MAX_CODE_BYTES),
            value,
            error,
            logs,
        }
    }
}

fn trim(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

#[must_use]
pub fn journal_dir(browserclaw_dir: &Path) -> PathBuf {
    browserclaw_dir.join(JOURNAL_DIR)
}

/// Session ids come from the server, but the file name must never escape the journal dir.
fn session_file(browserclaw_dir: &Path, session: &str) -> Option<PathBuf> {
    let safe = !session.is_empty()
        && session
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !session.starts_with('.');
    safe.then(|| journal_dir(browserclaw_dir).join(format!("{session}.jsonl")))
}

pub fn append(browserclaw_dir: &Path, session: &str, entry: &JournalEntry) -> io::Result<()> {
    let Some(path) = session_file(browserclaw_dir, session) else {
        return Ok(());
    };
    fs::create_dir_all(journal_dir(browserclaw_dir))?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let line = serde_json::to_string(entry).map_err(io::Error::other)?;
    writeln!(file, "{line}")
}

/// The session's entries in run order. Unreadable lines are skipped.
#[must_use]
pub fn read(browserclaw_dir: &Path, session: &str) -> Vec<JournalEntry> {
    let Some(path) = session_file(browserclaw_dir, session) else {
        return Vec::new();
    };
    fs::read_to_string(path)
        .map(|content| {
            content
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect()
        })
        .unwrap_or_default()
}

pub fn remove(browserclaw_dir: &Path, session: &str) {
    if let Some(path) = session_file(browserclaw_dir, session) {
        let _ = fs::remove_file(path);
    }
}

/// Deletes journals not written to within `max_age`. Returns how many were removed.
pub fn sweep(browserclaw_dir: &Path, max_age: Duration) -> usize {
    let Ok(entries) = fs::read_dir(journal_dir(browserclaw_dir)) else {
        return 0;
    };
    let now = SystemTime::now();
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .metadata()
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age > max_age)
        })
        .filter(|entry| fs::remove_file(entry.path()).is_ok())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn entries_round_trip_and_are_trimmed() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let big = "x".repeat(MAX_VALUE_BYTES * 2);
        let entry = JournalEntry::from_run(
            "d1".to_string(),
            1,
            200,
            "return 1",
            &json!({ "ok": true, "value": big, "logs": ["a", "b"] }),
            false,
        );
        append(dir.path(), "sess-1", &entry)?;
        append(dir.path(), "sess-1", &entry)?;
        let read_back = read(dir.path(), "sess-1");
        assert_eq!(read_back.len(), 2);
        assert!(
            read_back[0]
                .value
                .as_ref()
                .is_some_and(|v| v.len() < MAX_VALUE_BYTES + 8)
        );
        assert_eq!(read_back[0].logs.as_deref(), Some("a\nb"));
        remove(dir.path(), "sess-1");
        assert!(read(dir.path(), "sess-1").is_empty());
        Ok(())
    }

    #[test]
    fn a_session_id_cannot_escape_the_journal_dir() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let entry = JournalEntry::from_run("d".into(), 0, 0, "x", &json!({}), false);
        append(dir.path(), "../evil", &entry)?;
        assert!(!dir.path().join("evil.jsonl").exists());
        assert!(read(dir.path(), "../evil").is_empty());
        Ok(())
    }

    #[test]
    fn sweep_keeps_fresh_journals() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let entry = JournalEntry::from_run("d".into(), 0, 0, "x", &json!({}), false);
        append(dir.path(), "fresh", &entry)?;
        assert_eq!(sweep(dir.path(), RETENTION), 0);
        assert_eq!(sweep(dir.path(), Duration::ZERO), 1);
        Ok(())
    }
}
