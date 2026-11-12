//! Which `Runtime` reports (console messages, uncaught exceptions) the event log already holds,
//! per session, so a lease's replay of one is not recorded a second time.
//!
//! A `Runtime` lease that switches reporting on makes the browser replay every message and
//! exception it stored, each with its original timestamp, and an exception with its original
//! `exceptionId` (measured). The watcher drops a replay stamped before the lease's floor, but
//! the floor is the script's `Date.now()`, in whole milliseconds, and the browser stamps with
//! microseconds: a wait made in the millisecond in which the error the previous wait returned
//! was thrown has a floor at or below that error's stamp (measured: 36 of 40 exceptions reached
//! Neo while `Date.now()` still read below their stamp). The replay then passed as news, and
//! the wait took the previous round's error (ER-006). Identity does not depend on two clocks:
//! within the floor's own millisecond, a report this session's log already recorded is a replay
//! (`Watcher::replayed`). Outside it the stamp decides, so a page that logs the same text twice
//! loses neither.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};

use browseros_core::SessionId;
use serde_json::Value;

/// How many recorded reports a session remembers. Chromium keeps at most 1,000 console
/// messages (exceptions among them) per page for a replay, so an older report cannot come back.
const REMEMBERED: usize = 1_024;

/// The reports recorded on each session the watcher reads `Runtime` from.
#[derive(Debug, Default)]
pub(crate) struct RecordedReports {
    sessions: HashMap<SessionId, Remembered>,
}

/// One session's recorded reports, oldest first.
#[derive(Debug, Default)]
struct Remembered {
    order: VecDeque<ReportKey>,
    keys: HashSet<ReportKey>,
}

/// What identifies one report across its live delivery and its replays: an exception's id with
/// its stamp (ids start over in a new renderer process, the stamp does not repeat), a console
/// message's stamp, type and arguments (no id; two calls are told apart by the microsecond).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ReportKey {
    Exception { id: i64, stamp: u64 },
    Console { digest: u64 },
}

impl ReportKey {
    /// The key of a `Runtime.exceptionThrown` or `Runtime.consoleAPICalled` event's params;
    /// `None` for one that carries no timestamp, which nothing could tell apart anyway.
    pub(crate) fn of(method: &str, params: &Value) -> Option<Self> {
        let stamp = params.get("timestamp").and_then(Value::as_f64)?.to_bits();
        if method == "Runtime.exceptionThrown" {
            let id = params
                .pointer("/exceptionDetails/exceptionId")
                .and_then(Value::as_i64)?;
            return Some(Self::Exception { id, stamp });
        }
        let mut hasher = DefaultHasher::new();
        stamp.hash(&mut hasher);
        params.get("type").map(Value::to_string).hash(&mut hasher);
        params.get("args").map(Value::to_string).hash(&mut hasher);
        Some(Self::Console {
            digest: hasher.finish(),
        })
    }
}

impl RecordedReports {
    /// Whether `session`'s log already recorded the report `key` names.
    pub(crate) fn contains(&self, session: &SessionId, key: ReportKey) -> bool {
        self.sessions
            .get(session)
            .is_some_and(|remembered| remembered.keys.contains(&key))
    }

    /// Remembers that the log recorded `key` from `session`, forgetting the oldest past
    /// [`REMEMBERED`].
    pub(crate) fn insert(&mut self, session: &SessionId, key: ReportKey) {
        let remembered = self.sessions.entry(session.clone()).or_default();
        if !remembered.keys.insert(key) {
            return;
        }
        remembered.order.push_back(key);
        if remembered.order.len() > REMEMBERED
            && let Some(oldest) = remembered.order.pop_front()
        {
            remembered.keys.remove(&oldest);
        }
    }

    /// The session went: nothing replays on it any more.
    pub(crate) fn forget(&mut self, session: &SessionId) {
        self.sessions.remove(session);
    }

    /// The connection changed: every session id died with the old socket.
    pub(crate) fn clear(&mut self) {
        self.sessions.clear();
    }
}

#[cfg(test)]
mod tests {
    use browseros_core::SessionId;
    use serde_json::json;

    use super::{REMEMBERED, RecordedReports, ReportKey};

    /// An exception is its id and exact stamp; a console message its stamp, type and
    /// arguments. A session forgets its oldest past the cap, and a forgotten session nothing.
    #[test]
    fn tests_that_a_report_is_known_by_identity_and_the_memory_is_bounded() {
        let thrown = |id: i64, stamp: f64| {
            ReportKey::of(
                "Runtime.exceptionThrown",
                &json!({ "timestamp": stamp, "exceptionDetails": { "exceptionId": id } }),
            )
        };
        let logged = |stamp: f64, text: &str| {
            ReportKey::of(
                "Runtime.consoleAPICalled",
                &json!({ "timestamp": stamp, "type": "log", "args": [{ "type": "string", "value": text }] }),
            )
        };
        assert_ne!(thrown(3, 1_000.6), thrown(3, 1_000.7), "an id reused later");
        assert_ne!(logged(1_000.6, "a"), logged(1_000.6, "b"));
        assert_eq!(logged(1_000.6, "a"), logged(1_000.6, "a"));
        assert_eq!(
            ReportKey::of("Runtime.consoleAPICalled", &json!({ "type": "log" })),
            None
        );

        let mut reports = RecordedReports::default();
        let page = SessionId::from("page".to_owned());
        let Some(first) = thrown(1, 1.0) else {
            panic!("an exception with an id and a stamp has a key");
        };
        reports.insert(&page, first);
        assert!(reports.contains(&page, first));
        assert!(!reports.contains(&SessionId::from("other".to_owned()), first));
        for id in 2..=i64::try_from(REMEMBERED).unwrap_or(i64::MAX) + 1 {
            if let Some(key) = thrown(id, 1.0) {
                reports.insert(&page, key);
            }
        }
        assert!(!reports.contains(&page, first), "the oldest is forgotten");
        reports.forget(&page);
        assert!(thrown(2, 1.0).is_some_and(|key| !reports.contains(&page, key)));
    }
}
