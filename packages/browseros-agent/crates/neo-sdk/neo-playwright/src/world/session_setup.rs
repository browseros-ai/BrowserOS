//! What each CDP session needs once before the first world on it is sealed.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use browseros_core::{CdpConnection, ProtocolSession, SessionId};
use serde_json::{Value, json};
use tokio::sync::OnceCell;

use crate::probe::ProbeFailure;
use crate::world::ContextWatch;

/// How a session reached the crate, which decides what its setup must enable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionKind {
    /// The page's own session. Core enabled `Runtime` and `Page` on it when it attached the tab.
    Page,
    /// An out-of-process iframe's session. Core enables only `DOM` and `Accessibility` there,
    /// and `Page` too on an agent's own tab, for its file chooser interception.
    Oopif,
}

/// Enables `Runtime` once per OOPIF session so world installs can observe context creation.
/// Core already enables it on page sessions. Concurrent installs share one setup attempt;
/// a new connection epoch invalidates the session ids and starts a fresh table.
pub(crate) struct SessionSetup {
    connection: Arc<dyn CdpConnection>,
    inner: Mutex<SetupInner>,
}

// By hand: the connection is a trait object, and the table is not worth printing.
impl fmt::Debug for SessionSetup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionSetup").finish_non_exhaustive()
    }
}

struct SetupInner {
    /// The connection epoch the entries belong to.
    epoch: u64,
    sessions: HashMap<SessionId, Arc<OnceCell<()>>>,
}

impl SessionSetup {
    /// The table for sessions on `connection`.
    pub(crate) fn new(connection: Arc<dyn CdpConnection>) -> Self {
        let epoch = connection.connection_epoch();
        Self {
            connection,
            inner: Mutex::new(SetupInner {
                epoch,
                sessions: HashMap::new(),
            }),
        }
    }

    /// Enables context events for sealed worlds, the first time only. CDP keeps this setting
    /// for the session's lifetime; a failed or cancelled attempt leaves the cell empty to retry.
    pub(crate) async fn ensure(
        &self,
        session: &ProtocolSession,
        kind: SessionKind,
    ) -> Result<(), ProbeFailure> {
        let Some(id) = session.session_id() else {
            return Err(ProbeFailure::Broken(
                "a sealed world needs a target session".to_owned(),
            ));
        };
        let done = {
            let mut inner = self.lock();
            Arc::clone(inner.sessions.entry(id.clone()).or_default())
        };
        done.get_or_try_init(|| async {
            if kind == SessionKind::Oopif {
                session
                    .send::<_, Value>("Runtime.enable", json!({}))
                    .await
                    .map_err(ProbeFailure::from_core)?;
            }
            Ok(())
        })
        .await?;
        Ok(())
    }

    /// A watch over the connection's `Runtime` context events, for an install to take right
    /// before it creates its world (`SealedWorld::install`).
    pub(crate) fn watch_contexts(&self) -> ContextWatch {
        ContextWatch::new(self.connection.events())
    }

    /// The table, emptied first if the connection moved to a new epoch since it was filled.
    fn lock(&self) -> MutexGuard<'_, SetupInner> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let epoch = self.connection.connection_epoch();
        if inner.epoch != epoch {
            inner.epoch = epoch;
            inner.sessions.clear();
        }
        inner
    }
}
