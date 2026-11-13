//! Lifetime ownership for remote CDP objects. Operations keep temporary query objects; runs
//! keep chooser inputs alive after event delivery, so detached nodes survive browser GC.
use browseros_core::ProtocolSession;
use serde_json::{Value, json};
use std::mem;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use tokio::runtime::Handle;

/// Strong CDP references owned by one operation or one run. Dropping the owner releases
/// every session's group, including when cancellation drops its future.
#[derive(Debug)]
pub(crate) struct ObjectGroup {
    /// `neo-playwright-op-<n>`, unique in the process.
    name: String,
    /// The sessions that hold an object in the group. A std mutex, never held across an await.
    sessions: Mutex<Vec<ProtocolSession>>,
}

/// Numbers the ops' object groups.
static NEXT_GROUP: AtomicU64 = AtomicU64::new(1);

impl ObjectGroup {
    pub(crate) fn new() -> Self {
        Self {
            name: format!(
                "neo-playwright-op-{}",
                NEXT_GROUP.fetch_add(1, Ordering::Relaxed)
            ),
            sessions: Mutex::default(),
        }
    }
}

/// Releases the group with the op, cancelled or not. The release is spawned, so it runs after
/// a dropped future too; it is idempotent, and a session that is gone has nothing left to free.
impl Drop for ObjectGroup {
    fn drop(&mut self) {
        let sessions =
            mem::take(&mut *self.sessions.lock().unwrap_or_else(PoisonError::into_inner));
        if sessions.is_empty() {
            return;
        }
        let Ok(runtime) = Handle::try_current() else {
            return;
        };
        let name = mem::take(&mut self.name);
        drop(runtime.spawn(async move {
            for session in sessions {
                let _released = session
                    .send::<_, Value>("Runtime.releaseObjectGroup", json!({ "objectGroup": name }))
                    .await;
            }
        }));
    }
}

impl Default for ObjectGroup {
    fn default() -> Self {
        Self::new()
    }
}

impl ObjectGroup {
    /// Releases every object in the group now, on every session that holds one, and keeps the
    /// group: an object asked for later under the same name joins it again, and the owner's
    /// drop releases that one. For an op that tries again and again with nothing shared between
    /// tries (`locator.waitForFunction`), so a long wait does not keep every try's document and
    /// element, and the subtrees the page replaced under them, alive until it ends.
    pub(crate) async fn release_now(&self) {
        // Cloned out: the std mutex is never held across the sends.
        let sessions = self
            .sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for session in sessions {
            let _released = session
                .send::<_, Value>(
                    "Runtime.releaseObjectGroup",
                    json!({ "objectGroup": self.name }),
                )
                .await;
        }
    }

    pub(crate) fn on(&self, session: &ProtocolSession) -> &str {
        let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        if !sessions.iter().any(|known| known.same_session(session)) {
            // Runs may outlive a socket reconnect. Cleanup must retain this object's
            // original session/epoch, rather than send to a replacement connection.
            sessions.push(session.cancellation_endpoint());
        }
        &self.name
    }
}
