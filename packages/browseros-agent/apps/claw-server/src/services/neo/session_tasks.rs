//! Session task identity is host state, separate from the SDK's tab ownership.
//! Each session starts with one deterministic host task and follows explicit
//! script task picks reported by the SDK, including picks from failed runs.

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
