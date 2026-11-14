//! Session task identity is host state, separate from the SDK's tab ownership.
//! Each session starts with one deterministic host task and follows explicit
//! tasks reported by the SDK, including touches from failed runs. Metadata lists
//! each task once in first-touch order, so current follows the last listed open
//! task and cannot infer a script's final selection when it revisits an earlier task.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Deserializer, Serialize, de};
use sha2::{Digest, Sha256};

use super::{TaskTouch, TaskTouchKind};
use crate::ids::ConvoId;

/// An SDK task id, constrained at the wire boundary to its 1..=128-byte contract.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct TaskKey(String);

impl TaskKey {
    #[must_use]
    pub fn for_session(convo: &ConvoId) -> Self {
        let id = format!("claw-{convo}");
        if id.len() <= 128 {
            Self(id)
        } else {
            // Legacy conversation ids can contain an unbounded client name.
            // Hash only those ids so truncation cannot merge two sessions.
            Self(format!(
                "claw-{:x}",
                Sha256::digest(convo.as_str().as_bytes())
            ))
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for TaskKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        if value.is_empty() || value.len() > 128 {
            return Err(de::Error::custom("an SDK task id must be 1..=128 bytes"));
        }
        Ok(Self(value))
    }
}

#[derive(Default)]
pub(super) struct SessionTasks {
    sessions: HashMap<ConvoId, Session>,
}

struct Session {
    current: TaskKey,
    // Keep every open task the session touched, not only current. A later census
    // may find tabs under any of them, including the implicit host task.
    tasks: HashSet<TaskKey>,
}

impl SessionTasks {
    fn session(&mut self, convo: &ConvoId) -> &mut Session {
        self.sessions.entry(convo.clone()).or_insert_with(|| {
            let current = TaskKey::for_session(convo);
            Session {
                tasks: HashSet::from([current.clone()]),
                current,
            }
        })
    }

    pub(super) fn current(&mut self, convo: &ConvoId) -> TaskKey {
        self.session(convo).current.clone()
    }

    pub(super) fn observe(&mut self, convo: &ConvoId, touches: &[TaskTouch]) {
        let session = self.session(convo);
        for touch in touches {
            match touch.kind {
                TaskTouchKind::Created | TaskTouchKind::Fetched => {
                    session.tasks.insert(touch.id.clone());
                    session.current = touch.id.clone();
                }
                TaskTouchKind::Closed => {
                    session.tasks.remove(&touch.id);
                    if session.current == touch.id {
                        session.current = TaskKey::for_session(convo);
                        session.tasks.insert(session.current.clone());
                    }
                }
            }
        }
    }

    pub(super) fn tasks(&mut self, convo: &ConvoId) -> Vec<TaskKey> {
        self.session(convo).tasks.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(id: &str, kind: TaskTouchKind) -> TaskTouch {
        TaskTouch {
            id: TaskKey::for_session(&ConvoId::new(id)),
            name: None,
            kind,
            tabs: Vec::new(),
        }
    }

    #[test]
    fn implicit_host_task_is_kept_without_metadata_touches() {
        let convo = ConvoId::new("session");
        let mut sessions = SessionTasks::default();
        sessions.observe(&convo, &[]);
        assert_eq!(sessions.current(&convo), TaskKey::for_session(&convo));
        assert_eq!(sessions.tasks(&convo), vec![TaskKey::for_session(&convo)]);
    }

    #[test]
    fn all_open_tasks_remain_eligible_for_claims_independently_of_current() {
        let convo = ConvoId::new("session");
        let mut sessions = SessionTasks::default();
        let a = touch("a", TaskTouchKind::Created);
        let b = touch("b", TaskTouchKind::Fetched);
        sessions.observe(&convo, &[a.clone(), b.clone()]);
        assert_eq!(sessions.current(&convo), b.id.clone());
        assert_eq!(
            sessions.tasks(&convo).into_iter().collect::<HashSet<_>>(),
            HashSet::from([TaskKey::for_session(&convo), a.id, b.id]),
        );
    }

    #[test]
    fn closing_current_removes_it_and_restores_the_host_task() {
        let convo = ConvoId::new("session");
        let mut sessions = SessionTasks::default();
        sessions.observe(&convo, &[touch("a", TaskTouchKind::Created)]);
        sessions.observe(&convo, &[touch("a", TaskTouchKind::Closed)]);
        assert_eq!(sessions.current(&convo), TaskKey::for_session(&convo));
        assert_eq!(sessions.tasks(&convo), vec![TaskKey::for_session(&convo)]);
    }

    #[test]
    fn last_listed_open_task_wins_without_guessing_a_revisited_task() {
        let convo = ConvoId::new("session");
        let mut sessions = SessionTasks::default();
        // A script selecting A, B, A reports only A, B. Only another run that
        // explicitly fetches A supplies evidence for changing current back to A.
        sessions.observe(
            &convo,
            &[
                touch("a", TaskTouchKind::Fetched),
                touch("b", TaskTouchKind::Fetched),
            ],
        );
        assert_eq!(
            sessions.current(&convo),
            touch("b", TaskTouchKind::Fetched).id
        );
        sessions.observe(&convo, &[touch("a", TaskTouchKind::Fetched)]);
        assert_eq!(
            sessions.current(&convo),
            touch("a", TaskTouchKind::Fetched).id
        );
    }
}
